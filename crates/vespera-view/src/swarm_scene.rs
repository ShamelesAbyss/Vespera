use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use tokio::sync::{mpsc, watch};
use vespera_core::{AgentId, AgentStatus, EventEnvelope, EventKind, TaskId, TaskState};
use vespera_events::{Cursor, Journal, JournalError, MAX_PAGE};

use crate::Update;
use crate::atlas::{Layer, Layers, register};
use crate::scene::{CameraFocus, Orbit, camera, desktop};
use crate::swarm_art::{Foundry, HAZE, SignalColor};

type Focus = CameraFocus<{ Layer::Swarm as usize }>;

const MAX_OBSERVATIONS: usize = 128;
const MAX_AGENTS: usize = 24;
const TRANSIT_SECONDS: f64 = 0.9;

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

    fn latest_attempt(&self, agent: AgentId) -> Option<(TaskId, u64, Option<TaskState>)> {
        self.edges
            .iter()
            .rev()
            .find_map(|edge| match edge.relation {
                Relation::Delegation {
                    task,
                    observed_live: true,
                    ..
                } if edge.to == agent => {
                    Some((task, edge.sequence, self.outcome(task, edge.sequence)))
                }
                _ => None,
            })
    }

    fn next_workflow(&self, agent: AgentId, current: Option<TaskId>) -> Option<TaskId> {
        let mut seen = BTreeSet::new();
        let linked: Vec<_> = self
            .edges
            .iter()
            .rev()
            .filter_map(|edge| {
                if edge.from != agent && edge.to != agent {
                    return None;
                }
                match edge.relation {
                    Relation::Delegation { workflow, .. }
                    | Relation::Dependency { workflow, .. } => workflow,
                    _ => None,
                }
            })
            .filter(|root| seen.insert(*root))
            .collect();
        let next = current
            .and_then(|root| linked.iter().position(|&id| id == root))
            .and_then(|index| linked.get(index + 1));
        next.or_else(|| linked.first()).copied()
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

#[derive(Component)]
struct Readout;

#[derive(Component)]
enum Beacon {
    Runtime(AgentId),
    Attempt(AgentId),
}

pub(super) fn run(
    path: PathBuf,
    reduced_motion: bool,
    capture: Option<PathBuf>,
    capture_transit: bool,
    agent: Option<AgentId>,
    measure_frames: Option<u32>,
) -> Result<(), JournalError> {
    let mut app = desktop("Vespera | Swarm", HAZE, Layer::Swarm);
    install_journal(&mut app, path, reduced_motion)?;
    if agent.is_some() {
        let mut requested = agent;
        app.add_systems(
            Update,
            (move |mut layout: ResMut<Layout>, mut orbit: Single<&mut Orbit>| {
                if let Some(id) = requested
                    && let Some((_, point)) = layout.agents.get(&id)
                {
                    orbit.focus_on(*point + Vec3::Y * 1.40, 9.0);
                    layout.selected = Some(id);
                    requested = None;
                }
            })
            .after(navigate)
            .before(camera::<{ Layer::Swarm as usize }>),
        );
    }
    if let Some(frames) = measure_frames {
        crate::scene_measurement::install(&mut app, frames as usize);
    }
    if let Some(path) = capture {
        let mut settled = 0;
        let mut saved = false;
        app.add_systems(
            Last,
            move |mut commands: Commands,
                  feed: Res<Feed>,
                  time: Res<Time<Real>>,
                  transits: Query<&Transit>| {
                if saved || feed.evidence.status != "Following committed journal" {
                    return;
                }
                let ready = if capture_transit {
                    transits.iter().any(|transit| {
                        time.elapsed_secs_f64() - transit.started >= TRANSIT_SECONDS * 0.5
                    })
                } else {
                    settled += 1;
                    settled >= 120
                };
                if ready {
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()));
                    saved = true;
                }
            },
        );
    }
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
        status: "Waiting for desktop observations",
        ..default()
    };
    let (send, input) = watch::channel(evidence.clone());
    std::thread::spawn(move || {
        if let Err(error) = crate::desktop_evidence::restore(&dir) {
            warn!("Desktop evidence recovery: {error}");
        }
        observe(&crate::desktop_evidence::journal_path(&dir), &send, true);
    });
    install(app, input, evidence, reduced_motion);
}

pub(super) fn run_broker(updates: mpsc::Receiver<Update>, reduced_motion: bool) {
    let mut app = desktop("Vespera | Swarm", HAZE, Layer::Swarm);
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
        home_point: Vec3::new(3.0, 0.7, 0.0),
        home_distance: 17.5,
        reduced_motion,
    })
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            sync,
            navigate,
            camera::<{ Layer::Swarm as usize }>,
            sync_routes,
            update_beacons,
            update_transits,
            draw_selection,
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
                evidence.status = "Waiting for desktop observations";
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
    let art = Foundry::new(&mut meshes, &mut materials);
    art.hall(&mut commands, layers.parent(Layer::Swarm).0);
    commands.insert_resource(art);
    let camera = commands
        .spawn((
            Layer::Swarm,
            Camera {
                is_active: layers.active == Layer::Swarm,
                ..default()
            },
            Camera3d::default(),
            Tonemapping::AcesFitted,
            DebandDither::Enabled,
            AmbientLight {
                color: Color::srgb(0.72, 0.80, 1.0),
                brightness: 1150.0,
                ..default()
            },
            DistanceFog {
                color: HAZE,
                falloff: FogFalloff::Linear {
                    start: 18.0,
                    end: 43.0,
                },
                ..default()
            },
            Orbit::at_home(&focus),
        ))
        .id();
    commands.spawn((UiTargetCamera(camera), crate::branding::header_node(&brand)));
    commands.spawn((
        layers.parent(Layer::Swarm),
        DirectionalLight {
            illuminance: 6700.0,
            color: Color::srgb(1.0, 0.86, 0.72),
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -0.63, -0.95, 0.0)),
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
    commands.spawn((
        UiTargetCamera(camera),
        Node {
            position_type: PositionType::Absolute,
            left: px(20.0),
            bottom: px(56.0),
            width: percent(47.0),
            max_width: px(530.0),
            padding: UiRect::all(px(12.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.033, 0.05, 0.088, 0.94)),
        Pickable::IGNORE,
        children![(
            Text::new("CYAN delegation  |  GREEN dependency\nAMBER message  |  PINK receipt\nUpper beacon: runtime  |  Lower beacon: attempt\nAmber accepted  |  Green complete  |  Red failed\nViolet cancelled or suspended\nN select  |  F focus  |  G cycle workflows  |  Tab filter\nT freeze/live  |  [ ] replay  |  R reset  |  WASD pan\nRight drag orbit  |  Scroll zoom"),
            TextFont::from_font_size(14.0),
            TextColor(Color::srgb(0.77, 0.86, 0.97)),
            Pickable::IGNORE,
        )],
    ));
}

fn sync(
    mut commands: Commands,
    mut feed: ResMut<Feed>,
    mut layout: ResMut<Layout>,
    art: Res<Foundry>,
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
    } else if feed.frozen.is_none()
        && keys.just_pressed(KeyCode::KeyG)
        && let Some(root) = layout
            .selected
            .and_then(|agent| feed.live.next_workflow(agent, selected_workflow.0))
        && selected_workflow.0 != Some(root)
    {
        selected_workflow.0 = Some(root);
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
    if focus_changed && layout.selected.is_some_and(|id| !agents.contains(&id)) {
        layout.selected = None;
    }
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
        let point = if columns == 3 && rows == 1 {
            [
                Vec3::new(0.0, 0.04, -2.7),
                Vec3::new(-3.2, 0.04, 1.6),
                Vec3::new(3.2, 0.04, 1.6),
            ][index]
        } else {
            Vec3::new(x * 3.65, 0.04, z * 3.65)
        };
        if let Some((entity, prior)) = layout.agents.get_mut(&id) {
            if *prior != point {
                commands
                    .entity(*entity)
                    .insert(Transform::from_translation(point).with_scale(Vec3::splat(1.24)));
                *prior = point;
            }
        } else {
            let entity = commands
                .spawn((
                    layers.parent(Layer::Swarm),
                    Visibility::Inherited,
                    Transform::from_translation(point).with_scale(Vec3::splat(1.24)),
                ))
                .observe(move |click: On<PointerClick>, mut layout: ResMut<Layout>| {
                    if click.button == PointerButton::Primary {
                        layout.selected = Some(id);
                    }
                })
                .id();
            art.agent(&mut commands, entity, id);
            for (kind, y, width) in [
                (Beacon::Runtime(id), 2.30, 0.35),
                (Beacon::Attempt(id), 2.12, 0.61),
            ] {
                commands.spawn((
                    ChildOf(entity),
                    kind,
                    art.signal_mesh(),
                    MeshMaterial3d(art.signal_material(SignalColor::Cyan)),
                    Transform::from_xyz(0.0, y, 0.0).with_scale(Vec3::new(width, 0.085, 0.12)),
                    Visibility::Hidden,
                    Pickable::IGNORE,
                ));
            }
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
        focus.point = layout.selected.and_then(|id| {
            layout
                .agents
                .get(&id)
                .map(|(_, point)| *point + Vec3::Y * 1.40)
        });
    }
}

#[derive(Default)]
struct RouteCache {
    root: Option<Entity>,
    spans: Vec<(Vec3, Vec3, usize)>,
}

fn sync_routes(
    mut commands: Commands,
    feed: Res<Feed>,
    layout: Res<Layout>,
    layers: Res<Layers>,
    art: Res<Foundry>,
    mut cache: Local<RouteCache>,
) {
    if !feed.is_changed() && !layout.is_changed() {
        return;
    }
    let mut unique = BTreeSet::new();
    let spans: Vec<_> = feed
        .evidence
        .edges
        .iter()
        .filter_map(|edge| {
            if !layout.filter.includes(edge) {
                return None;
            }
            let lane = match edge.relation {
                Relation::Delegation { .. } => 0,
                Relation::Dependency { .. } => 1,
                Relation::Message => 2,
                Relation::MessageAcknowledged => 3,
            };
            if !unique.insert((edge.from, edge.to, lane)) {
                return None;
            }
            Some((
                layout.agents.get(&edge.from)?.1,
                layout.agents.get(&edge.to)?.1,
                lane,
            ))
        })
        .collect();
    if spans == cache.spans {
        return;
    }
    if let Some(root) = cache.root.take() {
        commands.entity(root).despawn();
    }
    if !spans.is_empty() {
        let root = commands
            .spawn((
                layers.parent(Layer::Swarm),
                Transform::default(),
                Visibility::Inherited,
            ))
            .id();
        for &(from, to, lane) in &spans {
            art.route(&mut commands, root, from, to, lane);
        }
        cache.root = Some(root);
    }
    cache.spans = spans;
}

fn draw_selection(layout: Res<Layout>, mut gizmos: Gizmos) {
    if let Some((_, point)) = layout.selected.and_then(|id| layout.agents.get(&id)) {
        gizmos.cube(
            Transform::from_translation(*point + Vec3::Y * 0.24)
                .with_scale(Vec3::new(1.76, 0.16, 1.76)),
            Color::WHITE,
        );
    }
}

fn update_beacons(
    feed: Res<Feed>,
    art: Res<Foundry>,
    mut beacons: Query<(
        &Beacon,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    for (beacon, mut visibility, mut material) in &mut beacons {
        let color = match *beacon {
            Beacon::Runtime(id) => match feed.evidence.agent_status(id).map(|(_, state)| state) {
                Some(AgentStatus::Running) => Some(SignalColor::Cyan),
                Some(AgentStatus::Failed) => Some(SignalColor::Red),
                Some(AgentStatus::Suspended) => Some(SignalColor::Violet),
                _ => None,
            },
            Beacon::Attempt(id) => {
                feed.evidence
                    .latest_attempt(id)
                    .map(|(_, _, state)| match state {
                        Some(TaskState::Completed) => SignalColor::Green,
                        Some(TaskState::Failed) => SignalColor::Red,
                        Some(TaskState::Cancelled) => SignalColor::Violet,
                        Some(TaskState::Running) => SignalColor::Cyan,
                        _ => SignalColor::Amber,
                    })
            }
        };
        let next = if let Some(color) = color {
            let handle = art.signal_material(color);
            if material.0 != handle {
                material.0 = handle;
            }
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != next {
            *visibility = next;
        }
    }
}

#[derive(Component)]
struct Transit {
    edge: Edge,
    started: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TransitCursor {
    generation: u64,
    head: u64,
    workflow: Option<TaskId>,
    frozen: bool,
}

fn update_transits(
    mut commands: Commands,
    time: Res<Time<Real>>,
    scene: (Res<Feed>, Res<Layout>, Res<WorkflowSelection>),
    settings: (Res<Layers>, Res<Focus>),
    art: Res<Foundry>,
    mut last: Local<Option<TransitCursor>>,
    mut transits: Query<(Entity, &Transit, &mut Transform)>,
) {
    let (feed, layout, workflow) = scene;
    let (layers, focus) = settings;
    let cursor = TransitCursor {
        generation: feed.evidence.generation,
        head: feed.evidence.head,
        workflow: workflow.0,
        frozen: feed.frozen.is_some(),
    };
    let previous = last.replace(cursor);
    let enabled = !cursor.frozen && !focus.reduced_motion && !layers.switched;
    let same_feed = previous.is_some_and(|old| {
        !old.frozen && old.generation == cursor.generation && old.workflow == cursor.workflow
    });
    let now = time.elapsed_secs_f64();
    let endpoints = |edge: Edge| {
        let from = layout.agents.get(&edge.from)?.1;
        let to = layout.agents.get(&edge.to)?.1;
        if from == to || !layout.filter.includes(&edge) {
            return None;
        }
        let (lane, color) = match edge.relation {
            Relation::Delegation {
                observed_live: true,
                ..
            } => (0.0, SignalColor::Cyan),
            Relation::Message => (2.0, SignalColor::Amber),
            Relation::MessageAcknowledged => (3.0, SignalColor::Pink),
            _ => return None,
        };
        let direction = (to - from).normalize();
        let offset = Vec3::Y * (0.29 + lane * 0.13) + direction.cross(Vec3::Y) * 0.17;
        Some((
            from + direction * 0.80 + offset,
            to - direction * 0.80 + offset,
            color,
        ))
    };
    let mut occupied = 0;
    for (entity, transit, mut transform) in &mut transits {
        let progress = ((now - transit.started) / TRANSIT_SECONDS) as f32;
        if enabled
            && same_feed
            && progress < 1.0
            && let Some((from, to, _)) = endpoints(transit.edge)
        {
            transform.translation = from.lerp(to, progress);
            occupied += 1;
        } else {
            commands.entity(entity).despawn();
        }
    }
    if !enabled {
        return;
    }
    let Some(previous) = previous.filter(|old| same_feed && old.head > 0 && old.head < cursor.head)
    else {
        return;
    };
    for (edge, from, color) in feed
        .evidence
        .edges
        .iter()
        .rev()
        .take_while(|edge| edge.sequence > previous.head)
        .filter_map(|&edge| endpoints(edge).map(|(from, _, color)| (edge, from, color)))
        .take(8usize.saturating_sub(occupied))
    {
        commands.spawn((
            layers.parent(Layer::Swarm),
            Transit { edge, started: now },
            art.signal_mesh(),
            MeshMaterial3d(art.signal_material(color)),
            Transform::from_translation(from).with_scale(Vec3::splat(0.15)),
            Pickable::IGNORE,
        ));
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
        if let Some((task, sequence, outcome)) = evidence.latest_attempt(id) {
            let label = outcome.map_or_else(
                || "Accepted, no terminal evidence".into(),
                |state| format!("{state:?}"),
            );
            detail.push_str(&format!(
                "\nLatest assigned attempt at #{sequence}\n{task}\n{label}"
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
        "VESPERA / SWARM\n{timeline}\n{}\n{history}{workflow}{} visible agents / {} links observed\n{}\n\n{detail}",
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
        assert_eq!(evidence.next_workflow(owner, None), Some(roots[1]));
        assert_eq!(
            evidence.next_workflow(owner, Some(roots[1])),
            Some(roots[0])
        );
        assert_eq!(
            evidence.next_workflow(owner, Some(roots[0])),
            Some(roots[1])
        );
        assert_eq!(evidence.next_workflow(peers[0], None), Some(roots[0]));
        let focused = evidence.for_workflow(roots[0]);
        assert_eq!(focused.next_workflow(owner, None), Some(roots[0]));
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
        assert_eq!(
            projection.latest_attempt(agent),
            Some((task, 3, Some(TaskState::Completed)))
        );
        assert_eq!(projection.outcome(root, 3), Some(TaskState::Completed));
        frozen.step(true);
        assert_eq!(frozen.projection().outcome(root, 3), None);
        frozen.step(true);
        assert_eq!(frozen.projection().outcome(task, 3), None);
        assert_eq!(
            frozen.projection().latest_attempt(agent),
            Some((task, 3, None))
        );
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
