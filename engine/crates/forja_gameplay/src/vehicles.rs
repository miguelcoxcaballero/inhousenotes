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

/// Places the wheels of `config` at the model's `wheel*` nodes (scaled by
/// `scale`) and sizes the chassis from the remaining geometry, so any car
/// model with named wheels drives with any handling file.
///
/// Returns the config and the model offset: draw the model at
/// `body_iso * translate(offset) * scale`.
pub fn bind_vehicle_model(mut config: VehicleConfig, scene: &SceneData, scale: f32) -> (VehicleConfig, Vec3) {
    let world = scene.world_matrices();
    let mut wheels: Vec<(Vec3, f32)> = Vec::new();
    let mut body = forja_math::Aabb::EMPTY;
    for (i, n) in scene.nodes.iter().enumerate() {
        let Some(m) = n.mesh else { continue };
        let mut b = forja_math::Aabb::EMPTY;
        for p in &scene.meshes[m] {
            for v in &p.positions {
                b.grow_point(world[i].transform_point3(*v) * scale);
            }
        }
        if b.is_empty() {
            continue;
        }
        if n.name.to_ascii_lowercase().starts_with("wheel") {
            let h = b.half_extents();
            wheels.push((b.center(), h.y.max(h.z)));
        } else {
            body = body.union(&b);
        }
    }
    if wheels.len() < 2 || body.is_empty() {
        return (config, Vec3::ZERO);
    }
    let front_z = wheels.iter().map(|w| w.0.z).fold(f32::MIN, f32::max);
    let template_front = config.wheels.iter().find(|w| w.steer).cloned().unwrap_or_default();
    let template_rear = config.wheels.iter().find(|w| !w.steer).cloned().unwrap_or_default();
    // Chassis origin: body center; suspension top sits above the wheel center.
    let center = body.center();
    config.chassis_half = body.half_extents() * Vec3::new(1.0, 0.6, 1.0);
    // Static sag so wheels sit where the artist modelled them at rest.
    let sag = (config.mass * 9.81 / wheels.len() as f32 / config.suspension_stiffness).min(0.9 * template_rear.suspension_length);
    config.wheels = wheels
        .iter()
        .map(|(c, r)| {
            let front = (c.z - front_z).abs() < 0.2;
            let t = if front { &template_front } else { &template_rear };
            WheelConfig { position: *c - center + Vec3::Y * (t.suspension_length - sag), radius: *r, ..t.clone() }
        })
        .collect();
    config.com = Vec3::new(0.0, -config.chassis_half.y * 0.6, 0.0);
    (config, -center)
}
