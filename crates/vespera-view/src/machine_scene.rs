use std::time::{Duration, Instant};

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, System};
use tokio::sync::watch;

use crate::scene::{CameraFocus, Orbit, camera, desktop};

const INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct Sample {
    observed: Instant,
    threads: Option<usize>,
    cpu: Option<f32>,
    memory: Option<(u64, u64)>,
}

#[derive(Resource)]
struct Telemetry(watch::Receiver<Option<Sample>>);

#[derive(Component, Clone, Copy, Default)]
enum Device {
    #[default]
    Cpu,
    Memory,
    Adapter,
}

impl Device {
    fn position(self) -> Vec3 {
        Vec3::new(
            match self {
                Self::Cpu => -3.6,
                Self::Memory => 0.0,
                Self::Adapter => 3.6,
            },
            0.0,
            0.0,
        )
    }
}

#[derive(Resource, Default)]
struct Selected(Device);

#[derive(Component)]
struct Meter(Device);

#[derive(Component)]
struct Readout;

pub(super) fn run(reduced_motion: bool) {
    let (send, receive) = watch::channel(None);
    std::thread::spawn(move || observe(&send));
    desktop("Vespera | The Machine", Color::srgb(0.017, 0.023, 0.043))
        .insert_resource(Telemetry(receive))
        .init_resource::<Selected>()
        .insert_resource(CameraFocus {
            point: None,
            home_point: Vec3::new(1.6, 0.0, 0.0),
            home_distance: 16.0,
            reduced_motion,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (navigate, camera, present).chain())
        .run();
}

fn observe(send: &watch::Sender<Option<Sample>>) {
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
    focus: Res<CameraFocus>,
) {
    let plate = meshes.add(Cuboid::new(2.9, 0.25, 2.45));
    let chip = meshes.add(Cuboid::new(1.35, 0.5, 1.35));
    let meter = meshes.add(Cuboid::new(1.0, 0.10, 0.20));
    let base = materials.add(Color::srgb(0.085, 0.12, 0.19));
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Orbit::at_home(&focus),
        Transform::from_xyz(1.6, 10.0, 13.0).looking_at(focus.home_point, Vec3::Y),
    ));
    commands.spawn((
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
    ] {
        let point = device.position();
        let material = materials.add(color);
        commands.spawn((
            Mesh3d(plate.clone()),
            MeshMaterial3d(base.clone()),
            Transform::from_translation(point),
        ));
        commands
            .spawn((
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
        if matches!(device, Device::Cpu | Device::Memory) {
            commands.spawn((
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
    mut focus: ResMut<CameraFocus>,
) {
    for (key, device) in [
        (KeyCode::Digit1, Device::Cpu),
        (KeyCode::Digit2, Device::Memory),
        (KeyCode::Digit3, Device::Adapter),
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
    for (meter, mut transform, mut visibility) in &mut meters {
        let fraction = match meter.0 {
            Device::Cpu => cpu.map(|percent| percent / 100.0),
            Device::Memory => {
                memory.map(|(total, available)| (total - available) as f32 / total as f32)
            }
            Device::Adapter => None,
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
    };
    let age = sample.map_or(
        "Measurements unavailable",
        |_| "Live / sampled every second",
    );
    let text = format!(
        "VESPERA  /  THE MACHINE\n{age}\n\n{detail}\n\nScheduler and provider telemetry unavailable.\n\n1 CPU | 2 Memory | 3 Adapter\nClick to select | F focus | R reset\nRight drag orbit | WASD pan | Scroll zoom"
    );
    if readout.0 != text {
        readout.0 = text;
    }
}
