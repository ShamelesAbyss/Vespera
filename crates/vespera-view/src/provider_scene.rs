use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use bevy::input::{ButtonState, keyboard::KeyboardInput};
use bevy::prelude::*;
use bevy::ui::prelude::UiPickingCamera;
use vespera_app::runtime::{RuntimeError, configured_registry};
use vespera_config::{AppConfig, CONFIG_SCHEMA_VERSION, ProviderConfig};
use vespera_core::{ProviderId, ProviderKind};
use vespera_providers::{
    CancellationToken, DiscoveredModel, ProviderHealthReport, ProviderLifecycleSupport,
};

use crate::atlas::{self, Layer, Layers};
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
}

#[derive(Component)]
struct Readout;

struct Discovery {
    health: ProviderHealthReport,
    support: ProviderLifecycleSupport,
    models: Result<Vec<DiscoveredModel>, vespera_providers::ProviderError>,
}

type ProbeResult = Result<Discovery, &'static str>;

#[derive(Resource)]
struct Panel {
    kind: ProviderKind,
    endpoint: String,
    draft: Option<String>,
    pending: Option<Mutex<Receiver<ProbeResult>>>,
    outcome: Option<ProbeResult>,
    page: usize,
}

pub(super) fn install(app: &mut App) {
    atlas::register(app, Layer::Providers);
    app.insert_resource(Panel {
        kind: ProviderKind::Ollama,
        endpoint: OLLAMA.into(),
        draft: None,
        pending: None,
        outcome: None,
        page: 0,
    })
    .add_systems(Startup, setup)
    .add_systems(Update, (keyboard, receive, readout).chain());
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
    if panel.pending.is_some() || panel.draft.is_some() {
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
        Action::Edit => panel.draft = Some(panel.endpoint.clone()),
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
    let id = ProviderId::new();
    let config = AppConfig {
        schema_version: CONFIG_SCHEMA_VERSION,
        network: default(),
        resources: default(),
        inference_overhead: default(),
        scheduling: default(),
        providers: vec![ProviderConfig {
            id,
            name: "Local provider".into(),
            kind,
            endpoint,
            credential_env: None,
            request_timeout_seconds: 5,
        }],
        models: vec![],
        agents: vec![],
    };
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
        let Some(draft) = panel.draft.as_mut() else {
            if event.key_code == KeyCode::Enter && !event.repeat {
                apply(Action::Discover, &mut panel);
            }
            continue;
        };
        match event.key_code {
            KeyCode::Enter if !event.repeat => {
                if let Some(endpoint) = panel.draft.take() {
                    panel.endpoint = endpoint;
                    panel.outcome = None;
                    panel.page = 0;
                }
            }
            KeyCode::Escape => panel.draft = None,
            KeyCode::Backspace => {
                draft.pop();
            }
            KeyCode::Delete => draft.clear(),
            _ => {
                if let Some(text) = &event.text {
                    draft.extend(
                        text.chars()
                            .filter(|c| c.is_ascii_alphanumeric() || ":/.[_-]".contains(*c))
                            .take(ENDPOINT_LIMIT.saturating_sub(draft.len())),
                    );
                }
            }
        }
    }
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
    let endpoint = panel.draft.as_ref().unwrap_or(&panel.endpoint);
    let mut content = format!("Adapter  |  {adapter}\nEndpoint  |  {endpoint}\n");
    if panel.draft.is_some() {
        content.push_str("\nEnter saves the endpoint, Esc cancels, Delete clears the field.");
    } else if panel.pending.is_some() {
        content.push_str("\nChecking health and discovering models...");
    } else {
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
                            "\n{} exposed models  |  Page {} of {}",
                            models.len(),
                            panel.page + 1,
                            models.len().div_ceil(MODELS_PER_PAGE)
                        );
                        for model in models
                            .iter()
                            .skip(panel.page * MODELS_PER_PAGE)
                            .take(MODELS_PER_PAGE)
                        {
                            let name: String = model
                                .provider_model_name
                                .chars()
                                .filter(|c| !c.is_control())
                                .take(90)
                                .collect();
                            let _ = writeln!(content, "  {name}");
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
