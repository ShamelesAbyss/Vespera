use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use bevy::input::{ButtonState, keyboard::KeyboardInput};
use bevy::prelude::*;
use bevy::ui::prelude::UiPickingCamera;
use vespera_app::runtime::{RuntimeError, configured_registry};
use vespera_core::ProviderKind;
use vespera_providers::{
    CancellationToken, DiscoveredModel, ProviderHealthReport, ProviderLifecycleSupport,
};

use crate::atlas::{self, Layer, Layers};
use crate::desktop_agent::{self, Binding, FinishedTask, Registration};
use crate::home_scene::button;

const MODELS_PER_PAGE: usize = 8;
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
        model: DiscoveredModel,
        fields: [String; 3],
        index: usize,
    },
    Prompt(String),
}

enum Work {
    Registration(Mutex<Receiver<Result<Binding, &'static str>>>),
    Task {
        result: Mutex<Receiver<Result<FinishedTask, String>>>,
        cancel: CancellationToken,
    },
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
    binding: Option<Binding>,
    work: Option<Work>,
    feedback: String,
}

pub(super) fn install(app: &mut App, dir: Option<PathBuf>) {
    atlas::register(app, Layer::Providers);
    let restored = dir.as_deref().map_or(Ok(None), desktop_agent::load);
    let (binding, feedback) = match restored {
        Ok(binding) => (binding, String::new()),
        Err(error) => (None, error.into()),
    };
    let (kind, endpoint) = binding.as_ref().map_or_else(
        || (ProviderKind::Ollama, OLLAMA.into()),
        |binding| (binding.provider().kind, binding.provider().endpoint.clone()),
    );
    app.insert_resource(Panel {
        kind,
        endpoint,
        draft: None,
        pending: None,
        outcome: None,
        page: 0,
        dir,
        binding,
        work: None,
        feedback,
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
                (Readout, Text::default(), TextFont::from_font_size(16.0)),
                (
                    Node {
                        column_gap: px(10),
                        ..default()
                    },
                    children![
                        button(Action::Previous, "Previous models"),
                        button(Action::Next, "Next models"),
                        button(Action::Prompt, "Run agent task"),
                        button(Action::Cancel, "Cancel task"),
                    ]
                ),
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
        if let Some(Work::Task { cancel, .. }) = &panel.work {
            cancel.cancel();
            panel.feedback = "Cancellation requested. Waiting for local cleanup.".into();
        }
        return;
    }
    if panel.pending.is_some() || panel.draft.is_some() || panel.work.is_some() {
        return;
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
            if panel.binding.is_some() {
                panel.draft = Some(Edit::Prompt(String::new()));
            } else {
                panel.feedback =
                    "Discover a model and press its number to register it and bind an agent."
                        .into();
            }
        }
        Action::Cancel => {}
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
        Action::Previous => panel.page = panel.page.saturating_sub(1),
        Action::Next => {
            if let Some(Ok(result)) = &panel.outcome
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
                    select_model(&mut panel, index);
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
        panel.draft = Some(Edit::Registration {
            model: model.clone(),
            fields: [
                panel
                    .binding
                    .as_ref()
                    .map_or_else(String::new, |binding| binding.agent().identity.name.clone()),
                String::new(),
                String::new(),
            ],
            index: 0,
        });
    }
}

fn update_edit(edit: &mut Edit, event: &KeyboardInput) {
    let (field, limit, accepts): (&mut String, usize, fn(char) -> bool) = match edit {
        Edit::Endpoint(value) => (value, ENDPOINT_LIMIT, |c| {
            c.is_ascii_alphanumeric() || ":/.[_-]".contains(c)
        }),
        Edit::Registration {
            fields, index: 0, ..
        } => (&mut fields[0], 64, |c| !c.is_control()),
        Edit::Registration { fields, index, .. } => {
            (&mut fields[*index], 8, |c| c.is_ascii_digit())
        }
        Edit::Prompt(value) => (value, 1200, |c| !c.is_control()),
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
        } => {
            let valid = match index {
                0 => !fields[0].trim().is_empty(),
                1 => fields[1].parse::<u32>().is_ok_and(|ram| ram > 0),
                _ => fields[2].parse::<u32>().is_ok(),
            };
            if !valid || index < 2 {
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
                });
                return;
            }
            if let Some(dir) = panel.dir.clone() {
                let [agent_name, ram, vram] = fields;
                let registration = Registration {
                    provider: desktop_agent::provider(panel.kind, panel.endpoint.clone()),
                    model,
                    agent_name,
                    ram_mib: ram.parse().expect("checked RAM input"),
                    vram_mib: vram.parse().expect("checked VRAM input"),
                };
                let previous = panel.binding.clone();
                let receiver = background(move |runtime| {
                    runtime.block_on(desktop_agent::register(
                        &dir,
                        previous.as_ref(),
                        registration,
                    ))
                });
                panel.work = Some(Work::Registration(receiver));
                panel.feedback = "Validating the model and saving the agent binding...".into();
            }
        }
        Edit::Prompt(prompt) => {
            if prompt.trim().is_empty() {
                panel.feedback = "Enter a prompt before starting a task.".into();
                panel.draft = Some(Edit::Prompt(prompt));
            } else if let (Some(dir), Some(binding)) = (panel.dir.clone(), panel.binding.clone()) {
                let cancel = CancellationToken::new();
                let task_cancel = cancel.clone();
                let result = background(move |runtime| {
                    runtime.block_on(desktop_agent::run(&dir, binding, prompt, task_cancel))
                });
                panel.work = Some(Work::Task { result, cancel });
                panel.feedback = "Task in progress. Waiting for the provider and scheduler.".into();
            }
        }
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
        Some(Work::Registration(result)) => match poll(result) {
            Ok(Ok(binding)) => {
                panel.binding = Some(binding);
                panel.feedback = "Registered the model and saved the agent binding.".into();
            }
            Ok(Err(error)) => panel.feedback = error.into(),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panel.feedback = "The registration worker stopped.".into()
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
    if let Some(binding) = &panel.binding {
        let _ = writeln!(
            content,
            "Saved agent  |  {}  |  Model {}\nTask endpoint  |  {}",
            binding.agent().identity.name,
            binding
                .model()
                .identifier
                .chars()
                .take(90)
                .collect::<String>(),
            binding.provider().endpoint
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
            } => {
                let labels = [
                    "Agent name",
                    "Estimated model RAM in MiB (positive)",
                    "Estimated model VRAM in MiB (0 for CPU-only)",
                ];
                let _ = write!(
                    content,
                    "\nRegister {}\n{}: {}_\nThese operator estimates also set this profile's memory ceilings.\nEnter continues.",
                    model
                        .provider_model_name
                        .chars()
                        .take(90)
                        .collect::<String>(),
                    labels[*index],
                    fields[*index]
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
    } else if panel.pending.is_some() {
        content.push_str("\nChecking health and discovering models...");
    } else if panel.work.is_none() && !panel.feedback.starts_with("Task ") {
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
