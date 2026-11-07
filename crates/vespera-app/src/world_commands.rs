use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{ArgAction, Args, Subcommand};
use serde::{Deserialize, Serialize};
use vespera_agents::ExecutionOptions;
use vespera_app::runtime::{CleanupReport, RuntimeError, VesperaRuntime};
use vespera_config::AppConfig;
use vespera_core::{AgentId, Capability, Task, TaskId, TaskState};
use vespera_providers::CancellationToken;
use vespera_world::{Decision, DecisionSource, Position, World, WorldError, WorldStore};

use crate::AppError;

#[derive(Debug, Args)]
pub(super) struct WorldArgs {
    /// SQLite world database.
    #[arg(long)]
    database: PathBuf,
    #[command(subcommand)]
    command: WorldCommand,
}

#[derive(Debug, Subcommand)]
enum WorldCommand {
    /// Initialize a world from strict JSON.
    Init {
        #[arg(long)]
        spec: PathBuf,
    },
    /// Read the current public state as JSON.
    Inspect,
    /// Advance deterministic ticks and commit them together.
    Advance {
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=1000))]
        ticks: u32,
    },
    /// Activate or suspend one resident at an observed world revision.
    SetActive {
        #[arg(long)]
        resident: AgentId,
        #[arg(long, action = ArgAction::Set)]
        active: bool,
        #[arg(long)]
        revision: u64,
    },
    /// Move an active resident to an adjacent cell at an observed revision.
    Move {
        #[arg(long)]
        resident: AgentId,
        #[arg(long)]
        x: u16,
        #[arg(long)]
        z: u16,
        #[arg(long)]
        revision: u64,
    },
    /// Read one resident's memories as the local database operator.
    Memory {
        #[arg(long)]
        resident: AgentId,
    },
    /// Ask a resident's configured model to choose from legal world actions.
    Decide(DecisionArgs),
}

#[derive(Debug, Args)]
struct DecisionArgs {
    /// Resident UUID, also the identity of the configured agent.
    #[arg(long)]
    resident: AgentId,
    /// World revision from inspect or the graphical view.
    #[arg(long)]
    revision: u64,
    #[arg(long, default_value = "config/local.toml")]
    config: PathBuf,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=120))]
    timeout_seconds: u64,
    /// Activate the configured agent in this invocation only.
    #[arg(long)]
    resume: bool,
    /// Apply the chosen action if the world still matches. Default previews.
    #[arg(long)]
    apply: bool,
    #[arg(long)]
    unload_after: bool,
}

pub(super) async fn run(args: WorldArgs) -> Result<(), AppError> {
    let database = &args.database;
    let world = match args.command {
        WorldCommand::Init { spec } => {
            let world = load_spec(&spec)?;
            WorldStore::initialize(database, &world)?;
            world
        }
        WorldCommand::Inspect => WorldStore::read(database)?,
        WorldCommand::Advance { ticks } => WorldStore::update(database, |world| {
            for _ in 0..ticks {
                world.advance()?;
            }
            Ok(())
        })?,
        WorldCommand::SetActive {
            resident,
            active,
            revision,
        } => WorldStore::update(database, |world| {
            world.set_active(resident, active, revision)
        })?,
        WorldCommand::Move {
            resident,
            x,
            z,
            revision,
        } => WorldStore::update(database, |world| {
            world.move_resident(resident, Position { x, z }, revision)
        })?,
        WorldCommand::Memory { resident } => {
            let world = WorldStore::read(database)?;
            println!("{}", serde_json::to_string_pretty(world.memory(resident)?)?);
            return Ok(());
        }
        WorldCommand::Decide(args) => return decide(database, args).await,
    };
    println!("{}", serde_json::to_string_pretty(&world.snapshot())?);
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    choice: usize,
}

#[derive(Serialize)]
struct DecisionReport {
    resident: AgentId,
    observed_revision: u64,
    committed_revision: Option<u64>,
    decision: Option<Decision>,
    source: Option<DecisionSource>,
    cleanup: CleanupReport,
    error: Option<String>,
}

async fn decide(database: &Path, args: DecisionArgs) -> Result<(), AppError> {
    let world = WorldStore::read(database)?;
    let options = world.decision_options(args.resident, args.revision)?;
    let state = world.snapshot();
    let resident = state
        .residents
        .iter()
        .find(|resident| resident.id == args.resident)
        .expect("validated resident");
    let nearest_site = state
        .sites
        .iter()
        .filter(|site| site.stock > 0)
        .min_by_key(|site| {
            (
                resident.position.x.abs_diff(site.position.x)
                    + resident.position.z.abs_diff(site.position.z),
                site.position,
            )
        })
        .map(|site| site.position);
    let facts = serde_json::json!({
        "role": resident.role,
        "goal": resident.goal,
        "position": resident.position,
        "carried": resident.carried,
        "depot": state.depot,
        "shared_stock": state.shared_stock,
        "nearest_stocked_site": nearest_site,
        "choices_in_index_order": options,
    });
    let task = Task {
        id: TaskId::new(),
        prompt: format!(
            "Choose one legal action for this simulated resident. Reply only with {{\"choice\":N}}, using a zero-based integer index into choices_in_index_order. World facts: {facts}"
        ),
        required_capabilities: [Capability::new("text-generation").expect("static capability")]
            .into(),
        state: TaskState::Queued,
    };
    let config = AppConfig::load(&args.config)?;
    let runtime = VesperaRuntime::from_config_for_agent(&config, args.resident).await?;
    runtime
        .agents()
        .authorize_simulation_action(args.resident)
        .await
        .map_err(RuntimeError::from)?;
    let timeout = Duration::from_secs(args.timeout_seconds);
    let execution = ExecutionOptions {
        max_output_tokens: 32,
        max_response_bytes: 256,
        temperature: Some(0.0),
        timeout,
        ..Default::default()
    };
    let cancel = CancellationToken::new();
    let outcome = crate::commands::cancellable(
        timeout,
        cancel.clone(),
        runtime.run_agent(args.resident, task, execution, args.resume, cancel),
    )
    .await;
    let mut decision = None;
    let mut source = None;
    let mut committed_revision = None;
    let result = outcome.map_err(AppError::from).and_then(|observed| {
        let provenance = DecisionSource {
            task_id: observed.task.id,
            model_id: observed.model_id,
            provider_id: observed.provider_id,
        };
        source = Some(provenance);
        let choice: Choice = serde_json::from_str(&observed.response.content)
            .map_err(|_| AppError::InvalidWorldChoice)?;
        let picked = *options
            .get(choice.choice)
            .ok_or(AppError::InvalidWorldChoice)?;
        decision = Some(picked);
        if args.apply {
            let updated = WorldStore::update(database, |current| {
                if *current != world {
                    return Err(WorldError::StaleRevision);
                }
                current.apply_model_decision(args.resident, args.revision, picked, provenance)?;
                Ok(())
            })?;
            committed_revision = Some(updated.snapshot().revision);
        }
        Ok(())
    });
    let cleanup = runtime
        .cleanup(args.unload_after, Duration::from_secs(5))
        .await;
    let cleanup_failed = !cleanup.drained || !cleanup.unload_errors.is_empty();
    println!(
        "{}",
        serde_json::to_string_pretty(&DecisionReport {
            resident: args.resident,
            observed_revision: args.revision,
            committed_revision,
            decision,
            source,
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

fn load_spec(path: &Path) -> Result<World, WorldError> {
    const LIMIT: u64 = 64 * 1024;
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|_| WorldError::Invalid("could not read world specification"))?;
    if bytes.len() as u64 > LIMIT {
        return Err(WorldError::Invalid("world specification exceeds 64 KiB"));
    }
    let spec = serde_json::from_slice(&bytes)
        .map_err(|_| WorldError::Invalid("world specification JSON"))?;
    World::new(spec)
}
