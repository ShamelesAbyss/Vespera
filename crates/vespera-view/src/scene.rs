use std::collections::BTreeMap;

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use tokio::sync::mpsc;
use vespera_core::{AgentId, TaskId, TaskState};
use vespera_view::Graph;

use crate::Update as BrokerUpdate;

#[derive(Resource)]
struct Connection {
    updates: mpsc::Receiver<BrokerUpdate>,
    cancel: Option<mpsc::Sender<TaskId>>,
    status: &'static str,
}

#[derive(Resource, Default)]
struct Scene {
    graph: Graph,
    entities: BTreeMap<NodeKey, Entity>,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NodeKey {
    Task(TaskId),
    Worker(AgentId),
}

#[derive(Resource, Default)]
struct Selection(Option<NodeKey>);

#[derive(Resource)]
struct Palette {
    task: Handle<Mesh>,
    worker: Handle<Mesh>,
    materials: [Handle<StandardMaterial>; 7],
}

#[derive(Component)]
struct Inspector;

#[derive(Component)]
pub(super) struct Orbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            yaw: 0.45,
            pitch: 0.62,
            distance: 17.0,
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
}

#[derive(Resource)]
pub(super) struct CameraFocus {
    pub point: Option<Vec3>,
    pub home_distance: f32,
}

pub(super) fn desktop(title: &str, clear: Color) -> App {
    let mut app = App::new();
    app.insert_resource(ClearColor(clear)).add_plugins((
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
    app
}

pub fn run(updates: mpsc::Receiver<BrokerUpdate>, cancel: Option<mpsc::Sender<TaskId>>) {
    desktop("Vespera | Orchestration", Color::srgb(0.035, 0.055, 0.09))
        .insert_resource(Connection {
            updates,
            cancel,
            status: "Connecting to broker...",
        })
        .init_resource::<Scene>()
        .init_resource::<Selection>()
        .insert_resource(CameraFocus {
            point: None,
            home_distance: 17.0,
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                receive,
                sync_scene,
                focus_selection,
                camera,
                draw_links,
                inspect,
            )
                .chain(),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let colors = [
        Color::srgb(0.70, 0.76, 0.87),
        Color::srgb(0.10, 0.72, 1.0),
        Color::srgb(0.24, 0.84, 0.54),
        Color::srgb(0.98, 0.30, 0.34),
        Color::srgb(0.99, 0.63, 0.25),
        Color::srgb(0.64, 0.40, 1.0),
        Color::srgb(1.0, 0.93, 0.44),
    ];
    commands.insert_resource(Palette {
        task: meshes.add(Cuboid::new(1.1, 1.1, 1.1)),
        worker: meshes.add(Sphere::new(0.65)),
        materials: colors.map(|color| {
            materials.add(StandardMaterial {
                base_color: color,
                unlit: true,
                ..default()
            })
        }),
    });
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Orbit::default(),
        Transform::from_xyz(8.0, 12.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(38.0, 0.1, 48.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.07, 0.11, 0.18),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.2, 12.0),
        Pickable::IGNORE,
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(22),
            left: px(24),
            ..default()
        },
        Text::new("VESPERA  /  ORCHESTRATION"),
        TextFont::from_font_size(24.0),
        TextColor(Color::srgb(0.86, 0.92, 1.0)),
    ));
    commands.spawn((
        Inspector,
        Node {
            position_type: PositionType::Absolute,
            top: px(66),
            right: px(22),
            width: px(315),
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
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    mut connection: ResMut<Connection>,
    mut selection: ResMut<Selection>,
) {
    for _ in 0..64 {
        let Ok(update) = connection.updates.try_recv() else {
            break;
        };
        match update {
            BrokerUpdate::Status(status) => connection.status = status,
            BrokerUpdate::Observation(event) => scene.graph.observe(event),
            BrokerUpdate::Cancelled(task, state) => {
                if let Some(node) = scene.graph.tasks.get_mut(&task) {
                    node.state = state;
                }
                connection.status = "Connected - cancellation response received";
            }
            BrokerUpdate::Reset => {
                for entity in std::mem::take(&mut scene.entities).into_values() {
                    commands.entity(entity).despawn();
                }
                scene.graph = Graph::default();
                selection.0 = None;
            }
        }
    }
}

fn position(key: NodeKey, graph: &Graph) -> Option<Vec3> {
    let (slot, columns, height, depth) = match key {
        NodeKey::Task(id) => (graph.tasks.get(&id)?.slot, 16, 0.7, 0.0),
        NodeKey::Worker(id) => (*graph.workers.get(&id)?, 8, 2.7, -4.0),
    };
    let column = (slot % columns) as i32;
    let lateral = ((column + 1) / 2) * if column % 2 == 0 { -1 } else { 1 };
    Some(Vec3::new(
        lateral as f32 * 2.1,
        height,
        depth + (slot / columns) as f32 * 2.1,
    ))
}

fn sync_scene(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    selection: Res<Selection>,
    palette: Res<Palette>,
    mut existing: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    if !scene.is_changed() && !selection.is_changed() {
        return;
    }
    let keys: Vec<_> = scene
        .graph
        .tasks
        .keys()
        .copied()
        .map(NodeKey::Task)
        .chain(scene.graph.workers.keys().copied().map(NodeKey::Worker))
        .collect();
    for key in keys {
        let (mesh, index) = match key {
            NodeKey::Task(id) => (
                &palette.task,
                match scene.graph.tasks[&id].state {
                    TaskState::Queued => 0,
                    TaskState::Running => 1,
                    TaskState::Completed => 2,
                    TaskState::Failed => 3,
                    TaskState::Cancelled => 4,
                },
            ),
            NodeKey::Worker(_) => (&palette.worker, 5),
        };
        let material = &palette.materials[if selection.0 == Some(key) { 6 } else { index }];
        if let Some(entity) = scene.entities.get(&key) {
            if let Ok(mut current) = existing.get_mut(*entity)
                && current.0 != *material
            {
                current.0 = material.clone();
            }
        } else if let Some(location) = position(key, &scene.graph) {
            let entity = commands
                .spawn((
                    key,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(location),
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
}

fn draw_links(scene: Res<Scene>, mut gizmos: Gizmos) {
    let grid = Color::srgb(0.12, 0.18, 0.27);
    for index in -9..=9 {
        let x = index as f32 * 2.0;
        gizmos.line(Vec3::new(x, -0.14, -12.0), Vec3::new(x, -0.14, 36.0), grid);
    }
    for index in -6..=18 {
        let z = index as f32 * 2.0;
        gizmos.line(Vec3::new(-18.0, -0.14, z), Vec3::new(18.0, -0.14, z), grid);
    }
    for (&id, task) in &scene.graph.tasks {
        if let Some(attempt) = task.attempt
            && let Some(start) = position(NodeKey::Worker(attempt.worker_id), &scene.graph)
            && let Some(end) = position(NodeKey::Task(id), &scene.graph)
        {
            let color = if task.state == TaskState::Running {
                Color::srgb(0.23, 0.77, 1.0)
            } else {
                Color::srgb(0.26, 0.35, 0.50)
            };
            gizmos.line(start, end, color);
        }
    }
}

fn focus_selection(graph: Res<Scene>, selection: Res<Selection>, mut focus: ResMut<CameraFocus>) {
    focus.point = selection.0.and_then(|key| position(key, &graph.graph));
}

pub(super) fn camera(
    mut camera: Single<(&mut Orbit, &mut Transform)>,
    (keys, buttons): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    focus: Res<CameraFocus>,
) {
    let (orbit, transform) = &mut *camera;
    if keys.just_pressed(KeyCode::KeyR) {
        **orbit = Orbit::with_distance(focus.home_distance);
    }
    if keys.just_pressed(KeyCode::KeyF)
        && let Some(focus) = focus.point
    {
        orbit.focus = focus;
        orbit.distance = 9.0;
    }
    if buttons.pressed(MouseButton::Right) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch = (orbit.pitch + motion.delta.y * 0.006).clamp(0.16, 1.46);
    }
    orbit.distance = (orbit.distance * (-scroll.delta.y * 0.08).exp()).clamp(3.0, 85.0);
    let lateral = f32::from(u8::from(keys.pressed(KeyCode::KeyD)))
        - f32::from(u8::from(keys.pressed(KeyCode::KeyA)));
    let forward = f32::from(u8::from(keys.pressed(KeyCode::KeyS)))
        - f32::from(u8::from(keys.pressed(KeyCode::KeyW)));
    let rotation = Quat::from_rotation_y(orbit.yaw);
    orbit.focus += rotation * Vec3::new(lateral, 0.0, forward) * time.delta_secs() * 8.0;
    let offset = rotation * Vec3::new(0.0, orbit.pitch.sin(), orbit.pitch.cos()) * orbit.distance;
    **transform =
        Transform::from_translation(orbit.focus + offset).looking_at(orbit.focus, Vec3::Y);
}

fn inspect(
    scene: Res<Scene>,
    selection: Res<Selection>,
    keys: Res<ButtonInput<KeyCode>>,
    mut connection: ResMut<Connection>,
    mut inspector: Single<&mut Text, With<Inspector>>,
) {
    let detail = match selection.0 {
        Some(NodeKey::Task(id)) => {
            let task = &scene.graph.tasks[&id];
            if keys.just_pressed(KeyCode::KeyC)
                && connection.status.starts_with("Connected")
                && let Some(control) = &connection.cancel
                && control.try_send(id).is_ok()
            {
                connection.status = "Cancellation requested...";
            }
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
                "TASK  {id}\n{} / {:?}\n{attempt}\nObserved Unix ms {}",
                task.state, task.change, task.observed_at_unix_ms
            )
        }
        Some(NodeKey::Worker(id)) => format!("WORKER  {id}\nObserved in a broker lease event"),
        None => "Select a task cube or worker sphere.".into(),
    };
    let control = if connection.cancel.is_some() {
        "\nC - cancel selected task"
    } else {
        ""
    };
    let text = format!(
        "{}\n\n{} tasks   {} workers observed\n\n{detail}\n\nResource measurements unavailable\n\nRight drag - orbit\nWheel - zoom   WASD - pan\nF - focus   R - reset{control}",
        connection.status,
        scene.graph.tasks.len(),
        scene.graph.workers.len()
    );
    if inspector.0 != text {
        inspector.0 = text;
    }
}
