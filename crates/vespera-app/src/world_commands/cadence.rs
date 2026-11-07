use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Args;
use serde::Serialize;
use vespera_app::runtime::{CleanupReport, RuntimeError, VesperaRuntime};
use vespera_config::AppConfig;
use vespera_core::{AgentId, AgentStatus};
use vespera_providers::CancellationToken;
use vespera_world::{World, WorldError, WorldStore};

use super::{DecisionAttempt, execution_options, plan, update_current};
use crate::AppError;

#[derive(Debug, Args)]
pub(super) struct RunArgs {
    /// Fence the first tick against an inspected revision.
    #[arg(long)]
    revision: u64,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=1000))]
    ticks: u32,
    /// Request one model decision after each group of this many ticks.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=1000))]
    reason_every: u32,
    /// Repeat to rotate model decisions through an explicitly approved roster.
    #[arg(long, required = true)]
    resident: Vec<AgentId>,
    /// Minimum delay before each deterministic tick.
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(u64).range(0..=5000))]
    tick_ms: u64,
    #[arg(long, default_value = "config/local.toml")]
    config: PathBuf,
    /// Per-decision wall-clock budget, including provider health and loading.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=120))]
    timeout_seconds: u64,
    /// Activate configured agents once at startup.
    #[arg(long)]
    resume: bool,
    #[arg(long)]
    unload_after: bool,
}

#[derive(Default, Serialize)]
struct Progress {
    committed_ticks: u32,
    last_committed_revision: u64,
    attempts: Vec<DecisionAttempt>,
}

#[derive(Serialize)]
struct RunReport {
    #[serde(flatten)]
    progress: Progress,
    cleanup: CleanupReport,
    error: Option<String>,
}

pub(super) async fn run(database: &Path, args: RunArgs) -> Result<(), AppError> {
    let planned = args.ticks / args.reason_every;
    if !(1..=32).contains(&planned) {
        return Err(
            WorldError::Invalid("run requires between 1 and 32 decision boundaries").into(),
        );
    }
    let unique: BTreeSet<_> = args.resident.iter().copied().collect();
    if unique.len() != args.resident.len() || unique.len() > 32 {
        return Err(WorldError::Invalid("run requires at most 32 distinct residents").into());
    }
    let world = WorldStore::read(database)?;
    for &resident in &args.resident {
        world.decision_options(resident, args.revision)?;
    }
    let config = AppConfig::load(&args.config)?;
    let runtime = VesperaRuntime::from_config_for_agents(&config, &args.resident).await?;
    for &agent_id in &args.resident {
        let agents = runtime.agents();
        agents
            .authorize_simulation_action(agent_id)
            .await
            .map_err(RuntimeError::from)?;
        let agent = agents
            .get_agent(agent_id)
            .await
            .map_err(RuntimeError::from)?;
        if agent.status != AgentStatus::Idle {
            if !args.resume {
                return Err(RuntimeError::ActivationRequired { agent_id }.into());
            }
            agents
                .set_agent_status(agent_id, AgentStatus::Idle)
                .await
                .map_err(RuntimeError::from)?;
        }
    }
    let mut progress = Progress {
        last_committed_revision: args.revision,
        ..Progress::default()
    };
    let result = tokio::select! {
        biased;
        signal = tokio::signal::ctrl_c() => {
            Err(if signal.is_ok() { RuntimeError::Cancelled } else { RuntimeError::Signal }.into())
        }
        result = advance(database, &args, &runtime, world, &mut progress) => result,
    };
    if let Err(error) = &result
        && let Some(last) = progress.attempts.last_mut()
        && last.committed_revision.is_none()
        && last.error.is_none()
    {
        last.error = Some(error.to_string());
    }
    let cleanup = runtime
        .cleanup(args.unload_after, Duration::from_secs(5))
        .await;
    let cleanup_failed = !cleanup.drained || !cleanup.unload_errors.is_empty();
    println!(
        "{}",
        serde_json::to_string_pretty(&RunReport {
            progress,
            cleanup,
            error: result.as_ref().err().map(ToString::to_string),
        })?
    );
    result?;
    if cleanup_failed {
        return Err(AppError::CleanupFailed);
    }
    Ok(())
}

async fn advance(
    database: &Path,
    args: &RunArgs,
    runtime: &VesperaRuntime,
    mut world: World,
    progress: &mut Progress,
) -> Result<(), AppError> {
    let timeout = Duration::from_secs(args.timeout_seconds);
    for tick in 1..=args.ticks {
        tokio::time::sleep(Duration::from_millis(args.tick_ms)).await;
        world = update_current(database, &world, World::advance)?;
        progress.committed_ticks = tick;
        progress.last_committed_revision = world.snapshot().revision;
        if tick % args.reason_every != 0 {
            continue;
        }
        let resident = args.resident[progress.attempts.len() % args.resident.len()];
        let revision = progress.last_committed_revision;
        progress
            .attempts
            .push(DecisionAttempt::new(resident, revision));
        let attempt = progress.attempts.last_mut().expect("inserted attempt");
        let (task, options) = plan(&world, resident, revision)?;
        let cancel = CancellationToken::new();
        let observed = tokio::time::timeout(
            timeout,
            runtime.run_agent(
                resident,
                task,
                execution_options(timeout),
                false,
                cancel.clone(),
            ),
        )
        .await;
        let choice = match observed {
            Ok(result) => result
                .map_err(AppError::from)
                .and_then(|observed| attempt.choose(observed, &options)),
            Err(_) => {
                cancel.cancel();
                Err(RuntimeError::Deadline.into())
            }
        };
        match choice {
            Ok(choice) => {
                world = attempt.commit(database, &world, choice)?;
                progress.last_committed_revision = world.snapshot().revision;
            }
            Err(error) => {
                attempt.error = Some(error.to_string());
                if WorldStore::read(database)? != world {
                    return Err(WorldError::StaleRevision.into());
                }
            }
        }
    }
    Ok(())
}
