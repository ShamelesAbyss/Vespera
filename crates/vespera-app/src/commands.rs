//! CLI operation handlers; service behavior lives in the application runtime.
use crate::{AppError, RunAgentArgs, RunWorkflowArgs};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::File;
use std::future::Future;
use std::io::Read;
use std::time::Duration;
use vespera_agents::{ExecutionOptions, TaskExecutionResult};
use vespera_app::runtime::{CleanupReport, RuntimeError, VesperaRuntime, check_providers};
use vespera_config::AppConfig;
use vespera_core::{Task, TaskId, TaskState};
use vespera_model_registry::ProviderHealthState;
use vespera_orchestrator::{Workflow, WorkflowReport};
use vespera_providers::CancellationToken;

#[derive(Serialize)]
struct WorkflowRunReport {
    task_id: TaskId,
    workflow: Option<WorkflowReport>,
    error: Option<String>,
    cleanup: CleanupReport,
}

pub(crate) async fn run_workflow(args: RunWorkflowArgs) -> Result<(), AppError> {
    let config = AppConfig::load(&args.config)?;
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
    let runtime = VesperaRuntime::from_config_for_workflow(&config, &workflow, args.resume).await?;
    let cancel = CancellationToken::new();
    let outcome = cancellable(
        Duration::from_secs(args.timeout_seconds),
        cancel.clone(),
        runtime.run_workflow(workflow, cancel),
    )
    .await;
    let cleanup = runtime
        .cleanup(args.unload_after, Duration::from_secs(5))
        .await;
    let cleanup_failed = !cleanup.drained || !cleanup.unload_errors.is_empty();
    let (workflow, error) = match outcome {
        Ok(report) => (Some(report), None),
        Err(error) => (runtime.orchestrator().report(task_id), Some(error)),
    };
    let succeeded = workflow
        .as_ref()
        .is_some_and(|report| report.task.state == TaskState::Completed);
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&WorkflowRunReport {
                task_id,
                workflow,
                error: error.as_ref().map(ToString::to_string),
                cleanup
            })?
        );
    } else {
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
    if cleanup_failed {
        return Err(AppError::CleanupFailed);
    }
    Ok(())
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
struct RunReport {
    task_id: TaskId,
    task_state: Option<TaskState>,
    execution: Option<TaskExecutionResult>,
    error: Option<String>,
    cleanup: CleanupReport,
}

pub(crate) async fn run_agent(args: RunAgentArgs) -> Result<(), AppError> {
    let config = AppConfig::load(&args.config)?;
    let mut prompt = String::new();
    File::open(&args.prompt_file)
        .map_err(AppError::PromptIo)?
        .take(65_537)
        .read_to_string(&mut prompt)
        .map_err(AppError::PromptIo)?;
    if prompt.len() > 65_536 || prompt.trim().is_empty() {
        return Err(AppError::InvalidPrompt);
    }
    let runtime = VesperaRuntime::from_config_for_agent(&config, args.agent).await?;
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
    let cancel = CancellationToken::new();
    let outcome = cancellable(
        timeout,
        cancel.clone(),
        runtime.run_agent(args.agent, task, options, args.resume, cancel),
    )
    .await;
    let cleanup = runtime
        .cleanup(args.unload_after, Duration::from_secs(5))
        .await;
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
    if args.json {
        let report = RunReport {
            task_id,
            task_state,
            error: error.as_ref().map(ToString::to_string),
            execution,
            cleanup,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
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
    if cleanup_failed {
        return Err(AppError::CleanupFailed);
    }
    Ok(())
}
