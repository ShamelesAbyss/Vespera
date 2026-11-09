use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::ui::prelude::UiPickingSettings;
use bevy::window::{PrimaryWindow, WindowCloseRequested};
use vespera_world::{World as Simulation, WorldStore};

use crate::atlas::{self, Layer, Layers};
use crate::desktop_update::Updates;
use crate::{machine_scene, provider_scene, scene, swarm_scene, world_scene};

pub(super) const PRESET: &str = include_str!("../../../config/world.example.json");

#[derive(Resource)]
struct LocalWorld {
    path: Option<PathBuf>,
    opening: Option<Mutex<Receiver<Result<world_scene::View, &'static str>>>>,
    connected: bool,
    open_requested: bool,
    status: &'static str,
}

#[derive(Component, Clone, Copy)]
enum Action {
    Machine,
    World,
    Providers,
    Swarm,
    Workflows,
    Experiments,
    Motion,
    CheckUpdates,
    InstallUpdate,
}

#[derive(Component)]
struct UpdateStatus;

#[derive(Component)]
struct InstallControl;

#[derive(Component)]
struct InstallLabel;

#[derive(Component)]
struct WorldStatus;

#[derive(Component)]
struct HardwareStatus;

#[derive(Component)]
struct MotionStatus;

#[derive(Resource)]
struct MotionPreference {
    path: Option<PathBuf>,
    enabled: bool,
    forced: bool,
    requested: bool,
    issue: Option<&'static str>,
}

impl MotionPreference {
    fn load(path: Option<PathBuf>, forced: bool) -> Self {
        let saved = path.as_deref().map(read_motion).transpose();
        Self {
            path,
            enabled: forced || matches!(saved, Ok(Some(true))),
            forced,
            requested: false,
            issue: saved
                .err()
                .map(|_| "Could not read the reduced-motion preference."),
        }
    }

    fn label(&self) -> &'static str {
        if self.forced {
            "Reduced motion on for this launch (--reduced-motion)"
        } else if let Some(issue) = self.issue {
            issue
        } else if self.path.is_none() {
            "Reduced-motion settings require a local data directory."
        } else if self.enabled {
            "Reduced motion on"
        } else {
            "Reduced motion off"
        }
    }
}

fn read_motion(path: &Path) -> io::Result<bool> {
    match path.symlink_metadata() {
        Ok(info) if info.is_file() && info.len() == 0 => Ok(true),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unrecognized motion preference",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn save_motion(path: &Path, enabled: bool) -> io::Result<()> {
    if read_motion(path)? == enabled {
        return Ok(());
    }
    if enabled {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && read_motion(path)? => {
                Ok(())
            }
            Err(error) => Err(error),
        }
    } else {
        match fs::remove_file(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

pub(super) fn run(data_dir: Option<PathBuf>, reduced_motion: bool) -> std::process::ExitCode {
    let data_dir = data_dir.or_else(default_data_dir);
    let motion = MotionPreference::load(
        data_dir
            .as_ref()
            .map(|dir| dir.join("reduced-motion.enabled")),
        reduced_motion,
    );
    let reduced_motion = motion.enabled;
    let path = data_dir.as_ref().map(|dir| dir.join("world.sqlite3"));
    let status = if path.is_some() {
        "Create the starter world or reopen its saved state."
    } else {
        "A local data directory is unavailable."
    };
    let mut app = scene::desktop("Vespera", Color::srgb(0.029, 0.040, 0.065), Layer::Home);
    machine_scene::install(&mut app, None, reduced_motion);
    world_scene::install_scene(&mut app, reduced_motion);
    if let Some(dir) = data_dir.clone() {
        swarm_scene::install_desktop(&mut app, dir.clone(), reduced_motion);
        crate::experiment_scene::install(&mut app, &dir);
        crate::workflow_history_scene::install(&mut app, dir);
    }
    provider_scene::install(&mut app, data_dir);
    atlas::register(&mut app, Layer::Home);
    app.world_mut()
        .resource_mut::<Layers>()
        .set_available(Layer::World, false);
    atlas::navigation(&mut app);
    app.insert_resource(UiPickingSettings {
        require_markers: true,
    })
    .insert_resource(LocalWorld {
        path,
        opening: None,
        connected: false,
        open_requested: false,
        status,
    })
    .insert_resource(motion)
    .init_resource::<Updates>()
    .add_systems(Startup, setup)
    .add_systems(Update, highlight)
    .add_systems(
        Update,
        (actions, finish_open, readouts, update_motion, update_status).chain(),
    );
    let handoff = app.world().resource::<Updates>().handoff.clone();
    if app.run() != AppExit::Success {
        return std::process::ExitCode::FAILURE;
    }
    let setup = handoff
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(setup) = setup
        && let Err(error) = setup.launch()
    {
        eprintln!("Could not open verified Vespera Setup: {error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

fn default_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    base.filter(|path| path.is_absolute())
        .map(|base| base.join("Vespera"))
}

fn setup(
    mut commands: Commands,
    layers: Res<Layers>,
    local: Res<LocalWorld>,
    motion: Res<MotionPreference>,
    updates: Res<Updates>,
    brand: Res<crate::branding::Brand>,
) {
    let camera = commands
        .spawn((
            Layer::Home,
            Camera2d,
            Camera {
                is_active: layers.active == Layer::Home,
                ..default()
            },
        ))
        .id();
    let world_path = local.path.as_ref().map_or_else(
        || "Local world location unavailable".to_owned(),
        |path| path.display().to_string(),
    );
    commands
        .spawn((
            UiTargetCamera(camera),
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::all(px(24)),
                row_gap: px(18),
                ..default()
            },
            children![
                (
                    Node {
                        align_items: AlignItems::Center,
                        column_gap: px(24),
                        ..default()
                    },
                    children![
                        brand.mark(144.0),
                        (
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: px(6),
                                ..default()
                            },
                            children![
                                (
                                    Text::new("VESPERA"),
                                    TextFont::from_font_size(48.0),
                                    TextColor(Color::srgb(0.92, 0.95, 1.0))
                                ),
                                (
                                    Text::new("Local intelligence workspace"),
                                    TextFont::from_font_size(22.0),
                                    TextColor(Color::srgb(0.61, 0.78, 0.95))
                                )
                            ]
                        )
                    ]
                ),
                (
                    HardwareStatus,
                    Text::new("Reading local hardware"),
                    TextFont::from_font_size(16.0)
                ),
                (
                    Node {
                        column_gap: px(12),
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    children![
                        button(Action::Machine, "Open Hardware  [F1]"),
                        button(Action::World, "Create or resume world  [Enter]"),
                        button(Action::Providers, "Models & Providers  [F6]"),
                        button(Action::Swarm, "Open Swarm  [F3]"),
                        button(Action::Workflows, "Workflow history  [F7]"),
                        button(Action::Experiments, "World experiments  [F8]"),
                        button(Action::Motion, "Reduced motion  [M]"),
                        button(Action::CheckUpdates, "Check for updates")
                    ]
                ),
                (
                    WorldStatus,
                    Text::new(local.status),
                    TextFont::from_font_size(16.0)
                ),
                (
                    Node {
                        max_width: px(800),
                        ..default()
                    },
                    Text::new(world_path),
                    TextFont::from_font_size(13.0),
                    TextColor(Color::srgb(0.69, 0.77, 0.85))
                ),
                (
                    Text::new("Local desktop  |  Inference runs only when requested"),
                    TextFont::from_font_size(14.0),
                    TextColor(Color::srgb(0.69, 0.77, 0.85))
                ),
                (
                    MotionStatus,
                    Text::new(motion.label()),
                    TextFont::from_font_size(14.0)
                ),
                (
                    UpdateStatus,
                    Text::new(&updates.status),
                    TextFont::from_font_size(14.0)
                ),
                (
                    InstallControl,
                    Node {
                        display: Display::None,
                        ..default()
                    },
                    children![button_with_label(
                        Action::InstallUpdate,
                        "Download update",
                        InstallLabel
                    )]
                ),
            ],
        ))
        .observe(
            |event: On<PointerClick>,
             actions: Query<&Action>,
             mut layers: ResMut<Layers>,
             mut local: ResMut<LocalWorld>,
             mut motion: ResMut<MotionPreference>,
             mut updates: ResMut<Updates>,
             primary: Single<Entity, With<PrimaryWindow>>,
             mut close: MessageWriter<WindowCloseRequested>| {
                if layers.active == Layer::Home && event.button == PointerButton::Primary {
                    match actions.get(event.original_event_target()) {
                        Ok(Action::Machine) => layers.request(Layer::Machine),
                        Ok(Action::World) => local.open_requested = true,
                        Ok(Action::Providers) => layers.request(Layer::Providers),
                        Ok(Action::Swarm) => layers.request(Layer::Swarm),
                        Ok(Action::Workflows) => layers.request(Layer::Workflows),
                        Ok(Action::Experiments) => layers.request(Layer::Experiments),
                        Ok(Action::Motion) => motion.requested = true,
                        Ok(Action::CheckUpdates) => updates.check(),
                        Ok(Action::InstallUpdate) => {
                            if updates.install() {
                                close.write(WindowCloseRequested { window: *primary });
                            }
                        }
                        Err(_) => {}
                    }
                }
            },
        );
}

fn update_status(
    mut updates: ResMut<Updates>,
    mut label: Single<&mut Text, (With<UpdateStatus>, Without<InstallLabel>)>,
    mut control: Single<&mut Node, With<InstallControl>>,
    mut install_label: Single<&mut Text, (With<InstallLabel>, Without<UpdateStatus>)>,
) {
    updates.poll();
    if label.0 != updates.status {
        label.0.clone_from(&updates.status);
    }
    if let Some(text) = updates.install_label() {
        if control.display != Display::Flex {
            control.display = Display::Flex;
        }
        if install_label.0 != text {
            install_label.0 = text.into();
        }
    } else if control.display != Display::None {
        control.display = Display::None;
    }
}

fn update_motion(
    keys: Res<ButtonInput<KeyCode>>,
    layers: Res<Layers>,
    mut motion: ResMut<MotionPreference>,
    mut machine: ResMut<scene::CameraFocus<{ Layer::Machine as usize }>>,
    mut world: ResMut<scene::CameraFocus<{ Layer::World as usize }>>,
    swarm: Option<ResMut<scene::CameraFocus<{ Layer::Swarm as usize }>>>,
    mut label: Single<&mut Text, With<MotionStatus>>,
) {
    if layers.active != Layer::Home
        || !(std::mem::take(&mut motion.requested) || keys.just_pressed(KeyCode::KeyM))
        || motion.forced
    {
        return;
    }
    if let Some(path) = &motion.path {
        let next = !motion.enabled;
        match save_motion(path, next) {
            Ok(()) => {
                motion.enabled = next;
                motion.issue = None;
                machine.reduced_motion = next;
                world.reduced_motion = next;
                if let Some(mut swarm) = swarm {
                    swarm.reduced_motion = next;
                }
            }
            Err(_) => motion.issue = Some("Could not save the reduced-motion preference."),
        }
    }
    label.0 = motion.label().into();
}

pub(super) fn button(action: impl Component, label: &'static str) -> impl Bundle {
    button_with_label(action, label, ())
}

fn button_with_label(
    action: impl Component,
    label: &'static str,
    marker: impl Bundle,
) -> impl Bundle {
    (
        action,
        Pickable::default(),
        Hovered::default(),
        Node {
            padding: UiRect::axes(px(16), px(13)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.11, 0.23, 0.37)),
        children![(
            marker,
            Text::new(label),
            TextFont::from_font_size(17.0),
            Pickable::IGNORE
        )],
    )
}

fn actions(
    keys: Res<ButtonInput<KeyCode>>,
    mut layers: ResMut<Layers>,
    mut world: ResMut<LocalWorld>,
) {
    if layers.active != Layer::Home {
        return;
    }
    let open_world = std::mem::take(&mut world.open_requested) || keys.just_pressed(KeyCode::Enter);
    if !open_world || world.opening.is_some() {
        return;
    }
    if world.connected {
        layers.request(Layer::World);
    } else if let Some(path) = world.path.clone() {
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = send.send(open_world_store(path));
        });
        world.opening = Some(Mutex::new(receive));
        world.status = "Opening the local world...";
    }
}

fn highlight(mut buttons: Query<(&Hovered, &mut BackgroundColor), Changed<Hovered>>) {
    for (hovered, mut color) in &mut buttons {
        color.0 = if hovered.0 {
            Color::srgb(0.16, 0.33, 0.49)
        } else {
            Color::srgb(0.11, 0.23, 0.37)
        };
    }
}

fn open_world_store(path: PathBuf) -> Result<world_scene::View, &'static str> {
    match path.symlink_metadata() {
        Ok(info) if !info.is_file() => return Err("The local world path is not a regular file."),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            fs::create_dir_all(parent).map_err(|_| "Could not create the local data directory.")?;
            let spec =
                serde_json::from_str(PRESET).map_err(|_| "The starter world preset is invalid.")?;
            let world =
                Simulation::new(spec).map_err(|_| "The starter world preset is invalid.")?;
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    drop(file);
                    WorldStore::initialize(&path, &world)
                        .map_err(|_| "Could not initialize the new world database.")?;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Err("The world path changed. Try opening it again.");
                }
                Err(_) => return Err("Could not create the local world database."),
            }
        }
        Err(_) => return Err("Could not inspect the local world database."),
    }
    world_scene::connect(path, None, true)
        .map_err(|_| "Could not open the world database. Check the path and storage format.")
}

fn finish_open(mut commands: Commands, mut layers: ResMut<Layers>, mut world: ResMut<LocalWorld>) {
    let Some(receiver) = world.opening.as_ref() else {
        return;
    };
    let result = match receiver
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .try_recv()
    {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            Err("The world reader stopped before opening the database.")
        }
    };
    world.opening = None;
    match result {
        Ok(view) => {
            commands.insert_resource(view);
            world.connected = true;
            world.status = "Local world connected. Return with F4 or Enter.";
            layers.set_available(Layer::World, true);
            if layers.active == Layer::Home {
                layers.request(Layer::World);
            }
        }
        Err(error) => world.status = error,
    }
}

fn readouts(
    layers: Res<Layers>,
    local: Res<LocalWorld>,
    hardware: Res<machine_scene::Telemetry>,
    adapter: Option<Res<RenderAdapterInfo>>,
    mut world: Single<&mut Text, (With<WorldStatus>, Without<HardwareStatus>)>,
    mut machine: Single<&mut Text, With<HardwareStatus>>,
) {
    if layers.active != Layer::Home {
        return;
    }
    if world.0 != local.status {
        world.0 = local.status.into();
    }
    let host = hardware
        .home_summary()
        .unwrap_or_else(|| "Host measurements unavailable".into());
    let renderer = adapter
        .as_ref()
        .map_or("Render adapter unavailable", |adapter| {
            adapter.name.as_str()
        });
    let text = format!("Local OS  |  {host}\nwgpu  |  {renderer}");
    if machine.0 != text {
        machine.0 = text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vespera_world::{WorldChange, WorldFence};

    #[test]
    fn home_motion_preference_survives_restart_and_preserves_unknown_files() {
        let dir =
            std::env::temp_dir().join(format!("vespera-motion-{}", vespera_core::TaskId::new()));
        let path = dir.join("reduced-motion.enabled");
        assert!(!MotionPreference::load(Some(path.clone()), false).enabled);
        save_motion(&path, true).unwrap();
        save_motion(&path, true).unwrap();
        assert!(MotionPreference::load(Some(path.clone()), false).enabled);
        save_motion(&path, false).unwrap();
        assert!(!MotionPreference::load(Some(path.clone()), false).enabled);
        assert!(MotionPreference::load(Some(path.clone()), true).enabled);
        assert!(!path.exists());
        fs::write(&path, b"Existing user data").unwrap();
        assert!(save_motion(&path, true).is_err());
        assert!(save_motion(&path, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"Existing user data");
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn home_creates_a_world_and_reopens_its_committed_revision() {
        let dir =
            std::env::temp_dir().join(format!("vespera-home-{}", vespera_core::TaskId::new()));
        let path = dir.join("world.sqlite3");
        drop(open_world_store(path.clone()).unwrap());
        assert_eq!(WorldStore::read(&path).unwrap().revision(), 0);
        WorldStore::commit(
            &path,
            WorldFence::Revision(0),
            WorldChange::Advance { ticks: 1 },
        )
        .unwrap();
        drop(open_world_store(path.clone()).unwrap());
        assert_eq!(WorldStore::read(&path).unwrap().revision(), 1);
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn home_preserves_unrecognized_files() {
        let path =
            std::env::temp_dir().join(format!("vespera-home-{}", vespera_core::TaskId::new()));
        let bytes = b"Existing user data";
        fs::write(&path, bytes).unwrap();
        assert!(open_world_store(path.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(path).unwrap();
    }
}
