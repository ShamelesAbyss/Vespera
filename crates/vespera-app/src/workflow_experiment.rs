use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vespera_app::runtime::{RuntimeError, VesperaRuntime};
use vespera_config::AppConfig;
use vespera_core::{AgentId, EventEnvelope, ModelId, ProviderId, TaskId, TaskState};
use vespera_events::{Cursor, Journal, JournalError};
use vespera_orchestrator::{ConflictPolicy, Failure, OutputPolicy, Workflow};
use vespera_providers::{CancellationToken, InferenceMetrics};

use crate::{
    AppError, catalog_commands,
    commands::{cancellable, read_workflow_document},
    event_commands::terminal_event,
    memory_commands::current_timestamp,
    model_experiment::{
        Case, Outcome, ProviderEvidence, Summary, load_cases, observe_provider,
        require_local_ollama,
    },
};

#[derive(Debug, clap::Args)]
pub(super) struct Args {
    #[arg(long, default_value = "config/local.toml")]
    config: PathBuf,
    #[arg(long)]
    model_catalog: Option<PathBuf>,
    #[arg(long)]
    baseline_plan: PathBuf,
    #[arg(long)]
    candidate_plan: PathBuf,
    /// Private JSON cases with prompt and expected text.
    #[arg(long)]
    workload: PathBuf,
    /// New directory for the redacted workflow event journal.
    #[arg(long)]
    evidence_dir: PathBuf,
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=120))]
    timeout_seconds: u64,
    #[arg(long)]
    resume: bool,
}

#[derive(Serialize)]
struct Attempt {
    step: usize,
    number: u32,
    task_id: TaskId,
    agent_id: AgentId,
    candidate_model_id: ModelId,
    task_state: Option<TaskState>,
    failure: Option<Failure>,
    executed_model_id: Option<ModelId>,
    executed_provider_id: Option<ProviderId>,
    inference: Option<InferenceMetrics>,
}

#[derive(Serialize)]
struct Trial {
    case: usize,
    arm: &'static str,
    task_id: TaskId,
    outcome: Outcome,
    workflow_state: Option<TaskState>,
    workflow_failure: Option<Failure>,
    end_to_end_ms: u128,
    attempts: Vec<Attempt>,
    post_workflow_model_samples: BTreeMap<ModelId, ProviderEvidence>,
    cleanup_confirmed: bool,
    event_cursor: Option<Cursor>,
}

pub(super) async fn run(args: Args) -> Result<(), AppError> {
    let cases = load_cases(&args.workload, false)?;
    let config = catalog_commands::load(&args.config, args.model_catalog.as_deref())?;
    let plans = [
        load_plan(&args.baseline_plan)?,
        load_plan(&args.candidate_plan)?,
    ];
    for plan in &plans {
        for case in &cases {
            let mut trial = plan.clone();
            trial.task.prompt.clone_from(&case.prompt);
            trial.validate()?;
        }
        let runtime = VesperaRuntime::from_config_for_workflow(&config, plan, args.resume).await?;
        let providers: BTreeSet<_> = runtime
            .registry()
            .list_registrations()
            .await
            .into_iter()
            .map(|model| model.provider_id)
            .collect();
        for provider in providers {
            require_local_ollama(&config, provider)?;
        }
    }
    std::fs::create_dir(&args.evidence_dir).map_err(AppError::ExperimentDirectory)?;
    let mut journal = Journal::initialize(&args.evidence_dir.join("events.sqlite3"), 4096)?;
    let mut trials = Vec::with_capacity(cases.len() * 2);
    let mut summaries = [Summary::default(), Summary::default()];
    let mut stopped = false;
    for (index, case) in cases.iter().enumerate() {
        for arm in [index % 2, 1 - index % 2] {
            let runner = TrialRunner {
                config: &config,
                plan: &plans[arm],
                arm: ["baseline", "candidate"][arm],
                resume: args.resume,
                timeout: Duration::from_secs(args.timeout_seconds),
            };
            let trial = runner.run(index + 1, case, &mut journal).await?;
            summaries[arm].record(trial.outcome);
            stopped = !trial.cleanup_confirmed
                || matches!(trial.outcome, Outcome::Deadline | Outcome::Interrupted);
            trials.push(trial);
            if stopped {
                break;
            }
        }
        if stopped {
            break;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "experiment": "live_workflow_comparison",
            "vespera_version": env!("CARGO_PKG_VERSION"),
            "nondeterministic": true,
            "configuration": {
                "baseline": describe_plan(&plans[0]),
                "candidate": describe_plan(&plans[1]),
                "budget": config.resources,
                "inference_overhead": config.inference_overhead,
                "prefer_observed_latency": config.scheduling.prefer_observed_latency,
                "timeout_seconds": args.timeout_seconds,
                "planned_per_arm": cases.len(),
                "scoring": "trimmed_exact_match",
                "seed": null
            },
            "summary": { "baseline": summaries[0], "candidate": summaries[1] },
            "stopped_early": stopped && trials.len() < cases.len() * 2,
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

fn load_plan(path: &Path) -> Result<Workflow, AppError> {
    let plan: Workflow = serde_json::from_str(&read_workflow_document(path)?)
        .map_err(|_| AppError::InvalidWorkflow)?;
    plan.validate()?;
    Ok(plan)
}

fn step_number(plan: &Workflow, name: &str) -> usize {
    1 + plan
        .steps
        .iter()
        .position(|step| step.name == name)
        .expect("validated step reference")
}

fn describe_plan(plan: &Workflow) -> Value {
    let output = match &plan.output {
        OutputPolicy::Step { step } => json!({ "kind": "step", "step": step_number(plan, step) }),
        OutputPolicy::Agreement { steps, on_conflict } => {
            let conflict = match on_conflict {
                ConflictPolicy::Fail => json!({ "kind": "fail" }),
                ConflictPolicy::Prefer { step } => {
                    json!({ "kind": "prefer", "step": step_number(plan, step) })
                }
            };
            json!({ "kind": "agreement", "steps": steps.iter().map(|step| step_number(plan, step)).collect::<Vec<_>>(), "on_conflict": conflict })
        }
    };
    json!({
        "coordinator": plan.coordinator,
        "max_total_attempts": plan.max_total_attempts,
        "max_total_output_tokens": plan.max_total_output_tokens,
        "timeout_ms": plan.timeout_ms,
        "output": output,
        "steps": plan.steps.iter().map(|step| json!({
            "candidates": step.candidates,
            "dependencies": step.dependencies.iter().map(|dep| step_number(plan, dep)).collect::<Vec<_>>(),
            "max_attempts": step.max_attempts,
            "max_output_tokens": step.max_output_tokens,
            "context_tokens": step.context_tokens,
            "timeout_ms": step.timeout_ms,
            "temperature": step.temperature,
        })).collect::<Vec<_>>()
    })
}

struct TrialRunner<'a> {
    config: &'a AppConfig,
    plan: &'a Workflow,
    arm: &'static str,
    resume: bool,
    timeout: Duration,
}

impl TrialRunner<'_> {
    async fn run(
        &self,
        index: usize,
        case: &Case,
        journal: &mut Journal,
    ) -> Result<Trial, AppError> {
        let mut plan = self.plan.clone();
        plan.task.id = TaskId::new();
        plan.task.prompt.clone_from(&case.prompt);
        let task_id = plan.task.id;
        let runtime =
            VesperaRuntime::from_config_for_workflow(self.config, &plan, self.resume).await?;
        let (sender, mut observed) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let start = Instant::now();
        let result = cancellable(self.timeout, cancel.clone(), async {
            runtime.reconcile_startup(cancel.clone()).await?;
            runtime
                .run_workflow_observed(plan, cancel, Some(&sender))
                .await
        })
        .await;
        let end_to_end_ms = start.elapsed().as_millis();
        let (report, interrupted) = match result {
            Ok(report) => (Some(report), None),
            Err(error) => {
                let outcome = match error {
                    RuntimeError::Cancelled | RuntimeError::Signal => Outcome::Interrupted,
                    RuntimeError::Deadline => Outcome::Deadline,
                    _ => Outcome::Failed,
                };
                (runtime.orchestrator().report(task_id), Some(outcome))
            }
        };
        let workflow_state = report.as_ref().map(|report| report.task.state);
        let workflow_failure = report.as_ref().and_then(|report| report.failure);
        let outcome = interrupted.unwrap_or_else(|| match workflow_failure {
            Some(Failure::Deadline) => Outcome::Deadline,
            Some(Failure::Cancelled) => Outcome::Interrupted,
            _ => match report.as_ref().and_then(|report| report.selected_result()) {
                Some(result) if result.response.content.trim() == case.expected.trim() => {
                    Outcome::Match
                }
                Some(_) => Outcome::Mismatch,
                None => Outcome::Failed,
            },
        });
        let attempts: Vec<_> = report
            .iter()
            .flat_map(|report| &report.attempts)
            .map(|attempt| Attempt {
                step: step_number(self.plan, &attempt.step),
                number: attempt.number,
                task_id: attempt.task_id,
                agent_id: attempt.agent_id,
                candidate_model_id: attempt.candidate_model_id,
                task_state: attempt.task_state,
                failure: attempt.failure,
                executed_model_id: attempt.result.as_ref().map(|result| result.model_id),
                executed_provider_id: attempt.result.as_ref().map(|result| result.provider_id),
                inference: attempt
                    .result
                    .as_ref()
                    .map(|result| result.response.metrics),
            })
            .collect();
        let model_samples = if matches!(outcome, Outcome::Deadline | Outcome::Interrupted) {
            BTreeMap::new()
        } else {
            sample_models(&runtime, self.config, &attempts).await
        };
        let cleanup = runtime.cleanup(true, Duration::from_secs(5)).await;
        let mut kinds = Vec::new();
        while let Ok(kind) = observed.try_recv() {
            kinds.push(kind);
        }
        kinds.extend(workflow_state.and_then(|state| terminal_event(task_id, state)));
        let event_cursor = if kinds.is_empty() {
            None
        } else {
            let timestamp = current_timestamp()?;
            let events = kinds
                .into_iter()
                .map(|kind| {
                    let mut event = EventEnvelope::new("experiment-workflows", timestamp, kind)
                        .map_err(|_| JournalError::Invalid("envelope"))?;
                    event.correlation_task_id = Some(task_id);
                    Ok(event)
                })
                .collect::<Result<Vec<_>, JournalError>>()?;
            Some(journal.append_batch(&events)?)
        };
        Ok(Trial {
            case: index,
            arm: self.arm,
            task_id,
            outcome,
            workflow_state,
            workflow_failure,
            end_to_end_ms,
            attempts,
            post_workflow_model_samples: model_samples,
            cleanup_confirmed: cleanup.drained
                && cleanup.unload_errors.is_empty()
                && cleanup.remaining_models.is_empty(),
            event_cursor,
        })
    }
}

async fn sample_models(
    runtime: &VesperaRuntime,
    config: &AppConfig,
    attempts: &[Attempt],
) -> BTreeMap<ModelId, ProviderEvidence> {
    let ids: BTreeSet<_> = attempts
        .iter()
        .filter_map(|attempt| attempt.executed_model_id)
        .collect();
    let mut sampling = tokio::task::JoinSet::new();
    for model in config.models.iter().filter(|model| ids.contains(&model.id)) {
        if let Ok(provider) = runtime.registry().get_provider(model.provider_id).await {
            let name = model.identifier.clone();
            let id = model.id;
            sampling
                .spawn(async move { (id, observe_provider(provider.as_ref(), &name, true).await) });
        }
    }
    let mut samples = BTreeMap::new();
    while let Some(result) = sampling.join_next().await {
        if let Ok((id, evidence)) = result {
            samples.insert(id, evidence);
        }
    }
    samples
}
