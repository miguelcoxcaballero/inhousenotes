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

/// Open ground near the player at `p` for something `n` spots long
/// (`spacing` apart along `dir`): every spot in sight of the player's
/// chest and on level ground within a step of the player's feet. Returns
/// the first spot (on the ground). Spots are tried on rings from `r0`.
pub fn open_ground(physics: &PhysicsWorld, p: Vec3, r0: f32, n: usize, spacing: f32, dir: Vec3) -> Option<Vec3> {
    use forja_math::Ray;
    let chest = p + Vec3::Y * 1.0;
    let ground = |at: Vec3| -> Option<f32> {
        let sight = at + Vec3::Y * 1.0 - chest;
        if physics.raycast(&Ray::new(chest, sight), sight.length() + 0.6, Default::default()).is_some() {
            return None;
        }
        let hit = physics.raycast(&Ray::new(Vec3::new(at.x, p.y + 3.0, at.z), Vec3::NEG_Y), 6.0, Default::default())?;
        ((hit.point.y - p.y).abs() < 1.2 && hit.normal.y > 0.9).then_some(hit.point.y)
    };
    for r in [r0, r0 * 0.75, r0 * 1.4, r0 * 1.8, r0 * 0.6] {
        for k in 0..16 {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            let at = p + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
            let hs: Option<Vec<f32>> = (0..n).map(|i| ground(at + dir * (i as f32 * spacing))).collect();
            if let Some(hs) = hs {
                let (lo, hi) = hs.iter().fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(*h), b.max(*h)));
                if hi - lo < 0.15 {
                    return Some(Vec3::new(at.x, hi + 0.01, at.z));
                }
            }
        }
    }
    None
}

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
            // A small stack next to the player in maps without prop spawns,
            // on open ground in sight of the player: on a hill town a fixed
            // offset put crates inside the facades (the city's collision
            // mesh does not push them out), where they hung in the air.
            let p = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).map(|s| s.pos).unwrap_or(Vec3::Y);
            if let Some(base) = open_ground(app.world.res::<PhysicsWorld>(), p, 4.0, 3, 0.85, Vec3::X) {
                for i in 0..10 {
                    spawns.push(Iso::from_pos(base + Vec3::new((i % 3) as f32 * 0.85, 0.42 + (i / 3) as f32 * 0.82, 0.0)));
                }
            }
        }
        let half = Vec3::splat(0.4);
        let (model, offset) = {
            let rs = app.world.res_mut::<RenderScene>();
            match rs.load(&paths.asset("polyhaven_wooden_crate/wooden_crate_02_1k.gltf")).or_else(|| rs.load(&paths.asset("kenney_blaster_kit/crate-medium.glb"))) {
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
            for mut iso in spawns {
                // Slide sideways until the crate does not overlap scenery.
                for _ in 0..12 {
                    let probe = forja_math::Aabb::from_center_half(iso.pos, half * 1.05);
                    let mut blocked = false;
                    physics.query_aabb(&probe, Default::default(), |_, c| {
                        if !matches!(c.shape, Shape::Cuboid { half: h } if h.x > 20.0) && !matches!(c.shape, Shape::TriMesh(_) | Shape::HeightField(_)) {
                            blocked = true;
                        }
                    });
                    if !blocked {
                        break;
                    }
                    iso.pos.x += 1.3;
                }
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
