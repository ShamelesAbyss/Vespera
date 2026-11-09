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
use vespera_world::{Role, WorldStore};

use crate::atlas::{self, Layer, Layers};
use crate::home_scene::{PRESET, button};

const TICKS: u32 = 60;
const SEEDS: u32 = 3;
const TARGET: u16 = 5;
const REPORT_LIMIT: u64 = 64 * 1024;

struct Report {
    directory: PathBuf,
    value: Result<Value, &'static str>,
}

struct TrialReplay {
    arm: Arm,
    seed: u64,
    revision: u64,
    tick: u64,
    shelters: u16,
    shared_stock: u16,
    active_residents: usize,
    advance_event: EventId,
}

enum Outcome {
    Report(Option<Report>),
    Replay(TrialReplay),
}

type Pending = Mutex<Receiver<Result<Outcome, String>>>;

#[derive(Resource)]
pub(super) struct ExperimentPanel {
    directory: PathBuf,
    pending: Option<Pending>,
    report: Option<Report>,
    seed_index: usize,
    replay: Option<TrialReplay>,
    status: String,
}

impl ExperimentPanel {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

#[derive(Component, Clone, Copy)]
enum Action {
    Compare(Role),
    Browse(Direction),
    NextSeed,
    Replay(Arm),
}

#[derive(Clone, Copy)]
enum Direction {
    Older,
    Newer,
}

#[derive(Clone, Copy)]
enum Arm {
    Baseline,
    Intervention,
}

impl Arm {
    fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Intervention => "intervention",
        }
    }
}

#[derive(Component)]
struct Readout;

pub(super) fn install(app: &mut App, dir: &Path) {
    atlas::register(app, Layer::Experiments);
    app.insert_resource(ExperimentPanel {
        directory: dir.join("experiments"),
        pending: None,
        report: None,
        seed_index: 0,
        replay: None,
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
                button(Action::Compare(Role::Forager), "Compare forager suspension  [1]"),
                button(Action::Compare(Role::Builder), "Compare builder suspension  [2]"),
                button(Action::Browse(Direction::Older), "Older report  [3]"),
                button(Action::Browse(Direction::Newer), "Newer report  [4]"),
                button(Action::NextSeed, "Next seed  [5]"),
                button(Action::Replay(Arm::Baseline), "Replay baseline  [6]"),
                button(Action::Replay(Arm::Intervention), "Replay intervention  [7]"),
             ]),
            (Readout, Text::default(), TextFont::from_font_size(16.0)),
        ])],
    )).observe(|event: On<PointerClick>, layers: Res<Layers>, actions: Query<&Action>, mut panel: ResMut<ExperimentPanel>| {
        if layers.active == Layer::Experiments && event.button == PointerButton::Primary
            && let Ok(&action) = actions.get(event.original_event_target())
        {
            apply(&mut panel, action);
        }
    });
}

fn input(keys: Res<ButtonInput<KeyCode>>, layers: Res<Layers>, mut panel: ResMut<ExperimentPanel>) {
    if layers.active != Layer::Experiments || panel.busy() {
        return;
    }
    let action = [
        (KeyCode::Digit1, Action::Compare(Role::Forager)),
        (KeyCode::Digit2, Action::Compare(Role::Builder)),
        (KeyCode::Digit3, Action::Browse(Direction::Older)),
        (KeyCode::Digit4, Action::Browse(Direction::Newer)),
        (KeyCode::Digit5, Action::NextSeed),
        (KeyCode::Digit6, Action::Replay(Arm::Baseline)),
        (KeyCode::Digit7, Action::Replay(Arm::Intervention)),
    ]
    .into_iter()
    .find_map(|(key, action)| keys.just_pressed(key).then_some(action));
    if let Some(action) = action {
        apply(&mut panel, action);
    } else if panel.status.is_empty() {
        let directory = panel.directory.clone();
        panel.pending = Some(background(move || {
            browse(&directory, None, Direction::Older).map(Outcome::Report)
        }));
        panel.status = "Reading saved experiment results...".into();
    }
}

fn apply(panel: &mut ExperimentPanel, action: Action) {
    if panel.busy() {
        return;
    }
    match action {
        Action::Compare(role) => {
            let directory = panel.directory.clone();
            panel.pending = Some(background(move || {
                run(&directory, role).map(|report| Outcome::Report(Some(report)))
            }));
            panel.status = "Running the paired simulation...".into();
        }
        Action::Browse(direction) => {
            let root = panel.directory.clone();
            let cursor = panel.report.as_ref().map(|report| report.directory.clone());
            panel.pending = Some(background(move || {
                let report = browse(&root, cursor.as_deref(), direction)?;
                if report.is_none() && cursor.is_some() {
                    return Err(match direction {
                        Direction::Older => "No older saved comparison.".into(),
                        Direction::Newer => "No newer saved comparison.".into(),
                    });
                }
                Ok(Outcome::Report(report))
            }));
            panel.status = "Reading saved experiment results...".into();
        }
        Action::NextSeed => {
            if let Some(pairs) = panel.report.as_ref().and_then(Report::pairs) {
                panel.seed_index = (panel.seed_index + 1) % pairs.len();
                panel.replay = None;
                panel.status = "Saved simulation comparison".into();
            }
        }
        Action::Replay(arm) => {
            let Some(report) = panel.report.as_ref() else {
                return;
            };
            let Some(pair) = report.pairs().and_then(|pairs| pairs.get(panel.seed_index)) else {
                return;
            };
            let directory = report.directory.clone();
            let index = panel.seed_index;
            let pair = pair.clone();
            panel.replay = None;
            panel.pending = Some(background(move || {
                replay(&directory, index, arm, &pair).map(Outcome::Replay)
            }));
            panel.status = "Replaying the retained trial...".into();
        }
    }
}

fn background(operation: impl FnOnce() -> Result<Outcome, String> + Send + 'static) -> Pending {
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
        Ok(Ok(Outcome::Report(report))) => {
            panel.status = match &report {
                Some(Report { value: Ok(_), .. }) => "Saved simulation comparison",
                Some(Report {
                    value: Err(error), ..
                }) => error,
                None => "No saved comparisons yet.",
            }
            .into();
            panel.report = report;
            panel.seed_index = 0;
            panel.replay = None;
        }
        Ok(Ok(Outcome::Replay(replay))) => {
            panel.status = "Replayed saved trial (read-only)".into();
            panel.replay = Some(replay);
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
    Ok(Report {
        directory,
        value: Ok(value),
    })
}

fn browse(
    root: &Path,
    cursor: Option<&Path>,
    direction: Direction,
) -> Result<Option<Report>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let candidates = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.join("report.json")
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_file())
        })
        .filter(|path| {
            cursor.is_none_or(|cursor| match direction {
                Direction::Older => path.as_path() < cursor,
                Direction::Newer => path.as_path() > cursor,
            })
        });
    let directory = if cursor.is_none() || matches!(direction, Direction::Older) {
        candidates.max()
    } else {
        candidates.min()
    };
    let Some(directory) = directory else {
        return Ok(None);
    };
    let value = load_report(&directory);
    Ok(Some(Report { directory, value }))
}

fn load_report(directory: &Path) -> Result<Value, &'static str> {
    let mut bytes = Vec::new();
    File::open(directory.join("report.json"))
        .map_err(|_| "Could not open the saved experiment report.")?
        .take(REPORT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the saved experiment report.")?;
    if bytes.len() as u64 > REPORT_LIMIT {
        return Err("The saved experiment report exceeds the supported size.");
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "The saved experiment report is invalid.")?;
    if value["schema_version"] != 1 || value["experiment"] != "world_roster" {
        return Err("The saved experiment report format is unsupported.");
    }
    if !value["pairs"]
        .as_array()
        .is_some_and(|pairs| pairs.len() == SEEDS as usize)
    {
        return Err("The saved desktop comparison has an invalid seed count.");
    }
    Ok(value)
}

impl Report {
    fn pairs(&self) -> Option<&[Value]> {
        self.value.as_ref().ok()?["pairs"]
            .as_array()
            .map(Vec::as_slice)
    }
}

fn replay(directory: &Path, index: usize, arm: Arm, pair: &Value) -> Result<TrialReplay, String> {
    const INVALID: &str = "The retained trial does not match its experiment report.";
    let trial = &pair[arm.name()];
    let evidence = &trial["evidence"];
    let filename = format!("{}-{index:02}.sqlite3", arm.name());
    if evidence["database"].as_str() != Some(&filename) {
        return Err(INVALID.into());
    }
    let revision = evidence["revision"].as_u64().ok_or(INVALID)?;
    let seed = pair["seed"].as_u64().ok_or(INVALID)?;
    let path = directory.join(filename);
    if !path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_file())
    {
        return Err("The retained trial database is missing or is not a regular file.".into());
    }
    let saved = WorldStore::replay(&path, Some(revision)).map_err(|error| error.to_string())?;
    let event = WorldStore::advance_event(&path, revision)
        .map_err(|error| error.to_string())?
        .ok_or(INVALID)?;
    let world = saved.world.snapshot();
    let active_residents = world
        .residents
        .iter()
        .filter(|resident| resident.active)
        .count();
    let site_stock: u64 = world.sites.iter().map(|site| u64::from(site.stock)).sum();
    if evidence["advance_event_id"]
        .as_str()
        .and_then(|id| id.parse().ok())
        != Some(event.event_id)
        || world.revision != revision
        || world.seed != seed
        || trial["tick"] != world.tick
        || trial["shelters"] != world.shelters
        || trial["shared_stock"] != world.shared_stock
        || trial["site_stock"] != site_stock
        || trial["active_residents"] != active_residents
    {
        return Err(INVALID.into());
    }
    Ok(TrialReplay {
        arm,
        seed,
        revision,
        tick: world.tick,
        shelters: world.shelters,
        shared_stock: world.shared_stock,
        active_residents,
        advance_event: event.event_id,
    })
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
    if let Some(Report {
        value: Ok(value), ..
    }) = &panel.report
    {
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
            for (index, pair) in pairs.iter().enumerate() {
                let _ = writeln!(
                    content,
                    "{}Seed {}  |  Shelters {} vs {}  |  Delta {}",
                    if index == panel.seed_index {
                        "Selected | "
                    } else {
                        ""
                    },
                    pair["seed"],
                    pair["baseline"]["shelters"],
                    pair["intervention"]["shelters"],
                    pair["shelter_delta"]
                );
            }
        }
    }
    if let Some(replay) = &panel.replay {
        let _ = writeln!(
            content,
            "\nRead-only {} replay | Seed {}\nTick {}  |  Revision {}  |  Active residents {}\nShelters {}  |  Shared stock {}\nAdvance event {}",
            replay.arm.name(),
            replay.seed,
            replay.tick,
            replay.revision,
            replay.active_residents,
            replay.shelters,
            replay.shared_stock,
            replay.advance_event,
        );
    }
    if let Some(name) = panel
        .report
        .as_ref()
        .and_then(|report| report.directory.file_name())
    {
        let _ = write!(
            content,
            "\nReport and replay databases in the desktop profile\nexperiments/{}",
            name.to_string_lossy()
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
    use vespera_world::World;

    fn wait(app: &mut App, label: Entity, expected: &str) {
        // Each comparison creates and flushes six SQLite stores. Hosted Windows
        // storage can take longer when the other desktop tests run in parallel.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            app.world_mut().run_system_once(receive).unwrap();
            let panel = app.world().resource::<ExperimentPanel>();
            if !panel.busy() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Experiment worker timed out waiting for {expected:?} (last status {:?})",
                panel.status
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.world_mut().run_system_once(readout).unwrap();
        let text = &app.world().get::<Text>(label).unwrap().0;
        assert!(text.contains(expected), "{text}");
    }

    fn act(app: &mut App, label: Entity, action: Action, expected: &str) {
        apply(
            &mut app.world_mut().resource_mut::<ExperimentPanel>(),
            action,
        );
        wait(app, label, expected);
    }

    #[test]
    fn desktop_experiment_retains_replay_and_leaves_the_live_world_intact() {
        let dir = std::env::temp_dir().join(format!("vespera-experiment-{}", EventId::new()));
        fs::create_dir(&dir).unwrap();
        let world = dir.join("world.sqlite3");
        let preset = serde_json::from_str(PRESET).unwrap();
        WorldStore::initialize(&world, &World::new(preset).unwrap()).unwrap();
        let before = fs::read(&world).unwrap();

        let open = || {
            let mut app = App::new();
            app.init_resource::<ButtonInput<KeyCode>>()
                .insert_resource(Layers::new(Layer::Experiments));
            install(&mut app, &dir);
            let label = app.world_mut().spawn((Readout, Text::default())).id();
            (app, label)
        };
        let (mut app, label) = open();
        act(
            &mut app,
            label,
            Action::Compare(Role::Builder),
            "Suspend one builder",
        );
        act(
            &mut app,
            label,
            Action::Compare(Role::Forager),
            "Suspend one forager",
        );
        drop(app);

        let root = dir.join("experiments");
        fs::create_dir(root.join("zzz-incomplete-run")).unwrap();
        let corrupt = root.join("zz-corrupt-run");
        fs::create_dir(&corrupt).unwrap();
        fs::write(corrupt.join("report.json"), b"broken").unwrap();
        let (mut app, label) = open();
        app.world_mut().run_system_once(input).unwrap();
        wait(&mut app, label, "saved experiment report is invalid");
        act(
            &mut app,
            label,
            Action::Browse(Direction::Older),
            "Suspend one forager",
        );
        act(
            &mut app,
            label,
            Action::Browse(Direction::Older),
            "Suspend one builder",
        );
        act(
            &mut app,
            label,
            Action::Browse(Direction::Older),
            "No older saved comparison.",
        );
        let panel = app.world().resource::<ExperimentPanel>();
        let saved = panel.report.as_ref().unwrap();
        let report = saved.value.as_ref().unwrap();
        assert_eq!(report["configuration"]["ticks"], TICKS);
        let directory = saved.directory.clone();
        let pairs = saved.pairs().unwrap().to_vec();
        for (index, pair) in pairs.iter().enumerate() {
            for arm in [Arm::Baseline, Arm::Intervention] {
                let filename = pair[arm.name()]["evidence"]["database"].as_str().unwrap();
                let path = directory.join(filename);
                let original = fs::read(&path).unwrap();
                act(
                    &mut app,
                    label,
                    Action::Replay(arm),
                    &format!("Read-only {} replay", arm.name()),
                );
                let replay = app
                    .world()
                    .resource::<ExperimentPanel>()
                    .replay
                    .as_ref()
                    .unwrap();
                assert_eq!(replay.tick, u64::from(TICKS));
                assert_eq!(Some(replay.seed), pair["seed"].as_u64());
                assert_eq!(
                    Some(u64::from(replay.shelters)),
                    pair[arm.name()]["shelters"].as_u64()
                );
                assert_eq!(fs::read(path).unwrap(), original);
            }
            assert_eq!(app.world().resource::<ExperimentPanel>().seed_index, index);
            act(
                &mut app,
                label,
                Action::NextSeed,
                "Saved simulation comparison",
            );
        }
        let mut modified = pairs[0].clone();
        modified["baseline"]["evidence"]["database"] = "../world.sqlite3".into();
        assert!(replay(&directory, 0, Arm::Baseline, &modified).is_err());
        modified.clone_from(&pairs[0]);
        modified["seed"] = 999.into();
        assert!(replay(&directory, 0, Arm::Baseline, &modified).is_err());
        modified.clone_from(&pairs[0]);
        modified["baseline"]["evidence"]["advance_event_id"] = EventId::new().to_string().into();
        assert!(replay(&directory, 0, Arm::Baseline, &modified).is_err());
        act(
            &mut app,
            label,
            Action::Browse(Direction::Newer),
            "Suspend one forager",
        );
        assert_eq!(fs::read(world).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }
}
