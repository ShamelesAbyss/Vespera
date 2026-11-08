use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use bevy::input::{ButtonState, keyboard::KeyboardInput};
use bevy::prelude::*;
use bevy::ui::prelude::UiPickingCamera;
use vespera_agents::{AgentTaskStore, AgentTaskStoreError, StoredAgentTask, StoredTaskSummary};
use vespera_app::runtime::{RuntimeError, configured_registry};
use vespera_core::{AgentId, Permission, ProviderKind};
use vespera_providers::{
    CancellationToken, DiscoveredModel, ProviderHealthReport, ProviderLifecycleSupport,
};

use crate::atlas::{self, Layer, Layers};
use crate::desktop_agent::{self, FinishedTask, Registration, Roster};
use crate::desktop_workflow::{self, FinishedWorkflow, WorkflowRequest};
use crate::home_scene::button;

const MODELS_PER_PAGE: usize = 8;
const TASKS_PER_PAGE: u32 = 8;
const ENDPOINT_LIMIT: usize = 160;
const OLLAMA: &str = "http://127.0.0.1:11434";
const COMPATIBLE: &str = "http://127.0.0.1:1234/v1";

#[derive(Component, Clone, Copy)]
enum Action {
    Ollama,
    Compatible,
    Edit,
    Discover,
    Previous,
    Next,
    Prompt,
    Cancel,
    PreviousAgent,
    NextAgent,
    NewAgent,
    Rebind,
    History,
    AllowDelegation,
    RevokeDelegation,
    Suspend,
    Resume,
    Workflow,
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
    Registration {
        model: Box<DiscoveredModel>,
        fields: [String; 4],
        index: usize,
        rebind: Option<AgentId>,
    },
    Prompt(String),
    Workflow {
        fields: [String; 3],
        index: usize,
    },
}

enum Work {
    SaveRoster(Mutex<Receiver<RegistrationResult>>, &'static str),
    Hold(
        Mutex<Receiver<Result<(), AgentTaskStoreError>>>,
        &'static str,
    ),
    History(Mutex<Receiver<Result<Vec<StoredTaskSummary>, AgentTaskStoreError>>>),
    Inspect(Mutex<Receiver<Result<StoredAgentTask, AgentTaskStoreError>>>),
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
}

#[derive(Resource)]
struct Panel {
    kind: ProviderKind,
    endpoint: String,
    draft: Option<Edit>,
    pending: Option<Mutex<Receiver<ProbeResult>>>,
    outcome: Option<ProbeResult>,
    page: usize,
    dir: Option<PathBuf>,
    roster: Option<Roster>,
    selected: usize,
    rebind: bool,
    work: Option<Work>,
    feedback: String,
    history: Option<History>,
}

pub(super) fn install(app: &mut App, dir: Option<PathBuf>) {
    atlas::register(app, Layer::Providers);
    let restored = dir.as_deref().map_or(Ok(None), desktop_agent::load);
    let (roster, feedback) = match restored {
        Ok(roster) => (roster, String::new()),
        Err(error) => (None, error.into()),
    };
    let (kind, endpoint) = roster
        .as_ref()
        .and_then(|roster| roster.selection(0))
        .map_or_else(
            || (ProviderKind::Ollama, OLLAMA.into()),
            |(_, _, provider)| (provider.kind, provider.endpoint.clone()),
        );
    app.insert_resource(Panel {
        kind,
        endpoint,
        draft: None,
        pending: None,
        outcome: None,
        page: 0,
        dir,
        roster,
        selected: 0,
        rebind: false,
        work: None,
        feedback,
        history: None,
    })
    .add_systems(Startup, setup)
    .add_systems(Update, (keyboard, receive, receive_work, readout).chain());
}

fn setup(mut commands: Commands) {
    let camera = commands
        .spawn((
            Layer::Providers,
            Camera2d,
            UiPickingCamera,
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
                    Text::new(
                        "Discover models exposed by a local inference server. F5 returns Home."
                    ),
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
                        button(Action::Discover, "Test & discover"),
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
                        button(Action::Cancel, "Cancel work"),
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
    if matches!(action, Action::Cancel) {
        if let Some(Work::Task { cancel, .. } | Work::Workflow { cancel, .. }) = &panel.work {
            cancel.cancel();
            panel.feedback = "Cancellation requested. Waiting for local cleanup.".into();
        }
        return;
    }
    if panel.pending.is_some() || panel.draft.is_some() || panel.work.is_some() {
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
        )
    {
        panel.history = None;
    }
    match action {
        Action::Ollama | Action::Compatible => {
            (panel.kind, panel.endpoint) = match action {
                Action::Ollama => (ProviderKind::Ollama, OLLAMA.into()),
                _ => (ProviderKind::OpenAiCompatible, COMPATIBLE.into()),
            };
            panel.outcome = None;
            panel.page = 0;
        }
        Action::Edit => panel.draft = Some(Edit::Endpoint(panel.endpoint.clone())),
        Action::Prompt => {
            if panel.roster.is_some() {
                panel.draft = Some(Edit::Prompt(String::new()));
            } else {
                panel.feedback =
                    "Discover a model and press its number to register it and bind an agent."
                        .into();
            }
        }
        Action::Cancel => {}
        Action::Workflow => {
            let Some(roster) = &panel.roster else {
                return;
            };
            if roster.len() < 2 {
                panel.feedback = "Create two agents before starting a workflow.".into();
            } else if roster
                .selection(panel.selected)
                .is_some_and(|(agent, _, _)| agent.permissions.contains(&Permission::Delegate))
            {
                panel.feedback.clear();
                panel.draft = Some(Edit::Workflow {
                    fields: Default::default(),
                    index: 0,
                });
            } else {
                panel.feedback = "Allow delegation for the selected coordinator first.".into();
            }
        }
        Action::AllowDelegation | Action::RevokeDelegation => {
            let (Some(dir), Some(roster)) = (panel.dir.clone(), panel.roster.clone()) else {
                return;
            };
            let allow = matches!(action, Action::AllowDelegation);
            let selected = panel.selected;
            panel.work = Some(Work::SaveRoster(
                background(move |_| desktop_agent::set_delegation(&dir, roster, selected, allow)),
                if allow {
                    "Delegation granted to the selected agent."
                } else {
                    "Delegation revoked."
                },
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
        Action::Rebind => {
            panel.rebind = panel.roster.is_some();
            if panel.rebind {
                panel.feedback =
                    "Select a discovered model number to rebind the selected agent.".into();
            }
        }
        Action::Discover => {
            let (sender, receiver) = mpsc::sync_channel(1);
            let kind = panel.kind;
            let endpoint = panel.endpoint.clone();
            std::thread::spawn(move || {
                let outcome = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| "Could not start the provider request.")
                    .and_then(|runtime| {
                        runtime.block_on(discover(kind, endpoint)).map_err(|error| match error {
                            RuntimeError::Config(_) => "Endpoint rejected. Enter a loopback HTTP or HTTPS URL.",
                            _ => "Could not verify provider health. Check the endpoint and server.",
                        })
                    });
                let _ = sender.send(outcome);
            });
            panel.pending = Some(Mutex::new(receiver));
            panel.outcome = None;
            panel.page = 0;
            panel.feedback.clear();
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

async fn discover(kind: ProviderKind, endpoint: String) -> Result<Discovery, RuntimeError> {
    let mut declaration = desktop_agent::provider(kind, endpoint);
    declaration.request_timeout_seconds = 5;
    let id = declaration.id;
    let config = desktop_agent::config(declaration);
    let registry = configured_registry(&config).await?;
    let provider = registry.get_provider(id).await?;
    let cancel = CancellationToken::new();
    let health = provider.health_check(cancel.clone()).await?;
    let support = provider.lifecycle_support();
    let models = provider.enumerate_models(cancel).await;
    Ok(Discovery {
        health,
        support,
        models,
    })
}

fn keyboard(
    mut input: MessageReader<KeyboardInput>,
    layers: Res<Layers>,
    mut panel: ResMut<Panel>,
) {
    if layers.active != Layer::Providers {
        if panel.draft.is_some() {
            panel.draft = None;
        }
        input.clear();
        return;
    }
    for event in input
        .read()
        .filter(|event| event.state == ButtonState::Pressed)
    {
        if panel.draft.is_none() {
            if !event.repeat {
                if event.key_code == KeyCode::Enter {
                    apply(Action::Discover, &mut panel);
                } else if let Some(index) = [
                    KeyCode::Digit1,
                    KeyCode::Digit2,
                    KeyCode::Digit3,
                    KeyCode::Digit4,
                    KeyCode::Digit5,
                    KeyCode::Digit6,
                    KeyCode::Digit7,
                    KeyCode::Digit8,
                ]
                .iter()
                .position(|key| *key == event.key_code)
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
            KeyCode::Escape => panel.draft = None,
            _ => {
                if let Some(edit) = panel.draft.as_mut() {
                    update_edit(edit, event);
                }
            }
        }
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
    let Some(entry) = panel
        .history
        .as_ref()
        .and_then(|history| history.tasks.get(index))
    else {
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

fn select_model(panel: &mut Panel, index: usize) {
    if panel.work.is_some() || panel.pending.is_some() {
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
    let (field, limit, accepts): (&mut String, usize, fn(char) -> bool) = match edit {
        Edit::Endpoint(value) => (value, ENDPOINT_LIMIT, |c| {
            c.is_ascii_alphanumeric() || ":/.[_-]".contains(c)
        }),
        Edit::Registration {
            fields,
            index: index @ (0 | 1),
            ..
        } => (&mut fields[*index], 64, |c| !c.is_control()),
        Edit::Registration { fields, index, .. } => {
            (&mut fields[*index], 8, |c| c.is_ascii_digit())
        }
        Edit::Prompt(value) => (value, 1200, |c| !c.is_control()),
        Edit::Workflow { fields, index: 0 } => (&mut fields[0], 2, |c| c.is_ascii_digit()),
        Edit::Workflow { fields, index } => (&mut fields[*index], 1200, |c| !c.is_control()),
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
            panel.endpoint = endpoint;
            panel.outcome = None;
            panel.page = 0;
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
                    provider: desktop_agent::provider(panel.kind, panel.endpoint.clone()),
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
        Edit::Prompt(prompt) => {
            if prompt.trim().is_empty() {
                panel.feedback = "Enter a prompt before starting a task.".into();
                panel.draft = Some(Edit::Prompt(prompt));
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
                        task_cancel,
                    ))
                });
                panel.work = Some(Work::Task { result, cancel });
                panel.feedback = "Task in progress. Waiting for the provider and scheduler.".into();
            }
        }
        Edit::Workflow { fields, index } => submit_workflow(panel, fields, index),
    }
}

fn submit_workflow(panel: &mut Panel, fields: [String; 3], mut index: usize) {
    let (Some(dir), Some(roster)) = (panel.dir.clone(), panel.roster.clone()) else {
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
    let request = WorkflowRequest {
        coordinator: panel.selected,
        reviewer: reviewer.expect("validated reviewer index"),
        draft_prompt,
        review_prompt,
    };
    let cancel = CancellationToken::new();
    let workflow_cancel = cancel.clone();
    let result = background(move |runtime| {
        runtime.block_on(desktop_workflow::run(
            &dir,
            roster,
            request,
            workflow_cancel,
        ))
    });
    panel.work = Some(Work::Workflow { result, cancel });
    panel.feedback = "Workflow in progress. Draft runs before the delegated review.".into();
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
            }
            Ok(Err(error)) => panel.feedback = error,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => panel.feedback = "The task worker stopped.".into(),
        },
        None => return,
    }
    panel.work = None;
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
            "\nReview: {}",
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

fn preview(value: &str, limit: usize) -> String {
    value
        .split_whitespace()
        .flat_map(|word| word.chars().filter(|c| !c.is_control()).chain([' ']))
        .take(limit)
        .collect::<String>()
        .trim_end()
        .into()
}

fn receive(mut panel: ResMut<Panel>) {
    let Some(receiver) = &panel.pending else {
        return;
    };
    let result = receiver
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .try_recv();
    panel.outcome = Some(match result {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("The provider request stopped early."),
    });
    panel.pending = None;
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
    let mut content = format!("Adapter  |  {adapter}\nEndpoint  |  {endpoint}\n");
    if let Some(roster) = &panel.roster
        && let Some((agent, model, provider)) = roster.selection(panel.selected)
    {
        let _ = writeln!(
            content,
            "Agent {} of {}  |  {}  |  {}  |  Delegation {}\nBound model  |  {}\nTask endpoint  |  {}",
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
            provider.endpoint
        );
    }
    if !panel.feedback.is_empty() {
        let _ = writeln!(content, "\n{}", panel.feedback);
    }
    if let Some(edit) = &panel.draft {
        match edit {
            Edit::Endpoint(_) => content.push_str("\nEnter saves the endpoint."),
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
            Edit::Prompt(prompt) => {
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
    } else if panel.pending.is_some() {
        content.push_str("\nChecking health and discovering models...");
    } else if panel.work.is_none()
        && !panel.feedback.starts_with("Task ")
        && !panel.feedback.starts_with("Workflow ")
    {
        match &panel.outcome {
            None => content.push_str(
                "\nLoopback servers without authentication. Select Test & discover or press Enter.",
            ),
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
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use vespera_providers::{LifecycleCapabilityState, ProviderHealthStatus};

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
        let result = discover(
            ProviderKind::OpenAiCompatible,
            format!("http://{address}/v1"),
        )
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
                discover(ProviderKind::OpenAiCompatible, endpoint.into()).await,
                Err(RuntimeError::Config(_))
            ));
        }
    }
}
