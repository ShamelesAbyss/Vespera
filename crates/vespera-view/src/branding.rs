use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::UiSystems;
use bevy::window::PrimaryWindow;
use bevy::winit::WINIT_WINDOWS;
use winit::window::Icon;

use crate::atlas::Layer;

const PNG: &[u8] = include_bytes!("../../../assets/branding/vespera.png");

#[derive(Resource)]
pub(super) struct Brand {
    image: Handle<Image>,
    icon: Icon,
}

impl Brand {
    pub fn mark(&self, size: f32) -> impl Bundle {
        (
            ImageNode::new(self.image.clone()),
            Node {
                width: px(size),
                height: px(size),
                flex_shrink: 0.0,
                ..default()
            },
            Pickable::IGNORE,
        )
    }
}

#[derive(Component)]
struct Header;

pub(super) fn install(app: &mut App) {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(PNG))
        .read_info()
        .expect("decode embedded Vespera branding");
    let mut pixels = vec![0; decoder.output_buffer_size()];
    let frame = decoder
        .next_frame(&mut pixels)
        .expect("read Vespera pixels");
    assert_eq!(frame.color_type, png::ColorType::Rgba);
    assert_eq!(frame.bit_depth, png::BitDepth::Eight);
    pixels.truncate(frame.buffer_size());
    let icon = Icon::from_rgba(pixels.clone(), frame.width, frame.height)
        .expect("construct the Vespera window icon");
    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new(
            Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ));
    app.insert_resource(Brand { image, icon })
        .add_systems(Startup, header)
        .add_systems(Update, window_icon)
        .add_systems(PostUpdate, track_camera.before(UiSystems::Prepare));
}

fn header(mut commands: Commands, brand: Res<Brand>) {
    commands.spawn((Header, header_node(&brand)));
}

pub(super) fn header_node(brand: &Brand) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
        children![(
            Node {
                align_items: AlignItems::Center,
                column_gap: px(9),
                padding: UiRect::right(px(12)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.035, 0.05, 0.085, 0.95)),
            Pickable::IGNORE,
            children![
                brand.mark(46.0),
                (
                    Text::new("VESPERA"),
                    TextFont::from_font_size(15.0),
                    Pickable::IGNORE
                ),
            ],
        )],
    )
}

fn track_camera(
    mut commands: Commands,
    cameras: Query<(Entity, &Camera, &Layer)>,
    mut header: Single<(Entity, &mut Visibility, Option<&UiTargetCamera>), With<Header>>,
) {
    let (entity, visible, target) = &mut *header;
    let Some((camera, _, layer)) = cameras.iter().find(|(_, camera, _)| camera.is_active) else {
        return;
    };
    visible.set_if_neq(if matches!(layer, Layer::Home | Layer::Swarm) {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    });
    if !target.is_some_and(|target| target.0 == camera) {
        commands.entity(*entity).insert(UiTargetCamera(camera));
    }
}

fn window_icon(
    _: NonSendMarker,
    window: Single<Entity, With<PrimaryWindow>>,
    brand: Res<Brand>,
    mut applied: Local<bool>,
) {
    if *applied {
        return;
    }
    WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(window) = windows.get_window(*window) {
            window.set_window_icon(Some(brand.icon.clone()));
            #[cfg(windows)]
            {
                use winit::platform::windows::WindowExtWindows;
                window.set_taskbar_icon(Some(brand.icon.clone()));
            }
            *applied = true;
        }
    });
}
