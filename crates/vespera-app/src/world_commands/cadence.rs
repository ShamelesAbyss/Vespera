use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Args;
use serde::Serialize;
use vespera_app::runtime::{CleanupReport, RuntimeError, VesperaRuntime};
use vespera_config::AppConfig;
use vespera_core::{AgentId, AgentStatus, Task, TaskId};
use vespera_providers::CancellationToken;
use vespera_world::{
    ReasoningPhase, ReasoningStatus, World, WorldChange, WorldError, WorldFence, WorldStore,
};

use super::{DecisionAttempt, execution_options, plan};
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
    #[serde(skip)]
    pending_task: Option<TaskId>,
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
    let mut result = tokio::select! {
        biased;
        signal = tokio::signal::ctrl_c() => {
            Err(if signal.is_ok() { RuntimeError::Cancelled } else { RuntimeError::Signal }.into())
        }
        result = advance(database, &args, &runtime, world, &mut progress) => result,
    };
    if let Some(task) = progress.pending_task.take() {
        let clear = WorldStore::clear_reasoning(database, task).map_err(AppError::from);
        result = result.and(clear);
    }
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
        world = WorldStore::commit(
            database,
            WorldFence::Snapshot(&world),
            WorldChange::Advance { ticks: 1 },
        )?;
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
        let task_id = task.id;
        progress.pending_task = Some(task_id);
        let mut status = ReasoningStatus {
            resident,
            task_id,
            revision,
            phase: ReasoningPhase::Preparing {},
            observed_unix_ms: crate::memory_commands::current_timestamp()?,
        };
        WorldStore::observe_reasoning(database, &world, status)?;
        let observed = infer(database, &world, runtime, task, timeout, &mut status).await?;
        let choice = observed
            .map_err(AppError::from)
            .and_then(|observed| attempt.choose(observed, &options));
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
        WorldStore::clear_reasoning(database, task_id)?;
        progress.pending_task = None;
    }
    Ok(())
}

async fn infer(
    database: &Path,
    world: &World,
    runtime: &VesperaRuntime,
    task: Task,
    timeout: Duration,
    status: &mut ReasoningStatus,
) -> Result<Result<vespera_agents::TaskExecutionResult, RuntimeError>, AppError> {
    let cancel = CancellationToken::new();
    let work = tokio::time::timeout(
        timeout,
        runtime.run_agent(
            status.resident,
            task,
            execution_options(timeout),
            false,
            cancel.clone(),
        ),
    );
    tokio::pin!(work);
    let cadence = Duration::from_millis(400);
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + cadence, cadence);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            result = &mut work => {
                return Ok(result.unwrap_or_else(|_| {
                    cancel.cancel();
                    Err(RuntimeError::Deadline)
                }));
            }
            _ = heartbeat.tick() => {
                status.phase = runtime.agents().active_inference(status.task_id).map_or(
                    ReasoningPhase::Preparing {},
                    |(model_id, provider_id)| ReasoningPhase::InferenceActive { model_id, provider_id },
                );
                status.observed_unix_ms = crate::memory_commands::current_timestamp()?;
                WorldStore::observe_reasoning(database, world, *status)?;
            }
        }
    }
}
