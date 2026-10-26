//! Procedural vehicle and character test track: flat pad, ramps, slopes at
//! 15/30/45 degrees, stairs, speed bumps, a jump and prop stacks.

use std::sync::Arc;

use forja_assets::{MaterialData, NodeData, SceneData};
use forja_math::{Iso, Mat4, Quat, Vec2, Vec3, Vec4};
use forja_physics::{ColliderDesc, CollisionGroups, Shape, layers};

use crate::builder::MeshBuilder;
use crate::{SpawnKind, SpawnPoint, WorldDescription, WorldError, WorldSource};

pub struct TestTrack {
    pub size: f32,
}

impl Default for TestTrack {
    fn default() -> Self {
        Self { size: 300.0 }
    }
}

impl WorldSource for TestTrack {
    fn name(&self) -> String {
        "Test track".into()
    }

    fn build(&self) -> Result<WorldDescription, WorldError> {
        let mut w = WorldDescription::new("Test track");
        let mat = |name: &str, c: [f32; 3], rough: f32, metal: f32| MaterialData {
            name: name.into(),
            base_color: Vec4::new(c[0], c[1], c[2], 1.0),
            roughness: rough,
            metallic: metal,
            ..Default::default()
        };
        let materials = vec![
            mat("asphalt", [0.08, 0.08, 0.09], 0.9, 0.0),
            mat("concrete", [0.55, 0.54, 0.52], 0.8, 0.0),
            mat("paint_yellow", [0.95, 0.75, 0.1], 0.5, 0.0),
            mat("steel", [0.75, 0.76, 0.78], 0.3, 1.0),
        ];
        let mut ground = MeshBuilder::new(0);
        let h = self.size * 0.5;
        ground.quad(Vec3::new(-h, 0.0, h), Vec3::new(h, 0.0, h), Vec3::new(h, 0.0, -h), Vec3::new(-h, 0.0, -h), 4.0);
        let mut concrete = MeshBuilder::new(1);
        let mut paint = MeshBuilder::new(2);
        let mut steel = MeshBuilder::new(3);
        // Lane stripes every 10 m.
        for i in -10..=10 {
            let x = i as f32 * 10.0;
            paint.quad(Vec3::new(x - 0.08, 0.01, h), Vec3::new(x + 0.08, 0.01, h), Vec3::new(x + 0.08, 0.01, -h), Vec3::new(x - 0.08, 0.01, -h), 1.0);
        }
        let mut boxes: Vec<(Vec3, Vec3, Quat, usize)> = Vec::new();
        // Slopes 15/30/45 degrees.
        for (k, deg) in [15.0f32, 30.0, 45.0].iter().enumerate() {
            let len = 12.0;
            let a = deg.to_radians();
            let center = Vec3::new(-60.0 + k as f32 * 12.0, len * 0.5 * a.sin() - 0.3, 40.0);
            boxes.push((center, Vec3::new(4.0, 0.3, len * 0.5), Quat::from_rotation_x(-a), 1));
        }
        // Jump ramp and landing.
        boxes.push((Vec3::new(0.0, 1.0, 60.0), Vec3::new(4.0, 0.25, 6.0), Quat::from_rotation_x(-0.2), 2));
        // Stairs: 12 steps of 0.18 m.
        for s in 0..12 {
            let y = 0.09 + s as f32 * 0.18;
            boxes.push((Vec3::new(40.0, y * 0.5, -30.0 - s as f32 * 0.3), Vec3::new(2.0, y * 0.5, 0.15), Quat::IDENTITY, 1));
        }
        boxes.push((Vec3::new(40.0, 1.08, -36.6), Vec3::new(2.0, 1.08, 3.0), Quat::IDENTITY, 1));
        // Speed bumps.
        for b in 0..5 {
            boxes.push((Vec3::new(-20.0, 0.04, -20.0 - b as f32 * 8.0), Vec3::new(4.0, 0.08, 0.4), Quat::IDENTITY, 2));
        }
        // Barrier walls around a skid pad.
        for (c, half) in [
            (Vec3::new(70.0, 0.6, 0.0), Vec3::new(0.3, 0.6, 30.0)),
            (Vec3::new(130.0, 0.6, 0.0), Vec3::new(0.3, 0.6, 30.0)),
            (Vec3::new(100.0, 0.6, 30.0), Vec3::new(30.0, 0.6, 0.3)),
        ] {
            boxes.push((c, half, Quat::IDENTITY, 3));
        }
        for (c, half, rot, m) in &boxes {
            let mut b = MeshBuilder::new(*m);
            b.cuboid(Vec3::ZERO, *half, 2.0);
            let mut mesh = b.finish();
            for p in &mut mesh.positions {
                *p = *rot * *p + *c;
            }
            for n in &mut mesh.normals {
                *n = *rot * *n;
            }
            for t in &mut mesh.tangents {
                let v = *rot * t.truncate();
                *t = v.extend(t.w);
            }
            mesh.aabb = forja_math::Aabb::from_points(mesh.positions.iter().copied());
            let target = match m {
                1 => &mut concrete,
                2 => &mut paint,
                _ => &mut steel,
            };
            let base = target.mesh.positions.len() as u32;
            target.mesh.positions.extend(mesh.positions);
            target.mesh.normals.extend(mesh.normals);
            target.mesh.uvs.extend(mesh.uvs);
            target.mesh.indices.extend(mesh.indices.iter().map(|i| i + base));
            w.colliders.push((
                ColliderDesc::new(Shape::cuboid(*half)).friction(1.0).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
                Iso::new(*c, *rot),
            ));
        }
        let prims = [ground, concrete, paint, steel].into_iter().filter(|b| !b.is_empty()).map(|b| b.finish()).collect();
        let scene = SceneData {
            name: "track".into(),
            nodes: vec![NodeData { name: "track".into(), mesh: Some(0), ..Default::default() }],
            roots: vec![0],
            meshes: vec![prims],
            materials,
            ..Default::default()
        };
        w.add_scene(Arc::new(scene), Mat4::IDENTITY);
        w.colliders.push((
            ColliderDesc::new(Shape::cuboid(Vec3::new(h, 1.0, h))).translation(Vec3::new(0.0, -1.0, 0.0)).friction(1.0).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
            Iso::IDENTITY,
        ));
        w.spawns.push(SpawnPoint { kind: SpawnKind::Player, iso: Iso::from_pos(Vec3::new(0.0, 1.2, 0.0)) });
        w.spawns.push(SpawnPoint { kind: SpawnKind::Vehicle, iso: Iso::from_pos(Vec3::new(5.0, 1.0, 5.0)) });
        w.spawns.push(SpawnPoint { kind: SpawnKind::Vehicle, iso: Iso::from_pos(Vec3::new(10.0, 1.0, 5.0)) });
        w.spawns.push(SpawnPoint { kind: SpawnKind::Bike, iso: Iso::from_pos(Vec3::new(-5.0, 1.0, 5.0)) });
        for i in 0..3 {
            w.spawns.push(SpawnPoint { kind: SpawnKind::Weapon, iso: Iso::from_pos(Vec3::new(-3.0 + i as f32 * 1.5, 0.6, -4.0)) });
        }
        for i in 0..20 {
            w.spawns.push(SpawnPoint { kind: SpawnKind::Prop, iso: Iso::from_pos(Vec3::new(20.0 + (i % 4) as f32 * 0.85, 0.42 + (i / 4) as f32 * 0.82, 10.0)) });
        }
        w.extent = Vec2::splat(h);
        Ok(w)
    }
}
