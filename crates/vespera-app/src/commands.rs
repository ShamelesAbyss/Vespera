//! CLI operation handlers; service behavior lives in the application runtime.
use crate::scheduler_telemetry::Publisher;
use crate::{AppError, RunAgentArgs, RunWorkflowArgs, event_commands};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::File;
use std::future::Future;
use std::io::{self, Read, Write};
use std::time::Duration;
use vespera_agents::{ExecutionOptions, MemoryContext, TaskExecutionResult};
use vespera_app::memory::configured_authorizer;
use vespera_app::runtime::{
    CleanupReport, ProviderCheck, RuntimeError, VesperaRuntime, check_providers,
    configured_registry, observed_provider_checks,
};
use vespera_config::AppConfig;
use vespera_core::{
    EventKind, MemoryId, MemoryLayer, MemoryRecord, Permission, Task, TaskId, TaskState,
};
use vespera_memory::{MemoryOrigin, MemoryStore, StorageLimits};
use vespera_model_registry::ProviderHealthState;
use vespera_orchestrator::{Workflow, WorkflowReport};
use vespera_providers::CancellationToken;
use vespera_scheduler::StartupResidencyReport;

#[derive(Serialize)]
struct WorkflowRunReport {
    task_id: TaskId,
    workflow: Option<WorkflowReport>,
    error: Option<String>,
    cleanup: CleanupReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    startup_residency: Option<StartupResidencyReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_publication: Option<event_commands::Publication>,
}

pub(crate) async fn run_workflow(args: RunWorkflowArgs) -> Result<(), AppError> {
    let config = crate::catalog_commands::load(&args.config, args.model_catalog.as_deref())?;
    let mut input = String::new();
    File::open(&args.workflow_file)
        .map_err(|_| AppError::WorkflowIo)?
        .take(1_048_577)
        .read_to_string(&mut input)
        .map_err(|_| AppError::WorkflowIo)?;
    if input.len() > 1_048_576 {
        return Err(AppError::InvalidWorkflow);
    }
    let workflow: Workflow = serde_json::from_str(&input).map_err(|_| AppError::InvalidWorkflow)?;
    workflow.validate()?;
    let task_id = workflow.task.id;
    let mut journal = event_commands::prepare(args.event_journal).await?;
    let runtime = VesperaRuntime::from_config_for_workflow(&config, &workflow, args.resume).await?;
    let publisher = Publisher::start(args.scheduler_telemetry, &runtime).await?;
    let live = if args.observe_workflow {
        journal
            .take()
            .map(|journal| event_commands::WorkflowJournal::start(journal, task_id))
    } else {
        None
    };
    let cancel = CancellationToken::new();
    let outcome = cancellable(
        Duration::from_secs(args.timeout_seconds),
        cancel.clone(),
        async {
            if args.reconcile_residency {
                runtime.reconcile_startup(cancel.clone()).await?;
            }
            runtime
                .run_workflow_observed(workflow, cancel, live.as_ref().map(|live| &live.sender))
                .await
        },
    )
    .await;
    let cleanup = runtime
        .cleanup(args.unload_after, Duration::from_secs(5))
        .await;
    let telemetry = Publisher::finish(publisher).await;
    let cleanup_failed = !cleanup.drained || !cleanup.unload_errors.is_empty();
    let (workflow, error) = match outcome {
        Ok(report) => (Some(report), None),
        Err(error) => (runtime.orchestrator().report(task_id), Some(error)),
    };
    let succeeded = workflow
        .as_ref()
        .is_some_and(|report| report.task.state == TaskState::Completed);
    let terminal = workflow
        .as_ref()
        .and_then(|report| event_commands::terminal_event(task_id, report.task.state));
    let event_publication = if let Some(live) = live {
        live.finish(terminal).await
    } else if let (Some(journal), Some(report)) = (journal, &workflow) {
        let mut events = Vec::new();
        if args.record_delegations {
            events.extend(
                report
                    .attempts
                    .iter()
                    .filter(|attempt| attempt.delegated)
                    .map(|attempt| EventKind::TaskDelegated {
                        task_id: attempt.task_id,
                        from_agent_id: report.coordinator,
                        to_agent_id: attempt.agent_id,
                    }),
            );
        }
        events.extend(terminal);
        if events.is_empty() {
            None
        } else {
            Some(event_commands::record_batch(journal, "run-workflow", task_id, events).await)
        }
    } else {
        None
    };
    let journal_failed = event_publication
        .as_ref()
        .is_some_and(event_commands::Publication::failed);
    let startup_residency = runtime.scheduler().startup_residency();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&WorkflowRunReport {
                task_id,
                workflow,
                error: error.as_ref().map(ToString::to_string),
                cleanup,
                startup_residency,
                event_publication
            })?
        );
    } else {
        print_startup_residency(startup_residency);
        if let Some(publication) = &event_publication {
            publication.report_failure();
        }
        if let Some(report) = &workflow {
            if let Some(result) = report.selected_result() {
                println!("{}", result.response.content);
            }
            println!(
                "Workflow {}: {:?}; attempts {}; failure {:?}",
                task_id,
                report.task.state,
                report.attempts.len(),
                report.failure
            );
        }
        println!(
            "Local admissions drained: {}; reservations without confirmed unload: {}",
            cleanup.drained,
            cleanup.remaining_models.len()
        );
        for (model, error) in &cleanup.unload_errors {
            eprintln!("Unload {model}: {error}");
        }
    }
    if let Some(error) = error {
        return Err(error.into());
    }
    if !succeeded {
        return Err(AppError::WorkflowFailed);
    }
    if journal_failed {
        return Err(vespera_events::JournalError::Database.into());
    }
    if cleanup_failed {
        return Err(AppError::CleanupFailed);
    }
    telemetry
}

pub(crate) async fn cancellable<T>(
    timeout: Duration,
    cancel: CancellationToken,
    work: impl Future<Output = Result<T, RuntimeError>>,
) -> Result<T, RuntimeError> {
    tokio::select! {
        biased;
        signal = tokio::signal::ctrl_c() => { cancel.cancel(); if signal.is_err() { Err(RuntimeError::Signal) } else { Err(RuntimeError::Cancelled) } },
        () = tokio::time::sleep(timeout) => { cancel.cancel(); Err(RuntimeError::Deadline) },
        result = work => result,
    }
}

pub(crate) async fn provider_health(
    config: AppConfig,
    timeout: Duration,
    json: bool,
) -> Result<(), AppError> {
    let cancel = CancellationToken::new();
    let checks = cancellable(timeout, cancel.clone(), check_providers(&config, cancel)).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        for check in &checks {
            println!(
                "{}: {:?} (observation age {:?} ms)",
                check.provider_id, check.state, check.checked_age_ms
            );
        }
    }
    if checks
        .iter()
        .any(|check| check.state != ProviderHealthState::Healthy)
    {
        return Err(AppError::UnhealthyProviders);
    }
    Ok(())
}

#[derive(Serialize)]
struct HealthRound {
    round: u64,
    timed_out: bool,
    providers: Vec<ProviderCheck>,
}

pub(crate) async fn watch_provider_health(
    config: AppConfig,
    timeout: Duration,
    interval: Duration,
    samples: Option<u64>,
    json: bool,
) -> Result<(), AppError> {
    let registry = configured_registry(&config).await?;
    let signal = tokio::signal::ctrl_c();
    tokio::pin!(signal);
    let mut round = 0_u64;
    loop {
        let cancel = CancellationToken::new();
        let poll = registry.health_check_all(cancel.clone());
        let timed_out = tokio::select! {
            biased;
            result = &mut signal => {
                cancel.cancel();
                return result.map_err(AppError::ShutdownSignal);
            }
            result = tokio::time::timeout(timeout, poll) => result.is_err(),
        };
        if timed_out {
            cancel.cancel();
        }
        round = round.saturating_add(1);
        let report = HealthRound {
            round,
            timed_out,
            providers: observed_provider_checks(&config, &registry).await?,
        };
        write_health_round(&report, json)?;
        if samples == Some(round) {
            if timed_out {
                return Err(RuntimeError::Deadline.into());
            }
            return if report
                .providers
                .iter()
                .all(|provider| provider.state == ProviderHealthState::Healthy)
            {
                Ok(())
            } else {
                Err(AppError::UnhealthyProviders)
            };
        }
        tokio::select! {
            biased;
            result = &mut signal => return result.map_err(AppError::ShutdownSignal),
            () = tokio::time::sleep(interval) => {}
        }
    }
}

fn write_health_round(report: &HealthRound, json: bool) -> Result<(), AppError> {
    let mut output = io::stdout().lock();
    if json {
        let line = serde_json::to_string(report)?;
        writeln!(output, "{line}").map_err(AppError::HealthOutput)?;
    } else {
        writeln!(
            output,
            "Health round {}{}",
            report.round,
            if report.timed_out {
                " (deadline reached)"
            } else {
                ""
            },
        )
        .map_err(AppError::HealthOutput)?;
        for provider in &report.providers {
            writeln!(
                output,
                "{}: {:?} (observation age {:?} ms)",
                provider.provider_id, provider.state, provider.checked_age_ms,
            )
            .map_err(AppError::HealthOutput)?;
        }
    }
    output.flush().map_err(AppError::HealthOutput)
}

#[derive(Serialize)]
struct RunReport {
    task_id: TaskId,
    task_state: Option<TaskState>,
    execution: Option<TaskExecutionResult>,
    error: Option<String>,
    cleanup: CleanupReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    startup_residency: Option<StartupResidencyReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stored_memory_id: Option<MemoryId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_publication: Option<event_commands::Publication>,
}

fn print_startup_residency(report: Option<StartupResidencyReport>) {
    if let Some(report) = report {
        println!(
            "External startup residency: {} models across {} providers, {} RAM bytes, {} VRAM bytes",
            report.external_models, report.providers_observed, report.ram_bytes, report.vram_bytes,
        );
    }
}

pub(crate) async fn run_agent(args: RunAgentArgs) -> Result<(), AppError> {
    let config = crate::catalog_commands::load(&args.config, args.model_catalog.as_deref())?;
    let mut prompt = String::new();
    File::open(&args.prompt_file)
        .map_err(AppError::PromptIo)?
        .take(65_537)
        .read_to_string(&mut prompt)
        .map_err(AppError::PromptIo)?;
    if prompt.len() > 65_536 || prompt.trim().is_empty() {
        return Err(AppError::InvalidPrompt);
    }
    let journal = event_commands::prepare(args.event_journal).await?;
    let mut required = Vec::new();
    if args.memory_query_file.is_some() {
        required.push(Permission::MemoryRead);
    }
    if args.remember_result {
        required.push(Permission::MemoryWrite);
    }
    let authority = if required.is_empty() {
        None
    } else {
        Some(configured_authorizer(&config, args.agent, &required)?)
    };
    let context = if let (Some(path), Some(authority)) = (args.memory_query_file, authority) {
        let database = args.memory_database.clone();
        let actor = args.agent;
        Some(
            tokio::task::spawn_blocking(move || {
                let phrase = crate::memory_commands::read_text(&path, 256, "memory_query_file")?;
                let store =
                    MemoryStore::open_read_only(database, StorageLimits::default(), authority)?;
                Ok::<_, AppError>(MemoryContext::retrieve(&store, actor, &phrase)?)
            })
            .await
            .map_err(|_| AppError::MemoryWorker)??,
        )
    } else {
        None
    };
    let runtime = VesperaRuntime::from_config_for_agent(&config, args.agent).await?;
    let result_store = if args.remember_result {
        let database = args.memory_database;
        let authority = runtime.agents().memory_authorizer();
        Some(
            tokio::task::spawn_blocking(move || {
                MemoryStore::open_existing(database, StorageLimits::default(), authority)
            })
            .await
            .map_err(|_| AppError::MemoryWorker)??,
        )
    } else {
        None
    };
    let task_id = TaskId::new();
    let task = Task {
        id: task_id,
        prompt,
        required_capabilities: BTreeSet::new(),
        state: TaskState::Queued,
    };
    let timeout = Duration::from_secs(args.timeout_seconds);
    let options = ExecutionOptions {
        max_output_tokens: args.max_output_tokens,
        context_tokens: args.context_tokens,
        timeout,
        temperature: args.temperature,
        ..Default::default()
    };
    let publisher = Publisher::start(args.scheduler_telemetry, &runtime).await?;
    let cancel = CancellationToken::new();
    let outcome = cancellable(timeout, cancel.clone(), async {
        if args.reconcile_residency {
            runtime.reconcile_startup(cancel.clone()).await?;
        }
        runtime
            .run_agent_with_context(args.agent, task, options, args.resume, context, cancel)
            .await
    })
    .await;
    let cleanup = runtime
        .cleanup(args.unload_after, Duration::from_secs(5))
        .await;
    let telemetry = Publisher::finish(publisher).await;
    let cleanup_failed = !cleanup.drained || !cleanup.unload_errors.is_empty();
    let task_state = runtime
        .agents()
        .get_task(task_id)
        .await
        .ok()
        .map(|task| task.state);
    let (execution, error) = match outcome {
        Ok(execution) => (Some(execution), None),
        Err(error) => (None, Some(error)),
    };
    let event = match &execution {
        Some(execution) => Some(EventKind::InferenceCompleted {
            task_id,
            model_id: execution.model_id,
            agent_id: Some(execution.agent_id),
        }),
        None => task_state.and_then(|state| event_commands::terminal_event(task_id, state)),
    };
    let event_publication = if let (Some(journal), Some(event)) = (journal, event) {
        Some(event_commands::record(journal, "run-agent", task_id, event).await)
    } else {
        None
    };
    let journal_failed = event_publication
        .as_ref()
        .is_some_and(event_commands::Publication::failed);
    let persistence = if let (Some(store), Some(execution)) = (result_store, &execution) {
        let actor = execution.agent_id;
        let content = execution.response.content.clone();
        tokio::task::spawn_blocking(move || {
            let timestamp = crate::memory_commands::current_timestamp()?;
            let id = MemoryId::new();
            store.insert(
                actor,
                MemoryRecord {
                    id,
                    agent_id: actor,
                    layer: MemoryLayer::Episodic,
                    content,
                    created_at_unix_ms: timestamp,
                    source_task_id: Some(task_id),
                },
                MemoryOrigin::Generated,
            )?;
            Ok::<_, AppError>(Some(id))
        })
        .await
        .unwrap_or(Err(AppError::MemoryWorker))
    } else {
        Ok(None)
    };
    let (stored_memory_id, memory_error) = match persistence {
        Ok(id) => (id, None),
        Err(error) => (None, Some(error)),
    };
    let startup_residency = runtime.scheduler().startup_residency();
    if args.json {
        let report = RunReport {
            task_id,
            task_state,
            error: error.as_ref().map(ToString::to_string),
            execution,
            cleanup,
            startup_residency,
            stored_memory_id,
            memory_error: memory_error.as_ref().map(ToString::to_string),
            event_publication,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_startup_residency(startup_residency);
        if let Some(publication) = &event_publication {
            publication.report_failure();
        }
        if let Some(id) = stored_memory_id {
            println!("Stored memory {id}");
        }
        if let Some(execution) = &execution {
            println!("{}", execution.response.content);
            println!(
                "Task {}: {:?}; model {}; provider {}",
                task_id, execution.task.state, execution.model_id, execution.provider_id
            );
        }
        println!(
            "Local admissions drained: {}; reservations without confirmed unload: {}",
            cleanup.drained,
            cleanup.remaining_models.len()
        );
        for (model, error) in &cleanup.unload_errors {
            eprintln!("Unload {model}: {error}");
        }
    }
    if let Some(error) = error {
        return Err(error.into());
    }
    if let Some(error) = memory_error {
        return Err(error);
    }
    if journal_failed {
        return Err(vespera_events::JournalError::Database.into());
    }
    if cleanup_failed {
        return Err(AppError::CleanupFailed);
    }
    telemetry
}
