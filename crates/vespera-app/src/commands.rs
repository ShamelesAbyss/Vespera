//! CLI operation handlers; service behavior lives in the application runtime.
use crate::{AppError, RunAgentArgs};
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
use vespera_providers::CancellationToken;

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
