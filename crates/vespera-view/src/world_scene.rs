use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::Duration;

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use tokio::sync::watch;
use vespera_core::AgentId;
use vespera_world::{Position, Role, Snapshot, World, WorldError, WorldStore};

use crate::scene::{CameraFocus, Orbit, camera, desktop};

const POLL_INTERVAL: Duration = Duration::from_millis(200);

struct Observation {
    world: World,
    status: &'static str,
}

enum Action {
    Tick,
    SetActive(AgentId, bool),
    Move(AgentId, Position),
}

struct Command {
    revision: u64,
    action: Action,
}

#[derive(Resource)]
struct View {
    updates: watch::Receiver<Observation>,
    commands: SyncSender<Command>,
    allow_control: bool,
    world: World,
    status: &'static str,
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
    allow_control: bool,
    reduced_motion: bool,
) -> Result<(), WorldError> {
    let world = WorldStore::read(&path)?;
    let state = world.snapshot();
    let home_distance = f32::from(state.width.max(state.depth))
        .mul_add(1.35, 4.0)
        .clamp(9.0, 85.0);
    let (observations, updates) = watch::channel(Observation {
        world: world.clone(),
        status: "Database connected",
    });
    let (commands, actions) = mpsc::sync_channel(1);
    std::thread::spawn(move || observe(path, observations, actions));
    desktop("Vespera | World Simulation", Color::srgb(0.04, 0.065, 0.06))
        .insert_resource(View {
            updates,
            commands,
            allow_control,
            world,
            status: "Database connected",
            pending: false,
            scene_dirty: true,
            playing: false,
            clock: Timer::from_seconds(0.5, TimerMode::Repeating),
            entities: BTreeMap::new(),
        })
        .insert_resource(CameraFocus {
            point: None,
            home_point: Vec3::ZERO,
            home_distance,
            reduced_motion,
        })
        .init_resource::<Selected>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (receive, controls, sync_scene, camera, links, inspect).chain(),
        )
        .run();
    Ok(())
}

fn observe(path: PathBuf, updates: watch::Sender<Observation>, actions: mpsc::Receiver<Command>) {
    while !updates.is_closed() {
        let (result, is_control) = match actions.recv_timeout(POLL_INTERVAL) {
            Ok(command) => (
                WorldStore::update(&path, |world| apply(world, command)),
                true,
            ),
            Err(RecvTimeoutError::Timeout) => (WorldStore::read(&path), false),
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match result {
            Ok(world) => {
                updates.send_if_modified(|last| {
                    let status = if is_control {
                        "World change committed"
                    } else {
                        "Database connected"
                    };
                    if is_control || last.world != world || last.status == "Database unavailable" {
                        *last = Observation { world, status };
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
                    let changed = last.status != status;
                    last.status = status;
                    is_control || changed
                });
            }
        }
    }
}

fn apply(world: &mut World, command: Command) -> Result<(), WorldError> {
    match command.action {
        Action::Tick => {
            if world.snapshot().revision != command.revision {
                return Err(WorldError::StaleRevision);
            }
            world.advance()
        }
        Action::SetActive(id, active) => world.set_active(id, active, command.revision),
        Action::Move(id, position) => world.move_resident(id, position, command.revision),
    }
}

fn setup(
    mut commands: Commands,
    view: Res<View>,
    focus: Res<CameraFocus>,
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
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Orbit::with_distance(focus.home_distance),
        Transform::from_xyz(8.0, 10.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
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

fn receive(mut view: ResMut<View>) {
    if !view.updates.has_changed().unwrap_or(false) {
        return;
    }
    let (world, status) = {
        let update = view.updates.borrow_and_update();
        (update.world.clone(), update.status)
    };
    if view.world != world {
        view.world = world;
        view.scene_dirty = true;
    }
    view.status = status;
    view.pending = false;
    if status == "Database unavailable" {
        view.playing = false;
    }
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    selected: Res<Selected>,
    mut view: ResMut<View>,
) {
    if !view.allow_control {
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
    if view.pending || !(step || step_key || toggle_key || shift.is_some()) {
        return;
    }
    let snapshot = view.world.snapshot();
    let action = if step_key {
        Some(Action::Tick)
    } else if let Some(Object::Resident(id)) = selected.0 {
        snapshot
            .residents
            .iter()
            .find(|resident| resident.id == id)
            .and_then(|resident| {
                if toggle_key {
                    return Some(Action::SetActive(id, !resident.active));
                }
                let (x, z) = shift?;
                Some(Action::Move(
                    id,
                    Position {
                        x: resident.position.x.checked_add_signed(x)?,
                        z: resident.position.z.checked_add_signed(z)?,
                    },
                ))
            })
    } else {
        None
    };
    if let Some(action) = action.or_else(|| step.then_some(Action::Tick))
        && view
            .commands
            .try_send(Command {
                revision: snapshot.revision,
                action,
            })
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
    selected: Res<Selected>,
    palette: Res<Palette>,
    mut focus: ResMut<CameraFocus>,
    mut objects: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
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
    let controls = if view.allow_control {
        "Space - step   P - play/pause\nEnter - activate/suspend\nArrows or IJKL - move"
    } else {
        "Read-only. Restart with\n--allow-world-control to edit."
    };
    let text = format!(
        "SIMULATED STATE\n{}\nTick {} | Revision {}\n{} | Stock {} | Shelters {}\n\n{detail}\n\n{controls}\n\nRight drag - orbit\nWheel - zoom   WASD - pan\nF - focus   R - reset",
        view.status,
        state.tick,
        state.revision,
        if !view.allow_control {
            "Observing"
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
        let mut world = World::new(spec).unwrap();
        for key in [KeyCode::KeyL, KeyCode::ArrowRight] {
            let mut keys = ButtonInput::default();
            keys.press(key);
            assert_eq!(movement(&keys), Some((1, 0)));
        }
        let to = Position { x: 2, z: 1 };
        apply(
            &mut world,
            Command {
                revision: 0,
                action: Action::Move(id, to),
            },
        )
        .unwrap();
        assert_eq!(world.snapshot().residents[0].position, to);
        assert!(matches!(
            apply(
                &mut world,
                Command {
                    revision: 0,
                    action: Action::Tick
                }
            ),
            Err(WorldError::StaleRevision)
        ));
        assert_eq!(world.snapshot().tick, 0);
    }
}
