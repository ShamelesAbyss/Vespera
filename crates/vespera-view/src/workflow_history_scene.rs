use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use bevy::prelude::*;
use vespera_core::TaskId;
use vespera_orchestrator::{StoredWorkflow, StoredWorkflowSummary, WorkflowStore};

use crate::atlas::{self, Layer, Layers};
use crate::desktop_evidence::workflow_path;
use crate::home_scene::button;
use crate::provider_scene::preview;
use crate::swarm_scene::WorkflowSelection;

#[derive(Clone, Copy, Component)]
enum Action {
    Refresh,
    Previous,
    Next,
    Swarm,
    Home,
}

#[derive(Component)]
struct Readout;

struct Checkpoints {
    recent: Vec<StoredWorkflowSummary>,
    selected: Option<(usize, StoredWorkflow)>,
}

type Load = Result<Checkpoints, String>;

#[derive(Resource)]
struct History {
    path: PathBuf,
    pending: Option<Mutex<Receiver<Load>>>,
    checkpoints: Option<Checkpoints>,
    error: Option<String>,
}

pub(super) fn install(app: &mut App, dir: PathBuf) {
    atlas::register(app, Layer::Workflows);
    app.insert_resource(History {
        path: workflow_path(&dir),
        pending: None,
        checkpoints: None,
        error: None,
    })
    .add_systems(Startup, setup)
    .add_systems(Update, (navigate, receive, render).chain());
}

fn setup(mut commands: Commands) {
    let camera = commands
        .spawn((
            Layer::Workflows,
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
                row_gap: px(14),
                ..default()
            },
            children![
                (
                    Text::new("Workflow history"),
                    TextFont::from_font_size(34.0)
                ),
                (
                    Text::new("Retained desktop checkpoints. Up and Down browse, F5 returns Home."),
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
                        button(Action::Refresh, "Refresh"),
                        button(Action::Previous, "Newer workflow"),
                        button(Action::Next, "Older workflow"),
                        button(Action::Swarm, "Open in Swarm"),
                        button(Action::Home, "Home"),
                    ],
                ),
                (Readout, Text::default(), TextFont::from_font_size(16.0)),
            ],
        ))
        .observe(
            |event: On<PointerClick>,
             actions: Query<&Action>,
             mut history: ResMut<History>,
             mut layers: ResMut<Layers>,
             mut swarm: ResMut<WorkflowSelection>| {
                if layers.active == Layer::Workflows
                    && event.button == PointerButton::Primary
                    && let Ok(&action) = actions.get(event.original_event_target())
                {
                    apply(action, &mut history, &mut layers, &mut swarm);
                }
            },
        );
}

fn navigate(
    keys: Res<ButtonInput<KeyCode>>,
    mut history: ResMut<History>,
    mut layers: ResMut<Layers>,
    mut swarm: ResMut<WorkflowSelection>,
) {
    if layers.active != Layer::Workflows {
        return;
    }
    let action = if layers.switched || keys.just_pressed(KeyCode::KeyR) {
        Action::Refresh
    } else if keys.just_pressed(KeyCode::ArrowUp) {
        Action::Previous
    } else if keys.just_pressed(KeyCode::ArrowDown) {
        Action::Next
    } else if keys.just_pressed(KeyCode::Enter) {
        Action::Swarm
    } else {
        return;
    };
    apply(action, &mut history, &mut layers, &mut swarm);
}

fn apply(
    action: Action,
    history: &mut History,
    layers: &mut Layers,
    swarm: &mut WorkflowSelection,
) {
    if matches!(action, Action::Home) {
        layers.request(Layer::Home);
        return;
    }
    if history.pending.is_some() {
        return;
    }
    let selected = history.checkpoints.as_ref().and_then(|checkpoints| {
        checkpoints
            .selected
            .as_ref()
            .map(|(index, stored)| (checkpoints, *index, stored.plan.task.id))
    });
    let task = match action {
        Action::Refresh => None,
        Action::Previous | Action::Next => {
            let Some((checkpoints, index, _)) = selected else {
                return;
            };
            let next = match action {
                Action::Previous => index.checked_sub(1),
                _ => index.checked_add(1),
            };
            let Some(entry) = next.and_then(|index| checkpoints.recent.get(index)) else {
                return;
            };
            Some(entry.id)
        }
        Action::Swarm => {
            if history.error.is_none()
                && let Some((_, _, task)) = selected
            {
                swarm.0 = Some(task);
                layers.request(Layer::Swarm);
            }
            return;
        }
        Action::Home => return,
    };
    history.error = None;
    let path = history.path.clone();
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = send.send(load(path, task));
    });
    history.pending = Some(Mutex::new(receive));
}

fn load(path: PathBuf, task: Option<TaskId>) -> Load {
    let recent = if path.try_exists().map_err(|error| error.to_string())? {
        WorkflowStore::recent(&path).map_err(|error| error.to_string())?
    } else {
        vec![]
    };
    let selected = match task {
        Some(task) => Some(
            recent
                .iter()
                .position(|entry| entry.id == task)
                .ok_or("The requested checkpoint is no longer retained.")?,
        ),
        None => (!recent.is_empty()).then_some(0),
    };
    let selected = selected
        .map(|index| {
            WorkflowStore::inspect(&path, recent[index].id).map(|workflow| (index, workflow))
        })
        .transpose()
        .map_err(|error| error.to_string())?;
    Ok(Checkpoints { recent, selected })
}

fn receive(mut history: ResMut<History>) {
    let Some(pending) = &history.pending else {
        return;
    };
    let result = pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .try_recv();
    match result {
        Ok(Ok(checkpoints)) => history.checkpoints = Some(checkpoints),
        Ok(Err(error)) => history.error = Some(error),
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => {
            history.error = Some("The history reader stopped.".into())
        }
    }
    history.pending = None;
}

fn render(layers: Res<Layers>, history: Res<History>, mut text: Single<&mut Text, With<Readout>>) {
    if layers.active != Layer::Workflows || !history.is_changed() {
        return;
    }
    text.0 = if history.pending.is_some() {
        "Reading retained workflow checkpoints...".into()
    } else if let Some(error) = &history.error {
        format!("{error}\nUse Refresh to try again after the workflow writer exits.")
    } else if let Some(checkpoints) = &history.checkpoints
        && let Some((index, workflow)) = &checkpoints.selected
    {
        describe(*index, checkpoints.recent.len(), workflow)
    } else {
        "No desktop workflows have been retained yet.".into()
    };
}

fn describe(index: usize, count: usize, workflow: &StoredWorkflow) -> String {
    use std::fmt::Write as _;

    let report = &workflow.report;
    let mut text = format!(
        "Workflow {} of {count}  |  {}\n{}  |  {} ms\nCoordinator  |  {}\n\nRequest  |  {}\n",
        index + 1,
        report.task.id,
        if workflow.interrupted {
            "interrupted".into()
        } else {
            report.task.state.to_string()
        },
        report.elapsed_ms,
        report.coordinator,
        preview(&workflow.plan.task.prompt, 150),
    );
    if let Some(failure) = report.failure {
        let _ = writeln!(text, "Outcome  |  {failure:?}");
    }
    for attempt in report.attempts.iter().take(8) {
        let state = attempt
            .task_state
            .map_or_else(|| "No task state".into(), |state| state.to_string());
        let _ = writeln!(
            text,
            "{} #{}  |  {}  |  {state}",
            attempt.step, attempt.number, attempt.agent_id
        );
    }
    if let Some(result) = report.selected_result() {
        let _ = writeln!(
            text,
            "\nSelected response  |  {}",
            preview(&result.response.content, 420)
        );
    }
    text
}
