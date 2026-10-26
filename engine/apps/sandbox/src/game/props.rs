//! Dynamic props: stackable crates and barrels that react to cars, bullets
//! and the player.

use std::sync::Arc;

use forja_assets::{MaterialData, NodeData, SceneData};
use forja_core::{App, Plugin, Stage};
use forja_math::{Iso, Mat4, Vec3, Vec4};
use forja_physics::{BodyHandle, ColliderDesc, CollisionGroups, PhysicsWorld, RigidBodyDesc, Shape, layers};
use forja_world::SpawnKind;

use super::world::GameWorld;
use super::{Paths, RenderItem, RenderScene};

pub struct PropsPlugin;

pub struct Prop {
    pub body: BodyHandle,
    pub model: usize,
    pub offset: Mat4,
}

#[derive(Default)]
pub struct Props(pub Vec<Prop>);

/// A box mesh as a scene, used when downloaded props are missing.
pub fn box_scene(half: Vec3, color: Vec4) -> SceneData {
    let mut b = forja_world::builder::MeshBuilder::new(0);
    b.cuboid(Vec3::ZERO, half, 0.5);
    SceneData {
        name: "box".into(),
        nodes: vec![NodeData { name: "box".into(), mesh: Some(0), ..Default::default() }],
        roots: vec![0],
        meshes: vec![vec![b.finish()]],
        materials: vec![MaterialData { base_color: color, roughness: 0.8, ..Default::default() }],
        ..Default::default()
    }
}

impl Plugin for PropsPlugin {
    fn name(&self) -> &'static str {
        "props"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["world"]
    }

    fn build(&self, app: &mut App) {
        let paths = app.world.res::<Paths>().clone();
        let mut spawns: Vec<Iso> = app.world.res::<GameWorld>().desc.spawns_of(SpawnKind::Prop).map(|s| s.iso).collect();
        if spawns.is_empty() {
            // A small stack next to the player in maps without prop spawns.
            let p = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).map(|s| s.pos).unwrap_or(Vec3::Y);
            for i in 0..10 {
                spawns.push(Iso::from_pos(p + Vec3::new(4.0 + (i % 3) as f32 * 0.85, 0.5 + (i / 3) as f32 * 0.82, 3.0)));
            }
        }
        let half = Vec3::splat(0.4);
        let (model, offset) = {
            let rs = app.world.res_mut::<RenderScene>();
            match rs.load(&paths.asset("kenney_blaster_kit/crate-medium.glb")) {
                Some(m) => {
                    let b = rs.model(m).bounds();
                    let s = (half * 2.0 / b.half_extents().max(Vec3::splat(1e-3)) / 2.0).min_element();
                    (m, Mat4::from_scale(Vec3::splat(s)) * Mat4::from_translation(-b.center()))
                }
                None => (rs.add_model(Arc::new(box_scene(half, Vec4::new(0.55, 0.38, 0.2, 1.0)))), Mat4::IDENTITY),
            }
        };
        let mut props = Props::default();
        {
            let physics = app.world.res_mut::<PhysicsWorld>();
            for iso in spawns {
                let (body, _) = physics.add_dynamic(
                    RigidBodyDesc::dynamic().position(iso.pos).rotation(iso.rot),
                    ColliderDesc::new(Shape::cuboid(half)).mass(25.0).friction(0.7).groups(CollisionGroups::new(layers::DYNAMIC, u32::MAX)),
                );
                props.0.push(Prop { body, model, offset });
            }
        }
        app.insert_resource(props);
        app.add_system_at(Stage::PostUpdate, "props_render", 10, |w| {
            let items: Vec<RenderItem> = {
                let physics = w.res::<PhysicsWorld>();
                w.res::<Props>()
                    .0
                    .iter()
                    .filter_map(|p| physics.body(p.body).map(|b| RenderItem::new(p.model, b.iso().to_mat4() * p.offset)))
                    .collect()
            };
            w.res_mut::<RenderScene>().dynamic.extend(items);
        });
    }
}
