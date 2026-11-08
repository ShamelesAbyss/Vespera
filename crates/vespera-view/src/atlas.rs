use std::collections::{BTreeMap, BTreeSet};

use bevy::input::InputSystems;
use bevy::prelude::*;
use vespera_core::AgentId;

#[derive(Clone, Copy, Component, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Layer {
    Machine,
    Network,
    Swarm,
    World,
    Home,
    Providers,
    Workflows,
}

impl Layer {
    const ALL: [Self; 7] = [
        Self::Machine,
        Self::Network,
        Self::Swarm,
        Self::World,
        Self::Home,
        Self::Providers,
        Self::Workflows,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Machine => "Machine",
            Self::Network => "Intelligence Network",
            Self::Swarm => "Swarm",
            Self::World => "Living World",
            Self::Home => "Home",
            Self::Providers => "Providers",
            Self::Workflows => "Workflows",
        }
    }
}

#[derive(Resource)]
pub(super) struct Layers {
    pub active: Layer,
    pub switched: bool,
    pub incoming: Option<AgentId>,
    roots: BTreeMap<Layer, Entity>,
    disabled: BTreeSet<Layer>,
    requested: Option<Layer>,
}

impl Layers {
    pub fn new(active: Layer) -> Self {
        Self {
            active,
            switched: false,
            incoming: None,
            roots: BTreeMap::new(),
            disabled: BTreeSet::new(),
            requested: None,
        }
    }

    pub fn parent(&self, layer: Layer) -> ChildOf {
        ChildOf(self.roots[&layer])
    }

    fn available(&self, layer: Layer) -> bool {
        self.roots.contains_key(&layer) && !self.disabled.contains(&layer)
    }

    pub fn set_available(&mut self, layer: Layer, available: bool) {
        if available {
            self.disabled.remove(&layer);
        } else {
            self.disabled.insert(layer);
        }
    }

    pub fn request(&mut self, layer: Layer) {
        self.requested = Some(layer);
    }
}

pub(super) fn register(app: &mut App, layer: Layer) {
    let world = app.world_mut();
    let visibility = visibility(world.resource::<Layers>().active == layer);
    let root = world.spawn((layer, Transform::default(), visibility)).id();
    world.resource_mut::<Layers>().roots.insert(layer, root);
}

fn visibility(active: bool) -> Visibility {
    if active {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

#[derive(Component)]
struct LayerBar;

pub(super) fn navigation(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(PreUpdate, switch.after(InputSystems));
}

fn setup(mut commands: Commands) {
    commands.spawn((
        LayerBar,
        Node {
            position_type: PositionType::Absolute,
            left: px(18),
            right: px(18),
            bottom: px(14),
            padding: UiRect::axes(px(12), px(8)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.045, 0.06, 0.11, 0.96)),
        Text::default(),
        TextFont::from_font_size(15.0),
        TextColor(Color::srgb(0.86, 0.92, 1.0)),
        Pickable::IGNORE,
    ));
}

fn switch(
    keys: Res<ButtonInput<KeyCode>>,
    mut layers: ResMut<Layers>,
    mut roots: Query<&mut Visibility, Without<Node>>,
    mut cameras: Query<(Entity, &Layer, &mut Camera)>,
    mut bar: Single<(Entity, &mut Text, Option<&UiTargetCamera>), With<LayerBar>>,
    mut commands: Commands,
) {
    let (entity, text, target) = &mut *bar;
    let previous = layers.active;
    if layers.switched {
        layers.switched = false;
        layers.incoming = None;
    }
    let keyboard = Layer::ALL
        .into_iter()
        .zip([
            KeyCode::F1,
            KeyCode::F2,
            KeyCode::F3,
            KeyCode::F4,
            KeyCode::F5,
            KeyCode::F6,
            KeyCode::F7,
        ])
        .find_map(|(layer, key)| {
            (keys.just_pressed(key) && layers.available(layer)).then_some(layer)
        });
    if let Some(layer) = layers
        .requested
        .take()
        .filter(|&layer| layers.available(layer))
        .or(keyboard)
    {
        layers.active = layer;
    }
    if target.is_some_and(|target| {
        cameras
            .get(target.0)
            .is_ok_and(|(_, layer, _)| *layer == layers.active)
    }) && layers.active == previous
    {
        return;
    }
    if layers.active != previous {
        layers.switched = true;
        commands.queue(move |world: &mut World| {
            let agent = match previous {
                Layer::Machine => crate::machine_scene::selected_agent(world),
                Layer::Network => crate::scene::selected_agent(world),
                Layer::Swarm => crate::swarm_scene::selected_agent(world),
                Layer::World => crate::world_scene::selected_agent(world),
                Layer::Home | Layer::Providers | Layer::Workflows => None,
            };
            world.resource_mut::<Layers>().incoming = agent;
        });
    }
    for (&layer, &root) in &layers.roots {
        if let Ok(mut visible) = roots.get_mut(root) {
            *visible = visibility(layer == layers.active);
        }
    }
    for (camera_entity, layer, mut camera) in &mut cameras {
        camera.is_active = *layer == layers.active;
        if camera.is_active {
            commands
                .entity(*entity)
                .insert(UiTargetCamera(camera_entity));
        }
    }
    text.0 = Layer::ALL
        .iter()
        .enumerate()
        .filter(|&(_, layer)| {
            !matches!(layer, Layer::Home | Layer::Providers | Layer::Workflows)
                || layers.roots.contains_key(layer)
        })
        .map(|(index, layer)| {
            let name = layer.name();
            let key = index + 1;
            if *layer == layers.active {
                format!("[F{key} {name}]")
            } else if layers.available(*layer) {
                format!("F{key} {name}")
            } else {
                format!("{name} unavailable")
            }
        })
        .collect::<Vec<_>>()
        .join("  |  ");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_selects_only_installed_layers_and_activates_one_camera() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(Layers::new(Layer::Machine));
        let world = app.world_mut();
        let machine = world.spawn((Layer::Machine, Camera::default())).id();
        let living = world.spawn((Layer::World, Camera::default())).id();
        register(&mut app, Layer::Machine);
        register(&mut app, Layer::World);
        navigation(&mut app);
        app.update();
        assert!(app.world().get::<Camera>(machine).unwrap().is_active);
        assert!(!app.world().get::<Camera>(living).unwrap().is_active);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F2);
        app.update();
        assert!(app.world().resource::<Layers>().active == Layer::Machine);
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.reset_all();
        keys.press(KeyCode::F4);
        app.update();
        let layers = app.world().resource::<Layers>();
        assert!(layers.active == Layer::World && layers.switched);
        assert!(!app.world().get::<Camera>(machine).unwrap().is_active);
        assert!(app.world().get::<Camera>(living).unwrap().is_active);
        assert_eq!(
            *app.world()
                .get::<Visibility>(layers.roots[&Layer::Machine])
                .unwrap(),
            Visibility::Hidden,
        );
    }
}
