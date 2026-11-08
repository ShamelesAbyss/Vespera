use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{ArgAction, Args, Subcommand};
use serde::{Deserialize, Serialize};
use vespera_agents::ExecutionOptions;
use vespera_app::runtime::{CleanupReport, RuntimeError, VesperaRuntime};
use vespera_core::{AgentId, Capability, EventEnvelope, Task, TaskId, TaskState};
use vespera_events::{Journal, JournalError};
use vespera_providers::CancellationToken;
use vespera_world::{
    Decision, DecisionSource, Position, World, WorldChange, WorldError, WorldFence, WorldStore,
};

use crate::{AppError, event_commands};

mod cadence;

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
    /// Reconstruct a retained committed revision without contacting a provider.
    Replay {
        /// Defaults to the latest committed revision.
        #[arg(long)]
        revision: Option<u64>,
    },
    /// Advance deterministic ticks and commit them together.
    Advance {
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=1000))]
        ticks: u32,
        /// Publish the committed tick event to an existing local journal.
        #[arg(long)]
        event_journal: Option<PathBuf>,
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
    /// Advance a bounded world run with sparse model decisions.
    Run(cadence::RunArgs),
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
    #[arg(long)]
    model_catalog: Option<PathBuf>,
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
        WorldCommand::Replay { revision } => {
            let replay = WorldStore::replay(database, revision)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "earliest_revision": replay.earliest_revision,
                    "latest_revision": replay.latest_revision,
                    "basis_revision": replay.basis_revision,
                    "world": replay.world.snapshot(),
                }))?
            );
            return Ok(());
        }
        WorldCommand::Advance {
            ticks,
            event_journal,
        } => {
            let mut journal = event_commands::prepare(event_journal).await?;
            let (world, event) = WorldStore::commit_observed(
                database,
                WorldFence::Current,
                WorldChange::Advance { ticks },
            )?;
            println!("{}", serde_json::to_string_pretty(&world.snapshot())?);
            return publish_advance(journal.as_mut(), event, world.revision());
        }
        WorldCommand::SetActive {
            resident,
            active,
            revision,
        } => WorldStore::commit(
            database,
            WorldFence::Revision(revision),
            WorldChange::SetActive { resident, active },
        )?,
        WorldCommand::Move {
            resident,
            x,
            z,
            revision,
        } => WorldStore::commit(
            database,
            WorldFence::Revision(revision),
            WorldChange::Move {
                resident,
                to: Position { x, z },
            },
        )?,
        WorldCommand::Memory { resident } => {
            let world = WorldStore::read(database)?;
            println!("{}", serde_json::to_string_pretty(world.memory(resident)?)?);
            return Ok(());
        }
        WorldCommand::Decide(args) => return decide(database, args).await,
        WorldCommand::Run(args) => return cadence::run(database, args).await,
    };
    println!("{}", serde_json::to_string_pretty(&world.snapshot())?);
    Ok(())
}

fn publish_advance(
    journal: Option<&mut Journal>,
    event: Option<EventEnvelope>,
    revision: u64,
) -> Result<(), AppError> {
    if let Some(journal) = journal {
        event
            .ok_or(JournalError::Invalid("missing world advance event"))
            .and_then(|event| journal.append(&event))
            .map_err(|source| AppError::WorldEventPublication { revision, source })?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    choice: usize,
}

#[derive(Serialize)]
struct DecisionReport {
    #[serde(flatten)]
    attempt: DecisionAttempt,
    cleanup: CleanupReport,
}

#[derive(Serialize)]
struct DecisionAttempt {
    resident: AgentId,
    observed_revision: u64,
    committed_revision: Option<u64>,
    decision: Option<Decision>,
    source: Option<DecisionSource>,
    error: Option<String>,
}

fn plan(world: &World, id: AgentId, revision: u64) -> Result<(Task, Vec<Decision>), WorldError> {
    let options = world.decision_options(id, revision)?;
    let state = world.snapshot();
    let resident = state
        .residents
        .iter()
        .find(|resident| resident.id == id)
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
    Ok((
        Task {
            id: TaskId::new(),
            prompt: format!(
                "Choose one legal action for this simulated resident. Reply only with {{\"choice\":N}}, using a zero-based integer index into choices_in_index_order. World facts: {facts}"
            ),
            required_capabilities: [Capability::new("text-generation").expect("static capability")]
                .into(),
            state: TaskState::Queued,
        },
        options,
    ))
}

fn execution_options(timeout: Duration) -> ExecutionOptions {
    ExecutionOptions {
        max_output_tokens: 32,
        max_response_bytes: 256,
        temperature: Some(0.0),
        timeout,
        ..Default::default()
    }
}

impl DecisionAttempt {
    fn new(resident: AgentId, observed_revision: u64) -> Self {
        Self {
            resident,
            observed_revision,
            committed_revision: None,
            decision: None,
            source: None,
            error: None,
        }
    }

    fn choose(
        &mut self,
        observed: vespera_agents::TaskExecutionResult,
        options: &[Decision],
    ) -> Result<(Decision, DecisionSource), AppError> {
        let source = DecisionSource {
            task_id: observed.task.id,
            model_id: observed.model_id,
            provider_id: observed.provider_id,
        };
        self.source = Some(source);
        let choice: Choice = serde_json::from_str(&observed.response.content)
            .map_err(|_| AppError::InvalidWorldChoice)?;
        let picked = *options
            .get(choice.choice)
            .ok_or(AppError::InvalidWorldChoice)?;
        self.decision = Some(picked);
        Ok((picked, source))
    }

    fn commit(
        &mut self,
        database: &Path,
        world: &World,
        (decision, source): (Decision, DecisionSource),
    ) -> Result<World, WorldError> {
        let updated = WorldStore::commit(
            database,
            WorldFence::Snapshot(world),
            WorldChange::ModelChoice {
                resident: self.resident,
                decision,
                source,
            },
        )?;
        self.committed_revision = Some(updated.snapshot().revision);
        Ok(updated)
    }
}

async fn decide(database: &Path, args: DecisionArgs) -> Result<(), AppError> {
    let world = WorldStore::read(database)?;
    let (task, options) = plan(&world, args.resident, args.revision)?;
    let config = crate::catalog_commands::load(&args.config, args.model_catalog.as_deref())?;
    let runtime = VesperaRuntime::from_config_for_agent(&config, args.resident).await?;
    runtime
        .agents()
        .authorize_simulation_action(args.resident)
        .await
        .map_err(RuntimeError::from)?;
    let timeout = Duration::from_secs(args.timeout_seconds);
    let cancel = CancellationToken::new();
    let outcome = crate::commands::cancellable(
        timeout,
        cancel.clone(),
        runtime.run_agent(
            args.resident,
            task,
            execution_options(timeout),
            args.resume,
            cancel,
        ),
    )
    .await;
    let mut attempt = DecisionAttempt::new(args.resident, args.revision);
    let result = outcome.map_err(AppError::from).and_then(|observed| {
        let choice = attempt.choose(observed, &options)?;
        if args.apply {
            attempt.commit(database, &world, choice)?;
        }
        Ok(())
    });
    let cleanup = runtime
        .cleanup(args.unload_after, Duration::from_secs(5))
        .await;
    let cleanup_failed = !cleanup.drained || !cleanup.unload_errors.is_empty();
    attempt.error = result.as_ref().err().map(ToString::to_string);
    println!(
        "{}",
        serde_json::to_string_pretty(&DecisionReport { attempt, cleanup })?
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
