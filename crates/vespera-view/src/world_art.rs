//! Presentation-only art for Living World. No geometry here changes the simulation grid.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use bevy::asset::{RenderAssetUsages, io::embedded::EmbeddedAssetRegistry};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::world_serialization::WorldInstanceReady;
use vespera_world::{Position, Snapshot};

pub(super) const SKY: Color = Color::srgb(0.22, 0.34, 0.50);

macro_rules! embed {
    ($registry:expr, $folder:literal, $($file:literal),+ $(,)?) => {
        $(
            let path = Path::new(concat!("living-world/", $folder, "/", $file));
            $registry.insert_asset(
                PathBuf::from(concat!("assets/world/", $folder, "/", $file)),
                path,
                include_bytes!(concat!("../../../assets/world/", $folder, "/", $file)),
            );
        )+
    };
}

pub(super) fn embed_assets(app: &App) {
    let registry = app.world().resource::<EmbeddedAssetRegistry>();
    embed!(
        registry,
        "space-base",
        "spacebits_texture.png",
        "cargodepot_A.gltf",
        "cargodepot_A.bin",
        "basemodule_B.gltf",
        "basemodule_B.bin",
        "drill_structure.gltf",
        "drill_structure.bin",
        "cargo_A.gltf",
        "cargo_A.bin",
        "rocks_A.gltf",
        "rocks_A.bin",
        "solarpanel.gltf",
        "solarpanel.bin",
        "lights.gltf",
        "lights.bin",
    );
    embed!(registry, "space-suits", "astronauta.glb", "astronautb.glb");
}

pub(super) fn grid_point(position: Position, state: &Snapshot<'_>) -> Vec2 {
    Vec2::new(
        f32::from(position.x) - f32::from(state.width - 1) * 0.5,
        f32::from(position.z) - f32::from(state.depth - 1) * 0.5,
    )
}

#[derive(Resource)]
pub(super) struct Landscape {
    seed: u64,
    half: Vec2,
    pub depot: Vec2,
    paths: Vec<[Vec2; 2]>,
    plots: Vec<Vec2>,
}

impl Landscape {
    pub fn new(state: &Snapshot<'_>) -> Self {
        let depot = grid_point(state.depot, state);
        let mut paths = Vec::new();
        let mut sites: Vec<_> = state
            .sites
            .iter()
            .map(|site| grid_point(site.position, state))
            .collect();
        sites.sort_by(|a, b| {
            a.distance_squared(depot)
                .total_cmp(&b.distance_squared(depot))
        });
        let mut connected = vec![depot];
        for end in sites {
            let start = connected
                .iter()
                .copied()
                .min_by(|a, b| a.distance_squared(end).total_cmp(&b.distance_squared(end)))
                .unwrap_or(depot);
            let control = start.lerp(end, 0.5) + (end - start).normalize_or_zero().perp() * 0.55;
            let mut last = start;
            for step in 1..=6 {
                let t = step as f32 / 6.0;
                let next = start.lerp(control, t).lerp(control.lerp(end, t), t);
                paths.push([last, next]);
                last = next;
            }
            connected.push(end);
        }
        let mut terrain = Self {
            seed: state.seed,
            half: Vec2::new(f32::from(state.width), f32::from(state.depth)) * 0.5,
            depot,
            paths,
            plots: Vec::new(),
        };
        // Reserve distinct visual plots away from roads, machinery, and the beach.
        for index in 0..96 {
            let direction = Vec2::from_angle(1.65 + index as f32 * 2.399_963);
            let point = depot + direction * (1.64 + (index as f32).sqrt() * 0.39);
            if terrain.coast(point) < 0.83
                && terrain.path_distance(point) > 0.83
                && terrain
                    .plots
                    .iter()
                    .all(|plot| plot.distance_squared(point) > 1.78)
            {
                terrain.plots.push(point);
                if terrain.plots.len() == 12 {
                    break;
                }
            }
        }
        terrain
    }

    fn coast(&self, point: Vec2) -> f32 {
        let p = point.abs() / self.shore_extent();
        (p.x.powi(8) + p.y.powi(8)).powf(0.125)
            + (point.x * 1.3 + point.y * 0.7).sin() * 0.028
            + (point.y * 1.9 - point.x * 0.3).cos() * 0.022
    }

    fn shore_extent(&self) -> Vec2 {
        self.half * 1.13 + Vec2::splat(2.15)
    }

    fn path_distance(&self, point: Vec2) -> f32 {
        self.paths
            .iter()
            .fold(point.distance_squared(self.depot), |best, &[a, b]| {
                let delta = b - a;
                let along =
                    ((point - a).dot(delta) / delta.length_squared().max(0.001)).clamp(0.0, 1.0);
                best.min(point.distance_squared(a + delta * along))
            })
            .sqrt()
    }

    pub fn height(&self, point: Vec2) -> f32 {
        self.height_at(point, self.path_distance(point), self.coast(point))
    }

    fn height_at(&self, point: Vec2, path_distance: f32, coast: f32) -> f32 {
        let phase = (self.seed & 255) as f32 * 0.21;
        let ridge = (point.x * 0.56 + phase).sin() * (point.y * 0.62).cos() * 0.24
            + (point.x * 1.43 + point.y * 0.81).sin() * 0.08;
        let rise = smooth(0.4, 3.0, path_distance);
        let shelf = smooth(0.80, 0.94, coast);
        let sea = smooth(0.94, 1.06, coast);
        0.22 + (0.30 + ridge) * rise - 0.35 * shelf - 1.55 * sea
    }

    pub fn point(&self, point: Vec2, lift: f32) -> Vec3 {
        Vec3::new(point.x, self.height(point) + lift, point.y)
    }

    fn color(&self, point: Vec2, path_distance: f32, coast: f32) -> [f32; 4] {
        let mottling =
            (point.x * 2.3 + point.y * 0.71).sin() * (point.y * 1.8 - point.x * 0.68).cos();
        let grass = mix(
            [0.085, 0.22, 0.15],
            [0.18, 0.32, 0.155],
            (mottling + 1.0) * 0.5,
        );
        let path = (1.0 - smooth(0.19, 0.43, path_distance + mottling * 0.04))
            .max(1.0 - smooth(0.75, 1.45, point.distance(self.depot)));
        let meadow = mix(grass, [0.49, 0.36, 0.22], path);
        let sand = mix(meadow, [0.52, 0.43, 0.29], smooth(0.84, 0.95, coast));
        let [r, g, b] = mix(sand, [0.068, 0.13, 0.16], smooth(0.95, 1.08, coast));
        [r, g, b, 1.0]
    }

    fn mesh(&self) -> Mesh {
        let extent = self.shore_extent() + Vec2::splat(1.6);
        let segments = (extent * 10.0).as_uvec2().min(UVec2::splat(256));
        let stride = segments.x + 1;
        let mut vertices = Vec::new();
        let mut uv = Vec::new();
        let mut colors = Vec::new();
        let mut indices = Vec::new();
        for z in 0..=segments.y {
            for x in 0..=segments.x {
                let point =
                    (Vec2::new(x as f32, z as f32) / segments.as_vec2() * 2.0 - 1.0) * extent;
                let path = self.path_distance(point);
                let coast = self.coast(point);
                vertices.push([point.x, self.height_at(point, path, coast), point.y]);
                uv.push((point * 0.42).to_array());
                colors.push(self.color(point, path, coast));
                if x < segments.x && z < segments.y {
                    let a = z * stride + x;
                    indices.extend([a, a + stride, a + 1, a + 1, a + stride, a + stride + 1]);
                }
            }
        }
        let mut mesh = colored_mesh(vertices, colors, indices);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        mesh.compute_smooth_normals();
        mesh
    }

    fn sea_mesh(&self) -> Mesh {
        let shore = self.shore_extent();
        let extent = (shore + Vec2::splat(3.4)).max(shore * 1.36);
        let segments = (extent * 13.0).as_uvec2().min(UVec2::splat(192));
        let stride = segments.x + 3;
        let mut vertices = Vec::new();
        let mut colors = Vec::new();
        let mut indices = Vec::new();
        let axis = |index, count, radius| match index {
            0 => -480.0,
            outer if outer == count + 2 => 480.0,
            _ => ((index - 1) as f32 / count as f32 * 2.0 - 1.0) * radius,
        };
        for z in 0..=segments.y + 2 {
            for x in 0..=segments.x + 2 {
                let point = Vec2::new(axis(x, segments.x, extent.x), axis(z, segments.y, extent.y));
                vertices.push([point.x, -0.43, point.y]);
                let coast = self.coast(point);
                let ripple =
                    (point.x * 0.83 + point.y * 2.9).sin() * (point.y * 1.72 - point.x * 0.6).sin();
                let shallow = 1.0 - smooth(1.02, 1.29, coast + ripple * 0.013);
                let water = mix([0.016, 0.065, 0.115], [0.045, 0.30, 0.33], shallow);
                let angle = point.y.atan2(point.x);
                let lip = 1.017 + angle.sin() * 0.017 + ripple * 0.008;
                let foam = (1.0 - smooth(0.002, 0.016, (coast - lip).abs()))
                    * smooth(-0.7, 0.8, ripple)
                    * 0.56;
                let [r, g, b] = mix(water, [0.36, 0.62, 0.58], foam);
                colors.push([r, g, b, 1.0]);
                if x < segments.x + 2 && z < segments.y + 2 {
                    let a = z * stride + x;
                    indices.extend([a, a + stride, a + 1, a + 1, a + stride, a + stride + 1]);
                }
            }
        }
        let normals = vec![[0.0, 1.0, 0.0]; vertices.len()];
        colored_mesh(vertices, colors, indices)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    }

    pub fn shelters(&self, count: u16) -> impl ExactSizeIterator<Item = Vec3> + '_ {
        self.plots
            .iter()
            .take(usize::from(count))
            .map(|&plot| self.point(plot, 0.0))
    }

    fn random(&self, index: u64) -> f32 {
        let mut bits = self
            .seed
            .wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        bits = (bits ^ (bits >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        bits = (bits ^ (bits >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((bits ^ (bits >> 31)) >> 40) as f32 / 16_777_216.0
    }
}

fn mix(a: [f32; 3], b: [f32; 3], weight: f32) -> [f32; 3] {
    Vec3::from_array(a)
        .lerp(Vec3::from_array(b), weight)
        .to_array()
}

fn smooth(min: f32, max: f32, value: f32) -> f32 {
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn colored_mesh(vertices: Vec<[f32; 3]>, colors: Vec<[f32; 4]>, indices: Vec<u32>) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

#[derive(Resource)]
pub(super) struct Models {
    pub depot: Handle<WorldAsset>,
    pub shelter: Handle<WorldAsset>,
    pub drill: Handle<WorldAsset>,
    pub cargo: Handle<WorldAsset>,
    pub suits: [Handle<WorldAsset>; 2],
    rock: Handle<WorldAsset>,
    solar: Handle<WorldAsset>,
    light: Handle<WorldAsset>,
}

#[derive(Resource)]
struct Basalt(Handle<StandardMaterial>);

impl Models {
    pub fn load(assets: &AssetServer) -> Self {
        let load = |name| {
            assets.load(
                GltfAssetLabel::Scene(0)
                    .from_asset(format!("embedded://living-world/space-base/{name}.gltf")),
            )
        };
        Self {
            depot: load("cargodepot_A"),
            shelter: load("basemodule_B"),
            drill: load("drill_structure"),
            cargo: load("cargo_A"),
            suits: ["astronauta", "astronautb"].map(|name| {
                assets.load(
                    GltfAssetLabel::Scene(0)
                        .from_asset(format!("embedded://living-world/space-suits/{name}.glb")),
                )
            }),
            rock: load("rocks_A"),
            solar: load("solarpanel"),
            light: load("lights"),
        }
    }
}

pub(super) fn build(
    commands: &mut Commands,
    parent: Entity,
    landscape: &Landscape,
    models: &Models,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    let ground = materials.add(StandardMaterial {
        perceptual_roughness: 0.97,
        ..default()
    });
    commands.insert_resource(Basalt(materials.add(StandardMaterial {
        base_color: Color::srgb(0.38, 0.43, 0.53),
        perceptual_roughness: 0.94,
        ..default()
    })));
    commands.spawn((
        ChildOf(parent),
        Mesh3d(meshes.add(landscape.mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(images.add(terrain_grain())),
            perceptual_roughness: 0.97,
            ..default()
        })),
        Pickable::IGNORE,
    ));
    commands.spawn((
        ChildOf(parent),
        Mesh3d(meshes.add(landscape.sea_mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            perceptual_roughness: 0.65,
            reflectance: 0.26,
            ..default()
        })),
        NotShadowCaster,
        Pickable::IGNORE,
    ));

    let firs = [
        meshes.add(fir(0.0)),
        meshes.add(fir(0.7)),
        meshes.add(fir(1.3)),
    ];
    let mut planted: Vec<Vec2> = Vec::new();
    let count = ((landscape.half.x * landscape.half.y) as usize * 14).clamp(260, 2200);
    for index in 0..count as u64 {
        let range = landscape.shore_extent();
        let point = (Vec2::new(landscape.random(index * 5), landscape.random(index * 5 + 1)) * 2.0
            - 1.0)
            * range;
        let path_distance = landscape.path_distance(point);
        let coast = landscape.coast(point);
        if path_distance < 1.25
            || coast > 0.91
            || point.distance(landscape.depot) < 2.05
            || landscape
                .plots
                .iter()
                .any(|plot| plot.distance_squared(point) < 0.75)
            || planted
                .iter()
                .any(|other| other.distance_squared(point) < 0.61)
        {
            continue;
        }
        let density = (point.x * 0.88 - point.y * 1.11).sin() * 0.5 + 0.5;
        if coast < 0.64 && landscape.random(index * 5 + 2) > density * 0.28 {
            continue;
        }
        planted.push(point);
        let size = 0.48 + landscape.random(index * 5 + 3) * 0.64;
        let transform = Transform::from_translation(landscape.point(point, -0.025))
            .with_scale(Vec3::splat(size))
            .with_rotation(Quat::from_rotation_y(landscape.random(index * 5 + 4) * TAU));
        if index % 5 == 0 {
            commands
                .spawn((
                    ChildOf(parent),
                    WorldAssetRoot(models.rock.clone()),
                    transform,
                ))
                .observe(tint_rock);
        } else {
            commands.spawn((
                ChildOf(parent),
                Mesh3d(firs[(index % 3) as usize].clone()),
                MeshMaterial3d(ground.clone()),
                transform,
                Pickable::IGNORE,
            ));
        }
    }
    let scrub = meshes.add(meadow_tuft());
    for index in 0..(count as u64).min(920) {
        let offset = 18_000 + index * 4;
        let point = (Vec2::new(landscape.random(offset), landscape.random(offset + 1)) * 2.0 - 1.0)
            * (landscape.half + Vec2::splat(2.1));
        let path = landscape.path_distance(point);
        if !(0.52..2.7).contains(&path)
            || landscape.coast(point) > 0.86
            || landscape
                .plots
                .iter()
                .any(|plot| plot.distance_squared(point) < 0.5)
        {
            continue;
        }
        commands.spawn((
            ChildOf(parent),
            Mesh3d(scrub.clone()),
            MeshMaterial3d(ground.clone()),
            Transform::from_translation(landscape.point(point, -0.018))
                .with_scale(Vec3::splat(0.45 + landscape.random(offset + 2) * 0.67))
                .with_rotation(Quat::from_rotation_y(landscape.random(offset + 3) * TAU)),
            NotShadowCaster,
            Pickable::IGNORE,
        ));
    }
    for (offset, scene, scale) in [
        (Vec2::new(-1.28, -0.94), &models.solar, 0.60),
        (Vec2::new(-1.42, 0.72), &models.light, 0.65),
    ] {
        commands
            .spawn((
                ChildOf(parent),
                WorldAssetRoot(scene.clone()),
                Transform::from_translation(landscape.point(landscape.depot + offset, 0.0))
                    .with_scale(Vec3::splat(scale)),
            ))
            .observe(ignore_decoration);
    }
}

fn terrain_grain() -> Image {
    const SIDE: u32 = 64;
    let mut pixels = Vec::new();
    for y in 0..SIDE {
        for x in 0..SIDE {
            let u = x as f32 * TAU / SIDE as f32;
            let v = y as f32 * TAU / SIDE as f32;
            let soft = (u * 4.0 + (v * 2.0).sin()).sin() * (v * 3.0 - u * 2.0).cos();
            let fine = (u * 15.0 + v * 11.0).sin() * (u * 13.0 - v * 17.0).cos();
            let value = (230.0 + soft * 14.0 + fine * 8.0) as u8;
            pixels.extend([value, value, value, 255]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

fn meadow_tuft() -> Mesh {
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for blade in 0..7 {
        let angle = blade as f32 * 2.399_963;
        let direction = Vec3::new(angle.cos(), 0.0, angle.sin());
        let edge = Vec3::new(-direction.z, 0.0, direction.x) * 0.026;
        let base = positions.len() as u32;
        let tip = direction * (0.13 + blade as f32 * 0.012)
            + Vec3::Y * (0.15 + (blade % 3) as f32 * 0.055);
        positions.extend([(-edge).to_array(), edge.to_array(), tip.to_array()]);
        colors.extend([
            [0.08, 0.22, 0.14, 1.0],
            [0.08, 0.22, 0.14, 1.0],
            [0.37, 0.54, 0.24, 1.0],
        ]);
        indices.extend([base, base + 1, base + 2, base + 1, base, base + 2]);
    }
    let mut mesh = colored_mesh(positions, colors, indices);
    mesh.duplicate_vertices();
    mesh.compute_flat_normals();
    mesh
}

pub(super) fn ignore_decoration(
    event: On<WorldInstanceReady>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    mut commands: Commands,
) {
    for entity in children.iter_descendants(event.entity) {
        if meshes.contains(entity) {
            commands.entity(entity).insert(Pickable::IGNORE);
        }
    }
}

fn tint_rock(
    event: On<WorldInstanceReady>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    basalt: Res<Basalt>,
    mut commands: Commands,
) {
    for entity in children.iter_descendants(event.entity) {
        if meshes.contains(entity) {
            commands
                .entity(entity)
                .insert((Pickable::IGNORE, MeshMaterial3d(basalt.0.clone())));
        }
    }
}

fn fir(phase: f32) -> Mesh {
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for (height, radius, length, color) in [
        (0.00, 0.074, 0.67, [0.20, 0.10, 0.064]),
        (0.37, 0.47, 0.84, [0.026, 0.19, 0.155]),
        (0.74, 0.38, 0.74, [0.04, 0.265, 0.19]),
        (1.04, 0.29, 0.67, [0.08, 0.34, 0.22]),
        (1.31, 0.20, 0.59, [0.13, 0.41, 0.28]),
    ] {
        let base = positions.len() as u32;
        for i in 0..8 {
            let angle = phase + i as f32 * TAU / 8.0;
            let edge = radius * if i % 2 == 0 { 1.0 } else { 0.79 };
            positions.push([
                angle.cos() * edge,
                height + (i % 2) as f32 * 0.08,
                angle.sin() * edge,
            ]);
            colors.push([color[0], color[1], color[2], 1.0]);
            indices.extend([base + i, base + 8, base + (i + 1) % 8]);
        }
        positions.push([phase.sin() * 0.07, height + length, phase.cos() * 0.07]);
        colors.push([color[0] * 1.2, color[1] * 1.2, color[2] * 1.2, 1.0]);
    }
    let mut mesh = colored_mesh(positions, colors, indices);
    mesh.duplicate_vertices();
    mesh.compute_flat_normals();
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn largest_grid_corners_and_their_display_offsets_stay_above_water() {
        let land = Landscape {
            seed: 42,
            half: Vec2::splat(32.0),
            depot: Vec2::splat(-31.5),
            paths: Vec::new(),
            plots: Vec::new(),
        };
        for x in [-31.5, 31.5] {
            for z in [-31.5, 31.5] {
                assert!(land.height(Vec2::new(x, z + 1.1)) > -0.43);
            }
        }
    }
}
