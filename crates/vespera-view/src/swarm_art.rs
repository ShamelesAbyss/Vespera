use bevy::prelude::*;
use vespera_core::AgentId;

pub(super) const HAZE: Color = Color::srgb(0.030, 0.045, 0.075);

#[derive(Resource)]
pub(super) struct Foundry {
    block: Handle<Mesh>,
    drum: Handle<Mesh>,
    obsidian: Handle<StandardMaterial>,
    alloy: Handle<StandardMaterial>,
    trim: Handle<StandardMaterial>,
    sigils: [Handle<StandardMaterial>; 6],
    fault: Handle<StandardMaterial>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SignalColor {
    Cyan,
    Amber,
    Green,
    Red,
    Violet,
    Pink,
}

impl Foundry {
    pub(super) fn signal_material(&self, color: SignalColor) -> Handle<StandardMaterial> {
        match color {
            SignalColor::Cyan => &self.sigils[0],
            SignalColor::Amber => &self.sigils[1],
            SignalColor::Violet => &self.sigils[2],
            SignalColor::Green => &self.sigils[3],
            SignalColor::Pink => &self.sigils[4],
            SignalColor::Red => &self.fault,
        }
        .clone()
    }

    pub(super) fn signal_mesh(&self) -> Mesh3d {
        Mesh3d(self.block.clone())
    }

    pub(super) fn route(
        &self,
        commands: &mut Commands,
        parent: Entity,
        from: Vec3,
        to: Vec3,
        lane: usize,
    ) {
        let marker = &self.sigils[[0, 3, 1, 4][lane]];
        let y = 0.18 + lane as f32 * 0.13;
        if from == to {
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                let turn = Quat::from_rotation_y(angle);
                self.part(
                    commands,
                    parent,
                    marker,
                    Transform::from_translation(from + Vec3::Y * y + turn * Vec3::X * 0.89)
                        .with_rotation(turn)
                        .with_scale(Vec3::new(0.08, 0.06, 0.58)),
                    false,
                    false,
                );
            }
            return;
        }
        let direction = (to - from).normalize();
        let side = direction.cross(Vec3::Y);
        let start = from + direction * 0.80 + Vec3::Y * y + side * 0.17;
        let end = to - direction * 0.80 + Vec3::Y * y + side * 0.17;
        let rotation = Quat::from_rotation_arc(Vec3::Z, direction);
        let length = start.distance(end);
        let center = (start + end) * 0.5;
        for (height, width, thickness, material) in [
            (0.0, 0.28, 0.13, &self.obsidian),
            (0.076, 0.056, 0.03, marker),
        ] {
            self.part(
                commands,
                parent,
                material,
                Transform::from_translation(center + Vec3::Y * height)
                    .with_rotation(rotation)
                    .with_scale(Vec3::new(width, thickness, length)),
                false,
                false,
            );
        }
        self.part(
            commands,
            parent,
            marker,
            Transform::from_translation(end).with_scale(Vec3::new(0.26, 0.18, 0.26)),
            true,
            false,
        );
    }

    pub(super) fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let material = |color, metallic, roughness| StandardMaterial {
            base_color: color,
            metallic,
            perceptual_roughness: roughness,
            ..default()
        };
        let sigils = [
            (0.18, 0.78, 0.94),
            (0.99, 0.55, 0.27),
            (0.62, 0.45, 0.99),
            (0.27, 0.90, 0.68),
            (0.96, 0.38, 0.62),
            (0.88, 0.82, 0.46),
        ]
        .map(|(r, g, b)| {
            materials.add(StandardMaterial {
                base_color: Color::srgb(r, g, b),
                emissive: LinearRgba::new(r * 0.5, g * 0.5, b * 0.5, 1.0),
                perceptual_roughness: 0.32,
                ..default()
            })
        });
        Self {
            block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            drum: meshes.add(Cylinder::new(0.5, 1.0).mesh().resolution(8)),
            obsidian: materials.add(material(Color::srgb(0.065, 0.088, 0.13), 0.35, 0.68)),
            alloy: materials.add(material(Color::srgb(0.24, 0.31, 0.40), 0.65, 0.38)),
            trim: materials.add(material(Color::srgb(0.50, 0.31, 0.20), 0.52, 0.42)),
            fault: materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.15, 0.12),
                emissive: LinearRgba::new(0.85, 0.025, 0.02, 1.0),
                ..default()
            }),
            sigils,
        }
    }

    fn part(
        &self,
        commands: &mut Commands,
        parent: Entity,
        material: &Handle<StandardMaterial>,
        transform: Transform,
        faceted: bool,
        pickable: bool,
    ) {
        let mut entity = commands.spawn((
            ChildOf(parent),
            Mesh3d(if faceted {
                self.drum.clone()
            } else {
                self.block.clone()
            }),
            MeshMaterial3d(material.clone()),
            transform,
        ));
        if !pickable {
            entity.insert(Pickable::IGNORE);
        }
    }

    pub(super) fn hall(&self, commands: &mut Commands, parent: Entity) {
        let block = |commands: &mut Commands, material, position, scale| {
            self.part(
                commands,
                parent,
                material,
                Transform::from_translation(position).with_scale(scale),
                false,
                false,
            );
        };
        block(
            commands,
            &self.obsidian,
            Vec3::new(0.0, -0.36, 0.0),
            Vec3::new(34.0, 0.7, 25.0),
        );
        for row in -4..=4 {
            for column in -6..=6 {
                let surface = if (-1..=1).contains(&row) || (row + column) % 2 == 0 {
                    &self.alloy
                } else {
                    &self.obsidian
                };
                block(
                    commands,
                    surface,
                    Vec3::new(column as f32 * 2.5, 0.0, row as f32 * 2.5),
                    Vec3::new(2.46, 0.035, 2.46),
                );
            }
        }
        for side in [-1.0, 1.0] {
            for index in -6..=6 {
                let x = index as f32 * 2.5;
                for inset in [0.0, 0.18] {
                    block(
                        commands,
                        &self.trim,
                        Vec3::new(x, 0.038, side * (3.80 + inset)),
                        Vec3::new(2.32, 0.035, 0.045),
                    );
                }
                block(
                    commands,
                    &self.obsidian,
                    Vec3::new(x, 0.09, side * 4.08),
                    Vec3::new(0.75, 0.15, 0.30),
                );
                for fin in [-0.22, 0.0, 0.22] {
                    block(
                        commands,
                        &self.alloy,
                        Vec3::new(x + fin, 0.18, side * 4.08),
                        Vec3::new(0.075, 0.05, 0.25),
                    );
                }
            }
            let z = side * 10.7;
            block(
                commands,
                &self.obsidian,
                Vec3::new(0.0, 0.4, z),
                Vec3::new(32.0, 0.8, 1.0),
            );
            block(
                commands,
                &self.trim,
                Vec3::new(0.0, 0.86, z),
                Vec3::new(32.0, 0.10, 0.16),
            );
            if side > 0.0 {
                continue;
            }
            block(
                commands,
                &self.alloy,
                Vec3::new(0.0, 4.45, z),
                Vec3::new(31.0, 0.26, 0.72),
            );
            block(
                commands,
                &self.trim,
                Vec3::new(0.0, 4.18, z + 0.48),
                Vec3::new(31.0, 0.10, 0.15),
            );
            for index in -3..=3 {
                let x = index as f32 * 4.6;
                block(
                    commands,
                    &self.obsidian,
                    Vec3::new(x, 2.25, z),
                    Vec3::new(1.05, 4.5, 1.08),
                );
                block(
                    commands,
                    &self.alloy,
                    Vec3::new(x, 4.40, z),
                    Vec3::new(1.6, 0.42, 1.8),
                );
                for fin in [-0.32, 0.0, 0.32] {
                    block(
                        commands,
                        &self.trim,
                        Vec3::new(x + fin, 2.38, z - side * 0.57),
                        Vec3::new(0.10, 3.3, 0.10),
                    );
                }
                block(
                    commands,
                    &self.sigils[0],
                    Vec3::new(x, 0.42, z - side * 0.57),
                    Vec3::new(0.46, 0.08, 0.12),
                );
            }
        }
    }

    pub(super) fn agent(&self, commands: &mut Commands, parent: Entity, id: AgentId) {
        // Identity derives from the agent UUID and survives a model change or replay.
        let signature = id.to_string().bytes().fold(0u32, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u32::from(byte))
        });
        let sigil = &self.sigils[(signature % 6) as usize];
        let wings = 3 + (signature / 6 % 3) as usize;
        let height = 1.30 + (signature / 18 % 4) as f32 * 0.17;
        let stem_base = height + 0.20;
        self.part(
            commands,
            parent,
            &self.trim,
            Transform::from_xyz(0.0, (stem_base + 2.33) * 0.5, 0.0).with_scale(Vec3::new(
                0.08,
                2.33 - stem_base,
                0.09,
            )),
            false,
            true,
        );
        for (y, radius, thickness, material) in [
            (0.19, 1.53, 0.30, &self.obsidian),
            (0.36, 1.28, 0.10, &self.trim),
            (0.47, 1.10, 0.12, &self.alloy),
            (0.60, 0.78, 0.16, &self.obsidian),
            (1.0, 0.46, 0.66, sigil),
            (height + 0.16, 0.58, 0.15, &self.trim),
        ] {
            self.part(
                commands,
                parent,
                material,
                Transform::from_xyz(0.0, y, 0.0).with_scale(Vec3::new(radius, thickness, radius)),
                true,
                true,
            );
        }
        for wing in 0..wings {
            let turn = Quat::from_rotation_y(wing as f32 * std::f32::consts::TAU / wings as f32);
            let part = |commands: &mut Commands, material, x, y, scale| {
                self.part(
                    commands,
                    parent,
                    material,
                    Transform::from_translation(turn * Vec3::new(x, y, 0.0))
                        .with_rotation(turn)
                        .with_scale(scale),
                    false,
                    true,
                );
            };
            part(
                commands,
                &self.alloy,
                0.55,
                height * 0.64,
                Vec3::new(0.24, height * 0.76, 0.29),
            );
            part(
                commands,
                &self.obsidian,
                0.70,
                0.78,
                Vec3::new(0.21, 0.53, 0.39),
            );
            part(commands, sigil, 0.82, 0.79, Vec3::new(0.045, 0.33, 0.19));
            part(
                commands,
                &self.trim,
                0.42,
                height + 0.05,
                Vec3::new(0.39, 0.17, 0.22),
            );
        }
    }
}
