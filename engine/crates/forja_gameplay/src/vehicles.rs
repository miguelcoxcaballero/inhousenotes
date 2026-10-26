//! Vehicle handling registry (`data/handling/*.toml`, `[[vehicle]]`) and
//! binding of handling data to a vehicle model's wheel nodes.

use std::collections::HashMap;
use std::path::Path;

use forja_assets::SceneData;
use forja_math::Vec3;
use forja_physics::{VehicleConfig, WheelConfig};

#[derive(Clone, Debug, Default)]
pub struct HandlingRegistry {
    defs: HashMap<String, VehicleConfig>,
}

impl HandlingRegistry {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let defs: Vec<VehicleConfig> = crate::load_dir(dir, "vehicle")?;
        Ok(Self { defs: defs.into_iter().map(|d| (d.name.clone(), d)).collect() })
    }

    pub fn get(&self, name: &str) -> Option<&VehicleConfig> {
        self.defs.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.defs.keys().map(String::as_str)
    }
}

/// Result of binding handling data to a vehicle model.
#[derive(Clone, Debug)]
pub struct BoundVehicle {
    pub config: VehicleConfig,
    /// Model-to-body transform (offset, orientation, scale).
    pub model_to_body: forja_math::Mat4,
    /// Mesh nodes of each wheel, in `config.wheels` order.
    pub wheel_nodes: Vec<Vec<usize>>,
    /// Wheel centers in model space, in `config.wheels` order.
    pub wheel_centers: Vec<Vec3>,
}

/// Places the wheels of `config` at the model's wheel nodes and sizes the
/// chassis from the rest, so any car model with named wheels drives with any
/// handling file. Wheel parts (tyre, rim, brake disc...) are grouped by
/// position; models whose front wheels sit on -Z are turned around.
pub fn bind_vehicle_model(config: VehicleConfig, scene: &SceneData, scale: f32) -> BoundVehicle {
    bind_vehicle_model_oriented(config, scene, scale, forja_math::Quat::IDENTITY)
}

/// Rotation that turns a model's longest horizontal axis into +Z (for
/// downloaded cars authored along X).
pub fn auto_orient(scene: &SceneData) -> forja_math::Quat {
    let b = scene.bounds();
    let e = b.max - b.min;
    if e.x > e.z * 1.2 { forja_math::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) } else { forja_math::Quat::IDENTITY }
}

/// Scale that brings a car model to a plausible length (models come in cm,
/// inches or arbitrary units).
pub fn auto_scale(scene: &SceneData, target_length: f32) -> f32 {
    let b = scene.bounds();
    let len = (b.max - b.min).max_element().max(1e-3);
    if (target_length * 0.6..target_length * 1.6).contains(&len) { 1.0 } else { target_length / len }
}

/// Like [`bind_vehicle_model`], with `pre` applied to the model first.
pub fn bind_vehicle_model_oriented(mut config: VehicleConfig, scene: &SceneData, scale: f32, pre: forja_math::Quat) -> BoundVehicle {
    let pre_m = forja_math::Mat4::from_quat(pre);
    let world: Vec<forja_math::Mat4> = scene.world_matrices().into_iter().map(|m| pre_m * m).collect();
    let mut parts: Vec<(usize, forja_math::Aabb, bool)> = Vec::new();
    let mut body = forja_math::Aabb::EMPTY;
    for (i, n) in scene.nodes.iter().enumerate() {
        let Some(m) = n.mesh else { continue };
        let mut b = forja_math::Aabb::EMPTY;
        for p in &scene.meshes[m] {
            for v in &p.positions {
                b.grow_point(world[i].transform_point3(*v));
            }
        }
        if b.is_empty() {
            continue;
        }
        let lname = n.name.to_ascii_lowercase();
        let wheelish = ["wheel", "tire", "tyre", "rim", "reifen", "rueda"].iter().any(|k| lname.contains(k)) && !lname.contains("steer");
        if wheelish {
            parts.push((i, b, lname.contains("front")));
        } else {
            body = body.union(&b);
        }
    }
    // Cluster wheel parts whose centers are close.
    let mut wheels: Vec<(Vec<usize>, forja_math::Aabb, bool)> = Vec::new();
    for (node, b, front) in parts {
        match wheels.iter_mut().find(|w| (w.1.center() - b.center()).length() < 0.35 * b.half_extents().max_element().max(0.2) * 4.0) {
            Some(w) => {
                w.0.push(node);
                w.1 = w.1.union(&b);
                w.2 |= front;
            }
            None => wheels.push((vec![node], b, front)),
        }
    }
    let identity = BoundVehicle { config: config.clone(), model_to_body: forja_math::Mat4::IDENTITY, wheel_nodes: Vec::new(), wheel_centers: Vec::new() };
    if wheels.len() < 2 || body.is_empty() {
        return identity;
    }
    let all = wheels.iter().fold(body, |acc, w| acc.union(&w.1));
    // Front axle: named "front", else the larger Z.
    let named_front = wheels.iter().any(|w| w.2);
    let max_z = wheels.iter().map(|w| w.1.center().z).fold(f32::MIN, f32::max);
    let is_front = |w: &(Vec<usize>, forja_math::Aabb, bool)| if named_front { w.2 } else { (w.1.center().z - max_z).abs() < 0.2 };
    let front_z: f32 = wheels.iter().filter(|w| is_front(w)).map(|w| w.1.center().z).sum::<f32>() / wheels.iter().filter(|w| is_front(w)).count().max(1) as f32;
    let rear_z: f32 = wheels.iter().filter(|w| !is_front(w)).map(|w| w.1.center().z).sum::<f32>() / wheels.iter().filter(|w| !is_front(w)).count().max(1) as f32;
    let flip = front_z < rear_z;
    let orient = if flip { forja_math::Mat4::from_rotation_y(std::f32::consts::PI) } else { forja_math::Mat4::IDENTITY };
    let center = all.center();
    let model_to_body = orient * forja_math::Mat4::from_scale(Vec3::splat(scale)) * forja_math::Mat4::from_translation(-center) * pre_m;
    let to_body = |p: Vec3| model_to_body.transform_point3(p);
    let body_half = (body.half_extents() * scale) * Vec3::new(1.0, 0.6, 1.0);
    let template_front = config.wheels.iter().find(|w| w.steer).cloned().unwrap_or_default();
    let template_rear = config.wheels.iter().find(|w| !w.steer).cloned().unwrap_or_default();
    config.chassis_half = body_half;
    // Static sag so wheels sit where the artist modelled them at rest.
    let sag = (config.mass * 9.81 / wheels.len() as f32 / config.suspension_stiffness).min(0.9 * template_rear.suspension_length);
    let mut wheel_nodes = Vec::new();
    let mut wheel_centers = Vec::new();
    config.wheels = wheels
        .iter()
        .map(|w| {
            let t = if is_front(w) { &template_front } else { &template_rear };
            let h = w.1.half_extents() * scale;
            wheel_nodes.push(w.0.clone());
            wheel_centers.push(pre.inverse() * w.1.center());
            WheelConfig { position: to_body(w.1.center()) + Vec3::Y * (t.suspension_length - sag), radius: h.y.max(h.z), ..t.clone() }
        })
        .collect();
    config.com = Vec3::new(0.0, -config.chassis_half.y * 0.6, 0.0);
    BoundVehicle { config, model_to_body, wheel_nodes, wheel_centers }
}
