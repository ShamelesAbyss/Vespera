use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, SystemTime};

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use tokio::sync::watch;
use vespera_core::AgentId;
use vespera_world::{
    Position, ReasoningPhase, ReasoningStatus, Replay, ReplayDirection, Role, Snapshot, World,
    WorldChange, WorldError, WorldFence, WorldStore,
};

use crate::atlas::{Layer, Layers, register};
use crate::scene::{CameraFocus, Orbit, camera, desktop};

type Focus = CameraFocus<{ Layer::World as usize }>;

const POLL_INTERVAL: Duration = Duration::from_millis(200);

struct Observation {
    world: World,
    status: &'static str,
    reasoning: Option<ReasoningStatus>,
    history: Option<HistoryPosition>,
    completed_requests: u64,
}

#[derive(Clone, Copy)]
struct HistoryPosition {
    earliest: u64,
    latest: u64,
    basis: u64,
}

impl From<&Replay> for HistoryPosition {
    fn from(replay: &Replay) -> Self {
        Self {
            earliest: replay.earliest_revision,
            latest: replay.latest_revision,
            basis: replay.basis_revision,
        }
    }
}

struct Command {
    revision: u64,
    change: WorldChange,
}

enum Request {
    Control(Command),
    Timeline(TimelineRequest),
}

enum TimelineRequest {
    Live,
    Freeze(u64),
    Neighbor(u64, ReplayDirection),
}

#[derive(Resource)]
pub(super) struct View {
    updates: watch::Receiver<Observation>,
    commands: SyncSender<Request>,
    allow_control: bool,
    history: Option<HistoryPosition>,
    world: World,
    status: &'static str,
    reasoning: Option<ReasoningStatus>,
    completed_requests: u64,
    pending: bool,
    scene_dirty: bool,
    playing: bool,
    clock: Timer,
    entities: BTreeMap<Object, Entity>,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Object {
    Depot,
    Site(Position),
    Resident(AgentId),
}

#[derive(Resource, Default)]
struct Selected(Option<Object>);

pub(super) fn selected_agent(world: &bevy::prelude::World) -> Option<AgentId> {
    match world.resource::<Selected>().0 {
        Some(Object::Resident(id)) => Some(id),
        _ => None,
    }
}

#[derive(Resource)]
struct Palette {
    depot: Handle<Mesh>,
    site: Handle<Mesh>,
    resident: Handle<Mesh>,
    material: [Handle<StandardMaterial>; 6],
}

#[derive(Component)]
struct Details;

pub(super) fn run(
    path: PathBuf,
    revision: Option<u64>,
    allow_control: bool,
    reduced_motion: bool,
) -> Result<(), WorldError> {
    let mut app = desktop(
        "Vespera | World Simulation",
        Color::srgb(0.04, 0.065, 0.06),
        Layer::World,
    );
    install(&mut app, path, revision, allow_control, reduced_motion)?;
    app.run();
    Ok(())
}

pub(super) fn install(
    app: &mut App,
    path: PathBuf,
    revision: Option<u64>,
    allow_control: bool,
    reduced_motion: bool,
) -> Result<(), WorldError> {
    let view = connect(path, revision, allow_control)?;
    install_scene(app, reduced_motion);
    app.insert_resource(view);
    Ok(())
}

pub(super) fn connect(
    path: PathBuf,
    revision: Option<u64>,
    allow_control: bool,
) -> Result<View, WorldError> {
    let (world, reasoning, status, history) = if let Some(revision) = revision {
        let replay = WorldStore::replay(&path, Some(revision))?;
        let history = HistoryPosition::from(&replay);
        (replay.world, None, "Historical revision", Some(history))
    } else {
        let (world, reasoning) = read_observed(&path)?;
        (world, reasoning, "Database connected", None)
    };
    let (observations, updates) = watch::channel(Observation {
        world: world.clone(),
        status,
        reasoning,
        history,
        completed_requests: 0,
    });
    let (commands, actions) = mpsc::sync_channel(1);
    std::thread::spawn(move || observe(path, observations, actions));
    Ok(View {
        updates,
        commands,
        allow_control,
        history,
        world,
        status,
        reasoning,
        completed_requests: 0,
        pending: false,
        scene_dirty: true,
        playing: false,
        clock: Timer::from_seconds(0.5, TimerMode::Repeating),
        entities: BTreeMap::new(),
    })
}

pub(super) fn install_scene(app: &mut App, reduced_motion: bool) {
    register(app, Layer::World);
    app.insert_resource(Focus {
        point: None,
        home_point: Vec3::ZERO,
        home_distance: 16.0,
        reduced_motion,
    })
    .init_resource::<Selected>()
    .add_systems(Update, setup.run_if(resource_added::<View>).before(receive))
    .add_systems(
        Update,
        (
            controls,
            sync_scene,
            camera::<{ Layer::World as usize }>,
            links,
            inspect,
        )
            .chain()
            .run_if(|layers: Res<Layers>| layers.active == Layer::World)
            .run_if(resource_exists::<View>)
            .after(receive),
    )
    .add_systems(Update, receive.run_if(resource_exists::<View>));
}

fn read_observed(path: &Path) -> Result<(World, Option<ReasoningStatus>), WorldError> {
    let now = SystemTime::UNIX_EPOCH
        .elapsed()
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0);
    WorldStore::read_observed(path, now)
}

fn observe(path: PathBuf, updates: watch::Sender<Observation>, actions: mpsc::Receiver<Request>) {
    while !updates.is_closed() {
        let (result, is_control) = match actions.recv_timeout(POLL_INTERVAL) {
            Ok(Request::Timeline(request)) => {
                apply_timeline(&path, &updates, request);
                continue;
            }
            Ok(Request::Control(command)) if updates.borrow().history.is_none() => {
                (commit(&path, command).map(|world| (world, None)), true)
            }
            Ok(Request::Control(_)) => (Err(WorldError::Invalid("historical control")), true),
            Err(RecvTimeoutError::Timeout) if updates.borrow().history.is_some() => continue,
            Err(RecvTimeoutError::Timeout) => (read_observed(&path), false),
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match result {
            Ok((world, reasoning)) => {
                updates.send_if_modified(|last| {
                    let status = if is_control {
                        "World change committed"
                    } else {
                        "Database connected"
                    };
                    if is_control
                        || last.world != world
                        || last.reasoning != reasoning
                        || last.status == "Database unavailable"
                    {
                        last.world = world;
                        last.status = status;
                        last.reasoning = reasoning;
                        if is_control {
                            last.completed_requests = last.completed_requests.wrapping_add(1);
                        }
                        true
                    } else {
                        false
                    }
                });
            }
            Err(error) => {
                let status = match error {
                    WorldError::StaleRevision => "World changed before the control was applied",
                    WorldError::Invalid(_) | WorldError::UnknownResident => {
                        "Resident control rejected"
                    }
                    _ => "Database unavailable",
                };
                updates.send_if_modified(|last| {
                    let changed = last.status != status || last.reasoning.is_some();
                    last.status = status;
                    last.reasoning = None;
                    if is_control {
                        last.completed_requests = last.completed_requests.wrapping_add(1);
                    }
                    is_control || changed
                });
            }
        }
    }
}

fn apply_timeline(path: &Path, updates: &watch::Sender<Observation>, request: TimelineRequest) {
    let result = match request {
        TimelineRequest::Live => read_observed(path)
            .map(|(world, reasoning)| (Some((world, reasoning, None)), "Following live world")),
        TimelineRequest::Freeze(revision) => {
            WorldStore::replay(path, Some(revision)).map(|replay| {
                let history = HistoryPosition::from(&replay);
                (
                    Some((replay.world, None, Some(history))),
                    "Historical revision",
                )
            })
        }
        TimelineRequest::Neighbor(from, direction) => {
            WorldStore::replay_neighbor(path, from, direction).map(|replay| match replay {
                Some(replay) => {
                    let history = HistoryPosition::from(&replay);
                    let status = if from.abs_diff(replay.world.revision()) > 1 {
                        "Skipped revisions outside retained commit boundaries"
                    } else {
                        "Historical revision"
                    };
                    (Some((replay.world, None, Some(history))), status)
                }
                None => (
                    None,
                    match direction {
                        ReplayDirection::Previous => "No earlier retained commit",
                        ReplayDirection::Next => "No later retained commit",
                    },
                ),
            })
        }
    };
    updates.send_modify(|last| {
        last.completed_requests = last.completed_requests.wrapping_add(1);
        match result {
            Ok((frame, status)) => {
                last.status = status;
                if let Some((world, reasoning, history)) = frame {
                    last.world = world;
                    last.reasoning = reasoning;
                    last.history = history;
                }
            }
            Err(WorldError::ReplayUnavailable { .. }) => {
                last.status = "Requested revision is unavailable"
            }
            Err(_) => last.status = "Could not read world history",
        }
    });
}

fn commit(path: &Path, command: Command) -> Result<World, WorldError> {
    WorldStore::commit(path, WorldFence::Revision(command.revision), command.change)
}

fn setup(
    mut commands: Commands,
    view: Res<View>,
    mut focus: ResMut<Focus>,
    layers: Res<Layers>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let colors = [
        Color::srgb(0.39, 0.72, 0.82),
        Color::srgb(0.31, 0.70, 0.39),
        Color::srgb(0.75, 0.56, 0.28),
        Color::srgb(0.60, 0.41, 0.84),
        Color::srgb(0.40, 0.45, 0.45),
        Color::srgb(1.0, 0.91, 0.42),
    ];
    let unlit = |color| StandardMaterial {
        base_color: color,
        unlit: true,
        ..default()
    };
    commands.insert_resource(Palette {
        depot: meshes.add(Cuboid::new(0.84, 0.75, 0.84)),
        site: meshes.add(Cylinder::new(0.32, 0.55)),
        resident: meshes.add(Capsule3d::new(0.24, 0.47)),
        material: colors.map(|color| materials.add(unlit(color))),
    });
    let state = view.world.snapshot();
    focus.home_distance = f32::from(state.width.max(state.depth))
        .mul_add(1.35, 4.0)
        .clamp(9.0, 85.0);
    let camera = commands
        .spawn((
            Layer::World,
            Camera {
                is_active: layers.active == Layer::World,
                ..default()
            },
            Camera3d::default(),
            Tonemapping::None,
            Orbit::with_distance(focus.home_distance),
            Transform::from_xyz(8.0, 10.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    commands.spawn((
        layers.parent(Layer::World),
        Mesh3d(meshes.add(Cuboid::new(
            f32::from(state.width),
            0.1,
            f32::from(state.depth),
        ))),
        MeshMaterial3d(materials.add(unlit(Color::srgb(0.11, 0.19, 0.16)))),
        Transform::from_xyz(0.0, -0.07, 0.0),
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
        Text::new("VESPERA  /  WORLD SIMULATION"),
        TextFont::from_font_size(24.0),
        TextColor(Color::srgb(0.86, 0.98, 0.90)),
    ));
    commands.spawn((
        UiTargetCamera(camera),
        Details,
        Node {
            position_type: PositionType::Absolute,
            top: px(66),
            right: px(22),
            width: px(315),
            padding: UiRect::all(px(15)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.04, 0.09, 0.08, 0.94)),
        Text::new("Loading committed simulation state..."),
        TextFont::from_font_size(15.0),
        TextColor(Color::srgb(0.87, 0.97, 0.90)),
    ));
}

fn receive(mut view: ResMut<View>, layers: Res<Layers>) {
    if layers.active != Layer::World && view.playing {
        view.playing = false;
    }
    if !view.updates.has_changed().unwrap_or(false) {
        return;
    }
    let (world, status, reasoning, history, completed_requests) = {
        let update = view.updates.borrow_and_update();
        (
            update.world.clone(),
            update.status,
            update.reasoning,
            update.history,
            update.completed_requests,
        )
    };
    if view.world != world {
        view.world = world;
        view.scene_dirty = true;
    }
    view.status = status;
    view.reasoning = reasoning;
    view.history = history;
    if view.completed_requests != completed_requests {
        view.completed_requests = completed_requests;
        view.pending = false;
    }
    if status == "Database unavailable" {
        view.playing = false;
    }
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    selected: Res<Selected>,
    mut view: ResMut<View>,
    layers: Res<Layers>,
) {
    if view.pending || layers.switched {
        return;
    }
    let revision = view.world.revision();
    let timeline = if keys.just_pressed(KeyCode::KeyT) {
        Some(if view.history.is_some() {
            TimelineRequest::Live
        } else {
            TimelineRequest::Freeze(revision)
        })
    } else if keys.just_pressed(KeyCode::BracketLeft) {
        Some(TimelineRequest::Neighbor(
            revision,
            ReplayDirection::Previous,
        ))
    } else if keys.just_pressed(KeyCode::BracketRight) && view.history.is_some() {
        Some(TimelineRequest::Neighbor(revision, ReplayDirection::Next))
    } else {
        None
    };
    if let Some(timeline) = timeline {
        if view.commands.try_send(Request::Timeline(timeline)).is_ok() {
            view.playing = false;
            view.pending = true;
        }
        return;
    }
    if !view.allow_control || view.history.is_some() {
        return;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        view.playing = !view.playing;
        view.clock.reset();
    }
    let step = view.playing && view.clock.tick(time.delta()).just_finished();
    let shift = movement(&keys);
    let step_key = keys.just_pressed(KeyCode::Space);
    let toggle_key = keys.just_pressed(KeyCode::Enter);
    if !(step || step_key || toggle_key || shift.is_some()) {
        return;
    }
    let snapshot = view.world.snapshot();
    let action = if step_key {
        Some(WorldChange::Advance { ticks: 1 })
    } else if let Some(Object::Resident(id)) = selected.0 {
        snapshot
            .residents
            .iter()
            .find(|resident| resident.id == id)
            .and_then(|resident| {
                if toggle_key {
                    return Some(WorldChange::SetActive {
                        resident: id,
                        active: !resident.active,
                    });
                }
                let (x, z) = shift?;
                Some(WorldChange::Move {
                    resident: id,
                    to: Position {
                        x: resident.position.x.checked_add_signed(x)?,
                        z: resident.position.z.checked_add_signed(z)?,
                    },
                })
            })
    } else {
        None
    };
    if let Some(change) = action.or_else(|| step.then_some(WorldChange::Advance { ticks: 1 }))
        && view
            .commands
            .try_send(Request::Control(Command {
                revision: snapshot.revision,
                change,
            }))
            .is_ok()
    {
        view.pending = true;
    }
}

fn movement(keys: &ButtonInput<KeyCode>) -> Option<(i16, i16)> {
    [
        ([KeyCode::ArrowLeft, KeyCode::KeyJ], (-1, 0)),
        ([KeyCode::ArrowRight, KeyCode::KeyL], (1, 0)),
        ([KeyCode::ArrowUp, KeyCode::KeyI], (0, -1)),
        ([KeyCode::ArrowDown, KeyCode::KeyK], (0, 1)),
    ]
    .into_iter()
    .find_map(|(pair, shift)| {
        pair.into_iter()
            .any(|key| keys.just_pressed(key))
            .then_some(shift)
    })
}

fn place(position: Position, state: &Snapshot<'_>, height: f32) -> Vec3 {
    Vec3::new(
        f32::from(position.x) - f32::from(state.width - 1) * 0.5,
        height,
        f32::from(position.z) - f32::from(state.depth - 1) * 0.5,
    )
}

fn sync_scene(
    mut commands: Commands,
    mut view: ResMut<View>,
    mut selected: ResMut<Selected>,
    palette: Res<Palette>,
    mut focus: ResMut<Focus>,
    layers: Res<Layers>,
    mut objects: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    if let Some(id) = layers.incoming
        && view
            .world
            .snapshot()
            .residents
            .iter()
            .any(|resident| resident.id == id)
    {
        selected.0 = Some(Object::Resident(id));
    }
    if !view.scene_dirty && !selected.is_changed() {
        return;
    }
    view.scene_dirty = false;
    let state = view.world.snapshot();
    let height = 1.0 + f32::from(state.shelters.min(20)) * 0.13;
    let mut render = vec![(
        Object::Depot,
        place(state.depot, &state, height * 0.37),
        height,
        &palette.depot,
        0,
    )];
    render.extend(state.sites.iter().map(|site| {
        let scale = 0.25 + f32::from(site.stock) / f32::from(site.capacity);
        (
            Object::Site(site.position),
            place(site.position, &state, 0.22 * scale),
            scale,
            &palette.site,
            1,
        )
    }));
    // Coincident residents use a small fixed display offset. Grid coordinates stay authoritative.
    render.extend(state.residents.iter().enumerate().map(|(index, resident)| {
        let elevation = if resident.position == state.depot {
            height * 0.75
        } else {
            state
                .sites
                .iter()
                .find(|site| site.position == resident.position)
                .map_or(0.0, |site| {
                    0.50 * (0.25 + f32::from(site.stock) / f32::from(site.capacity))
                })
        };
        let mut position = place(resident.position, &state, 0.50 + elevation);
        let angle = index as f32 * 2.399_963;
        position += Vec3::new(angle.cos() * 0.24, 0.0, angle.sin() * 0.24);
        let color = match (resident.active, resident.role) {
            (false, _) => 4,
            (true, Role::Forager) => 2,
            (true, Role::Builder) => 3,
        };
        (
            Object::Resident(resident.id),
            position,
            1.0,
            &palette.resident,
            color,
        )
    }));
    let mut remaining = std::mem::take(&mut view.entities);
    focus.point = None;
    for (key, position, scale, mesh, color) in render {
        let is_selected = selected.0 == Some(key);
        if is_selected {
            focus.point = Some(position);
        }
        let material = &palette.material[if is_selected { 5 } else { color }];
        let transform =
            Transform::from_translation(position).with_scale(Vec3::new(1.0, scale, 1.0));
        let entity = if let Some(entity) = remaining.remove(&key) {
            if let Ok((mut pose, mut surface)) = objects.get_mut(entity) {
                *pose = transform;
                surface.0 = material.clone();
            }
            entity
        } else {
            commands
                .spawn((
                    layers.parent(Layer::World),
                    key,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    transform,
                ))
                .observe(
                    move |event: On<PointerClick>, mut selected: ResMut<Selected>| {
                        if event.button == PointerButton::Primary {
                            selected.0 = Some(key);
                        }
                    },
                )
                .id()
        };
        view.entities.insert(key, entity);
    }
    for entity in remaining.into_values() {
        commands.entity(entity).despawn();
    }
}

fn links(view: Res<View>, selected: Res<Selected>, mut gizmos: Gizmos) {
    let state = view.world.snapshot();
    let half_width = f32::from(state.width) * 0.5;
    let half_depth = f32::from(state.depth) * 0.5;
    let grid = Color::srgb(0.19, 0.29, 0.23);
    for column in 0..=state.width {
        let x = f32::from(column) - half_width;
        gizmos.line(
            Vec3::new(x, -0.01, -half_depth),
            Vec3::new(x, -0.01, half_depth),
            grid,
        );
    }
    for row in 0..=state.depth {
        let z = f32::from(row) - half_depth;
        gizmos.line(
            Vec3::new(-half_width, -0.01, z),
            Vec3::new(half_width, -0.01, z),
            grid,
        );
    }
    if let Some(reasoning) = view.reasoning
        && let Some(resident) = state
            .residents
            .iter()
            .find(|resident| resident.id == reasoning.resident)
    {
        let color = match reasoning.phase {
            ReasoningPhase::Preparing {} => Color::srgb(1.0, 0.66, 0.20),
            ReasoningPhase::InferenceActive { .. } => Color::srgb(0.28, 0.90, 1.0),
        };
        let center = place(resident.position, &state, 0.08);
        gizmos.cube(
            Transform::from_translation(center).with_scale(Vec3::new(0.9, 0.15, 0.9)),
            color,
        );
    }
    if let Some(Object::Resident(id)) = selected.0
        && let Some(resident) = state.residents.iter().find(|resident| resident.id == id)
    {
        for other in &state.residents {
            if resident.relationships.contains_key(&other.id) {
                gizmos.line(
                    place(resident.position, &state, 0.9),
                    place(other.position, &state, 0.9),
                    Color::srgb(0.83, 0.68, 0.40),
                );
            }
        }
    }
}

fn inspect(view: Res<View>, selected: Res<Selected>, mut panel: Single<&mut Text, With<Details>>) {
    let state = view.world.snapshot();
    let detail = match selected.0 {
        Some(Object::Resident(id)) => state.residents.iter().find(|resident| resident.id == id).map_or_else(
            || "Resident is absent from the current world".into(),
            |resident| {
                let mut detail = format!("{} / {:?}\n{} | Goal: {:?}\nCell {}, {} | Carrying {}\n{} relationships\n{}",
                    resident.name, resident.role, if resident.active { "Active" } else { "Suspended" }, resident.goal,
                    resident.position.x, resident.position.z, resident.carried, resident.relationships.len(),
                    state.residents.iter().filter_map(|other| resident.relationships.get(&other.id).map(|count| format!("{}: {count} encounters", other.name))).take(4).collect::<Vec<_>>().join("\n")
                );
                if let Some(reasoning) = view.reasoning.filter(|status| status.resident == id) {
                    let phase = match reasoning.phase {
                        ReasoningPhase::Preparing {} => "Preparing model request".into(),
                        ReasoningPhase::InferenceActive { model_id, .. } => {
                            let model = model_id.to_string();
                            format!("Local inference lease active\nModel {}", &model[..8])
                        }
                    };
                    let task = reasoning.task_id.to_string();
                    detail.push_str(&format!("\n\nRUNTIME REASONING\n{phase}\nTask {}", &task[..8]));
                }
                if let Some(committed) = resident.last_model_decision {
                    use vespera_world::Decision;
                    let action = match committed.decision {
                        Decision::Move { to } => format!("Move to {}, {}", to.x, to.z),
                        Decision::Wait {} => "Wait".into(),
                        Decision::Gather {} => "Gather".into(),
                        Decision::Deliver {} => "Deliver".into(),
                        Decision::Build {} => "Build".into(),
                    };
                    let model = committed.source.model_id.to_string();
                    detail.push_str(&format!("\n\nLAST MODEL CHOICE\n{action} | Revision {}\nModel {}", committed.revision, &model[..8]));
                }
                detail
            }
        ),
        Some(Object::Site(position)) => state.sites.iter().find(|site| site.position == position).map_or_else(
            || "Site is absent from the current world".into(),
            |site| format!("RESOURCE SITE\nCell {}, {}\nStock {} / {}\nRegeneration {} per tick", position.x, position.z, site.stock, site.capacity, site.regeneration)
        ),
        Some(Object::Depot) => format!("DEPOT\nCell {}, {}\n{} stock | {} shelters", state.depot.x, state.depot.z, state.shared_stock, state.shelters),
        None => "Select a resident, resource site, or depot.".into(),
    };
    let controls = if view.history.is_some() {
        "[ / ] - earlier / later\nT - return to live"
    } else if view.allow_control {
        "Space - step   P - play/pause\nEnter - activate/suspend\nArrows or IJKL - move\nT - freeze   [ - earlier commit"
    } else {
        "T - freeze   [ - earlier commit"
    };
    let history = view.history.map_or_else(String::new, |position| {
        format!(
            "\nRetained {}-{} | Basis {}",
            position.earliest, position.latest, position.basis
        )
    });
    let text = format!(
        "SIMULATED STATE\n{}\nTick {} | Revision {}\n{} | Stock {} | Shelters {}{history}\n\n{detail}\n\n{controls}\n\nRight drag - orbit\nWheel - zoom   WASD - pan\nF - focus   R - reset",
        view.status,
        state.tick,
        state.revision,
        if view.history.is_some() {
            "Replay (read-only)"
        } else if !view.allow_control {
            "Live (read-only)"
        } else if view.playing {
            "Playing"
        } else {
            "Paused"
        },
        state.shared_stock,
        state.shelters
    );
    if panel.0 != text {
        panel.0 = text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeline_skips_batch_gaps_and_fences_controls_until_live() {
        let resident = AgentId::new();
        let world = World::new(vespera_world::WorldSpec {
            seed: 1,
            width: 3,
            depth: 3,
            depot: Position { x: 0, z: 0 },
            residents: vec![vespera_world::ResidentSpec {
                id: resident,
                name: "Fixture".into(),
                role: Role::Builder,
                position: Position { x: 1, z: 1 },
            }],
            sites: vec![],
        })
        .unwrap();
        let database = std::env::temp_dir().join(format!(
            "vespera-timeline-{}.sqlite3",
            vespera_core::TaskId::new()
        ));
        WorldStore::initialize(&database, &world).unwrap();
        let latest = WorldStore::commit(
            &database,
            WorldFence::Current,
            WorldChange::Advance { ticks: 3 },
        )
        .unwrap();
        let (updates, mut observed) = watch::channel(Observation {
            world: latest,
            status: "Database connected",
            reasoning: None,
            history: None,
            completed_requests: 0,
        });
        let (send, requests) = mpsc::sync_channel(1);
        let path = database.clone();
        let worker = std::thread::spawn(move || observe(path, updates, requests));

        send.send(Request::Timeline(TimelineRequest::Neighbor(
            3,
            ReplayDirection::Previous,
        )))
        .unwrap();
        {
            let frame = observed
                .wait_for(|frame| frame.completed_requests == 1)
                .await
                .unwrap();
            assert_eq!(frame.world.revision(), 0);
            assert!(frame.history.is_some());
            assert!(frame.status.starts_with("Skipped"));
        }
        send.send(Request::Control(Command {
            revision: 3,
            change: WorldChange::SetActive {
                resident,
                active: false,
            },
        }))
        .unwrap();
        {
            let frame = observed
                .wait_for(|frame| frame.completed_requests == 2)
                .await
                .unwrap();
            assert_eq!(frame.world.revision(), 0);
        }
        assert_eq!(WorldStore::read(&database).unwrap().revision(), 3);
        WorldStore::commit(
            &database,
            WorldFence::Revision(3),
            WorldChange::Advance { ticks: 1 },
        )
        .unwrap();
        send.send(Request::Timeline(TimelineRequest::Neighbor(
            0,
            ReplayDirection::Next,
        )))
        .unwrap();
        {
            let frame = observed
                .wait_for(|frame| frame.completed_requests == 3)
                .await
                .unwrap();
            assert_eq!(frame.world.revision(), 3);
            assert!(frame.history.is_some());
        }
        send.send(Request::Timeline(TimelineRequest::Live)).unwrap();
        {
            let frame = observed
                .wait_for(|frame| frame.completed_requests == 4)
                .await
                .unwrap();
            assert_eq!(frame.world.revision(), 4);
            assert!(frame.history.is_none());
        }
        drop(send);
        worker.join().unwrap();
        std::fs::remove_file(database).unwrap();
    }

    #[test]
    fn movement_keys_commit_only_against_the_displayed_revision() {
        let spec = vespera_world::WorldSpec {
            seed: 0,
            width: 3,
            depth: 3,
            depot: Position { x: 0, z: 0 },
            residents: vec![vespera_world::ResidentSpec {
                id: AgentId::new(),
                name: "Fixture".into(),
                role: Role::Builder,
                position: Position { x: 1, z: 1 },
            }],
            sites: vec![],
        };
        let id = spec.residents[0].id;
        let world = World::new(spec).unwrap();
        let database = std::env::temp_dir().join(format!(
            "vespera-view-{}.sqlite3",
            vespera_core::TaskId::new()
        ));
        WorldStore::initialize(&database, &world).unwrap();
        for key in [KeyCode::KeyL, KeyCode::ArrowRight] {
            let mut keys = ButtonInput::default();
            keys.press(key);
            assert_eq!(movement(&keys), Some((1, 0)));
        }
        let to = Position { x: 2, z: 1 };
        let moved = commit(
            &database,
            Command {
                revision: 0,
                change: WorldChange::Move { resident: id, to },
            },
        )
        .unwrap();
        assert_eq!(moved.snapshot().residents[0].position, to);
        assert!(matches!(
            commit(
                &database,
                Command {
                    revision: 0,
                    change: WorldChange::Advance { ticks: 1 }
                }
            ),
            Err(WorldError::StaleRevision)
        ));
        assert_eq!(WorldStore::replay(&database, Some(0)).unwrap().world, world);
        assert_eq!(WorldStore::replay(&database, Some(1)).unwrap().world, moved);
        assert_eq!(moved.snapshot().tick, 0);
        std::fs::remove_file(database).unwrap();
    }
}
