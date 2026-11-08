use std::path::Path;
use std::time::Duration;

use vespera_agents::AgentTaskStore;
use vespera_app::runtime::VesperaRuntime;
use vespera_core::{
    AgentId, Capability, Permission, ProviderKind, ResourceBudget, Task, TaskId, TaskState,
};
use vespera_orchestrator::{OutputPolicy, Step, Workflow, WorkflowReport, WorkflowStore};
use vespera_providers::CancellationToken;

use crate::desktop_agent::{self, Roster};
use crate::desktop_evidence::{self, Projection};

const STEP_OUTPUT_TOKENS: u32 = 128;
const STEP_TIMEOUT_MS: u64 = 50_000;
const WORKFLOW_TIMEOUT_MS: u64 = 100_000;

pub(super) struct WorkflowRequest {
    pub coordinator: usize,
    pub reviewer: usize,
    pub draft_prompt: String,
    pub review_prompt: String,
}

pub(super) struct FinishedWorkflow {
    pub report: WorkflowReport,
    pub cleanup_confirmed: bool,
    pub observation_error: Option<String>,
}

fn plan(coordinator: AgentId, reviewer: AgentId, request: WorkflowRequest) -> Workflow {
    let text = Capability::new("text-generation").expect("static capability is valid");
    let step = |name: &str, prompt, agent, dependencies| Step {
        name: name.into(),
        prompt,
        required_capabilities: [text.clone()].into(),
        candidates: vec![agent],
        dependencies,
        max_attempts: 1,
        max_output_tokens: STEP_OUTPUT_TOKENS,
        context_tokens: None,
        timeout_ms: STEP_TIMEOUT_MS,
        temperature: None,
    };
    Workflow {
        task: Task {
            id: TaskId::new(),
            prompt: request.draft_prompt.clone(),
            required_capabilities: [text.clone()].into(),
            state: TaskState::Queued,
        },
        coordinator,
        steps: vec![
            step("draft", request.draft_prompt, coordinator, vec![]),
            step(
                "review",
                request.review_prompt,
                reviewer,
                vec!["draft".into()],
            ),
        ],
        max_total_attempts: 2,
        max_total_output_tokens: u64::from(STEP_OUTPUT_TOKENS) * 2,
        timeout_ms: WORKFLOW_TIMEOUT_MS,
        output: OutputPolicy::Step {
            step: "review".into(),
        },
    }
}

pub(super) async fn run(
    dir: &Path,
    roster: Roster,
    request: WorkflowRequest,
    cancel: CancellationToken,
) -> Result<FinishedWorkflow, String> {
    if request.coordinator == request.reviewer {
        return Err("Choose a different saved agent for review.".into());
    }
    let (coordinator, draft, provider) = roster
        .selection(request.coordinator)
        .ok_or("Select a coordinator.")?;
    let (reviewer, review, review_provider) = roster
        .selection(request.reviewer)
        .ok_or("Select a reviewer.")?;
    if !coordinator.permissions.contains(&Permission::Delegate) {
        return Err("Allow delegation for the coordinator before starting a workflow.".into());
    }
    if provider.id != review_provider.id {
        return Err("This workflow requires two agents on the same saved provider.".into());
    }
    let tasks = AgentTaskStore::open(&dir.join("desktop-tasks.sqlite3"))
        .map_err(|error| error.to_string())?;
    if desktop_agent::load(dir)?.is_none_or(|current| current.revision != roster.revision) {
        return Err("The saved roster changed. Reopen the application.".into());
    }
    let suspended = tasks.suspensions().map_err(|error| error.to_string())?;
    if [coordinator.identity.id, reviewer.identity.id]
        .iter()
        .any(|id| suspended.contains(id))
    {
        return Err("Resume both agents before starting the workflow.".into());
    }
    let memory_limit = |draft: Option<u64>, review: Option<u64>| Some(draft?.max(review?));
    let native = provider.kind == ProviderKind::Ollama;
    let mut config = roster.config().clone();
    config.resources = ResourceBudget {
        ram_bytes: memory_limit(draft.resources.ram_bytes, review.resources.ram_bytes)
            .ok_or("Both models need RAM estimates.")?,
        vram_bytes: memory_limit(draft.resources.vram_bytes, review.resources.vram_bytes)
            .ok_or("Both models need VRAM estimates.")?,
        ..ResourceBudget::default()
    };
    config.scheduling.local_host_limits = true;
    config.scheduling.keep_alive_seconds = native.then_some(60);
    let workflow = plan(coordinator.identity.id, reviewer.identity.id, request);
    let runtime = VesperaRuntime::from_config_for_workflow_with_suspensions(
        &config, &workflow, false, &suspended,
    )
    .await
    .map_err(|error| error.to_string())?;
    runtime
        .agents()
        .attach_task_store(tasks)
        .map_err(|error| error.to_string())?;
    if native {
        runtime
            .reconcile_startup(cancel.clone())
            .await
            .map_err(|error| error.to_string())?;
    }
    let store = WorkflowStore::open(&desktop_evidence::workflow_path(dir))
        .map_err(|error| error.to_string())?;
    let task = workflow.task.id;
    let projection = Projection::start(dir, &store, task);
    let outcome = runtime
        .run_workflow_checkpointed(workflow, cancel, None, Some(&store))
        .await;
    let observation = match projection {
        Ok(projection) => projection.finish(dir, &store, task).await,
        Err(_) => desktop_evidence::replay(dir, &store),
    };
    let cleanup = runtime.cleanup(native, Duration::from_secs(5)).await;
    Ok(FinishedWorkflow {
        report: outcome.map_err(|error| error.to_string())?,
        cleanup_confirmed: cleanup.drained && cleanup.unload_errors.is_empty(),
        observation_error: observation.err(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_workflow_assigns_distinct_agents_and_fences_review_on_the_draft() {
        let coordinator = AgentId::new();
        let reviewer = AgentId::new();
        let workflow = plan(
            coordinator,
            reviewer,
            WorkflowRequest {
                coordinator: 0,
                reviewer: 1,
                draft_prompt: "Draft a sentence.".into(),
                review_prompt: "Shorten the draft.".into(),
            },
        );
        workflow.validate().unwrap();
        assert_eq!(workflow.steps[0].candidates, [coordinator]);
        assert_eq!(workflow.steps[1].candidates, [reviewer]);
        assert_eq!(workflow.steps[1].dependencies, ["draft"]);
        assert_eq!(
            workflow.output,
            OutputPolicy::Step {
                step: "review".into()
            }
        );
        assert_eq!(workflow.max_total_attempts, 2);
    }
}
