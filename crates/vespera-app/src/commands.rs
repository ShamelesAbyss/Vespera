//! CLI operation handlers; service behavior lives in the application runtime.
use crate::scheduler_telemetry::Publisher;
use crate::{
    AgentInboxAction, AgentStateAction, AppError, RunAgentArgs, RunWorkflowArgs, event_commands,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::File;
use std::future::Future;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;
use vespera_agents::{
    AgentRuntimeError, AgentTaskStore, AgentTaskStoreError, ExecutionOptions, MemoryContext,
    TaskExecutionResult,
};
use vespera_app::memory::configured_authorizer;
use vespera_app::runtime::{
    CleanupReport, LocalHostLimits, ProviderCheck, RuntimeError, VesperaRuntime, check_providers,
    configured_registry, observed_provider_checks,
};
use vespera_config::AppConfig;
use vespera_core::{
    AgentId, AgentStatus, EventEnvelope, EventKind, MemoryId, MemoryLayer, MemoryRecord,
    Permission, Task, TaskId, TaskState,
};
use vespera_memory::{MemoryOrigin, MemoryStore, StorageLimits};
use vespera_model_registry::ProviderHealthState;
use vespera_orchestrator::{
    DecompositionRequest, PlanningEvidence, Workflow, WorkflowReport, WorkflowStore,
    WorkflowStoreError,
};
use vespera_providers::CancellationToken;
use vespera_scheduler::StartupResidencyReport;

#[derive(Serialize)]
struct WorkflowRunReport {
    task_id: TaskId,
    workflow: Option<WorkflowReport>,
    error: Option<String>,
    cleanup: CleanupReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    decomposition: Option<DecompositionReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_host_limits: Option<LocalHostLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    startup_residency: Option<StartupResidencyReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    admission_residency: Option<StartupResidencyReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovered_reservations: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_publication: Option<event_commands::Publication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lifecycle_publication: Option<event_commands::LifecyclePublication>,
}

#[derive(Serialize)]
struct DecompositionReport {
    plan: Workflow,
    #[serde(flatten)]
    evidence: PlanningEvidence,
}

enum WorkflowInput {
    Authored(Workflow),
    Decompose(DecompositionRequest),
}

pub(crate) fn agent_state(
    config: &AppConfig,
    database: &Path,
    agent: AgentId,
    action: AgentStateAction,
) -> Result<(), AppError> {
    #[derive(Serialize)]
    struct Report {
        agent_id: AgentId,
        configured_status: AgentStatus,
        operator_suspended: bool,
        effective_status: AgentStatus,
    }
    let configured_status = config
        .agents
        .iter()
        .find(|definition| definition.identity.id == agent)
        .ok_or_else(|| RuntimeError::from(AgentRuntimeError::AgentNotFound { agent_id: agent }))?
        .status;
    let operator_suspended = match action {
        AgentStateAction::Inspect => AgentTaskStore::inspect_suspension(database, agent)?,
        AgentStateAction::Suspend => {
            AgentTaskStore::open(database)?.set_suspended(agent, true)?;
            true
        }
        AgentStateAction::Release => {
            AgentTaskStore::release_suspension(database, agent)?;
            false
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&Report {
            agent_id: agent,
            configured_status,
            operator_suspended,
            effective_status: if operator_suspended {
                AgentStatus::Suspended
            } else {
                configured_status
            },
        })?
    );
    Ok(())
}

pub(crate) async fn agent_inbox(
    mut config: AppConfig,
    database: &Path,
    actor: AgentId,
    action: AgentInboxAction,
) -> Result<(), AppError> {
    let recipient = match &action {
        AgentInboxAction::List => actor,
        AgentInboxAction::Send { to, .. } => *to,
    };
    // Compose an empty runtime and register only the offline command's participants.
    let agents = std::mem::take(&mut config.agents);
    config.providers.clear();
    config.models.clear();
    config.scheduling = Default::default();
    let runtime = VesperaRuntime::from_config(&config).await?;
    let runtime = runtime.agents();
    for agent_id in BTreeSet::from([actor, recipient]) {
        let definition = agents
            .iter()
            .find(|definition| definition.identity.id == agent_id)
            .ok_or_else(|| RuntimeError::from(AgentRuntimeError::AgentNotFound { agent_id }))?;
        runtime
            .register_agent(definition.clone())
            .await
            .map_err(RuntimeError::from)?;
    }
    let definition = runtime.get_agent(actor).await.map_err(RuntimeError::from)?;
    if !definition.permissions.contains(&Permission::Communicate) {
        return Err(RuntimeError::from(AgentRuntimeError::PermissionDenied {
            agent_id: actor,
            permission: Permission::Communicate,
        })
        .into());
    }
    match action {
        AgentInboxAction::List => {
            runtime
                .attach_task_store(AgentTaskStore::open_existing(database)?)
                .map_err(RuntimeError::from)?;
            let messages = runtime
                .inbox_messages(actor)
                .await
                .map_err(RuntimeError::from)?;
            println!("{}", serde_json::to_string_pretty(&messages)?);
        }
        AgentInboxAction::Send {
            to,
            message_file,
            task,
            event_journal,
        } => {
            let content =
                crate::memory_commands::read_text(&message_file, 64 * 1024, "message_file")?;
            let mut journal = event_commands::prepare(event_journal).await?;
            runtime
                .attach_task_store(AgentTaskStore::open(database)?)
                .map_err(RuntimeError::from)?;
            let (message, event) = runtime
                .send_message_observed(actor, to, content, task)
                .await
                .map_err(RuntimeError::from)?;
            println!("{}", serde_json::to_string_pretty(&message)?);
            if let Some(journal) = &mut journal {
                event
                    .ok_or(vespera_events::JournalError::Invalid("missing inbox event"))
                    .and_then(|event| journal.append(&event))
                    .map_err(|source| AppError::InboxEventPublication {
                        tick: message.logical_tick,
                        source,
                    })?;
            }
        }
    }
    Ok(())
}

pub(crate) async fn run_workflow(args: RunWorkflowArgs) -> Result<(), AppError> {
    let config = crate::catalog_commands::load(&args.config, args.model_catalog.as_deref())?;
    let mut input = String::new();
    let path = args
        .workflow_file
        .as_ref()
        .or(args.decomposition_file.as_ref())
        .ok_or(AppError::InvalidWorkflow)?;
    File::open(path)
        .map_err(|_| AppError::WorkflowIo)?
        .take(1_048_577)
        .read_to_string(&mut input)
        .map_err(|_| AppError::WorkflowIo)?;
    if input.len() > 1_048_576 {
        return Err(AppError::InvalidWorkflow);
    }
    let input = if args.workflow_file.is_some() {
        let workflow: Workflow =
            serde_json::from_str(&input).map_err(|_| AppError::InvalidWorkflow)?;
        workflow.validate()?;
        WorkflowInput::Authored(workflow)
    } else {
        let request: DecompositionRequest =
            serde_json::from_str(&input).map_err(|_| AppError::InvalidWorkflow)?;
        request.validate().map_err(RuntimeError::from)?;
        WorkflowInput::Decompose(request)
    };
    let task_id = match &input {
        WorkflowInput::Authored(workflow) => workflow.task.id,
        WorkflowInput::Decompose(request) => request.task.id,
    };
    let workflow_store = args
        .workflow_store
        .as_deref()
        .map(|path| {
            if args.resume_workflow {
                WorkflowStore::open_for_resume(path)
            } else {
                WorkflowStore::open(path)
            }
        })
        .transpose()?;
    if let Some(store) = &workflow_store {
        if let WorkflowInput::Authored(workflow) = &input
            && args.resume_workflow
        {
            store.ensure_resumable(workflow)?;
        } else if store.contains(task_id)? {
            return Err(WorkflowStoreError::Duplicate.into());
        }
    }
    let task_store = args
        .task_store
        .as_deref()
        .map(AgentTaskStore::open)
        .transpose()?;
    let suspended = task_store
        .as_ref()
        .map(AgentTaskStore::suspensions)
        .transpose()?
        .unwrap_or_default();
    let mut journal = event_commands::prepare(args.event_journal).await?;
    let lifecycle_journal = event_commands::prepare(args.lifecycle_journal).await?;
    let has_workflow_audit = match &workflow_store {
        Some(store) if args.resume_workflow => store.events(task_id)?.is_some(),
        Some(_) => true,
        None => false,
    };
    let runtime = match &input {
        WorkflowInput::Authored(workflow) => {
            VesperaRuntime::from_config_for_workflow_with_suspensions(
                &config,
                workflow,
                args.resume,
                &suspended,
            )
            .await?
        }
        WorkflowInput::Decompose(request) => {
            if let Some(agent_id) = std::iter::once(request.coordinator)
                .chain(request.workers.iter().copied())
                .find(|id| suspended.contains(id))
            {
                return Err(
                    RuntimeError::from(AgentRuntimeError::DurablySuspended { agent_id }).into(),
                );
            }
            VesperaRuntime::from_config_for_decomposition(&config, request, args.resume).await?
        }
    };
    if let Some(store) = task_store {
        runtime
            .agents()
            .attach_task_store(store)
            .map_err(RuntimeError::from)?;
    }
    let publisher = Publisher::start(args.scheduler_telemetry, &runtime).await?;
    let lifecycle = lifecycle_journal.map(|journal| {
        event_commands::LifecycleJournal::start(journal, runtime.scheduler().clone())
    });
    let durable_live = if args.observe_workflow && has_workflow_audit {
        journal.take().map(|journal| {
            let live = event_commands::WorkflowJournal::start_durable(journal);
            if let Some(store) = &workflow_store {
                store.observe_events(Some((task_id, live.sender.clone())));
            }
            live
        })
    } else {
        None
    };
    let live = if args.observe_workflow && !has_workflow_audit {
        journal
            .take()
            .map(|journal| event_commands::WorkflowJournal::start(journal, task_id))
    } else {
        None
    };
    let cancel = CancellationToken::new();
    let timeout = Duration::from_secs(args.timeout_seconds);
    let timeout = match &input {
        WorkflowInput::Authored(_) => timeout,
        WorkflowInput::Decompose(request) => timeout.min(Duration::from_millis(request.timeout_ms)),
    };
    let mut decomposition = None;
    let outcome = cancellable(timeout, cancel.clone(), async {
        if let Some(path) = &args.residency_journal {
            runtime
                .reconcile_startup_with_journal(
                    path,
                    args.acknowledge_residency_recovery,
                    cancel.clone(),
                )
                .await?;
        } else if args.reconcile_residency {
            runtime.reconcile_startup(cancel.clone()).await?;
        }
        let observer = live.as_ref().map(|live| &live.sender);
        let workflow = match input {
            WorkflowInput::Authored(workflow) => workflow,
            WorkflowInput::Decompose(request) => {
                let (plan, result) = runtime.decompose_workflow(&request, cancel.clone()).await?;
                decomposition = Some(DecompositionReport {
                    plan: plan.clone(),
                    evidence: PlanningEvidence {
                        planning_task_id: result.task.id,
                        agent_id: result.agent_id,
                        model_id: result.model_id,
                        provider_id: result.provider_id,
                    },
                });
                plan
            }
        };
        if args.resume_workflow {
            runtime
                .orchestrator()
                .resume_checkpointed(
                    workflow,
                    cancel,
                    observer,
                    workflow_store.as_ref().expect("resume requires store"),
                )
                .await
                .map_err(RuntimeError::from)
        } else if let Some(decomposition) = &decomposition {
            if cancel.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            runtime
                .orchestrator()
                .execute_planned(
                    workflow,
                    decomposition.evidence,
                    cancel,
                    observer,
                    workflow_store.as_ref(),
                )
                .await
                .map_err(RuntimeError::from)
        } else {
            runtime
                .run_workflow_checkpointed(workflow, cancel, observer, workflow_store.as_ref())
                .await
        }
    })
    .await;
    let cleanup = runtime
        .cleanup(args.unload_after, Duration::from_secs(5))
        .await;
    let telemetry = Publisher::finish(publisher).await;
    let lifecycle_publication = event_commands::LifecycleJournal::finish(lifecycle).await;
    let lifecycle_failed = lifecycle_publication
        .as_ref()
        .is_some_and(event_commands::LifecyclePublication::failed);
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
    let event_publication = if let Some(live) = durable_live {
        if let Some(store) = &workflow_store {
            store.observe_events(None);
        }
        live.finish(None).await
    } else if let Some(live) = live {
        live.finish(terminal).await
    } else if has_workflow_audit
        && journal.is_some()
        && let Some(store) = &workflow_store
    {
        let journal = journal.take().expect("existing workflow journal");
        let committed = store.events(task_id);
        event_commands::record_workflow_audit_summary(
            journal,
            task_id,
            committed,
            args.record_delegations,
        )
        .await
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
    let local_host_limits = runtime.local_host_limits();
    let admission_residency = runtime.scheduler().admission_residency();
    let recovered_reservations = runtime.scheduler().recovered_residency_count();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&WorkflowRunReport {
                task_id,
                workflow,
                error: error.as_ref().map(ToString::to_string),
                cleanup,
                decomposition,
                local_host_limits,
                startup_residency,
                admission_residency,
                recovered_reservations,
                event_publication,
                lifecycle_publication,
            })?
        );
    } else {
        if let Some(decomposition) = decomposition {
            println!(
                "Accepted {} planned steps from coordinator {}",
                decomposition.plan.steps.len(),
                decomposition.evidence.agent_id
            );
        }
        print_local_host_limits(local_host_limits);
        print_residency("startup", startup_residency);
        print_residency("latest admission", admission_residency);
        print_recovered_reservations(recovered_reservations);
        if let Some(publication) = &event_publication {
            publication.report_failure();
        }
        if let Some(publication) = &lifecycle_publication {
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
    if lifecycle_failed {
        return Err(AppError::LifecyclePublicationFailed);
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
    mut journal: Option<vespera_events::Journal>,
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
    publish_provider_checks(journal.as_mut(), 1, &checks)?;
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
    mut journal: Option<vespera_events::Journal>,
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
        publish_provider_checks(journal.as_mut(), round, &report.providers)?;
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

fn publish_provider_checks(
    journal: Option<&mut vespera_events::Journal>,
    round: u64,
    checks: &[ProviderCheck],
) -> Result<(), AppError> {
    let Some(journal) = journal else {
        return Ok(());
    };
    let timestamp = crate::memory_commands::current_timestamp()?;
    let publication_error = |source| AppError::ProviderHealthPublication { round, source };
    for chunk in checks.chunks(vespera_events::MAX_PAGE as usize) {
        let events = chunk
            .iter()
            .map(|check| {
                EventEnvelope::new(
                    "provider-health",
                    timestamp,
                    EventKind::ProviderHealthObserved {
                        provider_id: check.provider_id,
                        state: check.state,
                        checked_age_ms: check.checked_age_ms,
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                publication_error(vespera_events::JournalError::Invalid("health envelope"))
            })?;
        journal.append_batch(&events).map_err(publication_error)?;
    }
    Ok(())
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
    local_host_limits: Option<LocalHostLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    startup_residency: Option<StartupResidencyReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    admission_residency: Option<StartupResidencyReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovered_reservations: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stored_memory_id: Option<MemoryId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_publication: Option<event_commands::Publication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lifecycle_publication: Option<event_commands::LifecyclePublication>,
}

fn print_residency(phase: &str, report: Option<StartupResidencyReport>) {
    if let Some(report) = report {
        println!(
            "External {phase} residency: {} models across {} providers, {} RAM bytes, {} VRAM bytes",
            report.external_models, report.providers_observed, report.ram_bytes, report.vram_bytes,
        );
    }
}

fn print_local_host_limits(report: Option<LocalHostLimits>) {
    if let Some(report) = report {
        println!(
            "Host-constrained admission budget: {} RAM bytes, {} concurrent inferences",
            report.effective_budget.ram_bytes, report.effective_budget.max_concurrent_inferences,
        );
        if let Some(gpu) = report.nvidia_gpu {
            println!(
                "NVIDIA VRAM ceiling: {} bytes from {} free bytes at startup",
                report.effective_budget.vram_bytes, gpu.free_vram_bytes,
            );
        }
    }
}

fn print_recovered_reservations(count: Option<usize>) {
    if let Some(count) = count {
        println!("Unresolved residency records from earlier invocations: {count}");
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
    let task_id = args.task_id.unwrap_or_default();
    let task_store = args
        .task_store
        .as_deref()
        .map(AgentTaskStore::open)
        .transpose()?;
    if let Some(store) = &task_store {
        if store.is_suspended(args.agent)? {
            return Err(RuntimeError::from(AgentRuntimeError::DurablySuspended {
                agent_id: args.agent,
            })
            .into());
        }
        if store.contains(task_id)? {
            return Err(AgentTaskStoreError::Duplicate.into());
        }
    }
    let journal = event_commands::prepare(args.event_journal).await?;
    let lifecycle_journal = event_commands::prepare(args.lifecycle_journal).await?;
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
    if let Some(store) = task_store {
        runtime
            .agents()
            .attach_task_store(store)
            .map_err(RuntimeError::from)?;
    }
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
    let lifecycle = lifecycle_journal.map(|journal| {
        event_commands::LifecycleJournal::start(journal, runtime.scheduler().clone())
    });
    let cancel = CancellationToken::new();
    let outcome = cancellable(timeout, cancel.clone(), async {
        if let Some(path) = &args.residency_journal {
            runtime
                .reconcile_startup_with_journal(
                    path,
                    args.acknowledge_residency_recovery,
                    cancel.clone(),
                )
                .await?;
        } else if args.reconcile_residency {
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
    let lifecycle_publication = event_commands::LifecycleJournal::finish(lifecycle).await;
    let lifecycle_failed = lifecycle_publication
        .as_ref()
        .is_some_and(event_commands::LifecyclePublication::failed);
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
    let event_publication = match (journal, event) {
        (Some(journal), _) if args.task_store.is_some() => {
            event_commands::record_durable(
                journal,
                runtime.agents().durable_terminal_event(task_id),
            )
            .await
        }
        (Some(journal), Some(event)) => {
            Some(event_commands::record(journal, "run-agent", task_id, event).await)
        }
        _ => None,
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
    let local_host_limits = runtime.local_host_limits();
    let admission_residency = runtime.scheduler().admission_residency();
    let recovered_reservations = runtime.scheduler().recovered_residency_count();
    if args.json {
        let report = RunReport {
            task_id,
            task_state,
            error: error.as_ref().map(ToString::to_string),
            execution,
            cleanup,
            local_host_limits,
            startup_residency,
            admission_residency,
            recovered_reservations,
            stored_memory_id,
            memory_error: memory_error.as_ref().map(ToString::to_string),
            event_publication,
            lifecycle_publication,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_local_host_limits(local_host_limits);
        print_residency("startup", startup_residency);
        print_residency("latest admission", admission_residency);
        print_recovered_reservations(recovered_reservations);
        if let Some(publication) = &event_publication {
            publication.report_failure();
        }
        if let Some(publication) = &lifecycle_publication {
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
    if lifecycle_failed {
        return Err(AppError::LifecyclePublicationFailed);
    }
    if cleanup_failed {
        return Err(AppError::CleanupFailed);
    }
    telemetry
}
