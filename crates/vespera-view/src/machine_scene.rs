use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, System};
use tokio::sync::watch;
use vespera_core::SchedulerSample;

use crate::atlas::{Layer, Layers, register};
use crate::scene::{CameraFocus, Orbit, camera, desktop};

type Focus = CameraFocus<{ Layer::Machine as usize }>;

const INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct Sample {
    observed: Instant,
    threads: Option<usize>,
    cpu: Option<f32>,
    memory: Option<(u64, u64)>,
    scheduler: SchedulerObservation,
}

#[derive(Clone, Copy, Default)]
enum SchedulerObservation {
    #[default]
    NoSource,
    Unavailable,
    Stale,
    Sample(SchedulerSample),
}

impl SchedulerObservation {
    fn read(path: &Path) -> Self {
        if let Some(sample) = read_scheduler_sample(path) {
            let observed =
                SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(sample.observed_unix_ms));
            if !sample.finished
                && !observed
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age < INTERVAL * 3)
            {
                return Self::Stale;
            }
            Self::Sample(sample)
        } else {
            Self::Unavailable
        }
    }
}

fn read_scheduler_sample(path: &Path) -> Option<SchedulerSample> {
    let metadata = path.symlink_metadata().ok()?;
    if !metadata.is_file() || metadata.len() > SchedulerSample::MAX_FILE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(SchedulerSample::MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > SchedulerSample::MAX_FILE_BYTES {
        return None;
    }
    let sample: SchedulerSample = serde_json::from_slice(&bytes).ok()?;
    (sample.schema_version == SchedulerSample::SCHEMA_VERSION && sample.budget.validate().is_ok())
        .then_some(sample)
}

#[derive(Resource)]
struct Telemetry(watch::Receiver<Option<Sample>>);

#[derive(Component, Clone, Copy, Default)]
enum Device {
    #[default]
    Cpu,
    Memory,
    Adapter,
    Scheduler,
}

impl Device {
    fn position(self) -> Vec3 {
        match self {
            Self::Cpu => Vec3::new(-3.6, 0.0, 0.0),
            Self::Memory => Vec3::ZERO,
            Self::Adapter => Vec3::new(3.6, 0.0, 0.0),
            Self::Scheduler => Vec3::new(0.0, 0.0, -3.2),
        }
    }
}

#[derive(Resource, Default)]
struct Selected(Device);

#[derive(Component)]
struct Meter(Device);

#[derive(Component)]
struct Readout;

pub(super) fn run(telemetry_directory: Option<PathBuf>, reduced_motion: bool) {
    let mut app = desktop(
        "Vespera | The Machine",
        Color::srgb(0.017, 0.023, 0.043),
        Layer::Machine,
    );
    install(&mut app, telemetry_directory, reduced_motion);
    app.run();
}

pub(super) fn install(app: &mut App, telemetry_directory: Option<PathBuf>, reduced_motion: bool) {
    register(app, Layer::Machine);
    let source = telemetry_directory.map(|directory| directory.join(SchedulerSample::FILE_NAME));
    let (send, receive) = watch::channel(None);
    std::thread::spawn(move || observe(&send, source.as_deref()));
    app.insert_resource(Telemetry(receive))
        .init_resource::<Selected>()
        .insert_resource(Focus {
            point: None,
            home_point: Vec3::new(1.6, 0.0, 0.0),
            home_distance: 16.0,
            reduced_motion,
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (navigate, camera::<{ Layer::Machine as usize }>, present)
                .chain()
                .run_if(|layers: Res<Layers>| layers.active == Layer::Machine),
        );
}

fn observe(send: &watch::Sender<Option<Sample>>, source: Option<&Path>) {
    let mut system = System::new();
    let mut have_baseline = false;
    while !send.is_closed() {
        system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        system.refresh_cpu_specifics(CpuRefreshKind::nothing().with_cpu_usage());
        let total = system.total_memory();
        let available = system.available_memory();
        let cpu = system.global_cpu_usage();
        let sample = Sample {
            observed: Instant::now(),
            threads: std::thread::available_parallelism().ok().map(usize::from),
            cpu: (have_baseline && !system.cpus().is_empty() && (0.0..=100.0).contains(&cpu))
                .then_some(cpu),
            memory: (total > 0 && available <= total).then_some((total, available)),
            scheduler: source.map_or(SchedulerObservation::NoSource, SchedulerObservation::read),
        };
        if send.send(Some(sample)).is_err() {
            break;
        }
        have_baseline = true;
        std::thread::sleep(INTERVAL.max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL));
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    focus: Res<Focus>,
    layers: Res<Layers>,
) {
    let plate = meshes.add(Cuboid::new(2.9, 0.25, 2.45));
    let chip = meshes.add(Cuboid::new(1.35, 0.5, 1.35));
    let meter = meshes.add(Cuboid::new(1.0, 0.10, 0.20));
    let base = materials.add(Color::srgb(0.085, 0.12, 0.19));
    let camera = commands
        .spawn((
            Layer::Machine,
            Camera {
                is_active: layers.active == Layer::Machine,
                ..default()
            },
            Camera3d::default(),
            Tonemapping::None,
            Orbit::at_home(&focus),
            Transform::from_xyz(1.6, 10.0, 13.0).looking_at(focus.home_point, Vec3::Y),
        ))
        .id();
    commands.spawn((
        layers.parent(Layer::Machine),
        DirectionalLight {
            illuminance: 6500.0,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, -0.65, 0.0)),
    ));
    for (device, color) in [
        (Device::Cpu, Color::srgb(0.11, 0.70, 0.88)),
        (Device::Memory, Color::srgb(0.16, 0.77, 0.51)),
        (Device::Adapter, Color::srgb(0.62, 0.36, 0.86)),
        (Device::Scheduler, Color::srgb(0.95, 0.57, 0.18)),
    ] {
        let point = device.position();
        let material = materials.add(color);
        commands.spawn((
            layers.parent(Layer::Machine),
            Mesh3d(plate.clone()),
            MeshMaterial3d(base.clone()),
            Transform::from_translation(point),
        ));
        commands
            .spawn((
                layers.parent(Layer::Machine),
                Mesh3d(chip.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(point + Vec3::Y * 0.38),
            ))
            .observe(
                move |event: On<PointerClick>, mut selected: ResMut<Selected>| {
                    if event.button == PointerButton::Primary {
                        selected.0 = device;
                    }
                },
            );
        if !matches!(device, Device::Adapter) {
            commands.spawn((
                layers.parent(Layer::Machine),
                Meter(device),
                Mesh3d(meter.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(point + Vec3::new(0.0, 0.18, 0.94)),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        }
    }
    commands.spawn((
        UiTargetCamera(camera),
        Node {
            position_type: PositionType::Absolute,
            top: px(20.0),
            right: px(20.0),
            width: px(350.0),
            padding: UiRect::all(px(18.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.033, 0.05, 0.088, 0.96)),
        children![(
            Readout,
            Text::new("THE MACHINE\nWaiting for local measurements"),
            TextFont::from_font_size(16.0),
            TextColor(Color::srgb(0.84, 0.91, 1.0)),
        )],
    ));
}

fn navigate(
    keys: Res<ButtonInput<KeyCode>>,
    mut selected: ResMut<Selected>,
    mut focus: ResMut<Focus>,
) {
    for (key, device) in [
        (KeyCode::Digit1, Device::Cpu),
        (KeyCode::Digit2, Device::Memory),
        (KeyCode::Digit3, Device::Adapter),
        (KeyCode::Digit4, Device::Scheduler),
    ] {
        if keys.just_pressed(key) {
            selected.0 = device;
        }
    }
    if keys.just_pressed(KeyCode::KeyF) {
        focus.point = Some(selected.0.position());
    }
}

fn present(
    telemetry: Res<Telemetry>,
    selected: Res<Selected>,
    adapter: Option<Res<RenderAdapterInfo>>,
    mut readout: Single<&mut Text, With<Readout>>,
    mut meters: Query<(&Meter, &mut Transform, &mut Visibility)>,
    mut gizmos: Gizmos,
    mut previous: Local<Option<Instant>>,
) {
    gizmos.cube(
        Transform::from_translation(selected.0.position() + Vec3::Y * 0.13)
            .with_scale(Vec3::new(3.02, 0.30, 2.55)),
        Color::srgb(0.88, 0.94, 1.0),
    );
    let sample = (*telemetry.0.borrow()).filter(|sample| sample.observed.elapsed() < INTERVAL * 3);
    let observed = sample.map(|sample| sample.observed);
    if *previous == observed
        && !selected.is_changed()
        && !adapter.as_ref().is_some_and(|adapter| adapter.is_changed())
    {
        return;
    }
    *previous = observed;
    let cpu = sample.and_then(|sample| sample.cpu);
    let memory = sample.and_then(|sample| sample.memory);
    let scheduler = sample.map_or(SchedulerObservation::Unavailable, |sample| sample.scheduler);
    for (meter, mut transform, mut visibility) in &mut meters {
        let fraction = match meter.0 {
            Device::Cpu => cpu.map(|percent| percent / 100.0),
            Device::Memory => {
                memory.map(|(total, available)| (total - available) as f32 / total as f32)
            }
            Device::Adapter => None,
            Device::Scheduler => match scheduler {
                SchedulerObservation::Sample(sample) if !sample.finished => Some(
                    (sample.admitted as f32 / sample.budget.max_concurrent_inferences as f32)
                        .min(1.0),
                ),
                _ => None,
            },
        };
        *visibility = Visibility::Hidden;
        if let Some(fraction) = fraction.filter(|fraction| *fraction > 0.0) {
            transform.scale.x = fraction * 2.5;
            transform.translation.x = meter.0.position().x - 1.25 + transform.scale.x / 2.0;
            *visibility = Visibility::Inherited;
        }
    }
    let detail = match selected.0 {
        Device::Cpu => {
            let threads = sample
                .and_then(|sample| sample.threads)
                .map_or_else(|| "Unavailable".into(), |count| count.to_string());
            let utilization =
                cpu.map_or_else(|| "Unavailable".into(), |value| format!("{value:.1}%"));
            format!(
                "CPU  /  LOCAL HOST\n\nProcess-available logical threads  {threads}\nHost CPU utilization  {utilization}"
            )
        }
        Device::Memory => {
            let capacity = memory.map_or_else(
                || "Memory measurement unavailable".into(),
                |(total, available)| {
                    let gib = 1024.0_f64.powi(3);
                    format!(
                        "Total  {:.2} GiB\nAvailable  {:.2} GiB",
                        total as f64 / gib,
                        available as f64 / gib
                    )
                },
            );
            format!(
                "MEMORY  /  LOCAL HOST\n\n{capacity}\n\nHost values may exceed container limits."
            )
        }
        Device::Adapter => {
            let identity = adapter.map_or_else(
                || "Adapter unavailable".into(),
                |adapter| {
                    format!(
                        "{}\n{:?}  /  {:?}",
                        adapter.name, adapter.device_type, adapter.backend
                    )
                },
            );
            format!("WGPU RENDER ADAPTER\n\n{identity}\n\nVRAM and inference devices unavailable.")
        }
        Device::Scheduler => scheduler_detail(scheduler),
    };
    let age = sample.map_or(
        "Host measurements unavailable",
        |_| "Host measurements / live",
    );
    let text = format!(
        "VESPERA  /  THE MACHINE\n{age}\n\n{detail}\n\n1 CPU | 2 Memory | 3 Adapter\n4 Scheduler | Click to select\nF focus | R reset | WASD pan\nRight drag orbit | Scroll zoom"
    );
    if readout.0 != text {
        readout.0 = text;
    }
}

fn scheduler_detail(observation: SchedulerObservation) -> String {
    let detail = match observation {
        SchedulerObservation::NoSource => "No local run directory configured".into(),
        SchedulerObservation::Unavailable => "Waiting for a readable scheduler sample".into(),
        SchedulerObservation::Stale => "Last scheduler sample expired".into(),
        SchedulerObservation::Sample(sample) => {
            let state = if sample.finished {
                "Final snapshot"
            } else {
                "Recent sample"
            };
            let queue = if sample.queue_closed {
                "Closed"
            } else {
                "Open"
            };
            let health = sample.provider_health;
            format!(
                "{state}\nQueue {queue} | waiting {}\nAdmitted {} / {}\nAccounted inference slots {}\n\nEstimated reservations / budget\nRAM {} / {} bytes\nVRAM {} / {} bytes\nModels {} | uncertain {}\n\nCached provider health\nHealthy {} | degraded {}\nUnreachable {} | stale {}\nUnknown {}",
                sample.waiting,
                sample.admitted,
                sample.budget.max_concurrent_inferences,
                sample.reserved.active_inferences,
                sample.reserved.ram_bytes,
                sample.budget.ram_bytes,
                sample.reserved.vram_bytes,
                sample.budget.vram_bytes,
                sample.model_reservations,
                sample.uncertain_reservations,
                health.healthy,
                health.degraded,
                health.unreachable,
                health.stale,
                health.unknown,
            )
        }
    };
    format!("SCHEDULER / ONE LOCAL RUNTIME\n\n{detail}")
}
