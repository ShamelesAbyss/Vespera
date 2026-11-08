use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, SystemTime};

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;
use tokio::sync::watch;
use vespera_core::AgentId;
use vespera_world::{
    Action, Position, ReasoningPhase, ReasoningStatus, Replay, ReplayDirection, Role, World,
    WorldChange, WorldError, WorldFence, WorldStore,
};

use crate::atlas::{Layer, Layers, register};
use crate::scene::{CameraFocus, Orbit, camera, desktop};
use crate::world_art::{self, Landscape, Models, grid_point};

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
    shelters: Vec<Entity>,
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

#[derive(Component)]
struct Details;

pub(super) fn run(
    path: PathBuf,
    revision: Option<u64>,
    allow_control: bool,
    reduced_motion: bool,
) -> Result<(), WorldError> {
    let mut app = desktop("Vespera | World Simulation", world_art::SKY, Layer::World);
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
        shelters: Vec::new(),
    })
}

pub(super) fn install_scene(app: &mut App, reduced_motion: bool) {
    register(app, Layer::World);
    world_art::embed_assets(app);
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
            animate_residents,
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
    assets: (Res<AssetServer>, ResMut<Assets<Image>>),
) {
    let (assets, mut images) = assets;
    let state = view.world.snapshot();
    let landscape = Landscape::new(&state);
    let models = Models::load(&assets);
    let parent = layers.parent(Layer::World).0;
    world_art::build(
        &mut commands,
        parent,
        &landscape,
        &models,
        &mut meshes,
        &mut materials,
        &mut images,
    );
    commands.insert_resource(landscape);
    commands.insert_resource(models);
    let span = f32::from(state.width.max(state.depth));
    focus.home_distance = span.mul_add(1.39, 8.4) + (span - 20.0).max(0.0) * 0.44;
    focus.home_point = Vec3::new(2.15, -0.28, -0.45);
    let fog_end = (focus.home_distance * 3.3).max(65.0);
    let camera = commands
        .spawn((
            Layer::World,
            Camera {
                is_active: layers.active == Layer::World,
                clear_color: ClearColorConfig::Custom(world_art::SKY),
                ..default()
            },
            Camera3d::default(),
            Tonemapping::AcesFitted,
            Orbit::at_home(&focus),
            AmbientLight {
                color: Color::srgb(0.64, 0.77, 1.0),
                brightness: 1050.0,
                ..default()
            },
            DistanceFog {
                color: world_art::SKY,
                falloff: FogFalloff::Linear {
                    start: focus.home_distance * 1.02,
                    end: fog_end,
                },
                ..default()
            },
        ))
        .id();
    commands.spawn((
        ChildOf(parent),
        DirectionalLight {
            color: Color::srgb(1.0, 0.87, 0.73),
            illuminance: 6_500.0,
            shadow_maps_enabled: true,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: 2,
            first_cascade_far_bound: 14.0,
            maximum_distance: focus.home_distance + 24.0,
            ..default()
        }
        .build(),
        Transform::from_xyz(-5.0, 10.0, 7.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        UiTargetCamera(camera),
        Node {
            position_type: PositionType::Absolute,
            top: px(22),
            left: px(24),
            ..default()
        },
        Text::new("LIVING WORLD"),
        TextFont::from_font_size(24.0),
        TextColor(Color::srgb(0.89, 0.96, 1.0)),
        Pickable::IGNORE,
    ));
    commands.spawn((
        UiTargetCamera(camera),
        Details,
        Node {
            position_type: PositionType::Absolute,
            top: px(66),
            right: px(22),
            width: px(288),
            padding: UiRect::all(px(15)),
            border_radius: BorderRadius::all(px(12)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.038, 0.063, 0.102, 0.90)),
        Text::new("Loading committed simulation state..."),
        TextFont::from_font_size(15.0),
        TextColor(Color::srgb(0.89, 0.96, 1.0)),
    ));
    commands.spawn((
        UiTargetCamera(camera),
        Node {
            position_type: PositionType::Absolute,
            left: px(24),
            top: px(65),
            max_width: percent(45),
            padding: UiRect::axes(px(11), px(8)),
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.038, 0.063, 0.102, 0.85)),
        Text::new("Right drag  Orbit\nWheel  Zoom  |  WASD  Pan\nF  Focus  |  R  Overview"),
        TextFont::from_font_size(13.0),
        TextColor(Color::srgb(0.83, 0.91, 0.98)),
        Pickable::IGNORE,
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
    if view.world != world || view.history.is_some() != history.is_some() {
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

#[derive(Component, Default)]
struct SiteDisplay {
    cargo: Vec<Entity>,
}

#[derive(Component)]
struct ResidentMotion {
    goal: Vec2,
    cell: Position,
    heading: Quat,
    last_work: Option<(u64, Action)>,
    work_age: f32,
    active: bool,
}

#[derive(Component)]
struct SuitJoint {
    resident: Entity,
    kind: Joint,
    rest: Quat,
}

enum Joint {
    Arm(f32),
    Leg(f32),
    Head,
}

fn rig_suit(
    event: On<WorldInstanceReady>,
    children: Query<&Children>,
    nodes: Query<(&Name, &Transform)>,
    mut commands: Commands,
) {
    for entity in children.iter_descendants(event.entity) {
        let Ok((name, transform)) = nodes.get(entity) else {
            continue;
        };
        let kind = match name.as_str() {
            "armLeft" => Joint::Arm(1.0),
            "armRight" => Joint::Arm(-1.0),
            "legLeft" => Joint::Leg(1.0),
            "legRight" => Joint::Leg(-1.0),
            "head" => Joint::Head,
            _ => continue,
        };
        commands.entity(entity).insert(SuitJoint {
            resident: event.entity,
            kind,
            rest: transform.rotation,
        });
    }
}

fn sync_scene(
    mut commands: Commands,
    mut view: ResMut<View>,
    mut selected: ResMut<Selected>,
    art: (Res<Models>, Res<Landscape>),
    mut focus: ResMut<Focus>,
    layers: Res<Layers>,
    mut objects: Query<(
        &mut Transform,
        Option<&mut ResidentMotion>,
        Option<&SiteDisplay>,
    )>,
) {
    let view = &mut *view;
    let (models, land) = art;
    let state = view.world.snapshot();
    if let Some(id) = layers.incoming
        && state.residents.iter().any(|resident| resident.id == id)
    {
        selected.0 = Some(Object::Resident(id));
    }
    if !view.scene_dirty && !selected.is_changed() {
        return;
    }
    let changed = std::mem::replace(&mut view.scene_dirty, false);
    let snap = focus.reduced_motion || view.history.is_some();
    let parent = layers.parent(Layer::World).0;
    let mut remaining = std::mem::take(&mut view.entities);
    focus.point = None;
    let mut draw = |key: Object, transform: Transform, model: &Handle<WorldAsset>| {
        if selected.0 == Some(key) {
            focus.point = Some(transform.translation + Vec3::Y * 0.3);
        }
        if let Some(entity) = remaining.remove(&key) {
            view.entities.insert(key, entity);
            return (entity, false);
        }
        let entity = commands
            .spawn((
                ChildOf(parent),
                key,
                WorldAssetRoot(model.clone()),
                transform,
            ))
            .observe(
                move |event: On<PointerClick>, mut selected: ResMut<Selected>| {
                    if event.button == PointerButton::Primary {
                        selected.0 = Some(key);
                    }
                },
            )
            .id();
        view.entities.insert(key, entity);
        (entity, true)
    };

    draw(
        Object::Depot,
        Transform::from_translation(land.point(land.depot, 0.0)).with_scale(Vec3::splat(0.82)),
        &models.depot,
    );
    let mut fresh_sites = Vec::new();
    let mut cargo_updates = Vec::new();
    for site in state.sites {
        let point = grid_point(site.position, &state);
        let count = (u32::from(site.stock) * 3).div_ceil(u32::from(site.capacity)) as usize;
        let (entity, new) = draw(
            Object::Site(site.position),
            Transform::from_translation(land.point(point, 0.24)).with_scale(Vec3::splat(0.69)),
            &models.drill,
        );
        if new {
            fresh_sites.push((entity, count));
        } else if let Ok((_, _, Some(display))) = objects.get(entity) {
            for (index, &child) in display.cargo.iter().enumerate() {
                cargo_updates.push((child, stock_visibility(index, count)));
            }
        }
    }

    let mut fresh_suits = Vec::new();
    for (index, resident) in state.residents.iter().enumerate() {
        let key = Object::Resident(resident.id);
        // Display offsets keep residents beside the machinery on occupied grid cells.
        let mut point = grid_point(resident.position, &state);
        let angle = index as f32 * 2.399_963;
        let offset = Vec2::from_angle(angle) * 0.26;
        point += if resident.position == state.depot {
            Vec2::new(offset.x * 2.0, 1.06 + offset.y)
        } else if state
            .sites
            .iter()
            .any(|site| site.position == resident.position)
        {
            Vec2::new(offset.x * 1.6, 0.61 + offset.y * 0.65)
        } else {
            offset
        };
        let suit = match resident.role {
            Role::Forager => &models.suits[0],
            Role::Builder => &models.suits[1],
        };
        let scale = if resident.role == Role::Builder {
            1.50
        } else {
            1.40
        };
        let heading = Quat::from_rotation_y(std::f32::consts::PI + angle.sin() * 0.62);
        let transform = Transform::from_translation(land.point(point, 0.03))
            .with_scale(Vec3::splat(scale))
            .with_rotation(heading);
        let (entity, new) = draw(key, transform, suit);
        let last_work = view.world.memory(resident.id).ok().and_then(|memory| {
            memory.iter().rev().find_map(|entry| {
                matches!(entry.action, Action::Gathered { .. } | Action::Built {})
                    .then_some((entry.tick, entry.action))
            })
        });
        if new {
            fresh_suits.push((
                entity,
                ResidentMotion {
                    goal: point,
                    cell: resident.position,
                    heading,
                    last_work,
                    work_age: f32::INFINITY,
                    active: resident.active,
                },
            ));
        } else if changed && let Ok((mut pose, Some(mut motion), _)) = objects.get_mut(entity) {
            let delta = point - motion.goal;
            if view.history.is_some() {
                motion.heading = heading;
            } else if delta.length_squared() > 0.001 {
                // The suits face -Z in their source files.
                motion.heading = Quat::from_rotation_y((-delta.x).atan2(-delta.y));
            }
            let cells = motion.cell.x.abs_diff(resident.position.x)
                + motion.cell.z.abs_diff(resident.position.z);
            if snap || !resident.active || cells > 1 {
                pose.translation = transform.translation;
                pose.rotation = motion.heading;
                motion.work_age = f32::INFINITY;
            } else if let Some((tick, _)) = last_work
                && tick == state.tick
                && motion.last_work != last_work
            {
                motion.work_age = 0.0;
            }
            motion.goal = point;
            motion.cell = resident.position;
            motion.last_work = last_work;
            motion.active = resident.active;
        }
    }
    for (entity, visibility) in cargo_updates {
        commands.entity(entity).insert(visibility);
    }
    for (entity, count) in fresh_sites {
        let cargo = (0..3)
            .map(|index| {
                commands
                    .spawn((
                        ChildOf(entity),
                        WorldAssetRoot(models.cargo.clone()),
                        Transform::from_xyz(-0.58 + index as f32 * 0.36, -0.30, 0.69)
                            .with_scale(Vec3::splat(0.54)),
                        stock_visibility(index, count),
                    ))
                    .id()
            })
            .collect();
        commands.entity(entity).insert(SiteDisplay { cargo });
    }
    for (entity, motion) in fresh_suits {
        commands.entity(entity).insert(motion).observe(rig_suit);
    }
    for entity in remaining.into_values() {
        commands.entity(entity).despawn();
    }

    // Shelter coordinates aren't stored by the engine. The diorama displays a bounded cluster.
    let plots = land.shelters(state.shelters);
    let count = plots.len();
    while view.shelters.len() > count {
        if let Some(entity) = view.shelters.pop() {
            commands.entity(entity).despawn();
        }
    }
    for (index, plot) in plots.enumerate().skip(view.shelters.len()) {
        view.shelters.push(
            commands
                .spawn((
                    ChildOf(parent),
                    WorldAssetRoot(models.shelter.clone()),
                    Transform::from_translation(plot)
                        .with_scale(Vec3::splat(0.57))
                        .with_rotation(Quat::from_rotation_y(index as f32 * 2.399_963)),
                ))
                .observe(world_art::ignore_decoration)
                .id(),
        );
    }
}

fn stock_visibility(index: usize, count: usize) -> Visibility {
    if index < count {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

fn animate_residents(
    time: Res<Time>,
    view: Res<View>,
    focus: Res<Focus>,
    land: Res<Landscape>,
    mut residents: Query<(&mut Transform, &mut ResidentMotion)>,
    mut joints: Query<(&mut Transform, &SuitJoint), Without<ResidentMotion>>,
) {
    let snap = focus.reduced_motion || view.history.is_some();
    let step = time.delta_secs();
    let seconds = time.elapsed_secs();
    let blend = 1.0 - (-14.0 * step).exp();
    for (mut transform, mut motion) in &mut residents {
        let position = Vec2::new(transform.translation.x, transform.translation.z);
        let next = if snap || position.distance_squared(motion.goal) < 0.0001 {
            motion.goal
        } else {
            position.lerp(motion.goal, blend)
        };
        if next != position {
            transform.translation = land.point(next, 0.03);
        }
        if !transform.rotation.abs_diff_eq(motion.heading, 0.0001) {
            transform.rotation = transform.rotation.slerp(motion.heading, blend);
        }
        motion.work_age += step;
    }
    for (mut transform, joint) in &mut joints {
        let Ok((body, motion)) = residents.get(joint.resident) else {
            continue;
        };
        let distance = Vec2::new(body.translation.x, body.translation.z).distance(motion.goal);
        let gait = (seconds * 25.0).sin() * (distance * 3.5).min(0.66);
        let work = if motion.work_age < 0.42 {
            (motion.work_age / 0.42 * std::f32::consts::PI).sin()
        } else {
            0.0
        };
        let pitch = if snap || !motion.active {
            0.0
        } else {
            match joint.kind {
                Joint::Leg(side) => gait * side,
                Joint::Arm(side) => {
                    let lift = match motion.last_work {
                        Some((_, Action::Built {})) => work * (1.15 + side * 0.42),
                        _ => work * (0.72 - side * 0.24),
                    };
                    -gait * side * 0.72 + lift
                }
                Joint::Head => (seconds * 1.8).sin() * 0.035 + work * 0.22,
            }
        };
        transform.rotation = joint.rest * Quat::from_rotation_x(pitch);
    }
}

fn links(
    view: Res<View>,
    selected: Res<Selected>,
    land: Res<Landscape>,
    positions: Query<&Transform, With<Object>>,
    mut gizmos: Gizmos,
) {
    let state = view.world.snapshot();
    let position = |key| {
        positions
            .get(*view.entities.get(&key)?)
            .ok()
            .map(|pose| pose.translation)
    };
    let ring = |center: Vec3| {
        Isometry3d::new(
            center + Vec3::Y * 0.035,
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        )
    };
    if let Some(key) = selected.0
        && let Some(center) = position(key)
    {
        let (lift, radius) = match key {
            Object::Depot => (0.0, 1.21),
            Object::Site(_) => (-0.21, 0.60),
            Object::Resident(_) => (0.0, 0.36),
        };
        gizmos.circle(
            ring(center + Vec3::Y * lift),
            radius,
            Color::srgb(0.45, 0.95, 1.0),
        );
    }
    if let Some(reasoning) = view.reasoning
        && let Some(center) = position(Object::Resident(reasoning.resident))
    {
        let color = match reasoning.phase {
            ReasoningPhase::Preparing {} => Color::srgb(1.0, 0.66, 0.20),
            ReasoningPhase::InferenceActive { .. } => Color::srgb(0.70, 0.50, 1.0),
        };
        gizmos.circle(ring(center), 0.41, color);
    }
    if let Some(Object::Resident(id)) = selected.0
        && let Some(resident) = state.residents.iter().find(|resident| resident.id == id)
    {
        let from = position(Object::Resident(id))
            .unwrap_or_else(|| land.point(grid_point(resident.position, &state), 0.03));
        for other in &state.residents {
            if resident.relationships.contains_key(&other.id)
                && let Some(to) = position(Object::Resident(other.id))
            {
                let mid = from.lerp(to, 0.5) + Vec3::Y * 1.05;
                let color = Color::srgba(0.81, 0.72, 1.0, 0.65);
                gizmos.line(from + Vec3::Y * 0.60, mid, color);
                gizmos.line(mid, to + Vec3::Y * 0.60, color);
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
        "Space  Step    P  Play/pause\nEnter  Activate/suspend\nArrows or IJKL  Move\nT  Freeze     [  Earlier"
    } else {
        "T  Freeze     [  Earlier"
    };
    let history = view.history.map_or_else(String::new, |position| {
        format!(
            "\nRetained {}-{} | Basis {}",
            position.earliest, position.latest, position.basis
        )
    });
    let text = format!(
        "SIMULATED STATE\n{}\nTick {} | Revision {}\n{}\nStock {}  |  Shelters {}{history}\n\n{detail}\n\n{controls}",
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
