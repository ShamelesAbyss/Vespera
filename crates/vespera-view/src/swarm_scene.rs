use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use tokio::sync::{mpsc, watch};
use vespera_core::{AgentId, AgentStatus, EventEnvelope, EventKind, TaskId, TaskState};
use vespera_events::{Cursor, Journal, JournalError, MAX_PAGE};

use crate::Update;
use crate::atlas::{Layer, Layers, register};
use crate::scene::{CameraFocus, Orbit, camera, desktop};

type Focus = CameraFocus<{ Layer::Swarm as usize }>;

const MAX_OBSERVATIONS: usize = 128;
const MAX_AGENTS: usize = 24;

#[derive(Clone, Copy, PartialEq)]
enum Relation {
    Delegation {
        task: TaskId,
        workflow: Option<TaskId>,
        observed_live: bool,
    },
    Dependency {
        upstream: TaskId,
        downstream: TaskId,
        workflow: Option<TaskId>,
    },
    Message,
    MessageAcknowledged,
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
    statuses: VecDeque<(u64, AgentId, AgentStatus)>,
    floor: u64,
    clipped_through: u64,
    head: u64,
    generation: u64,
    status: &'static str,
}

impl Evidence {
    fn apply(&mut self, page: vespera_events::Page) {
        self.prune(page.discarded_through.unsigned_abs());
        for entry in page.entries {
            self.push(entry.cursor.sequence.unsigned_abs(), entry.event);
        }
        self.head = self.head.max(page.next_cursor.sequence.unsigned_abs());
        self.status = if page.has_more {
            "Catching up"
        } else {
            "Following committed journal"
        };
    }

    fn prune(&mut self, floor: u64) {
        self.floor = floor;
        self.head = self.head.max(floor);
        self.retain_window();
    }

    fn retain_window(&mut self) {
        let floor = self.floor.max(self.clipped_through);
        self.edges.retain(|edge| edge.sequence > floor);
        self.outcomes.retain(|(sequence, _, _)| *sequence > floor);
        self.statuses.retain(|(sequence, _, _)| *sequence > floor);
    }

    fn reset(&mut self, status: &'static str) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            status,
            ..default()
        };
    }

    fn push(&mut self, sequence: u64, event: EventEnvelope) {
        self.head = self.head.max(sequence);
        let workflow_evidence = matches!(
            event.source.as_str(),
            "run-workflow" | "workflow-store" | "experiment-workflows"
        );
        match event.kind {
            EventKind::AgentRegistered { agent_id, status }
            | EventKind::AgentStatusChanged {
                agent_id, status, ..
            } if event.source == "agent-runtime" => {
                self.statuses.push_back((sequence, agent_id, status));
            }
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
                        observed_live: false,
                    },
                });
            }
            EventKind::WorkflowAttemptAccepted {
                task_id,
                from_agent_id,
                to_agent_id,
                dependencies,
            } => {
                self.edges
                    .extend(dependencies.into_iter().map(|dependency| Edge {
                        sequence,
                        from: dependency.agent_id,
                        to: to_agent_id,
                        relation: Relation::Dependency {
                            upstream: dependency.task_id,
                            downstream: task_id,
                            workflow: event.correlation_task_id,
                        },
                    }));
                self.edges.push_back(Edge {
                    sequence,
                    from: from_agent_id,
                    to: to_agent_id,
                    relation: Relation::Delegation {
                        task: task_id,
                        workflow: event.correlation_task_id,
                        observed_live: true,
                    },
                });
            }
            EventKind::WorkflowAttemptFinished { task_id, state } if workflow_evidence => {
                self.outcomes.push_back((sequence, task_id, state));
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
            EventKind::AgentMessageAcknowledged {
                from_agent_id,
                to_agent_id,
            } => {
                self.edges.push_back(Edge {
                    sequence,
                    from: to_agent_id,
                    to: from_agent_id,
                    relation: Relation::MessageAcknowledged,
                });
            }
            EventKind::TaskCompleted { task_id } if workflow_evidence => {
                self.outcomes
                    .push_back((sequence, task_id, TaskState::Completed));
            }
            EventKind::TaskFailed { task_id } if workflow_evidence => {
                self.outcomes
                    .push_back((sequence, task_id, TaskState::Failed));
            }
            EventKind::TaskCancelled { task_id } if workflow_evidence => {
                self.outcomes
                    .push_back((sequence, task_id, TaskState::Cancelled));
            }
            _ => {}
        }
        // Keep each compound observation intact when advancing the window.
        if self.edges.len() > MAX_OBSERVATIONS {
            let through = self.edges[self.edges.len() - MAX_OBSERVATIONS - 1].sequence;
            self.clipped_through = self.clipped_through.max(through);
            self.retain_window();
        }
        if self.outcomes.len() > MAX_OBSERVATIONS {
            let through = self.outcomes[0].0;
            self.clipped_through = self.clipped_through.max(through);
            self.retain_window();
        }
        if self.statuses.len() > MAX_OBSERVATIONS {
            let through = self.statuses[0].0;
            self.clipped_through = self.clipped_through.max(through);
            self.retain_window();
        }
    }

    fn outcome(&self, task: TaskId, accepted_at: u64) -> Option<TaskState> {
        self.outcomes
            .iter()
            .rev()
            .find_map(|(sequence, id, state)| {
                (*id == task && *sequence > accepted_at).then_some(*state)
            })
    }

    fn for_workflow(mut self, root: TaskId) -> Self {
        let mut tasks = BTreeSet::from([root]);
        self.edges.retain(|edge| match edge.relation {
            Relation::Delegation {
                task,
                workflow: Some(workflow),
                ..
            } if workflow == root => {
                tasks.insert(task);
                true
            }
            Relation::Dependency {
                upstream,
                downstream,
                workflow: Some(workflow),
            } if workflow == root => {
                tasks.extend([upstream, downstream]);
                true
            }
            _ => false,
        });
        self.outcomes.retain(|(_, task, _)| tasks.contains(task));
        self.statuses.clear();
        self
    }

    fn agent_status(&self, agent: AgentId) -> Option<(u64, AgentStatus)> {
        self.statuses
            .iter()
            .rev()
            .find_map(|&(sequence, id, status)| (agent == id).then_some((sequence, status)))
    }

    fn visible_agents(&self) -> BTreeSet<AgentId> {
        let mut agents = BTreeSet::new();
        let related = self
            .edges
            .iter()
            .rev()
            .flat_map(|edge| [edge.from, edge.to]);
        let registered = self.statuses.iter().rev().map(|&(_, id, _)| id);
        for agent in related.chain(registered) {
            if agents.len() == MAX_AGENTS {
                break;
            }
            agents.insert(agent);
        }
        agents
    }
}

#[derive(Resource)]
struct Feed {
    input: watch::Receiver<Evidence>,
    live: Evidence,
    evidence: Evidence,
    frozen: Option<Frozen>,
}

#[derive(Resource, Default)]
pub(super) struct WorkflowSelection(pub Option<TaskId>);

struct Frozen {
    captured: Evidence,
    sequence: u64,
}

impl Frozen {
    fn new(captured: Evidence) -> Self {
        let sequence = captured.head;
        Self { captured, sequence }
    }

    fn step(&mut self, earlier: bool) {
        let sequences = self
            .captured
            .edges
            .iter()
            .map(|edge| edge.sequence)
            .chain(self.captured.outcomes.iter().map(|outcome| outcome.0))
            .chain(self.captured.statuses.iter().map(|status| status.0))
            .chain([
                self.captured.floor.max(self.captured.clipped_through),
                self.captured.head,
            ]);
        let next = if earlier {
            sequences.filter(|sequence| *sequence < self.sequence).max()
        } else {
            sequences.filter(|sequence| *sequence > self.sequence).min()
        };
        if let Some(sequence) = next {
            self.sequence = sequence;
        }
    }

    fn projection(&self) -> Evidence {
        let mut evidence = self.captured.clone();
        evidence.edges.retain(|edge| edge.sequence <= self.sequence);
        evidence
            .outcomes
            .retain(|(sequence, _, _)| *sequence <= self.sequence);
        evidence
            .statuses
            .retain(|(sequence, _, _)| *sequence <= self.sequence);
        evidence.head = self.sequence;
        evidence
    }
}

#[derive(Resource, Default)]
struct Layout {
    agents: BTreeMap<AgentId, (Entity, Vec3)>,
    selected: Option<AgentId>,
    filter: Filter,
}

pub(super) fn selected_agent(world: &World) -> Option<AgentId> {
    let layout = world.resource::<Layout>();
    layout.selected.filter(|id| layout.agents.contains_key(id))
}

#[derive(Clone, Copy, Default)]
enum Filter {
    #[default]
    All,
    Delegation,
    Dependencies,
    Messages,
}

impl Filter {
    fn includes(self, edge: &Edge) -> bool {
        match self {
            Self::All => true,
            Self::Delegation => matches!(edge.relation, Relation::Delegation { .. }),
            Self::Dependencies => matches!(edge.relation, Relation::Dependency { .. }),
            Self::Messages => matches!(
                edge.relation,
                Relation::Message | Relation::MessageAcknowledged
            ),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All observed relations",
            Self::Delegation => "Delegation",
            Self::Dependencies => "Dependencies",
            Self::Messages => "Messages",
        }
    }
}

#[derive(Resource)]
struct AgentMesh(Handle<Mesh>, Handle<StandardMaterial>);

#[derive(Component)]
struct Readout;

pub(super) fn run(path: PathBuf, reduced_motion: bool) -> Result<(), JournalError> {
    let mut app = desktop(
        "Vespera | Swarm",
        Color::srgb(0.018, 0.025, 0.047),
        Layer::Swarm,
    );
    install_journal(&mut app, path, reduced_motion)?;
    app.run();
    Ok(())
}

pub(super) fn install_journal(
    app: &mut App,
    path: PathBuf,
    reduced_motion: bool,
) -> Result<(), JournalError> {
    Journal::open_reader(&path)?;
    let evidence = Evidence {
        status: "Reading journal",
        ..default()
    };
    let (send, input) = watch::channel(evidence.clone());
    std::thread::spawn(move || observe(&path, &send, false));
    install(app, input, evidence, reduced_motion);
    Ok(())
}

pub(super) fn install_desktop(app: &mut App, dir: PathBuf, reduced_motion: bool) {
    let evidence = Evidence {
        status: "Waiting for desktop workflow observations",
        ..default()
    };
    let (send, input) = watch::channel(evidence.clone());
    std::thread::spawn(move || {
        if let Err(error) = crate::desktop_evidence::restore(&dir) {
            warn!("Desktop workflow evidence recovery: {error}");
        }
        observe(&crate::desktop_evidence::journal_path(&dir), &send, true);
    });
    install(app, input, evidence, reduced_motion);
}

pub(super) fn run_broker(updates: mpsc::Receiver<Update>, reduced_motion: bool) {
    let mut app = desktop(
        "Vespera | Swarm",
        Color::srgb(0.018, 0.025, 0.047),
        Layer::Swarm,
    );
    install_broker(&mut app, updates, reduced_motion);
    app.run();
}

pub(super) fn install_broker(
    app: &mut App,
    mut updates: mpsc::Receiver<Update>,
    reduced_motion: bool,
) {
    let evidence = Evidence {
        status: "Connecting to signed broker feed",
        ..default()
    };
    let (send, input) = watch::channel(evidence.clone());
    let mut observed = evidence.clone();
    std::thread::spawn(move || {
        while let Some(update) = updates.blocking_recv() {
            match update {
                Update::Ready { floor, .. } => {
                    observed.prune(floor);
                    observed.status = "Following signed broker feed";
                }
                Update::Observation(entry) => observed.push(entry.cursor.sequence, entry.event),
                Update::Status(status) => observed.status = status,
                Update::Reset => observed.reset("Broker history changed; rebuilding"),
                Update::Cancelled(..) | Update::CancelFailed | Update::CancelNotSent => {}
            }
            publish(&send, &observed);
            if send.is_closed() {
                break;
            }
        }
    });
    install(app, input, evidence, reduced_motion);
}

fn install(
    app: &mut App,
    input: watch::Receiver<Evidence>,
    evidence: Evidence,
    reduced_motion: bool,
) {
    register(app, Layer::Swarm);
    app.insert_resource(Feed {
        input,
        live: evidence.clone(),
        evidence,
        frozen: None,
    })
    .init_resource::<Layout>()
    .init_resource::<WorkflowSelection>()
    .insert_resource(Focus {
        point: None,
        home_point: Vec3::new(2.0, 0.0, 0.0),
        home_distance: 19.0,
        reduced_motion,
    })
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            sync,
            navigate,
            camera::<{ Layer::Swarm as usize }>,
            draw,
            inspect,
        )
            .chain()
            .run_if(|layers: Res<Layers>| layers.active == Layer::Swarm),
    );
}

fn observe(path: &Path, send: &watch::Sender<Evidence>, desktop: bool) {
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
                evidence.reset("Journal changed; rebuilding observations");
            }
            Err(_) if desktop && matches!(path.try_exists(), Ok(false)) => {
                evidence.status = "Waiting for desktop workflow observations";
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
    focus: Res<Focus>,
    layers: Res<Layers>,
    brand: Res<crate::branding::Brand>,
) {
    commands.insert_resource(AgentMesh(
        meshes.add(Sphere::new(0.48)),
        materials.add(Color::srgb(0.51, 0.43, 0.85)),
    ));
    let camera = commands
        .spawn((
            Layer::Swarm,
            Camera {
                is_active: layers.active == Layer::Swarm,
                ..default()
            },
            Camera3d::default(),
            Tonemapping::None,
            DebandDither::Disabled,
            Orbit::at_home(&focus),
        ))
        .id();
    commands.spawn((UiTargetCamera(camera), crate::branding::header_node(&brand)));
    commands.spawn((
        layers.parent(Layer::Swarm),
        DirectionalLight {
            illuminance: 5500.0,
            ..default()
        },
        Transform::from_rotation(Quat::from_rotation_x(-0.85)),
    ));
    commands.spawn((
        UiTargetCamera(camera),
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
    keys: Res<ButtonInput<KeyCode>>,
    layers: Res<Layers>,
    mut selected_workflow: ResMut<WorkflowSelection>,
) {
    let changed = feed.input.has_changed().unwrap_or(false);
    let toggle = keys.just_pressed(KeyCode::KeyT);
    let earlier = keys.just_pressed(KeyCode::BracketLeft);
    let later = keys.just_pressed(KeyCode::BracketRight) && feed.frozen.is_some();
    if selected_workflow.0.is_some() && keys.just_pressed(KeyCode::Backspace) {
        selected_workflow.0 = None;
    }
    let focus_changed = selected_workflow.is_changed();
    if !(changed || toggle || earlier || later || focus_changed) {
        return;
    }
    if changed {
        let latest = feed.input.borrow_and_update().clone();
        feed.live = latest;
    }
    if focus_changed {
        feed.frozen = None;
        layout.selected = None;
        layout.filter = Filter::All;
    } else if toggle {
        feed.frozen = if feed.frozen.is_some() {
            None
        } else {
            Some(Frozen::new(feed.evidence.clone()))
        };
    } else if earlier || later {
        if feed.frozen.is_none() {
            feed.frozen = Some(Frozen::new(feed.evidence.clone()));
        }
        if let Some(frozen) = &mut feed.frozen {
            frozen.step(earlier);
        }
    } else if feed.frozen.is_some() {
        return;
    }
    feed.evidence = feed.frozen.as_ref().map_or_else(
        || {
            let live = feed.live.clone();
            if let Some(root) = selected_workflow.0 {
                live.for_workflow(root)
            } else {
                live
            }
        },
        Frozen::projection,
    );
    let agents = feed.evidence.visible_agents();
    layout.agents.retain(|id, (entity, _)| {
        if agents.contains(id) {
            return true;
        }
        commands.entity(*entity).despawn();
        false
    });
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
                    layers.parent(Layer::Swarm),
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
    mut focus: ResMut<Focus>,
    layers: Res<Layers>,
) {
    if let Some(id) = layers.incoming
        && layout.agents.contains_key(&id)
    {
        layout.selected = Some(id);
    }
    if keys.just_pressed(KeyCode::Tab) {
        layout.filter = match layout.filter {
            Filter::All => Filter::Delegation,
            Filter::Delegation => Filter::Dependencies,
            Filter::Dependencies => Filter::Messages,
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
            Relation::Dependency { .. } => (Color::srgb(0.43, 0.96, 0.51), 0.34),
            Relation::Message => (Color::srgb(1.0, 0.62, 0.23), -0.12),
            Relation::MessageAcknowledged => (Color::srgb(0.98, 0.44, 0.83), 0.46),
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

fn inspect(
    feed: Res<Feed>,
    layout: Res<Layout>,
    selected_workflow: Res<WorkflowSelection>,
    mut readout: Single<&mut Text, With<Readout>>,
) {
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
        detail = if layout.agents.contains_key(&id) {
            format!("AGENT\n{id}\n\n{count} matching observations")
        } else {
            format!("AGENT\n{id}\n\nOutside the visible agent set")
        };
        if let Some((sequence, status)) = evidence.agent_status(id) {
            detail.push_str(&format!(
                "\nRuntime status {status:?} at sequence {sequence}"
            ));
        }
        if let Some(edge) = latest {
            let peer = if edge.from == id { edge.to } else { edge.from };
            let direction = if edge.relation == Relation::MessageAcknowledged {
                if edge.from == id {
                    "Acknowledged to"
                } else {
                    "Acknowledged by"
                }
            } else if edge.from == edge.to {
                "Self"
            } else if edge.from == id {
                "Sent to"
            } else {
                "Received from"
            };
            let observation = match edge.relation {
                Relation::Delegation {
                    task,
                    workflow,
                    observed_live,
                } => {
                    let outcome = workflow
                        .and_then(|root| evidence.outcome(root, edge.sequence))
                        .map_or_else(|| "Unavailable".into(), |state| format!("{state:?}"));
                    let root = workflow.map_or_else(|| "Unlinked".into(), |root| root.to_string());
                    let status = if observed_live {
                        evidence.outcome(task, edge.sequence).map_or_else(
                            || "Accepted / no terminal evidence".into(),
                            |state| format!("{state:?}"),
                        )
                    } else {
                        "Accepted / historical record".into()
                    };
                    format!(
                        "Delegation task\n{task}\nAttempt {status}\nWorkflow\n{root}\nOutcome {outcome}"
                    )
                }
                Relation::Dependency {
                    upstream,
                    downstream,
                    workflow,
                } => {
                    let root = workflow.map_or_else(|| "Unlinked".into(), |root| root.to_string());
                    format!(
                        "Predecessor supplied to attempt\n{upstream}\nDownstream attempt\n{downstream}\nWorkflow\n{root}"
                    )
                }
                Relation::Message => "Message accepted".into(),
                Relation::MessageAcknowledged => "Recipient acknowledged a message".into(),
            };
            detail.push_str(&format!(
                "\nLatest sequence {}\n{direction} {peer}\n{observation}",
                edge.sequence
            ));
        }
    }
    let floor = evidence.floor.max(evidence.clipped_through);
    let timeline = if let Some(frozen) = &feed.frozen {
        let live = if feed.live.generation == evidence.generation {
            format!("Live sequence {}", feed.live.head)
        } else {
            format!("Live feed reset / sequence {}", feed.live.head)
        };
        format!(
            "Frozen at {} / captured {floor}-{}\n{live}",
            frozen.sequence, frozen.captured.head
        )
    } else {
        format!("Live sequence {}", evidence.head)
    };
    let history = if floor > 0 {
        format!("History limited through sequence {floor}\n")
    } else {
        String::new()
    };
    let workflow = selected_workflow.0.map_or_else(String::new, |root| {
        format!("Workflow focus\n{root}\nBackspace clears workflow focus\n")
    });
    readout.0 = format!(
        "VESPERA / SWARM\n{timeline}\n{}\n{history}{workflow}{} visible agents / {} links observed\n{}\n\n{detail}\n\nCyan delegation | Green dependency\nAmber message | Pink acknowledgement\nT freeze / live | [ ] step evidence\nTab filter | N select | F focus\nR reset | WASD pan | Scroll zoom\nRight drag orbit",
        feed.live.status,
        layout.agents.len(),
        evidence.edges.len(),
        layout.filter.label(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use vespera_core::WorkflowDependency;

    #[test]
    fn workflow_focus_keeps_only_the_selected_workflows_links_and_outcomes() {
        let roots = [TaskId::new(), TaskId::new()];
        let tasks = [TaskId::new(), TaskId::new()];
        let owner = AgentId::new();
        let peers = [AgentId::new(), AgentId::new()];
        let mut evidence = Evidence::default();
        for index in 0..2 {
            for (offset, kind) in [
                (
                    1,
                    EventKind::WorkflowAttemptAccepted {
                        task_id: tasks[index],
                        from_agent_id: owner,
                        to_agent_id: peers[index],
                        dependencies: vec![],
                    },
                ),
                (
                    2,
                    EventKind::WorkflowAttemptFinished {
                        task_id: tasks[index],
                        state: TaskState::Completed,
                    },
                ),
                (
                    3,
                    EventKind::TaskCompleted {
                        task_id: roots[index],
                    },
                ),
            ] {
                let mut event = EventEnvelope::new("workflow-store", 0, kind).unwrap();
                event.correlation_task_id = Some(roots[index]);
                evidence.push(index as u64 * 3 + offset, event);
            }
        }
        let focused = evidence.for_workflow(roots[0]);
        assert_eq!(focused.edges.len(), 1);
        assert_eq!(focused.visible_agents(), BTreeSet::from([owner, peers[0]]));
        assert_eq!(focused.outcome(roots[0], 0), Some(TaskState::Completed));
        assert_eq!(focused.outcome(tasks[0], 0), Some(TaskState::Completed));
        assert_eq!(focused.outcome(roots[1], 0), None);
        assert_eq!(focused.outcome(tasks[1], 0), None);
    }

    #[test]
    fn receipt_adds_a_reverse_link_only_at_its_observed_sequence() {
        let from = AgentId::new();
        let to = AgentId::new();
        let mut live = Evidence::default();
        for (sequence, kind) in [
            (
                1,
                EventKind::AgentMessageSent {
                    from_agent_id: from,
                    to_agent_id: to,
                },
            ),
            (
                2,
                EventKind::AgentMessageAcknowledged {
                    from_agent_id: from,
                    to_agent_id: to,
                },
            ),
        ] {
            live.push(
                sequence,
                EventEnvelope::new("worker_broker", 0, kind).unwrap(),
            );
        }
        let mut frozen = Frozen::new(live);
        let projection = frozen.projection();
        let receipt = projection.edges.back().unwrap();
        assert_eq!((receipt.from, receipt.to), (to, from));
        assert!(receipt.relation == Relation::MessageAcknowledged);
        assert!(Filter::Messages.includes(receipt));
        frozen.step(true);
        let projection = frozen.projection();
        assert_eq!(projection.edges.len(), 1);
        assert!(projection.edges[0].relation == Relation::Message);
    }

    #[test]
    fn durable_workflow_outcomes_and_agent_status_follow_the_replay_cursor() {
        let agent = AgentId::new();
        let root = TaskId::new();
        let task = TaskId::new();
        let mut live = Evidence::default();
        live.push(
            1,
            EventEnvelope::new(
                "agent-runtime",
                0,
                EventKind::AgentRegistered {
                    agent_id: agent,
                    status: AgentStatus::Idle,
                },
            )
            .unwrap(),
        );
        assert_eq!(live.visible_agents(), BTreeSet::from([agent]));
        live.push(
            2,
            EventEnvelope::new(
                "agent-runtime",
                0,
                EventKind::AgentStatusChanged {
                    agent_id: agent,
                    previous: AgentStatus::Idle,
                    status: AgentStatus::Running,
                },
            )
            .unwrap(),
        );
        live.push(
            3,
            EventEnvelope::new(
                "workflow-store",
                0,
                EventKind::WorkflowAttemptAccepted {
                    task_id: task,
                    from_agent_id: agent,
                    to_agent_id: agent,
                    dependencies: vec![],
                },
            )
            .unwrap(),
        );
        live.push(
            4,
            EventEnvelope::new(
                "workflow-store",
                0,
                EventKind::WorkflowAttemptFinished {
                    task_id: task,
                    state: TaskState::Completed,
                },
            )
            .unwrap(),
        );
        live.push(
            5,
            EventEnvelope::new(
                "workflow-store",
                0,
                EventKind::TaskCompleted { task_id: root },
            )
            .unwrap(),
        );
        live.push(
            6,
            EventEnvelope::new(
                "agent-runtime",
                0,
                EventKind::AgentStatusChanged {
                    agent_id: agent,
                    previous: AgentStatus::Running,
                    status: AgentStatus::Idle,
                },
            )
            .unwrap(),
        );
        assert_eq!(live.agent_status(agent), Some((6, AgentStatus::Idle)));
        let mut frozen = Frozen::new(live);
        frozen.step(true);
        let projection = frozen.projection();
        assert_eq!(
            projection.agent_status(agent),
            Some((2, AgentStatus::Running))
        );
        assert_eq!(projection.outcome(task, 3), Some(TaskState::Completed));
        assert_eq!(projection.outcome(root, 3), Some(TaskState::Completed));
        frozen.step(true);
        assert_eq!(frozen.projection().outcome(root, 3), None);
        frozen.step(true);
        assert_eq!(frozen.projection().outcome(task, 3), None);
    }

    #[test]
    fn replay_steps_through_whole_observations_and_hides_future_results() {
        let task = TaskId::new();
        let from = AgentId::new();
        let to = AgentId::new();
        let mut live = Evidence::default();
        live.push(
            3,
            EventEnvelope::new(
                "experiment-workflows",
                0,
                EventKind::WorkflowAttemptAccepted {
                    task_id: task,
                    from_agent_id: from,
                    to_agent_id: to,
                    dependencies: vec![WorkflowDependency {
                        task_id: TaskId::new(),
                        agent_id: AgentId::new(),
                    }],
                },
            )
            .unwrap(),
        );
        live.push(
            7,
            EventEnvelope::new(
                "experiment-workflows",
                0,
                EventKind::WorkflowAttemptFinished {
                    task_id: task,
                    state: TaskState::Completed,
                },
            )
            .unwrap(),
        );
        let mut frozen = Frozen::new(live.clone());
        assert_eq!(
            frozen.projection().outcome(task, 3),
            Some(TaskState::Completed)
        );
        frozen.step(true);
        assert_eq!(frozen.sequence, 3);
        assert_eq!(frozen.projection().edges.len(), 2);
        assert_eq!(frozen.projection().outcome(task, 3), None);
        frozen.step(true);
        assert_eq!(frozen.sequence, 0);
        assert!(frozen.projection().edges.is_empty());
        for sequence in 8..8 + MAX_OBSERVATIONS as u64 - 1 {
            live.push(
                sequence,
                EventEnvelope::new(
                    "broker",
                    0,
                    EventKind::AgentMessageSent {
                        from_agent_id: from,
                        to_agent_id: to,
                    },
                )
                .unwrap(),
            );
        }
        assert_eq!(live.clipped_through, 3);
        assert_eq!(live.edges.len(), MAX_OBSERVATIONS - 1);
        assert!(
            live.edges
                .iter()
                .all(|edge| edge.relation == Relation::Message)
        );
        live.reset("reset");
        assert_ne!(live.generation, frozen.captured.generation);
        frozen.step(false);
        frozen.step(false);
        assert_eq!(frozen.sequence, 7);
        assert_eq!(
            frozen.projection().outcome(task, 3),
            Some(TaskState::Completed)
        );
    }
}
