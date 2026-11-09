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
pub(super) const MAX_CHAIN_STEPS: usize = 6;

pub(super) struct StepRequest {
    pub name: String,
    pub agent: usize,
    pub prompt: String,
    pub dependencies: Vec<String>,
}

pub(super) struct FinishedWorkflow {
    pub report: WorkflowReport,
    pub cleanup_confirmed: bool,
    pub observation_error: Option<String>,
}

fn plan(steps: Vec<(AgentId, StepRequest)>) -> Workflow {
    let text = Capability::new("text-generation").expect("static capability is valid");
    let (coordinator, first) = &steps[0];
    let coordinator = *coordinator;
    let prompt = first.prompt.clone();
    let step_count = steps.len() as u32;
    let output_step = steps
        .last()
        .expect("nonempty desktop workflow")
        .1
        .name
        .clone();
    let steps = steps
        .into_iter()
        .map(|(agent, request)| Step {
            dependencies: request.dependencies,
            name: request.name,
            prompt: request.prompt,
            required_capabilities: [text.clone()].into(),
            candidates: vec![agent],
            max_attempts: 1,
            max_output_tokens: STEP_OUTPUT_TOKENS,
            context_tokens: None,
            timeout_ms: STEP_TIMEOUT_MS,
            temperature: None,
        })
        .collect();
    Workflow {
        task: Task {
            id: TaskId::new(),
            prompt,
            required_capabilities: [text.clone()].into(),
            state: TaskState::Queued,
        },
        coordinator,
        steps,
        max_total_attempts: step_count,
        max_total_output_tokens: u64::from(STEP_OUTPUT_TOKENS) * u64::from(step_count),
        timeout_ms: STEP_TIMEOUT_MS * u64::from(step_count),
        output: OutputPolicy::Step { step: output_step },
    }
}

pub(super) async fn run(
    dir: &Path,
    roster: Roster,
    steps: Vec<StepRequest>,
    cancel: CancellationToken,
) -> Result<FinishedWorkflow, String> {
    if !(2..=MAX_CHAIN_STEPS).contains(&steps.len()) {
        return Err("Choose between two and six steps.".into());
    }
    let assigned = steps
        .into_iter()
        .map(|step| {
            let (agent, _, _) = roster
                .selection(step.agent)
                .ok_or("Choose a saved agent for each step.")?;
            Ok::<_, &str>((agent.identity.id, step))
        })
        .collect::<Result<_, _>>()?;
    execute(dir, roster, plan(assigned), false, cancel).await
}

pub(super) async fn resume(
    dir: &Path,
    roster: Roster,
    workflow: Workflow,
    cancel: CancellationToken,
) -> Result<FinishedWorkflow, String> {
    execute(dir, roster, workflow, true, cancel).await
}

async fn execute(
    dir: &Path,
    roster: Roster,
    workflow: Workflow,
    resume: bool,
    cancel: CancellationToken,
) -> Result<FinishedWorkflow, String> {
    workflow.validate().map_err(|error| error.to_string())?;
    if !(2..=MAX_CHAIN_STEPS).contains(&workflow.steps.len())
        || workflow.steps[0].candidates.as_slice() != [workflow.coordinator]
        || workflow.timeout_ms > STEP_TIMEOUT_MS * workflow.steps.len() as u64
        || workflow.max_total_attempts > workflow.steps.len() as u32
        || workflow.max_total_output_tokens
            > u64::from(STEP_OUTPUT_TOKENS) * workflow.steps.len() as u64
        || workflow.steps.iter().any(|step| {
            step.max_output_tokens > STEP_OUTPUT_TOKENS || step.timeout_ms > STEP_TIMEOUT_MS
        })
    {
        return Err("This checkpoint is outside the desktop workflow limits.".into());
    }
    let binding_for = |id| {
        let index = roster
            .config()
            .agents
            .iter()
            .position(|agent| agent.identity.id == id)?;
        roster.selection(index)
    };
    let (coordinator, _, provider) =
        binding_for(workflow.coordinator).ok_or("Select a coordinator.")?;
    if !coordinator.permissions.contains(&Permission::Delegate) {
        return Err("Allow delegation for the coordinator before starting a workflow.".into());
    }
    if desktop_agent::load(dir)?.is_none_or(|current| current.revision != roster.revision) {
        return Err("The saved roster changed. Reopen the application.".into());
    }
    let path = desktop_evidence::workflow_path(dir);
    let tasks = AgentTaskStore::open(&dir.join("desktop-tasks.sqlite3"))
        .map_err(|error| error.to_string())?;
    let recovery = if resume {
        let store = WorkflowStore::open_for_resume(&path).map_err(|error| error.to_string())?;
        store
            .ensure_resumable(&workflow)
            .map_err(|error| error.to_string())?;
        Some(store)
    } else {
        None
    };
    let suspended = tasks.suspensions().map_err(|error| error.to_string())?;
    let mut ram = 0;
    let mut vram = 0;
    for step in &workflow.steps {
        let [candidate] = step.candidates.as_slice() else {
            return Err("Choose one saved agent for each desktop workflow step.".into());
        };
        let (agent, model, binding) =
            binding_for(*candidate).ok_or("Choose a saved agent for each step.")?;
        if binding.id != provider.id {
            return Err("All workflow agents must use the same saved provider.".into());
        }
        if suspended.contains(&agent.identity.id) {
            return Err("Resume all assigned agents before starting the workflow.".into());
        }
        ram = ram.max(
            model
                .resources
                .ram_bytes
                .ok_or("All models need RAM estimates.")?,
        );
        vram = vram.max(
            model
                .resources
                .vram_bytes
                .ok_or("All models need VRAM estimates.")?,
        );
    }
    let native = provider.kind == ProviderKind::Ollama;
    let mut config = roster.config().clone();
    config.resources = ResourceBudget {
        ram_bytes: ram,
        vram_bytes: vram,
        ..ResourceBudget::default()
    };
    config.scheduling.local_host_limits = true;
    config.scheduling.keep_alive_seconds = native.then_some(60);
    let runtime = VesperaRuntime::from_config_for_workflow_with_suspensions(
        &config, &workflow, false, &suspended,
    )
    .await
    .map_err(|error| error.to_string())?;
    runtime
        .agents()
        .attach_task_store(tasks)
        .map_err(|error| error.to_string())?;
    let outcome = async {
        if native {
            runtime
                .reconcile_startup(cancel.clone())
                .await
                .map_err(|error| error.to_string())?;
        }
        if cancel.is_cancelled() {
            return Err("The workflow was cancelled before execution.".into());
        }
        let store = match recovery {
            Some(store) => store,
            None => WorkflowStore::open(&path).map_err(|error| error.to_string())?,
        };
        let task = workflow.task.id;
        let projection = Projection::start(dir, &store, task);
        let report = if resume {
            runtime
                .orchestrator()
                .resume_checkpointed(workflow, cancel, None, &store)
                .await
                .map_err(|error| error.to_string())
        } else {
            runtime
                .run_workflow_checkpointed(workflow, cancel, None, Some(&store))
                .await
                .map_err(|error| error.to_string())
        };
        let observation = match projection {
            Ok(projection) => projection.finish(dir, &store, task).await,
            Err(_) => desktop_evidence::replay(dir, &store),
        };
        report.map(|report| (report, observation.err()))
    }
    .await;
    let cleanup = runtime.cleanup(native, Duration::from_secs(5)).await;
    let (report, observation_error) = outcome?;
    Ok(FinishedWorkflow {
        report,
        cleanup_confirmed: cleanup.drained && cleanup.unload_errors.is_empty(),
        observation_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an isolated saved desktop roster and its running model provider"]
    async fn live_desktop_branch_join_reopens_committed_plan_and_evidence() {
        let dir = std::path::PathBuf::from(
            std::env::var_os("VESPERA_TEST_BRANCH_DATA")
                .expect("set isolated desktop data directory"),
        );
        let roster = desktop_agent::load(&dir).unwrap().unwrap();
        let coordinator = (0..roster.len())
            .find(|&index| {
                roster
                    .selection(index)
                    .unwrap()
                    .0
                    .permissions
                    .contains(&Permission::Delegate)
            })
            .expect("roster needs a delegation grant");
        let steps = [
            (
                "seed",
                "Name one benefit of testing software in one sentence.",
                vec![],
            ),
            (
                "left",
                "Restate the supplied benefit in five words.",
                vec!["seed".into()],
            ),
            (
                "right",
                "Give one limitation of the supplied benefit in one sentence.",
                vec!["seed".into()],
            ),
            (
                "join",
                "Combine both supplied observations in one short sentence.",
                vec!["left".into(), "right".into()],
            ),
        ]
        .into_iter()
        .map(|(name, prompt, dependencies)| StepRequest {
            name: name.into(),
            agent: coordinator,
            prompt: prompt.into(),
            dependencies,
        })
        .collect();
        let result = run(&dir, roster, steps, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.report.task.state, TaskState::Completed);
        assert!(result.cleanup_confirmed);
        assert!(result.observation_error.is_none());
        let task = result.report.task.id;
        let saved = WorkflowStore::inspect(&desktop_evidence::workflow_path(&dir), task).unwrap();
        assert!(!saved.interrupted);
        assert_eq!(saved.report.task.state, TaskState::Completed);
        assert_eq!(saved.plan.steps[2].dependencies, ["seed"]);
        assert_eq!(saved.plan.steps[3].dependencies, ["left", "right"]);
        let store = WorkflowStore::open(&desktop_evidence::workflow_path(&dir)).unwrap();
        desktop_evidence::replay(&dir, &store).unwrap();
        println!("Verified live desktop branch/join and reopened checkpoint {task}");
    }

    #[test]
    fn authored_workflow_fences_each_assignee_on_the_previous_step() {
        let coordinator = AgentId::new();
        let reviewer = AgentId::new();
        let step = |name: &str, agent, prompt: &str, dependencies: &[&str]| StepRequest {
            name: name.into(),
            agent,
            prompt: prompt.into(),
            dependencies: dependencies.iter().map(|value| (*value).into()).collect(),
        };
        let workflow = plan(vec![
            (coordinator, step("draft", 0, "Draft a sentence.", &[])),
            (
                reviewer,
                step("review", 1, "Shorten the draft.", &["draft"]),
            ),
            (
                coordinator,
                step("final", 0, "Return the shortened draft.", &["review"]),
            ),
        ]);
        workflow.validate().unwrap();
        assert_eq!(workflow.steps[0].candidates, [coordinator]);
        assert_eq!(workflow.steps[1].candidates, [reviewer]);
        assert_eq!(workflow.steps[1].dependencies, ["draft"]);
        assert_eq!(workflow.steps[2].candidates, [coordinator]);
        assert_eq!(workflow.steps[2].dependencies, ["review"]);
        assert_eq!(
            workflow.output,
            OutputPolicy::Step {
                step: "final".into()
            }
        );
        assert_eq!(workflow.max_total_attempts, 3);
        assert_eq!(workflow.max_total_output_tokens, 384);
        assert_eq!(workflow.timeout_ms, 150_000);
    }

    #[test]
    fn authored_branch_join_preserves_only_selected_inputs() {
        let owner = AgentId::new();
        let steps = [
            ("seed", vec![]),
            ("left", vec!["seed".into()]),
            ("right", vec!["seed".into()]),
            ("join", vec!["left".into(), "right".into()]),
        ]
        .into_iter()
        .map(|(name, dependencies)| {
            (
                owner,
                StepRequest {
                    name: name.into(),
                    agent: 0,
                    prompt: format!("Execute {name}"),
                    dependencies,
                },
            )
        })
        .collect();
        let workflow = plan(steps);
        workflow.validate().unwrap();
        assert_eq!(workflow.steps[2].dependencies, ["seed"]);
        assert_eq!(workflow.steps[3].dependencies, ["left", "right"]);
        assert_eq!(workflow.max_total_attempts, 4);
        assert_eq!(
            workflow.output,
            OutputPolicy::Step {
                step: "join".into()
            }
        );
    }
}
