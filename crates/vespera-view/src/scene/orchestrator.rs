use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::prelude::*;
use vespera_core::{AdmissionDecision, TaskState};

use crate::machine_scene::{SchedulerObservation, Telemetry};

use super::{Connection, Focus, Layer, Layers, NodeKey, Scene, Selection};

pub(super) const CENTER: Vec3 = Vec3::new(0.0, 1.88, -0.95);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Signal {
    #[default]
    Offline,
    Idle,
    Queued,
    Running,
    Attention,
}

impl Signal {
    fn label(self) -> &'static str {
        match self {
            Self::Offline => "Awaiting observations",
            Self::Idle => "No active work observed",
            Self::Queued => "Waiting work observed",
            Self::Running => "Active work observed",
            Self::Attention => "Failure or declined admission observed",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Scheduler {
    #[default]
    Unconfigured,
    Unavailable,
    Stale,
    Replay,
    Observed {
        waiting: usize,
        admitted: usize,
        finished: bool,
        needs_attention: bool,
    },
}

impl Scheduler {
    fn observe(telemetry: Option<&Telemetry>, replay: bool) -> Self {
        if replay {
            return Self::Replay;
        }
        match telemetry.map(Telemetry::scheduler_observation) {
            None | Some(SchedulerObservation::NoSource) => Self::Unconfigured,
            Some(SchedulerObservation::Unavailable) => Self::Unavailable,
            Some(SchedulerObservation::Stale) => Self::Stale,
            Some(SchedulerObservation::Sample(sample)) => Self::Observed {
                waiting: sample.waiting,
                admitted: sample.admitted,
                finished: sample.finished,
                needs_attention: sample.last_admission.is_some_and(|last| {
                    !matches!(
                        last.decision,
                        AdmissionDecision::Admitted { .. } | AdmissionDecision::Cancelled
                    )
                }),
            },
        }
    }

    fn label(self) -> String {
        match self {
            Self::Unconfigured => "Scheduler telemetry not configured".into(),
            Self::Unavailable => "Scheduler snapshot unavailable".into(),
            Self::Stale => "Scheduler snapshot stale".into(),
            Self::Replay => "Live scheduler omitted during replay".into(),
            Self::Observed {
                waiting,
                admitted,
                finished,
                needs_attention,
            } => format!(
                "Scheduler snapshot{}\n{waiting} waiting | {admitted} admitted{}",
                if finished { " (finished)" } else { "" },
                if needs_attention {
                    "\nLast admission declined"
                } else {
                    ""
                },
            ),
        }
    }
}

#[derive(Resource, Default, PartialEq, Eq)]
pub(super) struct Evidence {
    pub(super) broker_known: bool,
    queued: usize,
    running: usize,
    latest_failed: bool,
    scheduler: Scheduler,
}

impl Evidence {
    fn signal(&self) -> Signal {
        let (waiting, admitted, alert) = match self.scheduler {
            Scheduler::Observed {
                waiting,
                admitted,
                finished: false,
                needs_attention,
            } => (waiting, admitted, needs_attention),
            _ => (0, 0, false),
        };
        if self.latest_failed || alert {
            Signal::Attention
        } else if self.running > 0 || admitted > 0 {
            Signal::Running
        } else if self.queued > 0 || waiting > 0 {
            Signal::Queued
        } else if self.broker_known
            || matches!(
                self.scheduler,
                Scheduler::Observed {
                    finished: false,
                    ..
                }
            )
        {
            Signal::Idle
        } else {
            Signal::Offline
        }
    }

    pub(super) fn detail(&self, connection: &Connection) -> String {
        let broker = if self.broker_known {
            format!(
                "{} queued | {} running\nObserved broker tasks",
                self.queued, self.running
            )
        } else {
            "Broker activity unavailable".into()
        };
        format!(
            "CENTRAL ORCHESTRATOR\n{}\n{broker}\n{}\n\n{}",
            self.signal().label(),
            connection.timeline.label(connection.session.is_some()),
            self.scheduler.label()
        )
    }
}

#[derive(Resource)]
pub(super) struct Materials([Handle<StandardMaterial>; 5]);

#[derive(Component)]
pub(super) struct Core;

#[derive(Component)]
pub(super) struct Rotor(f32);

#[derive(Component)]
pub(super) struct SelectionRing;

fn material(materials: &mut Assets<StandardMaterial>, color: Color) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color,
        unlit: true,
        ..default()
    })
}

pub(super) fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    layers: &Layers,
) {
    let signal = [
        Color::srgb(0.27, 0.33, 0.48),
        Color::srgb(0.55, 0.39, 0.88),
        Color::srgb(1.0, 0.65, 0.26),
        Color::srgb(0.23, 0.88, 1.0),
        Color::srgb(1.0, 0.32, 0.48),
    ]
    .map(|color| {
        let linear = color.to_linear();
        materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::new(
                linear.red * 0.38,
                linear.green * 0.38,
                linear.blue * 0.38,
                1.0,
            ),
            perceptual_roughness: 0.52,
            ..default()
        })
    });
    let housing = material(materials, Color::srgb(0.13, 0.17, 0.29));
    let trim = material(materials, Color::srgb(0.34, 0.29, 0.61));
    let accent = material(materials, Color::srgb(0.57, 0.77, 0.98));
    let root = commands
        .spawn((
            layers.parent(Layer::Network),
            Transform::from_translation(CENTER).with_scale(Vec3::splat(1.2)),
            Visibility::Inherited,
        ))
        .observe(
            |event: On<PointerClick>, mut selection: ResMut<Selection>| {
                if event.button == PointerButton::Primary {
                    selection.0 = Some(NodeKey::Orchestrator);
                }
            },
        )
        .id();
    let ring = |inner, outer, sides| {
        Torus::new(inner, outer)
            .mesh()
            .minor_resolution(6)
            .major_resolution(sides)
            .build()
    };
    let shell = meshes.add(ring(1.05, 1.19, 12));
    let spoke = meshes.add(Cuboid::new(0.13, 0.14, 0.41));
    for (orientation, rate) in [
        (Quat::from_rotation_x(0.51), 0.22),
        (Quat::from_rotation_z(1.14), -0.31),
        (
            Quat::from_rotation_z(-0.91) * Quat::from_rotation_x(0.63),
            0.15,
        ),
    ] {
        let axis = commands
            .spawn((
                ChildOf(root),
                Transform::from_rotation(orientation),
                Visibility::Inherited,
            ))
            .id();
        let rotor = commands
            .spawn((
                ChildOf(axis),
                Transform::default(),
                Visibility::Inherited,
                Rotor(rate),
            ))
            .id();
        commands.spawn((
            ChildOf(rotor),
            Mesh3d(shell.clone()),
            MeshMaterial3d(housing.clone()),
            Transform::default(),
        ));
        for index in 0..6 {
            let angle = index as f32 * TAU / 6.0;
            let rotation = Quat::from_rotation_y(angle);
            commands.spawn((
                ChildOf(rotor),
                Mesh3d(spoke.clone()),
                MeshMaterial3d(trim.clone()),
                Transform::from_translation(rotation * Vec3::Z * 1.14).with_rotation(rotation),
            ));
        }
    }
    let mut core = Sphere::new(0.54).mesh().ico(0).expect("valid core mesh");
    core.duplicate_vertices();
    core.compute_flat_normals();
    commands.spawn((
        ChildOf(root),
        Core,
        Mesh3d(meshes.add(core)),
        MeshMaterial3d(signal[0].clone()),
        Transform::from_rotation(Quat::from_rotation_z(FRAC_PI_2 * 0.3)),
    ));
    commands.spawn((
        ChildOf(root),
        PointLight {
            color: Color::srgb(0.73, 0.85, 1.0),
            intensity: 230.0,
            range: 5.0,
            ..default()
        },
        Transform::from_xyz(0.8, 1.9, 1.7),
    ));
    commands.spawn((
        ChildOf(root),
        Mesh3d(meshes.add(ring(1.23, 1.28, 36))),
        MeshMaterial3d(trim),
        Transform::from_xyz(0.0, -1.43, 0.0),
        Pickable::IGNORE,
    ));
    commands.spawn((
        ChildOf(root),
        SelectionRing,
        Mesh3d(meshes.add(ring(1.35, 1.38, 36))),
        MeshMaterial3d(accent),
        Transform::from_xyz(0.0, -1.40, 0.0),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
    commands.insert_resource(Materials(signal));
}

pub(super) fn project(
    scene: Res<Scene>,
    connection: Res<Connection>,
    telemetry: Option<Res<Telemetry>>,
    mut evidence: ResMut<Evidence>,
) {
    let replay = connection.timeline.is_frozen();
    let broker_known = if replay {
        connection.timeline.has_replay_source()
    } else {
        connection.session.is_some()
    };
    let mut next = Evidence {
        broker_known,
        scheduler: Scheduler::observe(telemetry.as_deref(), replay),
        ..default()
    };
    if broker_known {
        for task in scene.graph.tasks.values() {
            next.queued += usize::from(task.state == TaskState::Queued);
            next.running += usize::from(task.state == TaskState::Running);
        }
        next.latest_failed = scene
            .recent
            .back()
            .and_then(|(_, id, _)| scene.graph.tasks.get(id))
            .is_some_and(|task| task.state == TaskState::Failed);
    }
    if *evidence != next {
        *evidence = next;
    }
}

pub(super) fn render(
    (connection, focus): (Res<Connection>, Res<Focus>),
    (evidence, materials): (Res<Evidence>, Res<Materials>),
    selection: Res<Selection>,
    time: Res<Time>,
    mut core: Single<&mut MeshMaterial3d<StandardMaterial>, With<Core>>,
    mut highlight: Single<&mut Visibility, With<SelectionRing>>,
    mut rotors: Query<(&Rotor, &mut Transform)>,
) {
    let signal = evidence.signal();
    let material = &materials.0[signal as usize];
    if core.0 != *material {
        core.0 = material.clone();
    }
    let visible = if selection.0 == Some(NodeKey::Orchestrator) {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if **highlight != visible {
        **highlight = visible;
    }
    if signal == Signal::Running && !connection.timeline.is_frozen() && !focus.reduced_motion {
        for (rotor, mut pose) in &mut rotors {
            pose.rotate_y(rotor.0 * time.delta_secs());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_scheduler_or_projected_broker_work_drives_activity() {
        let mut evidence = Evidence::default();
        assert!(evidence.signal() == Signal::Offline);
        evidence.broker_known = true;
        assert!(evidence.signal() == Signal::Idle);
        evidence.queued = 1;
        assert!(evidence.signal() == Signal::Queued);
        evidence.running = 1;
        assert!(evidence.signal() == Signal::Running);
        evidence.latest_failed = true;
        assert!(evidence.signal() == Signal::Attention);
        evidence = Evidence {
            scheduler: Scheduler::Observed {
                waiting: 1,
                admitted: 2,
                finished: true,
                needs_attention: true,
            },
            ..default()
        };
        assert!(evidence.signal() == Signal::Offline);
        if let Scheduler::Observed {
            finished,
            needs_attention,
            ..
        } = &mut evidence.scheduler
        {
            *finished = false;
            *needs_attention = false;
        }
        assert!(evidence.signal() == Signal::Running);
        assert!(Scheduler::observe(None, true) == Scheduler::Replay);
    }
}
