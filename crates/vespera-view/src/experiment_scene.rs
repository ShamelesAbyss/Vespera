use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use serde_json::Value;
use vespera_app::world_experiment::{self, Settings};
use vespera_core::EventId;
use vespera_world::Role;

use crate::atlas::{self, Layer, Layers};
use crate::home_scene::{PRESET, button};

const TICKS: u32 = 60;
const SEEDS: u32 = 3;
const TARGET: u16 = 5;
const REPORT_LIMIT: u64 = 64 * 1024;

struct Report {
    directory: PathBuf,
    value: Value,
}

type Pending = Mutex<Receiver<Result<Option<Report>, String>>>;

#[derive(Resource)]
pub(super) struct ExperimentPanel {
    directory: PathBuf,
    pending: Option<Pending>,
    report: Option<Report>,
    status: String,
}

impl ExperimentPanel {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

#[derive(Component, Clone, Copy)]
struct Compare(Role);

#[derive(Component)]
struct Readout;

pub(super) fn install(app: &mut App, dir: &Path) {
    atlas::register(app, Layer::Experiments);
    app.insert_resource(ExperimentPanel {
        directory: dir.join("experiments"),
        pending: None,
        report: None,
        status: String::new(),
    })
    .add_systems(Startup, setup)
    .add_systems(Update, (input, receive, readout).chain());
}

fn setup(mut commands: Commands) {
    let camera = commands
        .spawn((
            Layer::Experiments,
            Camera2d,
            Camera {
                is_active: false,
                ..default()
            },
        ))
        .id();
    commands.spawn((
        UiTargetCamera(camera),
        atlas::ScrollPanel(Layer::Experiments),
        atlas::scroll_panel(),
        children![(atlas::scroll_content(), children![
            (Text::new("World experiments"), TextFont::from_font_size(34.0)),
            (
                Text::new(format!("Starter world  |  {TICKS} ticks per arm  |  {SEEDS} paired seeds  |  Target {TARGET} shelters\nSuspend one resident from tick zero. Press 1 or 2 to start. F5 returns Home.")),
                TextFont::from_font_size(16.0)
            ),
            (Node { flex_wrap: FlexWrap::Wrap, column_gap: px(10), row_gap: px(10), ..default() },
             children![
                button(Compare(Role::Forager), "Compare forager suspension  [1]"),
                button(Compare(Role::Builder), "Compare builder suspension  [2]"),
             ]),
            (Readout, Text::default(), TextFont::from_font_size(16.0)),
        ])],
    )).observe(|event: On<PointerClick>, layers: Res<Layers>, actions: Query<&Compare>, mut panel: ResMut<ExperimentPanel>| {
        if layers.active == Layer::Experiments && event.button == PointerButton::Primary
            && let Ok(&Compare(role)) = actions.get(event.original_event_target())
        {
            start(&mut panel, role);
        }
    });
}

fn input(keys: Res<ButtonInput<KeyCode>>, layers: Res<Layers>, mut panel: ResMut<ExperimentPanel>) {
    if layers.active != Layer::Experiments || panel.busy() {
        return;
    }
    if keys.just_pressed(KeyCode::Digit1) {
        start(&mut panel, Role::Forager);
    } else if keys.just_pressed(KeyCode::Digit2) {
        start(&mut panel, Role::Builder);
    } else if layers.switched || panel.status.is_empty() {
        let directory = panel.directory.clone();
        panel.pending = Some(background(move || latest(&directory)));
        panel.status = "Reading saved experiment results…".into();
    }
}

fn start(panel: &mut ExperimentPanel, role: Role) {
    if !panel.busy() {
        let directory = panel.directory.clone();
        panel.pending = Some(background(move || run(&directory, role).map(Some)));
        panel.status = "Running the paired simulation…".into();
    }
}

fn background(
    operation: impl FnOnce() -> Result<Option<Report>, String> + Send + 'static,
) -> Pending {
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = send.send(operation());
    });
    Mutex::new(receive)
}

fn receive(mut panel: ResMut<ExperimentPanel>) {
    let Some(pending) = &panel.pending else {
        return;
    };
    let result = pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .try_recv();
    match result {
        Ok(Ok(report)) => {
            panel.status = if report.is_some() {
                "Saved simulation comparison".into()
            } else {
                "No saved comparisons yet.".into()
            };
            panel.report = report;
        }
        Ok(Err(error)) => panel.status = error,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => panel.status = "The experiment worker stopped.".into(),
    }
    panel.pending = None;
}

fn run(root: &Path, role: Role) -> Result<Report, String> {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    let directory = root.join(format!("{:020}-{}", started.as_millis(), EventId::new()));
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let spec = serde_json::from_str(PRESET).map_err(|_| "Invalid starter world preset.")?;
    let value = world_experiment::run(
        spec,
        &Settings {
            evidence_dir: directory.clone(),
            ticks: TICKS,
            seeds: SEEDS,
            suspend_role: role,
            suspend_count: 1,
            target_shelters: TARGET,
        },
    )
    .map_err(|error| error.to_string())?;
    let temporary = directory.join("report.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    serde_json::to_writer_pretty(&mut file, &value).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(temporary, directory.join("report.json")).map_err(|error| error.to_string())?;
    Ok(Report { directory, value })
}

fn latest(root: &Path) -> Result<Option<Report>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let directory = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.join("report.json")
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_file())
        })
        .max();
    let Some(directory) = directory else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    File::open(directory.join("report.json"))
        .map_err(|error| error.to_string())?
        .take(REPORT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > REPORT_LIMIT {
        return Err("The saved experiment report exceeds the supported size.".into());
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "The saved experiment report is invalid.")?;
    if value["schema_version"] != 1 || value["experiment"] != "world_roster" {
        return Err("The saved experiment report format is unsupported.".into());
    }
    Ok(Some(Report { directory, value }))
}

fn readout(
    layers: Res<Layers>,
    panel: Res<ExperimentPanel>,
    mut text: Single<&mut Text, With<Readout>>,
) {
    if layers.active != Layer::Experiments || (!panel.is_changed() && !layers.is_changed()) {
        return;
    }
    let mut content = format!(
        "{}\nOffline simulation. Results describe simulated outcomes.",
        panel.status
    );
    if let Some(report) = &panel.report {
        let value = &report.value;
        let role = value["configuration"]["intervention"]["role"]
            .as_str()
            .unwrap_or("Unknown");
        let _ = writeln!(content, "\n\nIntervention  |  Suspend one {role}");
        let summary = &value["summary"];
        let _ = writeln!(
            content,
            "Target hits  |  Baseline {}/{}  |  Intervention {}/{}",
            summary["baseline_target_met"],
            summary["pairs"],
            summary["intervention_target_met"],
            summary["pairs"]
        );
        if let Some(pairs) = value["pairs"].as_array() {
            for pair in pairs.iter().take(SEEDS as usize) {
                let _ = writeln!(
                    content,
                    "Seed {}  |  Shelters {} vs {}  |  Delta {}",
                    pair["seed"],
                    pair["baseline"]["shelters"],
                    pair["intervention"]["shelters"],
                    pair["shelter_delta"]
                );
            }
        }
        let _ = write!(
            content,
            "\nReport and replay databases\n{}",
            report.directory.display()
        );
    }
    if text.0 != content {
        text.0 = content;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use vespera_world::{World, WorldStore};

    #[test]
    fn desktop_experiment_retains_replay_and_leaves_the_live_world_intact() {
        let dir = std::env::temp_dir().join(format!("vespera-experiment-{}", EventId::new()));
        fs::create_dir(&dir).unwrap();
        let world = dir.join("world.sqlite3");
        let preset = serde_json::from_str(PRESET).unwrap();
        WorldStore::initialize(&world, &World::new(preset).unwrap()).unwrap();
        let before = fs::read(&world).unwrap();

        let mut app = App::new();
        app.insert_resource(Layers::new(Layer::Experiments));
        install(&mut app, &dir);
        let label = app.world_mut().spawn((Readout, Text::default())).id();
        start(
            &mut app.world_mut().resource_mut::<ExperimentPanel>(),
            Role::Builder,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.world().resource::<ExperimentPanel>().busy() {
            assert!(
                std::time::Instant::now() < deadline,
                "Experiment worker timed out"
            );
            app.world_mut().run_system_once(receive).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.world_mut().run_system_once(readout).unwrap();
        let text = &app.world().get::<Text>(label).unwrap().0;
        assert!(text.contains("Saved simulation comparison"), "{text}");
        assert!(text.contains("Suspend one builder"), "{text}");

        let root = dir.join("experiments");
        fs::create_dir(root.join("zzz-incomplete-run")).unwrap();
        let saved = latest(&root).unwrap().unwrap();
        assert_eq!(saved.value["configuration"]["ticks"], TICKS);
        assert_eq!(saved.value["summary"]["pairs"], SEEDS);
        for pair in saved.value["pairs"].as_array().unwrap() {
            for arm in ["baseline", "intervention"] {
                let trial = &pair[arm];
                let evidence = &trial["evidence"];
                let path = saved.directory.join(evidence["database"].as_str().unwrap());
                let replay = WorldStore::replay(&path, evidence["revision"].as_u64()).unwrap();
                let replay = replay.world.snapshot();
                assert_eq!(replay.tick, TICKS as u64);
                assert_eq!(Some(u64::from(replay.shelters)), trial["shelters"].as_u64());
            }
        }
        assert_eq!(fs::read(world).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }
}
