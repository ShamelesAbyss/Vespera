use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use bevy::clipboard::Clipboard;
use bevy::input::{
    ButtonState,
    keyboard::{KeyboardFocusLost, KeyboardInput},
};
use bevy::prelude::*;
use bevy::window::{ExitSystems, PrimaryWindow, WindowCloseRequested};
use vespera_agents::{AgentTaskStore, AgentTaskStoreError, StoredAgentTask, StoredTaskSummary};
use vespera_app::runtime::{RuntimeError, configured_registry, windows_credential_target};
use vespera_config::ProviderConfig;
use vespera_core::{AgentId, Permission, ProviderKind, TaskId};
use vespera_orchestrator::{OutputPolicy, Workflow};
use vespera_providers::{
    CancellationToken, DiscoveredModel, ProviderError, ProviderHealthReport,
    ProviderLifecycleSupport,
};

use crate::atlas::{self, Layer, Layers};
use crate::desktop_agent::{self, FinishedTask, Registration, Roster};
use crate::desktop_settings::{self, ENDPOINT_LIMIT};
use crate::desktop_workflow::{self, FinishedWorkflow, MAX_CHAIN_STEPS, StepRequest};
use crate::home_scene::button;

const MODELS_PER_PAGE: usize = 8;
const TASKS_PER_PAGE: u32 = 8;
const PROMPT_LIMIT: usize = 1200;
const OLLAMA: &str = "http://127.0.0.1:11434";
const COMPATIBLE: &str = "http://127.0.0.1:1234/v1";
const NUMBER_KEYS: [KeyCode; 8] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
];

#[derive(Component, Clone, Copy)]
enum Action {
    Ollama,
    Compatible,
    Edit,
    Credential,
    #[cfg(target_os = "windows")]
    WindowsCredential,
    Discover,
    SaveDiscovery,
    Previous,
    Next,
    Prompt,
    MemoryPrompt,
    ToggleMemoryRead,
    ToggleMemoryWrite,
    Remember,
    Cancel,
    PreviousAgent,
    NextAgent,
    NewAgent,
    EditIdentity,
    Rebind,
    History,
    AllowDelegation,
    RevokeDelegation,
    Suspend,
    Resume,
    Workflow,
    Chain,
}

#[derive(Component)]
struct Readout;

struct Discovery {
    health: ProviderHealthReport,
    support: ProviderLifecycleSupport,
    models: Result<Vec<DiscoveredModel>, vespera_providers::ProviderError>,
}

type ProbeResult = Result<Discovery, &'static str>;

enum Edit {
    Endpoint(String),
    Credential(String),
    Registration {
        model: Box<DiscoveredModel>,
        fields: [String; 4],
        index: usize,
        rebind: Option<AgentId>,
    },
    Identity {
        fields: [String; 2],
        index: usize,
    },
    MemoryPhrase(String),
    Prompt(String, Option<String>),
    Workflow {
        fields: [String; 3],
        index: usize,
    },
    Chain(ChainDraft),
}

#[derive(Default)]
struct ChainDraft {
    count: Option<usize>,
    steps: Vec<StepRequest>,
    revision: Option<usize>,
    assignee: Option<usize>,
    pending_prompt: Option<String>,
    input: String,
}

impl ChainDraft {
    fn reviewing(&self) -> bool {
        self.revision.is_none() && self.count == Some(self.steps.len())
    }

    fn step_index(&self) -> usize {
        self.revision.unwrap_or(self.steps.len())
    }

    fn revise(&mut self, index: usize) {
        let Some(step) = self.steps.get(index) else {
            return;
        };
        self.revision = Some(index);
        if index == 0 {
            self.assignee = Some(step.agent);
            self.input.clone_from(&step.prompt);
        } else {
            self.input = (step.agent + 1).to_string();
        }
    }

    fn revision_inputs(&self) -> String {
        let Some(index) = self.revision else {
            return String::new();
        };
        let numbers = self.steps[index]
            .dependencies
            .iter()
            .filter_map(|name| name.strip_prefix("step-"))
            .collect::<Vec<_>>();
        if numbers.is_empty() {
            "0".into()
        } else {
            numbers.join(",")
        }
    }

    fn record_step(&mut self, prompt: String, dependencies: Vec<String>) {
        let index = self.step_index();
        let step = StepRequest {
            name: format!("step-{}", index + 1),
            agent: self.assignee.take().expect("prompt has an assigned agent"),
            prompt,
            dependencies,
        };
        if self.revision.take().is_some() {
            self.steps[index] = step;
        } else {
            self.steps.push(step);
        }
        self.input.clear();
    }

    fn cancel_revision(&mut self) -> bool {
        if self.revision.take().is_none() {
            return false;
        }
        self.assignee = None;
        self.pending_prompt = None;
        self.input.clear();
        true
    }
}

enum Work {
    Discovery {
        result: Mutex<Receiver<ProbeResult>>,
        cancel: CancellationToken,
    },
    SaveRoster(Mutex<Receiver<RegistrationResult>>, &'static str),
    SaveDiscovery(Mutex<Receiver<Result<(), &'static str>>>),
    Hold(
        Mutex<Receiver<Result<(), AgentTaskStoreError>>>,
        &'static str,
    ),
    History(Mutex<Receiver<Result<Vec<StoredTaskSummary>, AgentTaskStoreError>>>),
    Inspect(Mutex<Receiver<Result<StoredAgentTask, AgentTaskStoreError>>>),
    Remember(Mutex<Receiver<Result<(), String>>>),
    Task {
        result: Mutex<Receiver<Result<FinishedTask, String>>>,
        cancel: CancellationToken,
    },
    Workflow {
        result: Mutex<Receiver<Result<FinishedWorkflow, String>>>,
        cancel: CancellationToken,
    },
}

type RegistrationResult = Result<(Roster, usize), &'static str>;

#[derive(Default)]
struct History {
    page: u32,
    tasks: Vec<StoredTaskSummary>,
    completed: Option<TaskId>,
}

#[derive(Resource)]
pub(super) struct Panel {
    kind: ProviderKind,
    endpoint: String,
    credential: Option<String>,
    credential_windows: bool,
    draft: Option<Edit>,
    pending_copy: Option<ChainDraft>,
    outcome: Option<ProbeResult>,
    page: usize,
    dir: Option<PathBuf>,
    roster: Option<Roster>,
    selected: usize,
    rebind: bool,
    work: Option<Work>,
    closing: bool,
    feedback: String,
    history: Option<History>,
}

impl Panel {
    pub(super) fn resume_workflow(&mut self, plan: Workflow) -> Result<(), &'static str> {
        if self.work.is_some() || self.closing {
            return Err("Wait for the current provider operation to finish.");
        }
        self.start_workflow(
            move |runtime, dir, roster, cancel| {
                runtime.block_on(desktop_workflow::resume(&dir, roster, plan, cancel))
            },
            "Checking the retained workflow and continuing from its committed steps.",
        )
    }

    fn start_workflow(
        &mut self,
        operation: impl FnOnce(
            tokio::runtime::Runtime,
            PathBuf,
            Roster,
            CancellationToken,
        ) -> Result<FinishedWorkflow, String>
        + Send
        + 'static,
        feedback: &str,
    ) -> Result<(), &'static str> {
        let (Some(dir), Some(roster)) = (self.dir.clone(), self.roster.clone()) else {
            return Err("Set up the agent roster first.");
        };
        let cancel = CancellationToken::new();
        let workflow_cancel = cancel.clone();
        let result = background(move |runtime| operation(runtime, dir, roster, workflow_cancel));
        self.pending_copy = None;
        self.draft = None;
        self.history = None;
        self.work = Some(Work::Workflow { result, cancel });
        self.feedback = feedback.into();
        Ok(())
    }

    pub(super) fn queue_workflow_copy(&mut self, plan: &Workflow) -> Result<(), &'static str> {
        if self.work.is_some() || self.closing {
            return Err("Wait for the current provider operation to finish.");
        }
        let roster = self
            .roster
            .as_ref()
            .ok_or("Set up the agent roster first.")?;
        let unsupported = "This checkpoint is outside the desktop editor's workflow limits.";
        if !(2..=MAX_CHAIN_STEPS).contains(&plan.steps.len())
            || plan.steps[0].candidates.as_slice() != [plan.coordinator]
            || !matches!(&plan.output, OutputPolicy::Step { step } if step == &plan.steps[plan.steps.len() - 1].name)
        {
            return Err(unsupported);
        }
        let mut steps = Vec::with_capacity(plan.steps.len());
        for (index, step) in plan.steps.iter().enumerate() {
            let [agent] = step.candidates.as_slice() else {
                return Err(unsupported);
            };
            if step.prompt.chars().count() > PROMPT_LIMIT {
                return Err(unsupported);
            }
            let agent = roster
                .config()
                .agents
                .iter()
                .position(|entry| entry.identity.id == *agent)
                .ok_or("An assigned agent is missing from the current roster.")?;
            let dependencies = step
                .dependencies
                .iter()
                .map(|name| {
                    plan.steps[..index]
                        .iter()
                        .position(|earlier| earlier.name == *name)
                        .map(|input| format!("step-{}", input + 1))
                        .ok_or(unsupported)
                })
                .collect::<Result<_, _>>()?;
            steps.push(StepRequest {
                name: format!("step-{}", index + 1),
                agent,
                prompt: step.prompt.clone(),
                dependencies,
            });
        }
        self.pending_copy = Some(ChainDraft {
            count: Some(steps.len()),
            steps,
            ..default()
        });
        Ok(())
    }
}

pub(super) fn install(app: &mut App, dir: Option<PathBuf>) {
    atlas::register(app, Layer::Providers);
    let restored = dir.as_deref().map_or(Ok(None), desktop_agent::load);
    let (roster, mut feedback) = match restored {
        Ok(roster) => (roster, String::new()),
        Err(error) => (None, error.into()),
    };
    let saved = match dir.as_deref().map_or(Ok(None), desktop_settings::load) {
        Ok(saved) => saved,
        Err(error) => {
            if !feedback.is_empty() {
                feedback.push('\n');
            }
            feedback.push_str(error);
            None
        }
    };
    let (kind, endpoint, credential, credential_windows) = saved
        .as_ref()
        .or_else(|| {
            roster
                .as_ref()?
                .selection(0)
                .map(|(_, _, provider)| provider)
        })
        .map_or_else(
            || (ProviderKind::Ollama, OLLAMA.into(), None, false),
            |provider| {
                (
                    provider.kind,
                    provider.endpoint.clone(),
                    provider.credential_env.clone(),
                    provider.credential_windows,
                )
            },
        );
    app.insert_resource(Panel {
        kind,
        endpoint,
        credential,
        credential_windows,
        draft: None,
        pending_copy: None,
        outcome: None,
        page: 0,
        dir,
        roster,
        selected: 0,
        rebind: false,
        work: None,
        closing: false,
        feedback,
        history: None,
    })
    .add_systems(Startup, setup)
    .add_systems(Update, (keyboard, receive_work, readout).chain())
    .add_systems(Last, close_after_provider_work.before(ExitSystems));
}

fn setup(mut commands: Commands) {
    #[cfg(target_os = "windows")]
    let windows_credential_button = button(Action::WindowsCredential, "Windows credential");
    #[cfg(not(target_os = "windows"))]
    let windows_credential_button = Node {
        display: Display::None,
        ..default()
    };
    let camera = commands
        .spawn((
            Layer::Providers,
            Camera2d,
            Camera {
                is_active: false,
                ..default()
            },
        ))
        .id();
    commands
        .spawn((
            UiTargetCamera(camera),
            Node {
                width: percent(100),
                height: percent(100),
                padding: UiRect::all(px(32)),
                flex_direction: FlexDirection::Column,
                row_gap: px(15),
                ..default()
            },
            children![
                (
                    Text::new("Models & Providers"),
                    TextFont::from_font_size(34.0)
                ),
                (
                    Text::new("Enter  Discover  |  S  Save setup  |  P  Run task  |  G  Draft + review  |  C  Workflow  |  Esc  Cancel  |  F5  Home"),
                    TextFont::from_font_size(16.0),
                ),
                (
                    Node {
                        column_gap: px(10),
                        row_gap: px(10),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    },
                    children![
                        button(Action::Ollama, "Ollama"),
                        button(Action::Compatible, "OpenAI-compatible"),
                        button(Action::Edit, "Edit endpoint"),
                        button(Action::Credential, "Credential reference"),
                        button(Action::Discover, "Test & discover"),
                        button(Action::SaveDiscovery, "Save setup"),
                    ]
                ),
                (
                    Node {
                        column_gap: px(10),
                        row_gap: px(10),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    },
                    children![
                        button(Action::PreviousAgent, "Previous agent"),
                        button(Action::NextAgent, "Next agent"),
                        button(Action::NewAgent, "New agent"),
                        button(Action::Rebind, "Rebind selected"),
                        button(Action::History, "Task history"),
                        button(Action::Workflow, "Draft + review"),
                    ]
                ),
                (
                    Node {
                        column_gap: px(10),
                        row_gap: px(10),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    },
                    children![
                        button(Action::Suspend, "Suspend"),
                        button(Action::Resume, "Resume"),
                        button(Action::AllowDelegation, "Allow delegation"),
                        button(Action::RevokeDelegation, "Revoke delegation"),
                        button(Action::Previous, "Previous page"),
                        button(Action::Next, "Next page"),
                        button(Action::Prompt, "Run agent task"),
                        button(Action::Chain, "Build workflow"),
                        button(Action::Cancel, "Cancel work"),
                        windows_credential_button,
                        button(Action::EditIdentity, "Edit agent identity"),
                        button(Action::ToggleMemoryRead, "Toggle memory read"),
                        button(Action::ToggleMemoryWrite, "Toggle memory write"),
                        button(Action::MemoryPrompt, "Run with memory"),
                        button(Action::Remember, "Remember task"),
                    ]
                ),
                (Readout, Text::default(), TextFont::from_font_size(16.0)),
            ],
        ))
        .observe(
            |event: On<PointerClick>,
             layers: Res<Layers>,
             actions: Query<&Action>,
             mut panel: ResMut<Panel>| {
                if layers.active == Layer::Providers
                    && event.button == PointerButton::Primary
                    && let Ok(&action) = actions.get(event.original_event_target())
                {
                    apply(action, &mut panel);
                }
            },
        );
}

fn apply(action: Action, panel: &mut Panel) {
    if panel.closing {
        return;
    }
    if matches!(action, Action::Cancel) {
        if let Some(
            Work::Discovery { cancel, .. }
            | Work::Task { cancel, .. }
            | Work::Workflow { cancel, .. },
        ) = &panel.work
        {
            cancel.cancel();
            panel.feedback = "Cancellation requested. Waiting for local cleanup.".into();
        }
        return;
    }
    if panel.draft.is_some() || panel.work.is_some() {
        return;
    }
    if panel.history.is_some()
        && !matches!(
            action,
            Action::Previous
                | Action::Next
                | Action::PreviousAgent
                | Action::NextAgent
                | Action::History
                | Action::Remember
        )
    {
        panel.history = None;
    }
    match action {
        Action::Ollama | Action::Compatible => {
            let (kind, endpoint) = match action {
                Action::Ollama => (ProviderKind::Ollama, OLLAMA),
                _ => (ProviderKind::OpenAiCompatible, COMPATIBLE),
            };
            if panel.kind != kind || panel.endpoint != endpoint {
                panel.credential = None;
                panel.credential_windows = false;
            }
            panel.kind = kind;
            panel.endpoint = endpoint.into();
            panel.outcome = None;
            panel.page = 0;
        }
        Action::Edit => panel.draft = Some(Edit::Endpoint(panel.endpoint.clone())),
        Action::Credential => {
            panel.feedback.clear();
            panel.draft = Some(Edit::Credential(
                panel.credential.clone().unwrap_or_default(),
            ));
        }
        #[cfg(target_os = "windows")]
        Action::WindowsCredential => {
            panel.credential_windows = !panel.credential_windows;
            panel.credential = None;
            panel.outcome = None;
            panel.page = 0;
            panel.feedback.clear();
        }
        Action::Prompt | Action::MemoryPrompt => {
            match panel
                .roster
                .as_ref()
                .and_then(|roster| roster.selection(panel.selected))
            {
                Some((agent, _, _)) if matches!(action, Action::MemoryPrompt) => {
                    if agent.permissions.contains(&Permission::MemoryRead) {
                        panel.feedback.clear();
                        panel.draft = Some(Edit::MemoryPhrase(String::new()));
                    } else {
                        panel.feedback = "Grant memory read to the selected agent first.".into();
                    }
                }
                Some(_) => panel.draft = Some(Edit::Prompt(String::new(), None)),
                None => {
                    panel.feedback =
                        "Discover a model and press its number to register it and bind an agent."
                            .into()
                }
            }
        }
        Action::Remember => remember_task(panel),
        Action::Cancel => {}
        Action::Workflow | Action::Chain => {
            let Some(roster) = &panel.roster else {
                return;
            };
            if matches!(action, Action::Workflow) && roster.len() < 2 {
                panel.feedback = "Create two agents before starting a workflow.".into();
            } else if roster
                .selection(panel.selected)
                .is_some_and(|(agent, _, _)| agent.permissions.contains(&Permission::Delegate))
            {
                panel.feedback.clear();
                panel.draft = Some(match action {
                    Action::Workflow => Edit::Workflow {
                        fields: Default::default(),
                        index: 0,
                    },
                    _ => Edit::Chain(ChainDraft::default()),
                });
            } else {
                panel.feedback = "Allow delegation for the selected coordinator first.".into();
            }
        }
        Action::AllowDelegation
        | Action::RevokeDelegation
        | Action::ToggleMemoryRead
        | Action::ToggleMemoryWrite => {
            let (Some(dir), Some(roster)) = (panel.dir.clone(), panel.roster.clone()) else {
                return;
            };
            let selected = panel.selected;
            let Some((agent, _, _)) = roster.selection(selected) else {
                return;
            };
            let (permission, allow, message) = match action {
                Action::AllowDelegation => (
                    Permission::Delegate,
                    true,
                    "Delegation granted to the selected agent.",
                ),
                Action::RevokeDelegation => (Permission::Delegate, false, "Delegation revoked."),
                Action::ToggleMemoryRead => (
                    Permission::MemoryRead,
                    !agent.permissions.contains(&Permission::MemoryRead),
                    "Memory read permission updated.",
                ),
                _ => (
                    Permission::MemoryWrite,
                    !agent.permissions.contains(&Permission::MemoryWrite),
                    "Memory write permission updated.",
                ),
            };
            panel.work = Some(Work::SaveRoster(
                background(move |_| {
                    desktop_agent::set_permission(&dir, roster, selected, permission, allow)
                }),
                message,
            ));
        }
        Action::Suspend | Action::Resume => {
            let Some((_, owner)) = history_source(panel) else {
                return;
            };
            let Some(dir) = panel.dir.clone() else {
                return;
            };
            let hold = matches!(action, Action::Suspend);
            panel.work = Some(Work::Hold(
                background(move |_| desktop_agent::set_hold(&dir, owner, hold)),
                if hold {
                    "Agent suspended. New tasks are blocked until resumed."
                } else {
                    "Agent hold cleared."
                },
            ));
        }
        Action::History => {
            panel.feedback.clear();
            if panel.history.take().is_none() {
                open_history(panel, 0);
            }
        }
        Action::PreviousAgent | Action::NextAgent => {
            let Some(roster) = &panel.roster else {
                return;
            };
            panel.selected = match action {
                Action::PreviousAgent => panel.selected.saturating_sub(1),
                _ => (panel.selected + 1).min(roster.len() - 1),
            };
            panel.rebind = false;
            panel.feedback.clear();
            if panel.history.is_some() {
                open_history(panel, 0);
            }
        }
        Action::NewAgent => {
            panel.rebind = false;
            panel.feedback = "Select a discovered model number to create a new agent.".into();
        }
        Action::EditIdentity => {
            if let Some((agent, _, _)) = panel
                .roster
                .as_ref()
                .and_then(|roster| roster.selection(panel.selected))
            {
                panel.feedback.clear();
                panel.draft = Some(Edit::Identity {
                    fields: [agent.identity.name.clone(), agent.identity.role.clone()],
                    index: 0,
                });
            } else {
                panel.feedback = "Select a saved agent to edit its identity.".into();
            }
        }
        Action::Rebind => {
            panel.rebind = panel.roster.is_some();
            if panel.rebind {
                panel.feedback =
                    "Select a discovered model number to rebind the selected agent.".into();
            }
        }
        Action::SaveDiscovery => {
            let Some(dir) = panel.dir.clone() else {
                panel.feedback =
                    "Saving discovery settings requires a local data directory.".into();
                return;
            };
            let settings = panel_provider(panel);
            panel.work = Some(Work::SaveDiscovery(background(move |_| {
                desktop_settings::save(&dir, &settings)
            })));
            panel.feedback = "Saving discovery settings...".into();
        }
        Action::Discover => {
            let declaration = panel_provider(panel);
            let cancel = CancellationToken::new();
            let probe_cancel = cancel.clone();
            let result = background(move |runtime| {
                let discovery = runtime.block_on(discover(declaration, probe_cancel));
                discovery.map_err(|error| match error {
                    RuntimeError::Config(_) =>
                        "Endpoint rejected. Enter a loopback HTTP or HTTPS URL.",
                    RuntimeError::Cancelled | RuntimeError::Provider(ProviderError::Cancelled { .. }) =>
                        "Provider discovery cancelled.",
                    RuntimeError::Provider(ProviderError::SecretResolutionFailed { .. }) =>
                        "Credential reference unavailable. Check the variable in the launch environment.",
                    RuntimeError::WindowsCredentialUnavailable { .. } =>
                        "Windows credential unavailable. Check the generic credential target shown above.",
                    RuntimeError::Provider(ProviderError::AuthenticationFailed { .. }) =>
                        "Provider authentication failed. Check the configured credential reference.",
                    _ => "Could not verify provider health. Check the endpoint and server.",
                })
            });
            panel.work = Some(Work::Discovery { result, cancel });
            panel.outcome = None;
            panel.page = 0;
            panel.feedback = "Checking health and discovering models...".into();
        }
        Action::Previous => {
            if let Some(history) = &panel.history {
                if history.page > 0 {
                    open_history(panel, history.page - 1);
                }
            } else {
                panel.page = panel.page.saturating_sub(1);
            }
        }
        Action::Next => {
            if let Some(history) = &panel.history {
                if history.tasks.len() > TASKS_PER_PAGE as usize {
                    open_history(panel, history.page + 1);
                }
            } else if let Some(Ok(result)) = &panel.outcome
                && let Ok(models) = &result.models
                && (panel.page + 1) * MODELS_PER_PAGE < models.len()
            {
                panel.page += 1;
            }
        }
    }
}

fn panel_provider(panel: &Panel) -> ProviderConfig {
    let mut provider = desktop_agent::provider(panel.kind, panel.endpoint.clone());
    provider.credential_env = panel.credential.clone();
    provider.credential_windows = panel.credential_windows;
    provider
}

async fn discover(
    mut declaration: ProviderConfig,
    cancel: CancellationToken,
) -> Result<Discovery, RuntimeError> {
    declaration.request_timeout_seconds = 5;
    let id = declaration.id;
    let config = desktop_agent::config(declaration);
    let registry = configured_registry(&config).await?;
    let provider = registry.get_provider(id).await?;
    let health = provider.health_check(cancel.clone()).await?;
    let support = provider.lifecycle_support();
    let models = provider.enumerate_models(cancel.clone()).await;
    if cancel.is_cancelled() {
        return Err(RuntimeError::Cancelled);
    }
    Ok(Discovery {
        health,
        support,
        models,
    })
}

fn keyboard(
    mut input: MessageReader<KeyboardInput>,
    mut focus_lost: MessageReader<KeyboardFocusLost>,
    mut modifiers: Local<ButtonInput<KeyCode>>,
    mut clipboard: Option<ResMut<Clipboard>>,
    layers: Res<Layers>,
    mut panel: ResMut<Panel>,
) {
    if focus_lost.read().count() != 0 {
        modifiers.reset_all();
        input.clear();
    }
    if layers.active != Layer::Providers || panel.closing {
        if panel.draft.is_some() {
            panel.draft = None;
        }
        input.clear();
        modifiers.reset_all();
        return;
    }
    if let Some(draft) = panel.pending_copy.take() {
        panel.selected = draft.steps[0].agent;
        panel.rebind = false;
        panel.history = None;
        panel.feedback = "Editing a copy of the retained workflow.".into();
        panel.draft = Some(Edit::Chain(draft));
        input.clear();
    }
    let primary = if cfg!(target_os = "macos") {
        [KeyCode::SuperLeft, KeyCode::SuperRight]
    } else {
        [KeyCode::ControlLeft, KeyCode::ControlRight]
    };
    for event in input.read() {
        if primary.contains(&event.key_code)
            || matches!(event.key_code, KeyCode::AltLeft | KeyCode::AltRight)
        {
            match event.state {
                ButtonState::Pressed => modifiers.press(event.key_code),
                ButtonState::Released => modifiers.release(event.key_code),
            }
            continue;
        }
        if event.state != ButtonState::Pressed {
            continue;
        }
        // Read the modifier events in order. The press and release can share one frame.
        if modifiers.any_pressed(primary)
            && !modifiers.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
        {
            if event.key_code == KeyCode::KeyV
                && !event.repeat
                && let Some(edit) = panel.draft.as_mut()
            {
                paste_prompt(edit, || {
                    clipboard.as_mut()?.fetch_text().poll_result()?.ok()
                });
            }
            continue;
        }
        if panel.draft.is_none() {
            if !event.repeat {
                if event.key_code == KeyCode::Escape {
                    apply(Action::Cancel, &mut panel);
                } else if event.key_code == KeyCode::Enter {
                    apply(Action::Discover, &mut panel);
                } else if event.key_code == KeyCode::KeyS {
                    apply(Action::SaveDiscovery, &mut panel);
                } else if event.key_code == KeyCode::KeyP {
                    apply(Action::Prompt, &mut panel);
                } else if event.key_code == KeyCode::KeyG {
                    apply(Action::Workflow, &mut panel);
                } else if event.key_code == KeyCode::KeyC {
                    apply(Action::Chain, &mut panel);
                } else if event.key_code == KeyCode::KeyI {
                    apply(Action::EditIdentity, &mut panel);
                } else if let Some(index) =
                    NUMBER_KEYS.iter().position(|key| *key == event.key_code)
                {
                    if panel.history.is_some() {
                        inspect_history(&mut panel, index);
                    } else {
                        select_model(&mut panel, index);
                    }
                }
            }
            continue;
        }
        match event.key_code {
            KeyCode::Enter if !event.repeat => submit(&mut panel),
            KeyCode::Escape => {
                if let Some(Edit::Chain(draft)) = panel.draft.as_mut()
                    && draft.cancel_revision()
                {
                    panel.feedback.clear();
                } else {
                    panel.draft = None;
                }
            }
            _ => {
                if let Some(edit) = panel.draft.as_mut() {
                    update_edit(edit, event);
                }
            }
        }
    }
}

fn paste_prompt(edit: &mut Edit, read: impl FnOnce() -> Option<String>) {
    let prompt = match edit {
        Edit::Prompt(prompt, _) => prompt,
        Edit::Workflow {
            fields,
            index: index @ (1 | 2),
        } => &mut fields[*index],
        Edit::Chain(draft) if draft.assignee.is_some() && draft.pending_prompt.is_none() => {
            &mut draft.input
        }
        _ => return,
    };
    if let Some(text) = read() {
        prompt.extend(
            text.chars()
                .map(|c| {
                    if matches!(c, '\r' | '\n' | '\t') {
                        ' '
                    } else {
                        c
                    }
                })
                .filter(|c| !c.is_control())
                .take(PROMPT_LIMIT.saturating_sub(prompt.chars().count())),
        );
    }
}

fn open_history(panel: &mut Panel, page: u32) {
    let Some((path, owner)) = history_source(panel) else {
        panel.feedback = "Select a saved agent to inspect its task history.".into();
        return;
    };
    panel.history = Some(History {
        page,
        ..History::default()
    });
    panel.feedback = "Reading retained tasks...".into();
    panel.work = Some(Work::History(background(move |_| {
        if !path
            .try_exists()
            .map_err(|_| AgentTaskStoreError::Database)?
        {
            return Ok(vec![]);
        }
        AgentTaskStore::recent(&path, owner, page * TASKS_PER_PAGE, TASKS_PER_PAGE + 1)
    })));
}

fn history_source(panel: &Panel) -> Option<(PathBuf, AgentId)> {
    let (agent, _, _) = panel.roster.as_ref()?.selection(panel.selected)?;
    Some((
        panel.dir.as_ref()?.join("desktop-tasks.sqlite3"),
        agent.identity.id,
    ))
}

fn inspect_history(panel: &mut Panel, index: usize) {
    if panel.work.is_some() {
        return;
    }
    let Some((path, owner)) = history_source(panel) else {
        return;
    };
    let Some(entry) = panel.history.as_mut().and_then(|history| {
        history.completed = None;
        history.tasks.get(index)
    }) else {
        return;
    };
    let id = entry.id;
    panel.feedback = "Reading task checkpoint...".into();
    panel.work = Some(Work::Inspect(background(move |_| {
        let task = AgentTaskStore::inspect(&path, id)?;
        if task.owner != owner {
            return Err(AgentTaskStoreError::InvalidCheckpoint);
        }
        Ok(task)
    })));
}

fn remember_task(panel: &mut Panel) {
    let Some(task) = panel.history.as_ref().and_then(|history| history.completed) else {
        panel.feedback = "Open Task history and select a completed task first.".into();
        return;
    };
    let (Some(dir), Some(roster), Some((_, owner))) =
        (panel.dir.clone(), &panel.roster, history_source(panel))
    else {
        return;
    };
    let revision = roster.revision;
    panel.work = Some(Work::Remember(background(move |_| {
        desktop_agent::remember(&dir, revision, owner, task)
    })));
}

fn select_model(panel: &mut Panel, index: usize) {
    if panel.work.is_some() {
        return;
    }
    if panel.dir.is_none() {
        panel.feedback = "A local desktop data directory is unavailable.".into();
        return;
    }
    if let Some(Ok(discovery)) = &panel.outcome
        && let Ok(models) = &discovery.models
        && let Some(model) = models.get(panel.page * MODELS_PER_PAGE + index)
    {
        panel.feedback.clear();
        let previous = panel
            .roster
            .as_ref()
            .and_then(|roster| roster.selection(panel.selected));
        let target = previous.filter(|_| panel.rebind);
        let mut fields = [
            target.map_or_else(String::new, |(agent, _, _)| agent.identity.name.clone()),
            target.map_or_else(String::new, |(agent, _, _)| agent.identity.role.clone()),
            String::new(),
            String::new(),
        ];
        if let Some((_, saved, provider)) = previous
            && saved.identifier == model.provider_model_name
            && provider.kind == panel.kind
            && provider.endpoint == panel.endpoint
            && provider.credential_env == panel.credential
            && provider.credential_windows == panel.credential_windows
        {
            fields[2] = saved
                .resources
                .ram_bytes
                .map_or_else(String::new, |bytes| (bytes / (1024 * 1024)).to_string());
            fields[3] = saved
                .resources
                .vram_bytes
                .map_or_else(String::new, |bytes| (bytes / (1024 * 1024)).to_string());
        }
        panel.draft = Some(Edit::Registration {
            model: Box::new(model.clone()),
            fields,
            index: 0,
            rebind: target.map(|(agent, _, _)| agent.identity.id),
        });
    }
}

fn update_edit(edit: &mut Edit, event: &KeyboardInput) {
    let credential = matches!(edit, Edit::Credential(_));
    let (field, limit, accepts): (&mut String, usize, fn(char) -> bool) = match edit {
        Edit::Endpoint(value) => (value, ENDPOINT_LIMIT, |c| {
            c.is_ascii_alphanumeric() || ":/.[_-]".contains(c)
        }),
        Edit::Credential(value) => (value, 128, |c| {
            c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'
        }),
        Edit::Registration {
            fields,
            index: index @ (0 | 1),
            ..
        } => (&mut fields[*index], 64, |c| !c.is_control()),
        Edit::Identity { fields, index } => (&mut fields[*index], 64, |c| !c.is_control()),
        Edit::Registration { fields, index, .. } => {
            (&mut fields[*index], 8, |c| c.is_ascii_digit())
        }
        Edit::Prompt(value, _) => (value, PROMPT_LIMIT, |c| !c.is_control()),
        Edit::MemoryPhrase(value) => (value, 256, |c| !c.is_control()),
        Edit::Workflow { fields, index: 0 } => (&mut fields[0], 2, |c| c.is_ascii_digit()),
        Edit::Workflow { fields, index } => {
            (&mut fields[*index], PROMPT_LIMIT, |c| !c.is_control())
        }
        Edit::Chain(draft) if draft.reviewing() => {
            if !event.repeat
                && let Some(index) = NUMBER_KEYS.iter().position(|key| *key == event.key_code)
            {
                draft.revise(index);
            }
            return;
        }
        Edit::Chain(draft) if draft.pending_prompt.is_some() => (&mut draft.input, 24, |c| {
            c.is_ascii_digit() || c == ',' || c == ' '
        }),
        Edit::Chain(draft) if draft.assignee.is_none() => {
            (&mut draft.input, 2, |c| c.is_ascii_digit())
        }
        Edit::Chain(draft) => (&mut draft.input, PROMPT_LIMIT, |c| !c.is_control()),
    };
    match event.key_code {
        KeyCode::Backspace => {
            field.pop();
        }
        KeyCode::Delete => field.clear(),
        _ => {
            if let Some(text) = &event.text {
                field.extend(
                    text.chars()
                        .map(|c| {
                            if credential {
                                c.to_ascii_uppercase()
                            } else {
                                c
                            }
                        })
                        .filter(|&c| accepts(c))
                        .take(limit.saturating_sub(field.chars().count())),
                );
            }
        }
    }
}

fn submit(panel: &mut Panel) {
    let Some(edit) = panel.draft.take() else {
        return;
    };
    match edit {
        Edit::Endpoint(endpoint) => {
            let declaration = desktop_agent::provider(panel.kind, endpoint.clone());
            if desktop_agent::config(declaration).validate().is_err() {
                panel.feedback = "Endpoint rejected. Enter a loopback HTTP or HTTPS URL.".into();
                panel.draft = Some(Edit::Endpoint(endpoint));
                return;
            }
            if panel.endpoint != endpoint {
                panel.credential = None;
                panel.credential_windows = false;
            }
            panel.endpoint = endpoint;
            panel.outcome = None;
            panel.page = 0;
            panel.feedback.clear();
        }
        Edit::Credential(name) => {
            if !name.is_empty() && !desktop_agent::credential_name(&name) {
                panel.feedback =
                    "Enter a VESPERA_ environment variable name or clear the field.".into();
                panel.draft = Some(Edit::Credential(name));
                return;
            }
            panel.credential = (!name.is_empty()).then_some(name);
            panel.credential_windows = false;
            panel.outcome = None;
            panel.page = 0;
            panel.feedback.clear();
        }
        Edit::Registration {
            model,
            fields,
            mut index,
            rebind,
        } => {
            let valid = match index {
                0 | 1 => !fields[index].trim().is_empty(),
                2 => fields[2].parse::<u32>().is_ok_and(|ram| ram > 0),
                _ => fields[3].parse::<u32>().is_ok(),
            };
            if !valid || index < 3 {
                panel.feedback = if valid {
                    String::new()
                } else {
                    "Enter a valid value to continue.".into()
                };
                index += usize::from(valid);
                panel.draft = Some(Edit::Registration {
                    model,
                    fields,
                    index,
                    rebind,
                });
                return;
            }
            if let Some(dir) = panel.dir.clone() {
                let [agent_name, role, ram, vram] = fields;
                let registration = Registration {
                    provider: panel_provider(panel),
                    model: *model,
                    agent_name,
                    role,
                    rebind,
                    ram_mib: ram.parse().expect("checked RAM input"),
                    vram_mib: vram.parse().expect("checked VRAM input"),
                };
                let previous = panel.roster.clone();
                let receiver = background(move |runtime| {
                    runtime.block_on(desktop_agent::register(
                        &dir,
                        previous.as_ref(),
                        registration,
                    ))
                });
                panel.work = Some(Work::SaveRoster(
                    receiver,
                    "Registered the model and saved the agent binding.",
                ));
                panel.feedback = "Validating the model and saving the agent binding...".into();
            }
        }
        Edit::Identity { fields, index } => {
            if fields[index].trim().is_empty() {
                panel.feedback = "Enter a nonempty value.".into();
                panel.draft = Some(Edit::Identity { fields, index });
            } else if index == 0 {
                panel.feedback.clear();
                panel.draft = Some(Edit::Identity { fields, index: 1 });
            } else if let (Some(dir), Some(roster)) = (panel.dir.clone(), panel.roster.clone()) {
                let [name, role] = fields;
                let selected = panel.selected;
                panel.work = Some(Work::SaveRoster(
                    background(move |_| {
                        desktop_agent::set_identity(&dir, roster, selected, name, role)
                    }),
                    "Agent name and role saved.",
                ));
                panel.feedback = "Saving agent identity...".into();
            }
        }
        Edit::MemoryPhrase(phrase) => {
            if phrase.trim().is_empty()
                || phrase.len() > 256
                || phrase.split_whitespace().count() > 32
            {
                panel.feedback = "Enter a phrase of 1 to 256 bytes and at most 32 words.".into();
                panel.draft = Some(Edit::MemoryPhrase(phrase));
            } else {
                panel.feedback.clear();
                panel.draft = Some(Edit::Prompt(String::new(), Some(phrase)));
            }
        }
        Edit::Prompt(prompt, memory_phrase) => {
            if prompt.trim().is_empty() {
                panel.feedback = "Enter a prompt before starting a task.".into();
                panel.draft = Some(Edit::Prompt(prompt, memory_phrase));
            } else if let (Some(dir), Some(roster)) = (panel.dir.clone(), panel.roster.clone()) {
                let cancel = CancellationToken::new();
                let task_cancel = cancel.clone();
                let selected = panel.selected;
                let result = background(move |runtime| {
                    runtime.block_on(desktop_agent::run(
                        &dir,
                        roster,
                        selected,
                        prompt,
                        memory_phrase,
                        task_cancel,
                    ))
                });
                panel.work = Some(Work::Task { result, cancel });
                panel.feedback = "Task in progress. Waiting for the provider and scheduler.".into();
            }
        }
        Edit::Workflow { fields, index } => submit_workflow(panel, fields, index),
        Edit::Chain(draft) => submit_chain(panel, draft),
    }
}

fn submit_workflow(panel: &mut Panel, fields: [String; 3], mut index: usize) {
    let Some(roster) = &panel.roster else {
        return;
    };
    let reviewer = fields[0]
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .filter(|&reviewer| reviewer < roster.len() && reviewer != panel.selected);
    let valid = if index == 0 {
        reviewer.is_some()
    } else {
        !fields[index].trim().is_empty()
    };
    if !valid || index < 2 {
        panel.feedback = if valid {
            String::new()
        } else {
            "Enter a valid reviewer or prompt.".into()
        };
        index += usize::from(valid);
        panel.draft = Some(Edit::Workflow { fields, index });
        return;
    }
    let [_, draft_prompt, review_prompt] = fields;
    start_workflow(
        panel,
        vec![
            StepRequest {
                name: "draft".into(),
                agent: panel.selected,
                prompt: draft_prompt,
                dependencies: vec![],
            },
            StepRequest {
                name: "review".into(),
                agent: reviewer.expect("validated reviewer index"),
                prompt: review_prompt,
                dependencies: vec!["draft".into()],
            },
        ],
    );
}

fn parse_predecessors(input: &str, earlier: usize) -> Option<Vec<String>> {
    if input.trim().is_empty() {
        return Some(
            earlier
                .checked_sub(1)
                .map(|index| format!("step-{}", index + 1))
                .into_iter()
                .collect(),
        );
    }
    if input.trim() == "0" {
        return Some(vec![]);
    }
    let mut numbers = std::collections::BTreeSet::new();
    for value in input.split(',') {
        let number = value.trim().parse::<usize>().ok()?;
        if number == 0 || number > earlier || !numbers.insert(number) {
            return None;
        }
    }
    Some(
        numbers
            .into_iter()
            .map(|number| format!("step-{number}"))
            .collect(),
    )
}

fn submit_chain(panel: &mut Panel, mut draft: ChainDraft) {
    let Some(roster) = &panel.roster else {
        return;
    };
    if draft.reviewing() {
        start_workflow(panel, draft.steps);
        return;
    }
    if let Some(prompt) = draft.pending_prompt.take() {
        match parse_predecessors(&draft.input, draft.step_index()) {
            Some(dependencies) => {
                draft.record_step(prompt, dependencies);
                panel.feedback.clear();
            }
            None => {
                draft.pending_prompt = Some(prompt);
                panel.feedback =
                    "Enter earlier step numbers separated by commas, or 0 for no dependencies."
                        .into();
            }
        }
        panel.draft = Some(Edit::Chain(draft));
        return;
    }
    let error = match (draft.count, draft.assignee) {
        (None, _) => match draft.input.parse::<usize>() {
            Ok(count) if (2..=MAX_CHAIN_STEPS).contains(&count) => {
                draft.count = Some(count);
                draft.assignee = Some(panel.selected);
                draft.input.clear();
                None
            }
            _ => Some("Enter a step count from 2 to 6."),
        },
        (Some(_), None) => {
            let candidate = draft
                .input
                .parse::<usize>()
                .ok()
                .and_then(|number| number.checked_sub(1));
            let coordinator_provider = roster
                .selection(panel.selected)
                .map(|(_, _, provider)| provider.id);
            match candidate.and_then(|index| {
                roster
                    .selection(index)
                    .map(|(_, _, provider)| (index, provider.id))
            }) {
                Some((index, provider)) if Some(provider) == coordinator_provider => {
                    draft.assignee = Some(index);
                    draft.input = draft
                        .revision
                        .map_or_else(String::new, |revision| draft.steps[revision].prompt.clone());
                    None
                }
                _ => Some("Choose a saved agent on the coordinator's provider."),
            }
        }
        (Some(_), Some(_)) if !draft.input.trim().is_empty() && draft.step_index() != 0 => {
            draft.pending_prompt = Some(std::mem::take(&mut draft.input));
            draft.input = draft.revision_inputs();
            None
        }
        (Some(_), Some(_)) if !draft.input.trim().is_empty() => {
            let prompt = std::mem::take(&mut draft.input);
            draft.record_step(prompt, vec![]);
            None
        }
        _ => Some("Enter a prompt for this step."),
    };
    panel.feedback = error.unwrap_or_default().into();
    panel.draft = Some(Edit::Chain(draft));
}

fn start_workflow(panel: &mut Panel, steps: Vec<StepRequest>) {
    if let Err(error) = panel.start_workflow(
        move |runtime, dir, roster, cancel| {
            runtime.block_on(desktop_workflow::run(&dir, roster, steps, cancel))
        },
        "Workflow in progress. Each step waits for its selected dependencies.",
    ) {
        panel.feedback = error.into();
    }
}

fn background<T: Send + 'static>(
    operation: impl FnOnce(tokio::runtime::Runtime) -> T + Send + 'static,
) -> Mutex<Receiver<T>> {
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            let _ = send.send(operation(runtime));
        }
    });
    Mutex::new(receive)
}

fn poll<T>(receiver: &Mutex<Receiver<T>>) -> Result<T, TryRecvError> {
    receiver
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .try_recv()
}

fn receive_work(mut panel: ResMut<Panel>) {
    match &panel.work {
        Some(Work::Discovery { result, .. }) => {
            panel.outcome = Some(match poll(result) {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err("The provider request stopped early."),
            });
            panel.feedback.clear();
        }
        Some(Work::SaveRoster(result, message)) => match poll(result) {
            Ok(Ok((roster, selected))) => {
                panel.feedback = (*message).into();
                panel.roster = Some(roster);
                panel.selected = selected;
                panel.rebind = false;
            }
            Ok(Err(error)) => panel.feedback = error.into(),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panel.feedback = "The registration worker stopped.".into()
            }
        },
        Some(Work::SaveDiscovery(result)) => match poll(result) {
            Ok(Ok(())) => panel.feedback = "Discovery setup saved for future launches.".into(),
            Ok(Err(error)) => panel.feedback = error.into(),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panel.feedback = "The discovery settings worker stopped.".into()
            }
        },
        Some(Work::Hold(result, message)) => match poll(result) {
            Ok(Ok(())) => panel.feedback = (*message).into(),
            Ok(Err(error)) => panel.feedback = error.to_string(),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panel.feedback = "The agent control worker stopped.".into()
            }
        },
        Some(Work::History(result)) => match poll(result) {
            Ok(Ok(tasks)) => {
                if let Some(history) = panel.history.as_mut() {
                    history.tasks = tasks;
                }
                panel.feedback.clear();
            }
            Ok(Err(error)) => panel.feedback = error.to_string(),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panel.feedback = "The history reader stopped.".into()
            }
        },
        Some(Work::Inspect(result)) => match poll(result) {
            Ok(Ok(task)) => {
                if let Some(history) = panel.history.as_mut() {
                    history.completed = task.result.as_ref().map(|_| task.task.id);
                }
                let prompt = preview(&task.task.prompt, 150);
                let response = task.result.as_ref().map_or_else(
                    || "No committed response".into(),
                    |result| preview(&result.response.content, 260),
                );
                panel.feedback = format!(
                    "Task {} | {}\nPrompt: {prompt}\nResponse: {response}",
                    task.task.id, task.task.state,
                );
            }
            Ok(Err(error)) => panel.feedback = error.to_string(),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => panel.feedback = "The task reader stopped.".into(),
        },
        Some(Work::Remember(result)) => match poll(result) {
            Ok(Ok(())) => {
                panel.feedback = "Saved the completed response to this agent's memory.".into()
            }
            Ok(Err(error)) => panel.feedback = error,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => panel.feedback = "The memory writer stopped.".into(),
        },
        Some(Work::Workflow { result, .. }) => match poll(result) {
            Ok(Ok(finished)) => panel.feedback = workflow_summary(finished, panel.roster.as_ref()),
            Ok(Err(error)) => panel.feedback = error,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panel.feedback = "The workflow worker stopped.".into()
            }
        },
        Some(Work::Task { result, .. }) => match poll(result) {
            Ok(Ok(task)) => {
                let preview: String = task
                    .response
                    .chars()
                    .filter(|c| !c.is_control() || *c == '\n')
                    .take(600)
                    .collect();
                panel.feedback = format!(
                    "Task {} completed. {}\n{preview}",
                    task.id,
                    if task.cleanup_confirmed {
                        "Local runtime drained."
                    } else {
                        "Cleanup was not confirmed."
                    }
                );
                if let Some(count) = task.memory_records {
                    use std::fmt::Write as _;
                    let _ = write!(panel.feedback, "\nMemory records included  |  {count}");
                }
            }
            Ok(Err(error)) => panel.feedback = error,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => panel.feedback = "The task worker stopped.".into(),
        },
        None => return,
    }
    panel.work = None;
}

fn close_after_provider_work(
    mut commands: Commands,
    mut requests: MessageReader<WindowCloseRequested>,
    mut primary: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut panel: ResMut<Panel>,
) {
    let requested = requests.read().any(|request| request.window == primary.0);
    if requested && !panel.closing {
        apply(Action::Cancel, &mut panel);
        panel.closing = true;
        panel.draft = None;
        if panel.work.is_some() {
            primary.1.title = "Vespera | Finishing local work…".into();
        }
    }
    if panel.closing && panel.work.is_none() {
        commands.entity(primary.0).despawn();
    }
}

fn workflow_summary(finished: FinishedWorkflow, roster: Option<&Roster>) -> String {
    use std::fmt::Write as _;

    let report = &finished.report;
    let mut summary = format!(
        "Workflow {} | {} | {} ms",
        report.task.id, report.task.state, report.elapsed_ms
    );
    if let Some(failure) = report.failure {
        let _ = write!(summary, "\nOutcome: {failure:?}");
    }
    for attempt in &report.attempts {
        let name = roster
            .and_then(|roster| {
                roster
                    .config()
                    .agents
                    .iter()
                    .find(|agent| agent.identity.id == attempt.agent_id)
            })
            .map_or("Unknown agent", |agent| agent.identity.name.as_str());
        let state = attempt
            .task_state
            .map_or_else(|| "No task state".into(), |state| state.to_string());
        let _ = write!(
            summary,
            "\n{} | {} | {}{}",
            attempt.step,
            name,
            state,
            if attempt.delegated {
                " | delegated"
            } else {
                ""
            }
        );
    }
    if let Some(result) = report.selected_result() {
        let _ = write!(
            summary,
            "\nResult: {}",
            preview(&result.response.content, 420)
        );
    }
    if !finished.cleanup_confirmed {
        summary.push_str("\nRuntime cleanup was not confirmed.");
    }
    if let Some(error) = finished.observation_error {
        let _ = write!(
            summary,
            "\nSwarm journal publication failed. Workflow evidence is retained. {error}"
        );
    } else {
        summary.push_str("\nF3 opens the committed observations in Swarm.");
    }
    summary
}

pub(super) fn preview(value: &str, limit: usize) -> String {
    value
        .split_whitespace()
        .flat_map(|word| word.chars().filter(|c| !c.is_control()).chain([' ']))
        .take(limit)
        .collect::<String>()
        .trim_end()
        .into()
}

fn readout(layers: Res<Layers>, panel: Res<Panel>, mut text: Single<&mut Text, With<Readout>>) {
    use std::fmt::Write as _;

    if layers.active != Layer::Providers || (!panel.is_changed() && !layers.is_changed()) {
        return;
    }
    let adapter = match panel.kind {
        ProviderKind::Ollama => "Ollama",
        _ => "OpenAI-compatible (including LM Studio Bionic)",
    };
    let endpoint = match &panel.draft {
        Some(Edit::Endpoint(endpoint)) => endpoint,
        _ => &panel.endpoint,
    };
    let credential = panel.credential.as_deref().unwrap_or("None");
    let mut content = format!(
        "Adapter  |  {adapter}\nEndpoint  |  {endpoint}\nCredential variable  |  {credential}\n"
    );
    if panel.credential_windows {
        let _ = writeln!(
            content,
            "Windows credential target  |  {}",
            windows_credential_target(&panel_provider(&panel))
        );
    }
    if let Some(roster) = &panel.roster
        && let Some((agent, model, provider)) = roster.selection(panel.selected)
    {
        let _ = writeln!(
            content,
            "Agent {} of {}  |  {}  |  {}  |  Delegation {}\nBound model  |  {}\nCapabilities  |  {}\nTask endpoint  |  {}",
            panel.selected + 1,
            roster.len(),
            agent.identity.name,
            agent.identity.role,
            if agent.permissions.contains(&Permission::Delegate) {
                "allowed"
            } else {
                "denied"
            },
            model.identifier.chars().take(90).collect::<String>(),
            preview(
                &model
                    .capabilities
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                90
            ),
            provider.endpoint
        );
        if let Some(name) = &provider.credential_env {
            let _ = writeln!(content, "Task credential variable  |  {name}");
        }
        let permission = |grant| {
            if agent.permissions.contains(&grant) {
                "allowed"
            } else {
                "denied"
            }
        };
        let _ = writeln!(
            content,
            "Memory read {}  |  Memory write {}",
            permission(Permission::MemoryRead),
            permission(Permission::MemoryWrite)
        );
        if provider.credential_windows {
            let _ = writeln!(content, "Task credential  |  Windows Credential Manager");
        }
    }
    if !panel.feedback.is_empty() {
        let _ = writeln!(content, "\n{}", panel.feedback);
    }
    if let Some(edit) = &panel.draft {
        match edit {
            Edit::Endpoint(_) => content.push_str("\nEnter applies the endpoint for this session."),
            Edit::Credential(name) => {
                let _ = write!(
                    content,
                    "\nEnvironment variable name  |  {name}_\nUse a VESPERA_ variable name. Enter saves. Delete clears."
                );
            }
            Edit::Registration {
                model,
                fields,
                index,
                rebind,
            } => {
                let labels = [
                    "Agent name",
                    "Agent role",
                    "Estimated model RAM in MiB (positive)",
                    "Estimated model VRAM in MiB (0 for CPU-only)",
                ];
                let _ = write!(
                    content,
                    "\n{} with {}\n{}: {}_\nThese operator estimates set the task's memory ceilings.\nEnter continues.",
                    if rebind.is_some() {
                        "Rebind agent"
                    } else {
                        "Create agent"
                    },
                    model
                        .provider_model_name
                        .chars()
                        .take(90)
                        .collect::<String>(),
                    labels[*index],
                    fields[*index]
                );
            }
            Edit::Identity { fields, index } => {
                let label = ["Agent name", "Agent role"][*index];
                let _ = write!(content, "\n{label}: {}_\nEnter continues.", fields[*index]);
            }
            Edit::Workflow { fields, index } => {
                if *index == 0
                    && let Some(roster) = &panel.roster
                {
                    content.push_str("\nChoose a different agent on the same provider:\n");
                    for (index, agent) in roster.config().agents.iter().enumerate() {
                        let _ = write!(
                            content,
                            "{}. {}{}",
                            index + 1,
                            agent.identity.name,
                            if index % 2 == 0 { "    " } else { "\n" }
                        );
                    }
                }
                let labels = ["Reviewer number", "Draft prompt", "Review instruction"];
                let _ = write!(
                    content,
                    "\n{}: {}_\n{}",
                    labels[*index],
                    fields[*index],
                    if *index == 2 {
                        "Enter runs the two steps. The reviewer receives the draft as untrusted input."
                    } else {
                        "Enter continues."
                    }
                );
            }
            Edit::Chain(draft) => {
                if draft.reviewing() {
                    content.push_str("\nReview this workflow before running it.\n");
                    for step in &draft.steps {
                        let owner = panel
                            .roster
                            .as_ref()
                            .and_then(|roster| roster.selection(step.agent))
                            .map_or("Unknown agent", |(agent, _, _)| {
                                agent.identity.name.as_str()
                            });
                        let _ = writeln!(
                            content,
                            "{} | {owner} | {} | inputs: {}",
                            step.name,
                            preview(&step.prompt, 40),
                            if step.dependencies.is_empty() {
                                "none".into()
                            } else {
                                step.dependencies.join(", ")
                            }
                        );
                    }
                    content.push_str("Press a step number to edit it, or Enter to run. Selected inputs are untrusted. The last step supplies the output.");
                } else if let Some(count) = draft.count {
                    let action = if draft.revision.is_some() {
                        "Editing step"
                    } else {
                        "Step"
                    };
                    let _ = writeln!(content, "\n{action} {} of {count}", draft.step_index() + 1);
                    if draft.pending_prompt.is_some() {
                        let _ = write!(
                            content,
                            "Inputs from earlier steps: {}_\nComma-separated step numbers; 0 means independent.\nBlank uses the previous step. Enter continues.",
                            draft.input
                        );
                    } else if let Some(index) = draft.assignee {
                        let owner = panel
                            .roster
                            .as_ref()
                            .and_then(|roster| roster.selection(index))
                            .map_or("Unknown agent", |(agent, _, _)| {
                                agent.identity.name.as_str()
                            });
                        let _ = write!(
                            content,
                            "Prompt for {owner}: {}_\nEnter continues.",
                            draft.input
                        );
                    } else {
                        if let Some(roster) = &panel.roster
                            && let Some((_, _, provider)) = roster.selection(panel.selected)
                        {
                            for (index, agent) in roster.config().agents.iter().enumerate() {
                                if roster
                                    .selection(index)
                                    .is_some_and(|(_, _, binding)| binding.id == provider.id)
                                {
                                    let _ = write!(
                                        content,
                                        "{}. {}   ",
                                        index + 1,
                                        agent.identity.name
                                    );
                                }
                            }
                        }
                        let _ = write!(
                            content,
                            "\nAgent number: {}_\nEnter continues.",
                            draft.input
                        );
                    }
                } else {
                    let _ = write!(
                        content,
                        "\nNumber of steps (2 to {MAX_CHAIN_STEPS}): {}_\nThe selected coordinator runs the first step. Enter continues.",
                        draft.input
                    );
                }
            }
            Edit::MemoryPhrase(phrase) => {
                let _ = write!(
                    content,
                    "\nMemory search phrase: {phrase}_\nEnter continues to the task prompt. Only this agent's matching records can be included."
                );
            }
            Edit::Prompt(prompt, memory_phrase) => {
                if memory_phrase.is_some() {
                    content.push_str("\nMatching memories will be passed to the bound provider as untrusted input.");
                }
                let _ = write!(
                    content,
                    "\nTask prompt: {prompt}_\nEnter runs one task (128 output tokens, 50-second task budget)."
                );
            }
        }
        content.push_str("\nEsc cancels the edit. Delete clears the field.");
    } else if let Some(history) = &panel.history {
        let _ = writeln!(
            content,
            "\nRetained tasks  |  Page {}  |  Press a task number for details",
            history.page + 1
        );
        if history.tasks.is_empty() && panel.work.is_none() {
            content.push_str("No checkpoints on this page.");
        }
        for (index, task) in history
            .tasks
            .iter()
            .take(TASKS_PER_PAGE as usize)
            .enumerate()
        {
            let _ = writeln!(
                content,
                "  {}. {}  {}{}",
                index + 1,
                task.id,
                task.state,
                if task.interrupted {
                    " (interrupted)"
                } else {
                    ""
                }
            );
        }
    } else if panel.work.is_none()
        && !panel.feedback.starts_with("Task ")
        && !panel.feedback.starts_with("Workflow ")
    {
        match &panel.outcome {
            None => {
                content.push_str("\nLoopback servers only. Select Test & discover or press Enter.")
            }
            Some(Err(error)) => content.push_str(error),
            Some(Ok(result)) => {
                let support = result.support;
                let _ = write!(
                    content,
                    "\nHealth  |  {:?}  |  {} ms\nAdapter support  |  Metadata {:?}  |  Load {:?}  |  Unload {:?}  |  Residency {:?}\n",
                    result.health.status,
                    result.health.latency_ms,
                    support.model_metadata,
                    support.explicit_model_load,
                    support.explicit_model_unload,
                    support.residency_telemetry,
                );
                match &result.models {
                    Err(_) => content.push_str(
                        "\nModel discovery failed. Check server permissions and protocol.",
                    ),
                    Ok(models) if models.is_empty() => {
                        content.push_str("\nThe server reported no models.")
                    }
                    Ok(models) => {
                        let _ = writeln!(
                            content,
                            "\n{} exposed models  |  Page {} of {}  |  Press a model number to register",
                            models.len(),
                            panel.page + 1,
                            models.len().div_ceil(MODELS_PER_PAGE)
                        );
                        for (index, model) in models
                            .iter()
                            .skip(panel.page * MODELS_PER_PAGE)
                            .take(MODELS_PER_PAGE)
                            .enumerate()
                        {
                            let name: String = model
                                .provider_model_name
                                .chars()
                                .filter(|c| !c.is_control())
                                .take(90)
                                .collect();
                            let _ = writeln!(content, "  {}. {name}", index + 1);
                        }
                    }
                }
            }
        }
    }
    if text.0 != content {
        text.0 = content;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::keyboard::Key;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use vespera_providers::{LifecycleCapabilityState, ProviderHealthStatus};

    async fn discover(declaration: ProviderConfig) -> Result<Discovery, RuntimeError> {
        super::discover(declaration, CancellationToken::new()).await
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_credential_selection_is_exclusive_and_clears_on_endpoint_edit() {
        let mut app = App::new();
        app.insert_resource(Layers::new(Layer::Providers));
        install(&mut app, None);
        let mut panel = app.world_mut().resource_mut::<Panel>();
        panel.credential = Some("VESPERA_FIXTURE".into());
        apply(Action::WindowsCredential, &mut panel);
        let configured = panel_provider(&panel);
        assert!(configured.credential_windows);
        assert!(configured.credential_env.is_none());
        assert!(windows_credential_target(&configured).starts_with("Vespera/provider/ollama/"));

        panel.draft = Some(Edit::Endpoint("http://127.0.0.1:8000".into()));
        submit(&mut panel);
        assert!(!panel_provider(&panel).credential_windows);
    }

    #[test]
    fn revising_a_step_preserves_other_steps_and_their_input_links() {
        let mut edit = Edit::Chain(ChainDraft {
            count: Some(3),
            steps: (1..=3)
                .map(|number| StepRequest {
                    name: format!("step-{number}"),
                    agent: 0,
                    prompt: format!("Original step {number}"),
                    dependencies: (number > 1)
                        .then(|| format!("step-{}", number - 1))
                        .into_iter()
                        .collect(),
                })
                .collect(),
            ..default()
        });

        update_edit(
            &mut edit,
            &KeyboardInput {
                key_code: KeyCode::Digit2,
                logical_key: Key::Character("2".into()),
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            },
        );
        let Edit::Chain(mut draft) = edit else {
            panic!("expected a workflow draft");
        };
        assert!(!draft.reviewing());
        assert_eq!(draft.input, "1");
        assert_eq!(draft.revision_inputs(), "1");
        assert!(parse_predecessors("2", draft.step_index()).is_none());
        draft.assignee = Some(1);
        draft.record_step("New prompt".into(), vec![]);
        assert!(draft.reviewing());
        assert_eq!(draft.steps[1].name, "step-2");
        assert_eq!(draft.steps[1].agent, 1);
        assert_eq!(draft.steps[1].prompt, "New prompt");
        assert!(draft.steps[1].dependencies.is_empty());
        assert_eq!(draft.steps[2].dependencies, ["step-2"]);

        draft.revise(0);
        assert_eq!(draft.input, "Original step 1");
        draft.input = "Discard this edit".into();
        assert!(draft.cancel_revision());
        assert!(draft.reviewing());
        assert_eq!(draft.steps[0].prompt, "Original step 1");
    }

    #[test]
    fn dependency_selection_rejects_forward_duplicate_and_invalid_inputs() {
        assert_eq!(parse_predecessors("", 2).unwrap(), ["step-2"]);
        assert_eq!(parse_predecessors("0", 2).unwrap(), Vec::<String>::new());
        assert_eq!(parse_predecessors("2, 1", 3).unwrap(), ["step-1", "step-2"]);
        for input in ["3", "1,1", "0,1", "1,", "-1", "text"] {
            assert!(parse_predecessors(input, 2).is_none());
        }
        let mut dependencies = Edit::Chain(ChainDraft {
            assignee: Some(0),
            pending_prompt: Some("Prompt".into()),
            ..default()
        });
        paste_prompt(&mut dependencies, || {
            panic!("dependency selection must not read clipboard")
        });
    }

    #[test]
    fn paste_only_reads_prompt_fields_and_respects_the_character_limit() {
        let mut prompt = Edit::Prompt("x".repeat(PROMPT_LIMIT - 4), None);
        paste_prompt(&mut prompt, || Some("é\n\u{0}β\tz".into()));
        assert!(
            matches!(prompt, Edit::Prompt(text, _) if text.chars().count() == PROMPT_LIMIT && text.ends_with("é β "))
        );

        let mut chain = Edit::Chain(ChainDraft {
            count: Some(2),
            assignee: Some(0),
            ..default()
        });
        paste_prompt(&mut chain, || Some("Use the preceding result".into()));
        assert!(matches!(chain, Edit::Chain(draft) if draft.input == "Use the preceding result"));

        for mut edit in [
            Edit::Credential(String::new()),
            Edit::Endpoint(String::new()),
            Edit::Workflow {
                fields: Default::default(),
                index: 0,
            },
            Edit::Chain(ChainDraft::default()),
        ] {
            paste_prompt(&mut edit, || panic!("clipboard read outside a prompt"));
        }
    }

    #[test]
    fn a_same_frame_paste_chord_does_not_type_the_shortcut_character() {
        let mut app = App::new();
        app.insert_resource(Layers::new(Layer::Providers))
            .add_message::<KeyboardInput>()
            .add_message::<KeyboardFocusLost>();
        install(&mut app, None);
        app.world_mut().resource_mut::<Panel>().draft = Some(Edit::Prompt(String::new(), None));
        // The hosted Linux job supplies a private Xvfb clipboard.
        let expected = if cfg!(target_os = "linux")
            && std::env::var_os("VESPERA_TEST_ISOLATED_X11_CLIPBOARD").is_some()
        {
            let mut clipboard = Clipboard::default();
            clipboard.set_text("native fixture ").unwrap();
            app.insert_resource(clipboard);
            "native fixture v"
        } else {
            "v"
        };
        let primary = if cfg!(target_os = "macos") {
            KeyCode::SuperLeft
        } else {
            KeyCode::ControlLeft
        };
        for (key_code, state, text) in [
            (primary, ButtonState::Pressed, None),
            (KeyCode::KeyV, ButtonState::Pressed, Some("v".into())),
            (primary, ButtonState::Released, None),
            (KeyCode::KeyV, ButtonState::Pressed, Some("v".into())),
        ] {
            app.world_mut().write_message(KeyboardInput {
                key_code,
                logical_key: Key::Character("v".into()),
                state,
                text,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
        }
        app.world_mut().run_system_once(keyboard).unwrap();
        assert!(
            matches!(&app.world().resource::<Panel>().draft, Some(Edit::Prompt(text, _)) if text == expected)
        );
    }

    #[test]
    fn closing_cancels_provider_work_and_waits_for_its_result() {
        let mut app = App::new();
        app.insert_resource(Layers::new(Layer::Providers))
            .add_message::<WindowCloseRequested>();
        install(&mut app, None);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let (send, result) = mpsc::sync_channel(1);
        let cancel = CancellationToken::new();
        app.world_mut().resource_mut::<Panel>().work = Some(Work::Task {
            result: Mutex::new(result),
            cancel: cancel.clone(),
        });

        app.world_mut()
            .write_message(WindowCloseRequested { window });
        app.world_mut()
            .run_system_once(close_after_provider_work)
            .unwrap();
        assert!(cancel.is_cancelled());
        assert!(
            app.world()
                .get::<Window>(window)
                .unwrap()
                .title
                .contains("Finishing local work")
        );
        app.world_mut().run_system_once(receive_work).unwrap();
        app.world_mut()
            .run_system_once(close_after_provider_work)
            .unwrap();
        assert!(app.world().get::<Window>(window).is_some());

        send.send(Err("fixture task cancelled".into())).unwrap();
        app.world_mut().run_system_once(receive_work).unwrap();
        apply(
            Action::Discover,
            &mut app.world_mut().resource_mut::<Panel>(),
        );
        assert!(app.world().resource::<Panel>().work.is_none());
        app.world_mut()
            .run_system_once(close_after_provider_work)
            .unwrap();
        assert!(app.world().get::<Window>(window).is_none());
    }

    #[tokio::test]
    async fn cancel_discovery_releases_the_panel_and_closes_the_pending_request() {
        for complete_health in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
            let (started, request) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                for index in 0..=usize::from(complete_health) {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut stream = BufReader::new(stream);
                    let mut line = String::new();
                    stream.read_line(&mut line).await.unwrap();
                    assert_eq!(line.trim_end(), "GET /v1/models HTTP/1.1");
                    while !line.trim_end().is_empty() {
                        line.clear();
                        assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                    }
                    if complete_health && index == 0 {
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"data\":[]}").await.unwrap();
                    } else {
                        started.send(()).unwrap();
                        let mut byte = [0];
                        assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
                        break;
                    }
                }
            });
            let mut app = App::new();
            app.insert_resource(Layers::new(Layer::Providers))
                .add_message::<KeyboardInput>()
                .add_message::<KeyboardFocusLost>();
            install(&mut app, None);
            {
                let mut panel = app.world_mut().resource_mut::<Panel>();
                panel.kind = ProviderKind::OpenAiCompatible;
                panel.endpoint = endpoint;
                apply(Action::Discover, &mut panel);
            }
            let bound = std::time::Duration::from_secs(3);
            tokio::time::timeout(bound, request).await.unwrap().unwrap();
            if complete_health {
                app.world_mut().write_message(KeyboardInput {
                    key_code: KeyCode::Escape,
                    logical_key: Key::Escape,
                    state: ButtonState::Pressed,
                    text: None,
                    repeat: false,
                    window: Entity::PLACEHOLDER,
                });
                app.world_mut().run_system_once(keyboard).unwrap();
            } else {
                apply(Action::Cancel, &mut app.world_mut().resource_mut::<Panel>());
            }
            tokio::time::timeout(bound, async {
                while app.world().resource::<Panel>().work.is_some() {
                    app.world_mut().run_system_once(receive_work).unwrap();
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                server.await.unwrap();
            })
            .await
            .unwrap();
            assert!(matches!(
                app.world().resource::<Panel>().outcome,
                Some(Err("Provider discovery cancelled."))
            ));
        }
    }

    #[tokio::test]
    async fn compatible_discovery_reports_server_models_and_explicit_lifecycle_limits() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                assert_eq!(line.trim_end(), "GET /v1/models HTTP/1.1");
                loop {
                    line.clear();
                    assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                    if line.trim_end().is_empty() {
                        break;
                    }
                }
                let body = r#"{"data":[{"id":"fixture-local-model"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let result = discover(desktop_agent::provider(
            ProviderKind::OpenAiCompatible,
            format!("http://{address}/v1"),
        ))
        .await
        .unwrap();
        assert_eq!(result.health.status, ProviderHealthStatus::Healthy);
        assert_eq!(
            result.support.residency_telemetry,
            LifecycleCapabilityState::Unsupported
        );
        assert_eq!(
            result.models.unwrap()[0].provider_model_name,
            "fixture-local-model"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn discovery_rejects_remote_endpoints_and_embedded_authentication() {
        for endpoint in [
            "https://192.0.2.1/v1",
            "http://user:password@127.0.0.1:1234/v1",
        ] {
            assert!(matches!(
                discover(desktop_agent::provider(
                    ProviderKind::OpenAiCompatible,
                    endpoint.into()
                ))
                .await,
                Err(RuntimeError::Config(_))
            ));
        }
    }

    #[tokio::test]
    async fn missing_credential_reference_fails_before_any_provider_request() {
        let mut provider =
            desktop_agent::provider(ProviderKind::OpenAiCompatible, COMPATIBLE.into());
        provider.credential_env = Some(format!(
            "VESPERA_MISSING_{}",
            vespera_core::TaskId::new()
                .to_string()
                .replace('-', "")
                .to_ascii_uppercase()
        ));
        assert!(matches!(
            discover(provider).await,
            Err(RuntimeError::Provider(
                ProviderError::SecretResolutionFailed { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn authenticated_discovery_and_saved_task_use_launch_environment() {
        const ENDPOINT: &str = "VESPERA_INTERNAL_TEST_AUTH_ENDPOINT";
        const CREDENTIAL: &str = "VESPERA_TEST_DESKTOP_AUTH";
        const ANSWER: &str = "Authenticated fixture answer";
        if let Ok(endpoint) = std::env::var(ENDPOINT) {
            let mut provider = desktop_agent::provider(ProviderKind::OpenAiCompatible, endpoint);
            assert!(matches!(
                discover(provider.clone()).await,
                Err(RuntimeError::Provider(
                    ProviderError::AuthenticationFailed { .. }
                ))
            ));
            provider.credential_env = Some(CREDENTIAL.into());
            let model = discover(provider.clone())
                .await
                .unwrap()
                .models
                .unwrap()
                .remove(0);
            assert_eq!(model.provider_model_name, "fixture-authenticated-model");
            let dir =
                std::env::temp_dir().join(format!("vespera-auth-{}", vespera_core::TaskId::new()));
            desktop_agent::register(
                &dir,
                None,
                Registration {
                    provider,
                    model,
                    agent_name: "Authenticated fixture".into(),
                    role: "Fixture".into(),
                    rebind: None,
                    ram_mib: 1,
                    vram_mib: 0,
                },
            )
            .await
            .unwrap();
            let roster = desktop_agent::load(&dir).unwrap().unwrap();
            let (agent, _, provider) = roster.selection(0).unwrap();
            let owner = agent.identity.id;
            assert_eq!(provider.credential_env.as_deref(), Some(CREDENTIAL));
            let finished = desktop_agent::run(
                &dir,
                roster.clone(),
                0,
                "Fixture prompt".into(),
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
            assert_eq!(finished.response, ANSWER);
            let tasks = dir.join("desktop-tasks.sqlite3");
            let stored = AgentTaskStore::inspect(&tasks, finished.id).unwrap();
            assert_eq!(stored.owner, owner);
            assert_eq!(stored.task.state, vespera_core::TaskState::Completed);
            let memory = dir.join("desktop-memory.sqlite3");
            assert!(desktop_agent::remember(&dir, roster.revision, owner, finished.id).is_err());
            assert!(!memory.exists());
            let (writer, _) =
                desktop_agent::set_permission(&dir, roster, 0, Permission::MemoryWrite, true)
                    .unwrap();
            let blocked = desktop_agent::run(
                &dir,
                writer.clone(),
                0,
                "Fixture recall".into(),
                Some(ANSWER.into()),
                CancellationToken::new(),
            )
            .await;
            assert!(matches!(blocked, Err(error) if error.contains("memory access denied")));
            desktop_agent::remember(&dir, writer.revision, owner, finished.id).unwrap();
            assert!(desktop_agent::remember(&dir, writer.revision, owner, finished.id).is_err());
            let (reader, _) = desktop_agent::set_permission(
                &dir,
                writer.clone(),
                0,
                Permission::MemoryRead,
                true,
            )
            .unwrap();
            assert!(desktop_agent::remember(&dir, writer.revision, owner, finished.id).is_err());
            let recalled = desktop_agent::run(
                &dir,
                reader,
                0,
                "Fixture recall".into(),
                Some(ANSWER.into()),
                CancellationToken::new(),
            )
            .await
            .unwrap();
            assert_eq!(recalled.memory_records, Some(1));
            let recall = AgentTaskStore::inspect(&tasks, recalled.id)
                .unwrap()
                .result
                .unwrap();
            assert_eq!(
                recall.memory_context_ids[0].to_string(),
                finished.id.to_string()
            );
            let token = std::env::var(CREDENTIAL).unwrap();
            for file in [dir.join("desktop.sqlite3"), tasks, memory] {
                let bytes = std::fs::read(&file).unwrap();
                assert!(
                    !bytes
                        .windows(token.len())
                        .any(|window| window == token.as_bytes())
                );
                std::fs::remove_file(file).unwrap();
            }
            std::fs::remove_dir(dir).unwrap();
            return;
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let token = vespera_core::TaskId::new().to_string();
        let expected_header = format!("Bearer {token}");
        let child = tokio::task::spawn_blocking(move || {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "provider_scene::tests::authenticated_discovery_and_saved_task_use_launch_environment"])
                .env(ENDPOINT, endpoint)
                .env(CREDENTIAL, token)
                .status()
                .unwrap()
        });
        let server = async {
            for (method, authorized, uses_memory) in [
                ("GET /v1/models HTTP/1.1", false, false),
                ("GET /v1/models HTTP/1.1", true, false),
                ("GET /v1/models HTTP/1.1", true, false),
                ("GET /v1/models HTTP/1.1", true, false),
                ("POST /v1/chat/completions HTTP/1.1", true, false),
                ("GET /v1/models HTTP/1.1", true, false),
                ("POST /v1/chat/completions HTTP/1.1", true, true),
            ] {
                let (stream, _) = listener.accept().await.unwrap();
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                assert_eq!(line.trim_end(), method);
                let mut has_auth = false;
                let mut content_length = 0;
                loop {
                    line.clear();
                    assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                    if line.trim_end().is_empty() {
                        break;
                    }
                    if let Some((header, value)) = line.split_once(':') {
                        if header.eq_ignore_ascii_case("authorization") {
                            assert!(value.trim() == expected_header);
                            has_auth = true;
                        } else if header.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse().unwrap();
                        }
                    }
                }
                assert_eq!(has_auth, authorized);
                let mut request = vec![0; content_length];
                stream.read_exact(&mut request).await.unwrap();
                let request = String::from_utf8(request).unwrap();
                assert_eq!(request.contains("untrusted_memory"), uses_memory);
                assert_eq!(request.contains(ANSWER), uses_memory);
                let (status, body) = if !authorized {
                    ("401 Unauthorized", r#"{"error":"authentication required"}"#)
                } else if method.starts_with("GET") {
                    (
                        "200 OK",
                        r#"{"data":[{"id":"fixture-authenticated-model"}]}"#,
                    )
                } else {
                    (
                        "200 OK",
                        r#"{"choices":[{"message":{"role":"assistant","content":"Authenticated fixture answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":3}}"#,
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(15), server)
            .await
            .unwrap();
        assert!(child.await.unwrap().success());
    }
}
