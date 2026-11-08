use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, System};
use tokio::sync::watch;
use vespera_core::{
    AgentId, ModelId, ModelReservationSample, ProviderId, ReservationState, SchedulerSample,
    TaskId, TaskLeaseSample,
};

use crate::atlas::{Layer, Layers, register};
use crate::gpu_telemetry::{Gpus, Probe};
use crate::scene::{CameraFocus, Orbit, camera, desktop};

type Focus = CameraFocus<{ Layer::Machine as usize }>;

const INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct Sample {
    observed: Instant,
    threads: Option<usize>,
    cpu: Option<f32>,
    memory: Option<(u64, u64)>,
    gpus: Gpus,
    scheduler: SchedulerObservation,
}

#[derive(Clone, Default)]
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
    sample.is_supported().then_some(sample)
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

#[derive(Clone, Copy)]
enum Target {
    Device(Device),
    Runtime(RuntimeNode),
}

#[derive(Resource)]
struct Selected(Target);

#[derive(Resource, Default)]
struct SelectedGpu(String);

impl Default for Selected {
    fn default() -> Self {
        Self(Target::Device(Device::Cpu))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RuntimeNode {
    Provider(ProviderId),
    Model(ModelId),
    Task(TaskId),
    Memory(TaskId),
}

pub(super) fn selected_agent(world: &World) -> Option<AgentId> {
    let Target::Runtime(RuntimeNode::Task(id) | RuntimeNode::Memory(id)) =
        world.get_resource::<Selected>()?.0
    else {
        return None;
    };
    world
        .resource::<RuntimeGraph>()
        .task(id)
        .map(|task| task.agent_id)
}

#[derive(Resource, Default)]
struct RuntimeGraph {
    nodes: BTreeMap<RuntimeNode, (Entity, Vec3)>,
    sample: Option<Sample>,
}

impl RuntimeGraph {
    fn scheduler(&self) -> &SchedulerObservation {
        self.sample
            .as_ref()
            .map_or(&SchedulerObservation::Unavailable, |sample| {
                &sample.scheduler
            })
    }

    fn models(&self) -> &[ModelReservationSample] {
        match self.scheduler() {
            SchedulerObservation::Sample(sample) => sample.models.as_deref().unwrap_or_default(),
            _ => &[],
        }
    }

    fn tasks(&self) -> &[TaskLeaseSample] {
        match self.scheduler() {
            SchedulerObservation::Sample(sample) => sample.tasks.as_deref().unwrap_or_default(),
            _ => &[],
        }
    }

    fn task(&self, id: TaskId) -> Option<&TaskLeaseSample> {
        self.tasks().iter().find(|task| task.task_id == id)
    }
}

#[derive(Resource)]
struct RuntimeMeshes {
    provider: Handle<Mesh>,
    model: Handle<Mesh>,
    task: Handle<Mesh>,
    memory: Handle<Mesh>,
    material: [Handle<StandardMaterial>; 6],
}

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
        .init_resource::<SelectedGpu>()
        .init_resource::<RuntimeGraph>()
        .insert_resource(Focus {
            point: None,
            home_point: Vec3::new(1.6, 0.0, 0.0),
            home_distance: 16.0,
            reduced_motion,
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                sync_runtime,
                navigate,
                camera::<{ Layer::Machine as usize }>,
                draw_runtime,
                present,
            )
                .chain()
                .run_if(|layers: Res<Layers>| layers.active == Layer::Machine),
        );
}

fn observe(send: &watch::Sender<Option<Sample>>, source: Option<&Path>) {
    let mut system = System::new();
    let gpus = Probe::open();
    let mut have_baseline = false;
    while !send.is_closed() {
        system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        system.refresh_cpu_specifics(CpuRefreshKind::nothing().with_cpu_usage());
        let total = system.total_memory();
        let available = system.available_memory();
        let cpu = system.global_cpu_usage();
        let gpus = gpus.sample();
        let sample = Sample {
            observed: Instant::now(),
            threads: std::thread::available_parallelism().ok().map(usize::from),
            cpu: (have_baseline && !system.cpus().is_empty() && (0.0..=100.0).contains(&cpu))
                .then_some(cpu),
            memory: (total > 0 && available <= total).then_some((total, available)),
            gpus,
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
    commands.insert_resource(RuntimeMeshes {
        provider: meshes.add(Sphere::new(0.40)),
        model: meshes.add(Cuboid::new(0.62, 0.48, 0.62)),
        task: meshes.add(Capsule3d::new(0.23, 0.28)),
        memory: meshes.add(Cylinder::new(0.24, 0.25)),
        material: [
            Color::srgb(0.56, 0.40, 0.88),
            Color::srgb(0.26, 0.69, 0.55),
            Color::srgb(0.20, 0.82, 0.98),
            Color::srgb(0.98, 0.64, 0.24),
            Color::srgb(0.95, 0.89, 0.48),
            Color::srgb(0.44, 0.93, 0.75),
        ]
        .map(|base_color| {
            materials.add(StandardMaterial {
                base_color,
                unlit: true,
                ..default()
            })
        }),
    });
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
                        selected.0 = Target::Device(device);
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
            width: px(400.0),
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

fn sync_runtime(
    mut commands: Commands,
    telemetry: Res<Telemetry>,
    layers: Res<Layers>,
    meshes: Res<RuntimeMeshes>,
    mut graph: ResMut<RuntimeGraph>,
    mut previous: Local<Option<Instant>>,
) {
    let source = telemetry.0.borrow();
    let sample = source
        .as_ref()
        .filter(|sample| sample.observed.elapsed() < INTERVAL * 3);
    let observed = sample.map(|sample| sample.observed);
    if *previous == observed {
        return;
    }
    *previous = observed;
    graph.sample = sample.cloned();
    let mut models: Vec<_> = graph.models().iter().collect();
    models.sort_unstable_by_key(|model| model.model_id);
    let providers = graph
        .models()
        .iter()
        .map(|model| model.provider_id)
        .collect::<BTreeSet<_>>();
    let position = |index: usize, z: f32| {
        Vec3::new(
            (index % 6) as f32 * 1.65 - 3.8,
            0.56,
            z - (index / 6) as f32,
        )
    };
    let mut nodes = BTreeMap::new();
    for (index, id) in providers.into_iter().enumerate() {
        nodes.insert(
            RuntimeNode::Provider(id),
            (position(index, -5.2), &meshes.provider, 0),
        );
    }
    for (index, model) in models.into_iter().enumerate() {
        let material = match model.state {
            ReservationState::Loading | ReservationState::Unloading | ReservationState::Unknown => {
                3
            }
            _ if model.issued_leases > 0 => 2,
            _ => 1,
        };
        nodes.insert(
            RuntimeNode::Model(model.model_id),
            (position(index, -8.6), &meshes.model, material),
        );
    }
    for (index, task) in graph.tasks().iter().enumerate() {
        let point = position(index, -12.0);
        nodes.insert(RuntimeNode::Task(task.task_id), (point, &meshes.task, 4));
        if task.memory_context_records.is_some_and(|count| count > 0) {
            nodes.insert(
                RuntimeNode::Memory(task.task_id),
                (point + Vec3::new(0.85, 0.26, -0.45), &meshes.memory, 5),
            );
        }
    }
    graph.nodes.retain(|node, (entity, _)| {
        if nodes.contains_key(node) {
            return true;
        }
        commands.entity(*entity).despawn();
        false
    });
    for (node, (point, mesh, material)) in nodes {
        let material = MeshMaterial3d(meshes.material[material].clone());
        if let Some((entity, prior)) = graph.nodes.get_mut(&node) {
            commands
                .entity(*entity)
                .insert((Transform::from_translation(point), material));
            *prior = point;
        } else {
            let entity = commands
                .spawn((
                    layers.parent(Layer::Machine),
                    Mesh3d(mesh.clone()),
                    material,
                    Transform::from_translation(point),
                ))
                .observe(
                    move |click: On<PointerClick>, mut selected: ResMut<Selected>| {
                        if click.button == PointerButton::Primary {
                            selected.0 = Target::Runtime(node);
                        }
                    },
                )
                .id();
            graph.nodes.insert(node, (entity, point));
        }
    }
}

fn navigate(
    keys: Res<ButtonInput<KeyCode>>,
    mut selected: ResMut<Selected>,
    mut selected_gpu: ResMut<SelectedGpu>,
    mut focus: ResMut<Focus>,
    graph: Res<RuntimeGraph>,
    layers: Res<Layers>,
) {
    if let Some(agent) = layers.incoming {
        let mut matches = graph.tasks().iter().filter(|task| task.agent_id == agent);
        if let Some(task) = matches.next()
            && matches.next().is_none()
        {
            selected.0 = Target::Runtime(RuntimeNode::Task(task.task_id));
        }
    }
    for (key, device) in [
        (KeyCode::Digit1, Device::Cpu),
        (KeyCode::Digit2, Device::Memory),
        (KeyCode::Digit3, Device::Adapter),
        (KeyCode::Digit4, Device::Scheduler),
    ] {
        if keys.just_pressed(key) {
            selected.0 = Target::Device(device);
        }
    }
    for key in [
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
    ] {
        if keys.just_pressed(key) {
            let matches_kind = |node| {
                matches!(
                    (key, node),
                    (KeyCode::Digit5, RuntimeNode::Model(_))
                        | (KeyCode::Digit6, RuntimeNode::Provider(_))
                        | (KeyCode::Digit7, RuntimeNode::Task(_))
                        | (KeyCode::Digit8, RuntimeNode::Memory(_))
                )
            };
            let mut nodes = graph
                .nodes
                .keys()
                .copied()
                .filter(|node| matches_kind(*node));
            let mut cycle = nodes.clone();
            if let Some(node) = cycle
                .find(|node| match selected.0 {
                    Target::Runtime(prior) if matches_kind(prior) => *node > prior,
                    _ => true,
                })
                .or_else(|| nodes.next())
            {
                selected.0 = Target::Runtime(node);
            }
        }
    }
    if matches!(selected.0, Target::Device(Device::Adapter))
        && keys.just_pressed(KeyCode::KeyG)
        && let Some(bus) = graph
            .sample
            .as_ref()
            .and_then(|sample| sample.gpus.next(&selected_gpu.0))
    {
        selected_gpu.0.clone_from(bus);
    }
    if keys.just_pressed(KeyCode::KeyF) {
        focus.point = match selected.0 {
            Target::Device(device) => Some(device.position()),
            Target::Runtime(node) => graph.nodes.get(&node).map(|(_, point)| *point),
        };
    }
}

fn draw_runtime(graph: Res<RuntimeGraph>, selected: Res<Selected>, mut gizmos: Gizmos) {
    let accounting = Color::srgb(0.35, 0.33, 0.53);
    for (node, (_, point)) in &graph.nodes {
        if matches!(node, RuntimeNode::Provider(_)) {
            gizmos.line(
                Device::Scheduler.position() + Vec3::Y * 0.58,
                *point,
                accounting,
            );
        }
    }
    for model in graph.models() {
        let (_, from) = graph.nodes[&RuntimeNode::Provider(model.provider_id)];
        let (_, to) = graph.nodes[&RuntimeNode::Model(model.model_id)];
        let color = if model.issued_leases > 0 {
            Color::srgb(0.20, 0.82, 0.98)
        } else {
            accounting
        };
        gizmos.line(from, to, color);
    }
    for task in graph.tasks() {
        let (_, from) = graph.nodes[&RuntimeNode::Model(task.model_id)];
        let (_, to) = graph.nodes[&RuntimeNode::Task(task.task_id)];
        gizmos.line(from, to, Color::srgb(0.95, 0.89, 0.48));
        if let Some((_, memory)) = graph.nodes.get(&RuntimeNode::Memory(task.task_id)) {
            gizmos.line(*memory, to, Color::srgb(0.44, 0.93, 0.75));
        }
    }
    let outline = match selected.0 {
        Target::Device(device) => Some((
            device.position() + Vec3::Y * 0.13,
            Vec3::new(3.02, 0.30, 2.55),
        )),
        Target::Runtime(node) => graph
            .nodes
            .get(&node)
            .map(|(_, point)| (*point, Vec3::splat(0.91))),
    };
    if let Some((point, scale)) = outline {
        gizmos.cube(
            Transform::from_translation(point).with_scale(scale),
            Color::srgb(0.88, 0.94, 1.0),
        );
    }
}

fn present(
    graph: Res<RuntimeGraph>,
    selected: Res<Selected>,
    selected_gpu: Res<SelectedGpu>,
    adapter: Option<Res<RenderAdapterInfo>>,
    mut readout: Single<&mut Text, With<Readout>>,
    mut meters: Query<(&Meter, &mut Transform, &mut Visibility)>,
    mut previous: Local<Option<Instant>>,
) {
    let sample = graph.sample.as_ref();
    let observed = sample.map(|sample| sample.observed);
    if *previous == observed
        && !selected.is_changed()
        && !selected_gpu.is_changed()
        && !adapter.as_ref().is_some_and(|adapter| adapter.is_changed())
    {
        return;
    }
    *previous = observed;
    let cpu = sample.and_then(|sample| sample.cpu);
    let memory = sample.and_then(|sample| sample.memory);
    let scheduler = graph.scheduler();
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
        Target::Device(Device::Cpu) => {
            let threads = sample
                .and_then(|sample| sample.threads)
                .map_or_else(|| "Unavailable".into(), |count| count.to_string());
            let utilization =
                cpu.map_or_else(|| "Unavailable".into(), |value| format!("{value:.1}%"));
            format!(
                "CPU  /  LOCAL HOST\n\nProcess-available logical threads  {threads}\nHost CPU utilization  {utilization}"
            )
        }
        Target::Device(Device::Memory) => {
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
        Target::Device(Device::Adapter) => {
            let identity = adapter.map_or_else(
                || "Adapter unavailable".into(),
                |adapter| {
                    format!(
                        "{}\n{:?}  /  {:?}",
                        adapter.name, adapter.device_type, adapter.backend
                    )
                },
            );
            let devices = gpu_detail(sample.map(|sample| &sample.gpus), &selected_gpu.0);
            format!("WGPU RENDER ADAPTER\n\n{identity}\n\n{devices}")
        }
        Target::Device(Device::Scheduler) => scheduler_detail(scheduler),
        Target::Runtime(node) => runtime_detail(node, &graph),
    };
    let age = sample.map_or(
        "Host measurements unavailable",
        |_| "Host measurements / live",
    );
    let text = format!(
        "VESPERA  /  THE MACHINE\n{age}\n\n{detail}\n\n1 CPU | 2 RAM | 3 Adapter\n4 Scheduler | 5 Models | 6 Providers\n7 Tasks | 8 Request memory\nClick to select | F focus | R reset\nWASD pan | Scroll zoom | Right drag"
    );
    if readout.0 != text {
        readout.0 = text;
    }
}

fn gpu_detail(gpus: Option<&Gpus>, selected_bus: &str) -> String {
    let Some(gpus) = gpus else {
        return "Local NVIDIA device measurements unavailable".into();
    };
    let Some(reported) = gpus.reported else {
        return "Local NVIDIA device measurements unavailable".into();
    };
    let Some((bus, gpu)) = gpus.select(selected_bus) else {
        return format!(
            "NVML reports {reported} NVIDIA devices\nNo accessible PCI device measurements"
        );
    };
    let name = gpu.name.as_deref().unwrap_or("Device name unavailable");
    let memory = gpu.memory.map_or_else(
        || "Device VRAM unavailable".into(),
        |memory| {
            const MIB: f64 = 1024.0 * 1024.0;
            format!(
                "Used {:.0} / {:.0} MiB\nFree {:.0} MiB",
                memory.used as f64 / MIB,
                memory.total as f64 / MIB,
                memory.free as f64 / MIB
            )
        },
    );
    format!(
        "NVML  /  LOCAL NVIDIA DEVICES\n{} accessible of {reported} reported\n{name}\nPCI {bus}\n{memory}\nDevice-wide readings\nG cycle accessible NVIDIA devices",
        gpus.devices.len(),
    )
}

fn scheduler_detail(observation: &SchedulerObservation) -> String {
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
            let detail = sample.models.as_ref().map_or_else(
                || "This sample has aggregate counts only".into(),
                |models| {
                    format!(
                        "Showing {} of {} model reservations",
                        models.len(),
                        sample.model_reservations
                    )
                },
            );
            format!(
                "{state}\nQueue {queue} | waiting {}\nAdmitted {} / {}\nAccounted inference slots {}\n\nEstimated reservations / budget\nRAM {} / {} bytes\nVRAM {} / {} bytes\nModels {} | uncertain {}\n{detail}\n\nCached provider health\nHealthy {} | degraded {}\nUnreachable {} | stale {}\nUnknown {}",
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

fn runtime_detail(node: RuntimeNode, graph: &RuntimeGraph) -> String {
    let status = match graph.scheduler() {
        SchedulerObservation::Sample(sample) if sample.finished => "Final snapshot",
        SchedulerObservation::Sample(_) => "Recent sample",
        SchedulerObservation::Stale => "Last scheduler sample expired",
        _ => "Scheduler sample unavailable",
    };
    let detail = match node {
        RuntimeNode::Model(id) => {
            if let Some(model) = graph.models().iter().find(|model| model.model_id == id) {
                let state = match model.state {
                    ReservationState::Loading => "Loading",
                    ReservationState::Loaded => "Load acknowledged",
                    ReservationState::ProviderManaged => "Provider managed",
                    ReservationState::Unloading => "Unloading",
                    ReservationState::Unknown => "Residency uncertain",
                };
                format!(
                    "MODEL RESERVATION\n{id}\n\nProvider\n{}\n\n{state}\nIssued local leases {}\nCharged slots {}\n\nEstimated reservation\nRAM {} bytes\nVRAM {} bytes",
                    model.provider_id,
                    model.issued_leases,
                    model.charged_slots,
                    model.ram_bytes,
                    model.vram_bytes,
                )
            } else {
                format!("MODEL RESERVATION\n{id}\n\nOutside the displayed sample")
            }
        }
        RuntimeNode::Provider(id) => {
            let mut count = 0;
            let mut leases = 0_u64;
            for model in graph
                .models()
                .iter()
                .filter(|model| model.provider_id == id)
            {
                count += 1;
                leases += u64::from(model.issued_leases);
            }
            format!(
                "PROVIDER ACCOUNTING\n{id}\n\nDisplayed reservations {count}\nIssued local leases {leases}"
            )
        }
        RuntimeNode::Task(id) => {
            if let Some(task) = graph.task(id) {
                let memory = task.memory_context_records.map_or_else(
                    || "Request memory unavailable".into(),
                    |count| format!("Memory records included {count}"),
                );
                format!(
                    "LOCAL TASK LEASE\n{id}\n\nAgent\n{}\n\nLease\n{}\n\nModel\n{}\n\nProvider\n{}\n\n{memory}",
                    task.agent_id, task.lease_id, task.model_id, task.provider_id,
                )
            } else {
                format!("LOCAL TASK LEASE\n{id}\n\nOutside the displayed sample")
            }
        }
        RuntimeNode::Memory(id) => {
            let title = format!("REQUEST MEMORY\n\nTask\n{id}");
            if let Some(task) = graph.task(id)
                && let Some(count @ 1..) = task.memory_context_records
            {
                format!(
                    "{title}\n\nAgent\n{}\n\nRetrieved records included {count}\nLocal request assembly",
                    task.agent_id,
                )
            } else {
                format!("{title}\n\nOutside the displayed sample")
            }
        }
    };
    format!("{status}\n\n{detail}")
}

#[cfg(test)]
mod gpu_readout_tests {
    use super::*;

    #[test]
    fn missing_nvml_and_missing_memory_are_labeled_without_a_value() {
        assert_eq!(
            gpu_detail(Some(&Gpus::default()), ""),
            "Local NVIDIA device measurements unavailable"
        );
        let gpus = Gpus {
            reported: Some(1),
            devices: [(
                "00000000:01:00.0".into(),
                crate::gpu_telemetry::Gpu {
                    name: Some("Fixture device".into()),
                    memory: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        let detail = gpu_detail(Some(&gpus), "");
        assert!(detail.contains("Device VRAM unavailable"));
        assert!(!detail.contains("MiB"));
    }
}
