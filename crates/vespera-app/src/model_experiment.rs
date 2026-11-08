use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::json;
use vespera_agents::{ExecutionError, ExecutionOptions};
use vespera_app::runtime::{RuntimeError, VesperaRuntime};
use vespera_config::AppConfig;
use vespera_core::{
    AgentId, AgentStatus, EventEnvelope, EventKind, ModelDescriptor, ModelId, ModelSelectionPolicy,
    ProviderKind, Task, TaskId, TaskState,
};
use vespera_events::{Cursor, Journal};
use vespera_providers::{
    CancellationToken, EndpointPolicy, InferenceMetrics, InferenceProvider, ValidatedEndpoint,
};

use crate::{AppError, catalog_commands, commands::cancellable, event_commands::terminal_event};

#[derive(Debug, clap::Args)]
pub(super) struct Args {
    #[arg(long, default_value = "config/local.toml")]
    config: PathBuf,
    #[arg(long)]
    model_catalog: Option<PathBuf>,
    #[arg(long)]
    agent: AgentId,
    #[arg(long)]
    baseline_model: ModelId,
    #[arg(long)]
    candidate_model: ModelId,
    /// Private JSON array of cases with prompt and expected text.
    #[arg(long)]
    workload: PathBuf,
    /// New directory for the redacted outcome journal.
    #[arg(long)]
    evidence_dir: PathBuf,
    #[arg(long, default_value_t = 512, value_parser = clap::value_parser!(u32).range(64..=32768))]
    context_tokens: u32,
    #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u32).range(1..=1024))]
    max_output_tokens: u32,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=120))]
    timeout_seconds: u64,
    /// Activate a suspended or failed agent only in each trial's runtime.
    #[arg(long)]
    resume: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    prompt: String,
    expected: String,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Match,
    Mismatch,
    Failed,
    Deadline,
    Interrupted,
}

#[derive(Serialize)]
struct Trial {
    case: usize,
    arm: &'static str,
    task_id: TaskId,
    outcome: Outcome,
    end_to_end_ms: u128,
    inference: Option<InferenceMetrics>,
    provider: ProviderEvidence,
    cleanup_confirmed: bool,
    event_cursor: Option<Cursor>,
}

#[derive(Default, Serialize)]
struct ProviderEvidence {
    version: Option<String>,
    model_digest: Option<String>,
    resident_ram_bytes: Option<u64>,
    resident_vram_bytes: Option<u64>,
}

#[derive(Default, Serialize)]
struct Summary {
    attempted: usize,
    matches: usize,
    mismatches: usize,
    failed: usize,
    deadlines: usize,
    interrupted: usize,
}

pub(super) async fn run(args: Args) -> Result<(), AppError> {
    let workload = load_cases(&args)?;
    let config = catalog_commands::load(&args.config, args.model_catalog.as_deref())?;
    let models = select_models(&config, &args)?;
    let configs = [models[0].id, models[1].id].map(|model_id| {
        let mut config = config.clone();
        let agent = config
            .agents
            .iter_mut()
            .find(|agent| agent.identity.id == args.agent)
            .expect("validated agent");
        agent.model_policy = ModelSelectionPolicy::Fixed { model_id };
        config
    });
    std::fs::create_dir(&args.evidence_dir).map_err(AppError::ExperimentDirectory)?;
    let mut journal = Journal::initialize(&args.evidence_dir.join("events.sqlite3"), 32)?;
    let mut trials = Vec::with_capacity(workload.len() * 2);
    let mut summaries = [Summary::default(), Summary::default()];
    let mut stopped = false;
    for (index, case) in workload.iter().enumerate() {
        for arm in [index % 2, 1 - index % 2] {
            let runner = TrialRunner {
                args: &args,
                config: &configs[arm],
                model: models[arm],
                arm: ["baseline", "candidate"][arm],
            };
            let trial = runner.run(index + 1, case, &mut journal).await?;
            let summary = &mut summaries[arm];
            summary.attempted += 1;
            match trial.outcome {
                Outcome::Match => summary.matches += 1,
                Outcome::Mismatch => summary.mismatches += 1,
                Outcome::Failed => summary.failed += 1,
                Outcome::Deadline => summary.deadlines += 1,
                Outcome::Interrupted => summary.interrupted += 1,
            }
            stopped = !trial.cleanup_confirmed
                || matches!(trial.outcome, Outcome::Interrupted | Outcome::Deadline);
            trials.push(trial);
            if stopped {
                break;
            }
        }
        if stopped {
            break;
        }
    }
    let arm_config = |model: &ModelDescriptor| {
        json!({
            "model_id": model.id,
            "provider_id": model.provider_id,
            "provider_kind": ProviderKind::Ollama,
            "estimated_resources": model.resources,
            "declared_context_tokens": model.context_window_tokens,
        })
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "experiment": "live_model_comparison",
            "vespera_version": env!("CARGO_PKG_VERSION"),
            "nondeterministic": true,
            "configuration": {
                "agent_id": args.agent,
                "baseline": arm_config(models[0]),
                "candidate": arm_config(models[1]),
                "context_tokens": args.context_tokens,
                "max_output_tokens": args.max_output_tokens,
                "timeout_seconds": args.timeout_seconds,
                "temperature": 0,
                "budget": config.resources,
                "inference_overhead": config.inference_overhead,
                "local_host_limits": config.scheduling.local_host_limits,
                "planned_per_arm": workload.len(),
                "seed": null,
                "scoring": "trimmed_exact_match",
            },
            "summary": { "baseline": summaries[0], "candidate": summaries[1] },
            "stopped_early": stopped && trials.len() < workload.len() * 2,
            "journal": "events.sqlite3",
            "trials": trials,
        }))?
    );
    if stopped {
        Err(AppError::ModelExperimentStopped)
    } else {
        Ok(())
    }
}

fn load_cases(args: &Args) -> Result<Vec<Case>, AppError> {
    const LIMIT: u64 = 128 * 1024;
    let mut bytes = Vec::new();
    File::open(&args.workload)
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|_| AppError::ModelExperimentInput("could not read workload"))?;
    if bytes.len() as u64 > LIMIT {
        return Err(AppError::ModelExperimentInput("workload exceeds 128 KiB"));
    }
    let cases: Vec<Case> = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::ModelExperimentInput("workload must be strict case JSON"))?;
    if !(1..=16).contains(&cases.len())
        || cases.iter().any(|case| {
            case.prompt.trim().is_empty()
                || case.prompt.len() > 16_384
                || case.expected.trim().is_empty()
                || case.expected.len() > 4096
        })
    {
        return Err(AppError::ModelExperimentInput(
            "expected 1–16 cases with nonblank prompts up to 16 KiB and expected text up to 4 KiB",
        ));
    }
    Ok(cases)
}

fn select_models<'a>(
    config: &'a AppConfig,
    args: &Args,
) -> Result<[&'a ModelDescriptor; 2], AppError> {
    let agent = config
        .agents
        .iter()
        .find(|agent| agent.identity.id == args.agent)
        .ok_or(AppError::ModelExperimentInput("agent is not configured"))?;
    if agent.status != AgentStatus::Idle && !args.resume {
        return Err(AppError::ModelExperimentInput(
            "agent is not idle; use --resume to activate it per trial",
        ));
    }
    let find = |id| {
        config
            .models
            .iter()
            .find(|model| model.id == id)
            .ok_or(AppError::ModelExperimentInput("model is not registered"))
    };
    let models = [find(args.baseline_model)?, find(args.candidate_model)?];
    if models[0].id == models[1].id {
        return Err(AppError::ModelExperimentInput(
            "choose distinct model registrations",
        ));
    }
    for model in models {
        let provider = config
            .providers
            .iter()
            .find(|provider| provider.id == model.provider_id)
            .ok_or(AppError::ModelExperimentInput("model provider is missing"))?;
        if provider.kind != ProviderKind::Ollama
            || !ValidatedEndpoint::parse(provider.id, &provider.endpoint, EndpointPolicy::default())
                .is_ok_and(|endpoint| endpoint.is_loopback())
        {
            return Err(AppError::ModelExperimentInput(
                "models must use native loopback Ollama providers",
            ));
        }
        if model.availability == vespera_core::ModelAvailability::Unavailable {
            return Err(AppError::ModelExperimentInput(
                "selected model is marked unavailable",
            ));
        }
    }
    Ok(models)
}

struct TrialRunner<'a> {
    args: &'a Args,
    config: &'a AppConfig,
    model: &'a ModelDescriptor,
    arm: &'static str,
}

impl TrialRunner<'_> {
    async fn run(
        &self,
        index: usize,
        case: &Case,
        journal: &mut Journal,
    ) -> Result<Trial, AppError> {
        let runtime = VesperaRuntime::from_config_for_agent(self.config, self.args.agent).await?;
        let provider = runtime
            .registry()
            .get_provider(self.model.provider_id)
            .await
            .map_err(RuntimeError::from)?;
        let task_id = TaskId::new();
        let task = Task {
            id: task_id,
            prompt: case.prompt.clone(),
            required_capabilities: Default::default(),
            state: TaskState::Queued,
        };
        let timeout = Duration::from_secs(self.args.timeout_seconds);
        let options = ExecutionOptions {
            max_output_tokens: self.args.max_output_tokens,
            context_tokens: Some(self.args.context_tokens),
            temperature: Some(0.0),
            timeout,
            ..ExecutionOptions::default()
        };
        let cancel = CancellationToken::new();
        let start = Instant::now();
        let result = cancellable(timeout, cancel.clone(), async {
            runtime.reconcile_startup(cancel.clone()).await?;
            runtime
                .run_agent(self.args.agent, task, options, self.args.resume, cancel)
                .await
        })
        .await;
        let end_to_end_ms = start.elapsed().as_millis();
        let (outcome, inference, event) = match result {
            Ok(execution) => {
                let outcome = if execution.response.content.trim() == case.expected.trim() {
                    Outcome::Match
                } else {
                    Outcome::Mismatch
                };
                let event = EventKind::InferenceCompleted {
                    task_id,
                    model_id: execution.model_id,
                    agent_id: Some(execution.agent_id),
                };
                (outcome, Some(execution.response.metrics), Some(event))
            }
            Err(error) => {
                let outcome = match error {
                    RuntimeError::Deadline
                    | RuntimeError::Execution(ExecutionError::DeadlineExceeded) => {
                        Outcome::Deadline
                    }
                    RuntimeError::Cancelled
                    | RuntimeError::Signal
                    | RuntimeError::Execution(ExecutionError::Cancelled) => Outcome::Interrupted,
                    _ => Outcome::Failed,
                };
                let event = runtime
                    .agents()
                    .get_task(task_id)
                    .await
                    .ok()
                    .and_then(|task| terminal_event(task_id, task.state));
                (outcome, None, event)
            }
        };
        let evidence = if matches!(outcome, Outcome::Interrupted | Outcome::Deadline) {
            ProviderEvidence::default()
        } else {
            observe_provider(
                provider.as_ref(),
                &self.model.identifier,
                inference.is_some(),
            )
            .await
        };
        let cleanup = runtime.cleanup(true, Duration::from_secs(5)).await;
        let event_cursor = event
            .map(|kind| {
                let now = SystemTime::UNIX_EPOCH.elapsed().map_err(|_| {
                    AppError::ModelExperimentInput("system clock predates the epoch")
                })?;
                let millis = u64::try_from(now.as_millis())
                    .map_err(|_| AppError::ModelExperimentInput("system clock is out of range"))?;
                let mut event = EventEnvelope::new("experiment-models", millis, kind)
                    .expect("validated task evidence");
                event.correlation_task_id = Some(task_id);
                Ok::<_, AppError>(journal.append(&event)?)
            })
            .transpose()?;
        Ok(Trial {
            case: index,
            arm: self.arm,
            task_id,
            outcome,
            end_to_end_ms,
            inference,
            provider: evidence,
            cleanup_confirmed: cleanup.drained
                && cleanup.unload_errors.is_empty()
                && cleanup.remaining_models.is_empty(),
            event_cursor,
        })
    }
}

async fn observe_provider(
    provider: &dyn InferenceProvider,
    model: &str,
    executed: bool,
) -> ProviderEvidence {
    let deadline = Duration::from_secs(3);
    let key = provider.residency_model_key(model);
    let token = CancellationToken::new();
    let (health, models, residency) = tokio::join!(
        tokio::time::timeout(deadline, provider.health_check(token.clone())),
        tokio::time::timeout(deadline, provider.enumerate_models(token.clone())),
        tokio::time::timeout(deadline, async {
            if executed {
                provider.query_residency(token.clone()).await.ok()
            } else {
                None
            }
        }),
    );
    let version = health
        .ok()
        .and_then(Result::ok)
        .and_then(|report| report.version)
        .filter(|version| {
            (1..=32).contains(&version.len())
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        });
    let model_digest = models
        .ok()
        .and_then(Result::ok)
        .and_then(|models| {
            let mut matches = models
                .into_iter()
                .filter(|entry| provider.residency_model_key(&entry.provider_model_name) == key);
            let found = matches.next()?;
            matches.next().is_none().then_some(found.digest).flatten()
        })
        .filter(|digest| {
            let value = digest.strip_prefix("sha256:").unwrap_or(digest);
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
    let memory = residency
        .ok()
        .flatten()
        .filter(|sample| sample.provider_id == provider.provider_id())
        .and_then(|sample| {
            let mut matches = sample
                .resident_models
                .into_iter()
                .filter(|entry| provider.residency_model_key(&entry.provider_model_name) == key);
            let found = matches.next()?;
            matches.next().is_none().then_some(found)
        });
    ProviderEvidence {
        version,
        model_digest,
        resident_ram_bytes: memory.as_ref().map(|model| model.ram_resident_bytes),
        resident_vram_bytes: memory.map(|model| model.vram_resident_bytes),
    }
}
