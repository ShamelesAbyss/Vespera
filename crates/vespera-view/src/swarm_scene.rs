use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use tokio::sync::{mpsc, watch};
use vespera_core::{AgentId, EventEnvelope, EventKind, TaskId, TaskState};
use vespera_events::{Cursor, Journal, JournalError, MAX_PAGE};

use crate::Update;
use crate::scene::{CameraFocus, Orbit, camera, desktop};

const MAX_OBSERVATIONS: usize = 128;
const MAX_AGENTS: usize = 24;

#[derive(Clone, Copy, PartialEq)]
enum Relation {
    Delegation {
        task: TaskId,
        workflow: Option<TaskId>,
    },
    Message,
}

#[derive(Clone, Copy, PartialEq)]
struct Edge {
    sequence: u64,
    from: AgentId,
    to: AgentId,
    relation: Relation,
}

#[derive(Clone, Default, PartialEq)]
struct Evidence {
    edges: VecDeque<Edge>,
    outcomes: VecDeque<(u64, TaskId, TaskState)>,
    floor: u64,
    clipped: bool,
    status: &'static str,
}

impl Evidence {
    fn apply(&mut self, page: vespera_events::Page) {
        self.prune(page.discarded_through.unsigned_abs());
        for entry in page.entries {
            self.push(entry.cursor.sequence.unsigned_abs(), entry.event);
        }
        self.status = if page.has_more {
            "Catching up"
        } else {
            "Following committed journal"
        };
    }

    fn prune(&mut self, floor: u64) {
        self.floor = floor;
        self.edges.retain(|edge| edge.sequence > self.floor);
        self.outcomes
            .retain(|(sequence, _, _)| *sequence > self.floor);
    }

    fn push(&mut self, sequence: u64, event: EventEnvelope) {
        match event.kind {
            EventKind::TaskDelegated {
                task_id,
                from_agent_id,
                to_agent_id,
            } => {
                self.edges.push_back(Edge {
                    sequence,
                    from: from_agent_id,
                    to: to_agent_id,
                    relation: Relation::Delegation {
                        task: task_id,
                        workflow: event.correlation_task_id,
                    },
                });
            }
            EventKind::AgentMessageSent {
                from_agent_id,
                to_agent_id,
            } => {
                self.edges.push_back(Edge {
                    sequence,
                    from: from_agent_id,
                    to: to_agent_id,
                    relation: Relation::Message,
                });
            }
            EventKind::TaskCompleted { task_id } if event.source == "run-workflow" => {
                self.outcomes
                    .push_back((sequence, task_id, TaskState::Completed));
            }
            EventKind::TaskFailed { task_id } if event.source == "run-workflow" => {
                self.outcomes
                    .push_back((sequence, task_id, TaskState::Failed));
            }
            EventKind::TaskCancelled { task_id } if event.source == "run-workflow" => {
                self.outcomes
                    .push_back((sequence, task_id, TaskState::Cancelled));
            }
            _ => {}
        }
        if self.edges.len() > MAX_OBSERVATIONS {
            self.edges.pop_front();
            self.clipped = true;
        }
        if self.outcomes.len() > MAX_OBSERVATIONS {
            self.outcomes.pop_front();
            self.clipped = true;
        }
    }
}

#[derive(Resource)]
struct Feed {
    input: watch::Receiver<Evidence>,
    evidence: Evidence,
}

#[derive(Resource, Default)]
struct Layout {
    agents: BTreeMap<AgentId, (Entity, Vec3)>,
    selected: Option<AgentId>,
    filter: Filter,
}

#[derive(Clone, Copy, Default)]
enum Filter {
    #[default]
    All,
    Delegation,
    Messages,
}

impl Filter {
    fn includes(self, edge: &Edge) -> bool {
        match self {
            Self::All => true,
            Self::Delegation => matches!(edge.relation, Relation::Delegation { .. }),
            Self::Messages => matches!(edge.relation, Relation::Message),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All observed relations",
            Self::Delegation => "Delegation",
            Self::Messages => "Messages",
        }
    }
}

#[derive(Resource)]
struct AgentMesh(Handle<Mesh>, Handle<StandardMaterial>);

#[derive(Component)]
struct Readout;

pub(super) fn run(path: PathBuf, reduced_motion: bool) -> Result<(), JournalError> {
    Journal::open_reader(&path)?;
    let evidence = Evidence {
        status: "Reading journal",
        ..default()
    };
    let (send, input) = watch::channel(evidence.clone());
    std::thread::spawn(move || observe(&path, &send));
    run_window(input, evidence, reduced_motion);
    Ok(())
}

pub(super) fn run_broker(mut updates: mpsc::Receiver<Update>, reduced_motion: bool) {
    let evidence = Evidence {
        status: "Connecting to signed broker feed",
        ..default()
    };
    let (send, input) = watch::channel(evidence.clone());
    let mut observed = evidence.clone();
    std::thread::spawn(move || {
        while let Some(update) = updates.blocking_recv() {
            match update {
                Update::Ready(floor) => {
                    observed.prune(floor);
                    observed.status = "Following signed broker feed";
                }
                Update::Observation(entry) => observed.push(entry.cursor.sequence, entry.event),
                Update::Status(status) => observed.status = status,
                Update::Reset => {
                    observed = Evidence {
                        status: "Broker history changed; rebuilding",
                        ..default()
                    };
                }
                Update::Cancelled(..) | Update::CancelFailed => {}
            }
            publish(&send, &observed);
            if send.is_closed() {
                break;
            }
        }
    });
    run_window(input, evidence, reduced_motion);
}

fn run_window(input: watch::Receiver<Evidence>, evidence: Evidence, reduced_motion: bool) {
    desktop("Vespera | Swarm", Color::srgb(0.018, 0.025, 0.047))
        .insert_resource(Feed { input, evidence })
        .init_resource::<Layout>()
        .insert_resource(CameraFocus {
            point: None,
            home_point: Vec3::new(2.0, 0.0, 0.0),
            home_distance: 19.0,
            reduced_motion,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (sync, navigate, camera, draw, inspect).chain())
        .run();
}

fn observe(path: &Path, send: &watch::Sender<Evidence>) {
    let mut cursor: Option<Cursor> = None;
    let mut evidence = Evidence::default();
    while !send.is_closed() {
        match Journal::open_reader(path).and_then(|mut journal| journal.replay(cursor, MAX_PAGE)) {
            Ok(page) => {
                cursor = Some(page.next_cursor);
                evidence.apply(page);
            }
            Err(
                JournalError::Expired { .. }
                | JournalError::ForeignCursor
                | JournalError::FutureCursor,
            ) => {
                cursor = None;
                evidence = Evidence {
                    status: "Journal changed; rebuilding observations",
                    ..default()
                };
            }
            Err(_) => evidence.status = "Journal unavailable; showing last observations",
        }
        publish(send, &evidence);
        let delay = if evidence.status == "Catching up" {
            20
        } else {
            400
        };
        std::thread::sleep(Duration::from_millis(delay));
    }
}

fn publish(send: &watch::Sender<Evidence>, evidence: &Evidence) {
    send.send_if_modified(|previous| {
        if previous == evidence {
            return false;
        }
        *previous = evidence.clone();
        true
    });
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    focus: Res<CameraFocus>,
) {
    commands.insert_resource(AgentMesh(
        meshes.add(Sphere::new(0.48)),
        materials.add(Color::srgb(0.51, 0.43, 0.85)),
    ));
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Orbit::at_home(&focus),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 5500.0,
            ..default()
        },
        Transform::from_rotation(Quat::from_rotation_x(-0.85)),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(20.0),
            right: px(20.0),
            width: px(400.0),
            padding: UiRect::all(px(18.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.033, 0.05, 0.088, 0.96)),
        children![(
            Readout,
            Text::new("SWARM / Reading journal"),
            TextFont::from_font_size(16.0),
            TextColor(Color::srgb(0.84, 0.91, 1.0)),
        )],
    ));
}

fn sync(
    mut commands: Commands,
    mut feed: ResMut<Feed>,
    mut layout: ResMut<Layout>,
    mesh: Res<AgentMesh>,
) {
    let Ok(true) = feed.input.has_changed() else {
        return;
    };
    let latest = feed.input.borrow_and_update().clone();
    feed.evidence = latest;
    let mut agents = BTreeSet::new();
    for edge in feed.evidence.edges.iter().rev() {
        for id in [edge.from, edge.to] {
            if agents.len() < MAX_AGENTS {
                agents.insert(id);
            }
        }
    }
    layout.agents.retain(|id, (entity, _)| {
        if agents.contains(id) {
            return true;
        }
        commands.entity(*entity).despawn();
        false
    });
    if layout.selected.is_some_and(|id| !agents.contains(&id)) {
        layout.selected = None;
    }
    let columns = agents.len().clamp(1, 6);
    let rows = agents.len().div_ceil(columns);
    for (index, id) in agents.into_iter().enumerate() {
        let x = (index % columns) as f32 - (columns - 1) as f32 / 2.0;
        let z = (index / columns) as f32 - rows.saturating_sub(1) as f32 / 2.0;
        let point = Vec3::new(x * 2.1, 0.55, z * 2.3);
        if let Some((entity, prior)) = layout.agents.get_mut(&id) {
            if *prior != point {
                commands
                    .entity(*entity)
                    .insert(Transform::from_translation(point));
                *prior = point;
            }
        } else {
            let entity = commands
                .spawn((
                    Mesh3d(mesh.0.clone()),
                    MeshMaterial3d(mesh.1.clone()),
                    Transform::from_translation(point),
                ))
                .observe(move |click: On<PointerClick>, mut layout: ResMut<Layout>| {
                    if click.button == PointerButton::Primary {
                        layout.selected = Some(id);
                    }
                })
                .id();
            layout.agents.insert(id, (entity, point));
        }
    }
}

fn navigate(
    keys: Res<ButtonInput<KeyCode>>,
    mut layout: ResMut<Layout>,
    mut focus: ResMut<CameraFocus>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        layout.filter = match layout.filter {
            Filter::All => Filter::Delegation,
            Filter::Delegation => Filter::Messages,
            Filter::Messages => Filter::All,
        };
    }
    if keys.just_pressed(KeyCode::KeyN) {
        layout.selected = layout
            .agents
            .keys()
            .find(|id| layout.selected.is_none_or(|selected| **id > selected))
            .or_else(|| layout.agents.keys().next())
            .copied();
    }
    if keys.just_pressed(KeyCode::KeyF) {
        focus.point = layout
            .selected
            .and_then(|id| layout.agents.get(&id).map(|(_, point)| *point));
    }
}

fn draw(feed: Res<Feed>, layout: Res<Layout>, mut gizmos: Gizmos) {
    for edge in feed
        .evidence
        .edges
        .iter()
        .filter(|edge| layout.filter.includes(edge))
    {
        let (Some((_, from)), Some((_, to))) =
            (layout.agents.get(&edge.from), layout.agents.get(&edge.to))
        else {
            continue;
        };
        let (color, height) = match edge.relation {
            Relation::Delegation { .. } => (Color::srgb(0.3, 0.85, 1.0), 0.15),
            Relation::Message => (Color::srgb(1.0, 0.62, 0.23), -0.12),
        };
        let offset = Vec3::Y * height;
        if edge.from == edge.to {
            gizmos.cube(
                Transform::from_translation(*from + offset).with_scale(Vec3::splat(1.2)),
                color,
            );
            continue;
        }
        let direction = (*to - *from).normalize();
        let tip = *to - direction * 0.62 + offset;
        let side = direction.cross(Vec3::Y) * 0.18;
        let base = tip - direction * 0.36;
        gizmos.line(*from + direction * 0.5 + offset, tip, color);
        gizmos.line(tip, base + side, color);
        gizmos.line(tip, base - side, color);
    }
    if let Some((_, point)) = layout.selected.and_then(|id| layout.agents.get(&id)) {
        gizmos.cube(
            Transform::from_translation(*point).with_scale(Vec3::splat(1.02)),
            Color::WHITE,
        );
    }
}

fn inspect(feed: Res<Feed>, layout: Res<Layout>, mut readout: Single<&mut Text, With<Readout>>) {
    if !feed.is_changed() && !layout.is_changed() {
        return;
    }
    let evidence = &feed.evidence;
    let mut detail = String::from("Pick an agent or press N.");
    if let Some(id) = layout.selected {
        let mut edges = evidence
            .edges
            .iter()
            .rev()
            .filter(|edge| layout.filter.includes(edge) && (edge.from == id || edge.to == id));
        let latest = edges.next();
        let count = edges.count() + usize::from(latest.is_some());
        detail = format!("AGENT\n{id}\n\n{count} matching observations");
        if let Some(edge) = latest {
            let peer = if edge.from == id { edge.to } else { edge.from };
            let direction = if edge.from == edge.to {
                "Self"
            } else if edge.from == id {
                "Sent to"
            } else {
                "Received from"
            };
            let observation = match edge.relation {
                Relation::Delegation { task, workflow } => {
                    let outcome = workflow
                        .and_then(|root| {
                            evidence
                                .outcomes
                                .iter()
                                .rev()
                                .find(|(_, id, _)| *id == root)
                                .map(|(_, _, state)| format!("{state:?}"))
                        })
                        .unwrap_or_else(|| "Outcome unavailable".into());
                    let root = workflow.map_or_else(|| "Unlinked".into(), |root| root.to_string());
                    format!("Delegation task\n{task}\nWorkflow\n{root}\nOutcome {outcome}")
                }
                Relation::Message => "Message accepted".into(),
            };
            detail.push_str(&format!(
                "\nLatest sequence {}\n{direction} {peer}\n{observation}",
                edge.sequence
            ));
        }
    }
    let history = if evidence.floor > 0 || evidence.clipped {
        format!("History limited / source floor {}", evidence.floor)
    } else {
        "Window shows up to 24 agents".into()
    };
    readout.0 = format!(
        "VESPERA / SWARM\n{}\n{history}\n{} visible agents / {} observations\n{}\n\n{detail}\n\nCyan delegation | Amber message\nTab filter | N select | F focus\nR reset | WASD pan | Scroll zoom\nRight drag orbit",
        evidence.status,
        layout.agents.len(),
        evidence.edges.len(),
        layout.filter.label(),
    );
}
