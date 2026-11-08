use std::collections::{BTreeMap, VecDeque};

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use tokio::sync::mpsc;
use vespera_core::{AgentId, ExternalTaskChange, TaskId, TaskState};
use vespera_view::Graph;

use crate::Update as BrokerUpdate;
use crate::atlas::{Layer, Layers, register};
use crate::task_timeline::Timeline;

#[derive(Resource)]
struct Connection {
    updates: mpsc::Receiver<BrokerUpdate>,
    cancel: Option<mpsc::Sender<TaskId>>,
    status: &'static str,
    cancellation: Cancellation,
    timeline: Timeline,
}

#[derive(Default, Clone, Copy)]
enum Cancellation {
    #[default]
    Idle,
    Confirm(TaskId),
    Pending(TaskId),
    Finished(TaskId, TaskState),
    Uncertain,
}

#[derive(Resource, Default)]
struct Scene {
    graph: Graph,
    entities: BTreeMap<NodeKey, Entity>,
    layout: BTreeMap<NodeKey, Vec3>,
    recent: VecDeque<(u64, TaskId, ExternalTaskChange)>,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NodeKey {
    Task(TaskId),
    Worker(AgentId),
}

#[derive(Resource, Default)]
struct Selection(Option<NodeKey>);

pub(super) fn selected_agent(world: &World) -> Option<AgentId> {
    match world.resource::<Selection>().0 {
        Some(NodeKey::Worker(id)) if world.resource::<Scene>().graph.workers.contains_key(&id) => {
            Some(id)
        }
        _ => None,
    }
}

#[derive(Resource, Default)]
enum Filter {
    #[default]
    All,
    Work,
    Outcomes,
}

impl Filter {
    fn shows(&self, state: TaskState) -> bool {
        let work = matches!(state, TaskState::Queued | TaskState::Running);
        match self {
            Self::All => true,
            Self::Work => work,
            Self::Outcomes => !work,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::All => "All tasks",
            Self::Work => "Queued + running",
            Self::Outcomes => "Terminal outcomes",
        }
    }
}

#[derive(Resource)]
struct Palette {
    task: Handle<Mesh>,
    worker: Handle<Mesh>,
    materials: [Handle<StandardMaterial>; 7],
}

#[derive(Component)]
struct Inspector;

#[derive(Component)]
struct StatusStrip;

#[derive(Component)]
struct LaneTarget(Vec3);

#[derive(Component)]
pub(super) struct Orbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    zoom_goal: Option<f32>,
    transition: Option<(Vec3, f32)>,
}

impl Default for Orbit {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            yaw: 0.45,
            pitch: 0.62,
            distance: 17.0,
            zoom_goal: None,
            transition: None,
        }
    }
}

impl Orbit {
    pub(super) fn with_distance(distance: f32) -> Self {
        Self {
            distance,
            ..default()
        }
    }

    pub(super) fn at_home<const L: usize>(home: &CameraFocus<L>) -> Self {
        Self {
            focus: home.home_point,
            pitch: if L == Layer::World as usize {
                0.73
            } else {
                Self::default().pitch
            },
            ..Self::with_distance(home.home_distance)
        }
    }
}

#[derive(Resource)]
pub(super) struct CameraFocus<const L: usize> {
    pub point: Option<Vec3>,
    pub home_point: Vec3,
    pub home_distance: f32,
    pub reduced_motion: bool,
}

type Focus = CameraFocus<{ Layer::Network as usize }>;

pub(super) fn desktop(title: &str, clear: Color, layer: Layer) -> App {
    let mut app = App::new();
    app.insert_resource(Layers::new(layer))
        .insert_resource(ClearColor(clear))
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: title.into(),
                    resolution: (1200, 800).into(),
                    ..default()
                }),
                ..default()
            }),
            MeshPickingPlugin,
        ));
    crate::branding::install(&mut app);
    app
}

pub fn run(
    updates: mpsc::Receiver<BrokerUpdate>,
    cancel: Option<mpsc::Sender<TaskId>>,
    reduced_motion: bool,
) {
    let mut app = desktop(
        "Vespera | Intelligence Atlas",
        Color::srgb(0.017, 0.023, 0.043),
        Layer::Network,
    );
    install(&mut app, updates, cancel, reduced_motion);
    app.run();
}

pub(super) fn install(
    app: &mut App,
    updates: mpsc::Receiver<BrokerUpdate>,
    cancel: Option<mpsc::Sender<TaskId>>,
    reduced_motion: bool,
) {
    register(app, Layer::Network);
    app.insert_resource(Connection {
        updates,
        cancel,
        status: "Connecting to broker...",
        cancellation: Cancellation::Idle,
        timeline: Timeline::default(),
    })
    .init_resource::<Scene>()
    .init_resource::<Selection>()
    .init_resource::<Filter>()
    .insert_resource(Focus {
        point: None,
        home_point: Vec3::new(2.65, 0.0, -1.25),
        home_distance: 25.0,
        reduced_motion,
    })
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            navigate,
            sync_scene,
            move_nodes,
            focus_selection,
            camera::<{ Layer::Network as usize }>,
            draw_links,
            inspect,
        )
            .chain()
            .run_if(|layers: Res<Layers>| layers.active == Layer::Network)
            .after(receive),
    )
    .add_systems(Update, receive);
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    focus: Res<Focus>,
    layers: Res<Layers>,
) {
    let colors = [
        Color::srgb(0.99, 0.65, 0.27),
        Color::srgb(0.14, 0.84, 1.0),
        Color::srgb(0.19, 0.81, 0.53),
        Color::srgb(0.96, 0.30, 0.39),
        Color::srgb(0.49, 0.53, 0.66),
        Color::srgb(0.66, 0.43, 1.0),
        Color::srgb(0.96, 0.95, 1.0),
    ];
    commands.insert_resource(Palette {
        task: meshes.add(Cuboid::new(0.64, 0.44, 0.64)),
        worker: meshes.add(Sphere::new(0.46)),
        materials: colors.map(|color| {
            materials.add(StandardMaterial {
                base_color: color,
                unlit: true,
                ..default()
            })
        }),
    });
    let camera = commands
        .spawn((
            Layer::Network,
            Camera {
                is_active: layers.active == Layer::Network,
                ..default()
            },
            Camera3d::default(),
            Tonemapping::None,
            DebandDither::Disabled,
            Orbit {
                focus: focus.home_point,
                ..Orbit::with_distance(focus.home_distance)
            },
            Transform::from_xyz(8.0, 12.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    commands.spawn((
        layers.parent(Layer::Network),
        Mesh3d(meshes.add(Cuboid::new(25.0, 0.1, 68.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.038, 0.05, 0.082),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.2, 23.0),
        Pickable::IGNORE,
    ));
    commands.spawn((
        UiTargetCamera(camera),
        Node {
            position_type: PositionType::Absolute,
            top: px(22),
            left: px(24),
            ..default()
        },
        Text::new("VESPERA   /   LIVING INTELLIGENCE ATLAS"),
        TextFont::from_font_size(24.0),
        TextColor(Color::srgb(0.86, 0.92, 1.0)),
    ));
    commands.spawn((
        UiTargetCamera(camera),
        StatusStrip,
        Node {
            position_type: PositionType::Absolute,
            top: px(66),
            left: px(24),
            right: px(24),
            padding: UiRect::axes(px(12), px(8)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.045, 0.06, 0.11, 0.96)),
        Text::new("INTELLIGENCE NETWORK  |  Connecting..."),
        TextFont::from_font_size(15.0),
        TextColor(Color::srgb(0.73, 0.84, 0.98)),
    ));
    commands.spawn((
        UiTargetCamera(camera),
        Inspector,
        Node {
            position_type: PositionType::Absolute,
            top: px(146),
            right: px(22),
            width: px(340),
            padding: UiRect::all(px(15)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.045, 0.07, 0.12, 0.93)),
        Text::new("Connecting to broker..."),
        TextFont::from_font_size(15.0),
        TextColor(Color::srgb(0.85, 0.91, 0.98)),
    ));
}

fn receive(
    mut scene: ResMut<Scene>,
    mut connection: ResMut<Connection>,
    mut selection: ResMut<Selection>,
    keys: Res<ButtonInput<KeyCode>>,
    layers: Res<Layers>,
) {
    let active = layers.active == Layer::Network;
    if !active && matches!(connection.cancellation, Cancellation::Confirm(_)) {
        connection.cancellation = Cancellation::Idle;
    }
    let earlier = active && keys.just_pressed(KeyCode::BracketLeft);
    let later =
        active && connection.timeline.is_frozen() && keys.just_pressed(KeyCode::BracketRight);
    let mut refresh = active && keys.just_pressed(KeyCode::KeyT);
    if refresh {
        if connection.timeline.source_changed() {
            selection.0 = None;
        }
        connection.timeline.toggle();
    } else if earlier || later {
        connection.timeline.step(earlier);
        refresh = true;
    }
    for _ in 0..64 {
        let Ok(update) = connection.updates.try_recv() else {
            break;
        };
        match update {
            BrokerUpdate::Ready(discarded_through) => {
                connection.timeline.ready(discarded_through);
                connection.status = if discarded_through == 0 {
                    "Connected - live broker"
                } else {
                    "Connected - earlier history was pruned"
                };
            }
            BrokerUpdate::Status(status) => {
                connection.status = status;
                if !status.starts_with("Connected") {
                    connection.cancellation = match connection.cancellation {
                        Cancellation::Pending(_) => Cancellation::Uncertain,
                        Cancellation::Confirm(_) => Cancellation::Idle,
                        state => state,
                    };
                }
            }
            BrokerUpdate::Observation(event) => {
                refresh |= connection.timeline.observe(event);
            }
            BrokerUpdate::Cancelled(task, state) => {
                connection.cancellation = Cancellation::Finished(task, state);
            }
            BrokerUpdate::CancelFailed => connection.cancellation = Cancellation::Uncertain,
            BrokerUpdate::Reset => {
                connection.timeline.reset();
                if !connection.timeline.is_frozen() {
                    refresh = true;
                    selection.0 = None;
                }
                if matches!(connection.cancellation, Cancellation::Confirm(_)) {
                    connection.cancellation = Cancellation::Idle;
                }
            }
        }
    }
    if refresh {
        let projected = connection.timeline.projection();
        scene.graph = projected.graph;
        scene.recent = projected.recent;
    }
}

fn navigate(
    keys: Res<ButtonInput<KeyCode>>,
    scene: Res<Scene>,
    mut selection: ResMut<Selection>,
    mut filter: ResMut<Filter>,
    layers: Res<Layers>,
) {
    if let Some(id) = layers.incoming
        && scene.graph.workers.contains_key(&id)
    {
        selection.0 = Some(NodeKey::Worker(id));
    }
    if keys.just_pressed(KeyCode::Tab) {
        *filter = match *filter {
            Filter::All => Filter::Work,
            Filter::Work => Filter::Outcomes,
            Filter::Outcomes => Filter::All,
        };
    }
    let mut candidates: Vec<_> = if keys.just_pressed(KeyCode::KeyN) {
        scene
            .graph
            .tasks
            .iter()
            .filter(|(_, task)| filter.shows(task.state))
            .map(|(&id, task)| (task.slot, NodeKey::Task(id)))
            .collect()
    } else if keys.just_pressed(KeyCode::KeyB) {
        scene
            .graph
            .workers
            .iter()
            .map(|(&id, &slot)| (slot, NodeKey::Worker(id)))
            .collect()
    } else {
        return;
    };
    candidates.sort_unstable();
    let next = candidates
        .iter()
        .position(|(_, key)| Some(*key) == selection.0)
        .map_or(0, |index| index + 1);
    selection.0 = candidates
        .get(next)
        .or_else(|| candidates.first())
        .map(|(_, key)| *key);
}

const fn lane(state: TaskState) -> usize {
    match state {
        TaskState::Queued => 0,
        TaskState::Running => 1,
        TaskState::Completed => 2,
        TaskState::Failed => 3,
        TaskState::Cancelled => 4,
    }
}

fn layout(graph: &Graph) -> BTreeMap<NodeKey, Vec3> {
    let mut positions = BTreeMap::new();
    let mut lanes = [0; 5];
    let mut tasks: Vec<_> = graph.tasks.iter().collect();
    tasks.sort_unstable_by_key(|(_, task)| task.slot);
    for (&id, task) in tasks {
        let band = lane(task.state);
        let slot = lanes[band];
        lanes[band] += 1;
        let x = (band as f32 - 2.0) * 4.4 + (slot % 4) as f32 * 0.87 - 1.3;
        let y = if task.state == TaskState::Running {
            0.88
        } else {
            0.26
        };
        positions.insert(
            NodeKey::Task(id),
            Vec3::new(x, y, 0.9 + (slot / 4) as f32 * 0.88),
        );
    }
    for (&id, &slot) in &graph.workers {
        let column = (slot % 16) as i32;
        let alternating = ((column + 1) / 2) * if column % 2 == 0 { -1 } else { 1 };
        let x = alternating as f32 * 1.45;
        let row = (slot / 16) as f32;
        positions.insert(
            NodeKey::Worker(id),
            Vec3::new(x, 1.5 + row * 0.42, -3.2 - row * 1.45),
        );
    }
    positions
}

fn sync_scene(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    selection: Res<Selection>,
    filter: Res<Filter>,
    palette: Res<Palette>,
    layers: Res<Layers>,
    mut existing: Query<(
        &mut MeshMaterial3d<StandardMaterial>,
        &mut LaneTarget,
        &mut Visibility,
    )>,
) {
    if !scene.is_changed() && !selection.is_changed() && !filter.is_changed() {
        return;
    }
    let positions = layout(&scene.graph);
    scene.entities.retain(|key, entity| {
        if positions.contains_key(key) {
            return true;
        }
        commands.entity(*entity).despawn();
        false
    });
    for (&key, &location) in &positions {
        let (mesh, index, visible) = match key {
            NodeKey::Task(id) => (
                &palette.task,
                lane(scene.graph.tasks[&id].state),
                filter.shows(scene.graph.tasks[&id].state),
            ),
            NodeKey::Worker(_) => (&palette.worker, 5, true),
        };
        let visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let material = &palette.materials[if selection.0 == Some(key) { 6 } else { index }];
        if let Some(entity) = scene.entities.get(&key) {
            if let Ok((mut current, mut target, mut visibility_now)) = existing.get_mut(*entity) {
                if current.0 != *material {
                    current.0 = material.clone();
                }
                if target.0 != location {
                    target.0 = location;
                }
                if *visibility_now != visibility {
                    *visibility_now = visibility;
                }
            }
        } else {
            let entity = commands
                .spawn((
                    layers.parent(Layer::Network),
                    key,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(location)
                        .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4)),
                    LaneTarget(location),
                    visibility,
                ))
                .observe(
                    move |event: On<PointerClick>, mut selected: ResMut<Selection>| {
                        if event.button == PointerButton::Primary {
                            selected.0 = Some(key);
                        }
                    },
                )
                .id();
            scene.entities.insert(key, entity);
        }
    }
    scene.layout = positions;
}

fn move_nodes(
    time: Res<Time>,
    focus: Res<Focus>,
    connection: Res<Connection>,
    mut nodes: Query<(&LaneTarget, &mut Transform)>,
) {
    let factor = if focus.reduced_motion || connection.timeline.is_frozen() {
        1.0
    } else {
        1.0 - (-10.0 * time.delta_secs()).exp()
    };
    for (target, mut transform) in &mut nodes {
        if transform.translation != target.0 {
            transform.translation = if transform.translation.distance_squared(target.0) < 0.0001 {
                target.0
            } else {
                transform.translation.lerp(target.0, factor)
            };
        }
    }
}

fn draw_links(
    scene: Res<Scene>,
    selection: Res<Selection>,
    filter: Res<Filter>,
    nodes: Query<&Transform, With<NodeKey>>,
    mut gizmos: Gizmos,
) {
    let grid = Color::srgb(0.075, 0.11, 0.19);
    for index in -6..=6 {
        let x = index as f32 * 2.0;
        gizmos.line(Vec3::new(x, -0.14, -11.0), Vec3::new(x, -0.14, 57.0), grid);
    }
    for index in -5..=28 {
        let z = index as f32 * 2.0;
        gizmos.line(Vec3::new(-12.0, -0.14, z), Vec3::new(12.0, -0.14, z), grid);
    }
    for (band, color) in [
        Color::srgb(0.62, 0.39, 0.16),
        Color::srgb(0.09, 0.42, 0.51),
        Color::srgb(0.12, 0.39, 0.28),
        Color::srgb(0.51, 0.18, 0.27),
        Color::srgb(0.30, 0.33, 0.44),
    ]
    .into_iter()
    .enumerate()
    {
        let x = (band as f32 - 2.0) * 4.4 - 1.85;
        gizmos.line(Vec3::new(x, -0.12, 0.0), Vec3::new(x, -0.12, 57.0), color);
    }
    let point = |key: NodeKey| {
        scene
            .entities
            .get(&key)
            .and_then(|entity| nodes.get(*entity).ok())
            .map(|pose| pose.translation)
    };
    for (&id, task) in &scene.graph.tasks {
        if filter.shows(task.state)
            && let Some(attempt) = task.attempt
            && (task.state == TaskState::Running
                || selection.0 == Some(NodeKey::Task(id))
                || selection.0 == Some(NodeKey::Worker(attempt.worker_id)))
            && let Some(start) = point(NodeKey::Worker(attempt.worker_id))
            && let Some(end) = point(NodeKey::Task(id))
        {
            let color = if task.state == TaskState::Running {
                Color::srgb(0.23, 0.77, 1.0)
            } else {
                Color::srgb(0.26, 0.35, 0.50)
            };
            gizmos.line(start, end, color);
        }
    }
    if let Some(key) = selection.0
        && match key {
            NodeKey::Task(id) => scene
                .graph
                .tasks
                .get(&id)
                .is_some_and(|task| filter.shows(task.state)),
            NodeKey::Worker(_) => true,
        }
        && let Some(center) = point(key)
    {
        let ring = |index| {
            let angle = index as f32 * std::f32::consts::TAU / 24.0;
            center + Vec3::new(angle.cos() * 0.72, -0.26, angle.sin() * 0.72)
        };
        for index in 0..24 {
            gizmos.line(ring(index), ring(index + 1), Color::srgb(0.73, 0.84, 1.0));
        }
    }
}

fn focus_selection(graph: Res<Scene>, selection: Res<Selection>, mut focus: ResMut<Focus>) {
    focus.point = selection.0.and_then(|key| graph.layout.get(&key).copied());
}

pub(super) fn camera<const L: usize>(
    mut cameras: Query<(&Camera, &mut Orbit, &mut Transform)>,
    (keys, buttons): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    focus: Res<CameraFocus<L>>,
) {
    let Some((_, mut orbit, mut transform)) =
        cameras.iter_mut().find(|(camera, ..)| camera.is_active)
    else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyR) {
        let (previous, distance) = (orbit.focus, orbit.distance);
        *orbit = Orbit::at_home(&focus);
        orbit.focus = previous;
        orbit.distance = distance;
        orbit.transition = Some((focus.home_point, focus.home_distance));
    }
    if keys.just_pressed(KeyCode::KeyF)
        && let Some(focus) = focus.point
    {
        orbit.zoom_goal = None;
        orbit.transition = Some((focus, if L == Layer::World as usize { 6.5 } else { 9.0 }));
    }
    if buttons.pressed(MouseButton::Right) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch = (orbit.pitch + motion.delta.y * 0.006).clamp(0.16, 1.46);
    }
    if scroll.delta.y != 0.0 {
        let current = orbit.zoom_goal.unwrap_or(orbit.distance);
        let maximum = if L == Layer::World as usize {
            (focus.home_distance * 1.25).max(85.0)
        } else {
            85.0
        };
        let next = (current * (-scroll.delta.y * 0.08).exp()).clamp(3.0, maximum);
        if L == Layer::World as usize && !focus.reduced_motion {
            orbit.zoom_goal = Some(next);
        } else {
            orbit.distance = next;
        }
    }
    if let Some(goal) = orbit.zoom_goal {
        orbit.distance += (goal - orbit.distance) * (1.0 - (-17.0 * time.delta_secs()).exp());
        if (goal - orbit.distance).abs() < 0.01 {
            orbit.distance = goal;
            orbit.zoom_goal = None;
        }
    }
    let lateral = f32::from(u8::from(keys.pressed(KeyCode::KeyD)))
        - f32::from(u8::from(keys.pressed(KeyCode::KeyA)));
    let forward = f32::from(u8::from(keys.pressed(KeyCode::KeyS)))
        - f32::from(u8::from(keys.pressed(KeyCode::KeyW)));
    if (buttons.pressed(MouseButton::Right) && motion.delta != Vec2::ZERO)
        || scroll.delta != Vec2::ZERO
        || lateral != 0.0
        || forward != 0.0
    {
        orbit.transition = None;
    }
    if let Some((target, distance)) = orbit.transition {
        let factor = if focus.reduced_motion {
            1.0
        } else {
            1.0 - (-8.0 * time.delta_secs()).exp()
        };
        orbit.focus = orbit.focus.lerp(target, factor);
        orbit.distance += (distance - orbit.distance) * factor;
        if orbit.focus.distance_squared(target) < 0.0001 && (orbit.distance - distance).abs() < 0.01
        {
            orbit.focus = target;
            orbit.distance = distance;
            orbit.transition = None;
        }
    }
    let rotation = Quat::from_rotation_y(orbit.yaw);
    let pan_speed = if L == Layer::World as usize {
        (orbit.distance * 0.36).clamp(2.0, 18.0)
    } else {
        8.0
    };
    orbit.focus += rotation * Vec3::new(lateral, 0.0, forward) * time.delta_secs() * pan_speed;
    let offset = rotation * Vec3::new(0.0, orbit.pitch.sin(), orbit.pitch.cos()) * orbit.distance;
    *transform = Transform::from_translation(orbit.focus + offset).looking_at(orbit.focus, Vec3::Y);
}

fn inspect(
    scene: Res<Scene>,
    selection: Res<Selection>,
    filter: Res<Filter>,
    (keys, layers): (Res<ButtonInput<KeyCode>>, Res<Layers>),
    mut connection: ResMut<Connection>,
    mut inspector: Single<&mut Text, (With<Inspector>, Without<StatusStrip>)>,
    mut status: Single<&mut Text, (With<StatusStrip>, Without<Inspector>)>,
) {
    if !layers.switched {
        update_cancellation(&keys, selection.0, &scene.graph, &mut connection);
    }
    let mut counts = [0; 5];
    for task in scene.graph.tasks.values() {
        counts[lane(task.state)] += 1;
    }
    let headline = format!(
        "INTELLIGENCE NETWORK   |   {}   |   {}   |   {} workers\nQUEUED {}     RUNNING {}     COMPLETE {}     FAILED {}     CANCELLED {}\n{}",
        connection.status,
        filter.label(),
        scene.graph.workers.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4],
        connection.timeline.label(),
    );
    if status.0 != headline {
        status.0 = headline;
    }
    let detail = match selection.0 {
        Some(NodeKey::Task(id)) => {
            if let Some(task) = scene.graph.tasks.get(&id) {
                let attempt = task.attempt.map_or_else(
                    || "No attempt in this observation".into(),
                    |attempt| {
                        format!(
                            "Worker {}\nAttempt {}\nGeneration {}",
                            attempt.worker_id, attempt.attempt_id, attempt.generation
                        )
                    },
                );
                format!(
                    "TASK  {id}\n{} / {}{}\n{attempt}\nObserved Unix ms {}",
                    task.state,
                    change_label(task.change),
                    if filter.shows(task.state) { "" } else { " (filtered)" },
                    task.observed_at_unix_ms
                )
            } else {
                format!("TASK  {id}\nUnavailable at this sequence")
            }
        }
        Some(NodeKey::Worker(id)) => {
            if !scene.graph.workers.contains_key(&id) {
                format!("WORKER  {id}\nUnavailable at this sequence")
            } else {
                let mut seen = 0;
                let mut running = 0;
                for task in scene.graph.tasks.values().filter(|task| {
                    task.attempt.is_some_and(|attempt| attempt.worker_id == id)
                }) {
                    seen += 1;
                    running += usize::from(task.state == TaskState::Running);
                }
                format!("WORKER  {id}\n{running} running / {seen} attributed tasks\nLatest observed attempts only")
            }
        }
        None => "Pick a task or worker to inspect it.\nCyan links: observed running attempts.\nDim links: selected attempt history.".into(),
    };
    let receipt = match connection.cancellation {
        Cancellation::Confirm(id) => format!(
            "Confirm cancel {}?\nY - confirm   Esc - dismiss",
            short_id(id)
        ),
        Cancellation::Pending(id) => {
            format!("Cancellation sent for {}. Awaiting broker.", short_id(id))
        }
        Cancellation::Finished(id, state) => {
            format!("Broker response: {} is {state}.", short_id(id))
        }
        Cancellation::Uncertain => "Cancellation outcome uncertain. Check the broker.".into(),
        Cancellation::Idle if connection.timeline.is_frozen() => {
            "Replay controls are read-only".into()
        }
        Cancellation::Idle if connection.cancel.is_some() => "C - request cancellation".into(),
        Cancellation::Idle => "Read-only controls".into(),
    };
    let activity = scene
        .recent
        .iter()
        .rev()
        .map(|(sequence, id, change)| {
            format!(
                "#{sequence:<4} {}  {}",
                short_id(*id),
                change_label(*change)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!(
        "INSPECTOR\n\n{detail}\n\n{receipt}\n\nRECENT OBSERVATIONS\n{}\n\nResource measurements unavailable\n\nT - freeze / live   [ ] - step\nTab - filter   N - next task\nB - next worker   F - focus\nRight drag - orbit   R - reset\nWheel - zoom   WASD - pan",
        if activity.is_empty() {
            "No task observations at this sequence"
        } else {
            &activity
        }
    );
    if inspector.0 != text {
        inspector.0 = text;
    }
}

fn update_cancellation(
    keys: &ButtonInput<KeyCode>,
    selected: Option<NodeKey>,
    graph: &Graph,
    connection: &mut Connection,
) {
    if connection.timeline.is_frozen() || keys.just_pressed(KeyCode::KeyT) {
        if matches!(connection.cancellation, Cancellation::Confirm(_)) {
            connection.cancellation = Cancellation::Idle;
        }
        return;
    }
    let Some(control) = &connection.cancel else {
        return;
    };
    let task = match selected {
        Some(NodeKey::Task(id))
            if graph.tasks.get(&id).is_some_and(|task| {
                matches!(task.state, TaskState::Queued | TaskState::Running)
            }) =>
        {
            Some(id)
        }
        _ => None,
    };
    match connection.cancellation {
        Cancellation::Confirm(id) => {
            if task != Some(id) || keys.just_pressed(KeyCode::Escape) {
                connection.cancellation = Cancellation::Idle;
            } else if keys.just_pressed(KeyCode::KeyY) {
                connection.cancellation =
                    if connection.status.starts_with("Connected") && control.try_send(id).is_ok() {
                        Cancellation::Pending(id)
                    } else {
                        Cancellation::Uncertain
                    };
            }
        }
        Cancellation::Pending(_) => {}
        _ if keys.just_pressed(KeyCode::KeyC) && connection.status.starts_with("Connected") => {
            if let Some(id) = task {
                connection.cancellation = Cancellation::Confirm(id);
            }
        }
        _ => {}
    }
}

fn short_id(id: TaskId) -> String {
    id.to_string()[..8].to_owned()
}

fn change_label(change: ExternalTaskChange) -> &'static str {
    match change {
        ExternalTaskChange::Submitted => "Submitted",
        ExternalTaskChange::Claimed => "Claimed",
        ExternalTaskChange::LeaseExpired => "Lease expired",
        ExternalTaskChange::WorkerRevoked => "Worker revoked",
        ExternalTaskChange::Completed => "Completed",
        ExternalTaskChange::Cancelled => "Cancelled",
        ExternalTaskChange::DeadlineExceeded => "Deadline exceeded",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vespera_view::TaskNode;

    #[test]
    fn cancellation_requires_confirmation_for_the_same_observed_task() {
        let id = TaskId::new();
        let mut graph = Graph::default();
        graph.tasks.insert(
            id,
            TaskNode {
                slot: 0,
                state: TaskState::Queued,
                attempt: None,
                change: ExternalTaskChange::Submitted,
                observed_at_unix_ms: 1,
            },
        );
        let (_, updates) = mpsc::channel(1);
        let (cancel, mut requests) = mpsc::channel(1);
        let mut connection = Connection {
            updates,
            cancel: Some(cancel),
            status: "Connected",
            cancellation: Cancellation::Idle,
            timeline: Timeline::default(),
        };
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyC);
        update_cancellation(&keys, Some(NodeKey::Task(id)), &graph, &mut connection);
        assert!(matches!(connection.cancellation, Cancellation::Confirm(task) if task == id));
        assert!(requests.try_recv().is_err());
        keys.reset_all();
        keys.press(KeyCode::KeyY);
        update_cancellation(&keys, None, &graph, &mut connection);
        assert!(requests.try_recv().is_err());
        keys.reset_all();
        keys.press(KeyCode::KeyC);
        update_cancellation(&keys, Some(NodeKey::Task(id)), &graph, &mut connection);
        connection.timeline.step(true);
        keys.reset_all();
        keys.press(KeyCode::KeyY);
        update_cancellation(&keys, Some(NodeKey::Task(id)), &graph, &mut connection);
        assert!(requests.try_recv().is_err());
        assert!(matches!(connection.cancellation, Cancellation::Idle));
        connection.timeline.toggle();
        keys.reset_all();
        keys.press(KeyCode::KeyY);
        update_cancellation(&keys, Some(NodeKey::Task(id)), &graph, &mut connection);
        assert!(requests.try_recv().is_err());
        keys.reset_all();
        keys.press(KeyCode::KeyC);
        update_cancellation(&keys, Some(NodeKey::Task(id)), &graph, &mut connection);
        keys.reset_all();
        keys.press(KeyCode::KeyY);
        update_cancellation(&keys, Some(NodeKey::Task(id)), &graph, &mut connection);
        assert_eq!(requests.try_recv().unwrap(), id);
        assert!(matches!(connection.cancellation, Cancellation::Pending(task) if task == id));
    }
}
