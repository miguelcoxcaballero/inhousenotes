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
    /// Per wheel, the rotation (model space, about the wheel center) that
    /// undoes the steering the model was posed with; apply it before the
    /// physics' steer and spin.
    pub wheel_rest: Vec<forja_math::Quat>,
    /// Per wheel, parts that steer with it but do not spin (brake pads,
    /// calipers).
    pub wheel_calipers: Vec<Vec<usize>>,
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
    let wheel_name = |name: &str| {
        let l = name.to_ascii_lowercase();
        ["wheel", "tire", "tyre", "rim", "reifen", "rueda"].iter().any(|k| l.contains(k)) && !l.contains("steer")
    };
    // A part belongs to a wheel when it or one of its ancestors is named
    // like one: the tyres of the Khronos CarConcept are unnamed meshes under
    // the `WheelFrontL` group, and were left in the body (frozen at the
    // model's rest pose, with the front wheels steered 30 degrees).
    let ancestry = |mut i: usize| {
        let mut wheel = false;
        let mut front = false;
        loop {
            let n = &scene.nodes[i];
            if wheel_name(&n.name) {
                wheel = true;
                front |= n.name.to_ascii_lowercase().contains("front");
            }
            match n.parent {
                Some(p) if p != i => i = p,
                _ => break,
            }
        }
        (wheel, front)
    };
    // `(node, bounds, front, points, caliper)`.
    let mut parts: Vec<(usize, forja_math::Aabb, bool, Vec<Vec3>, bool)> = Vec::new();
    let mut body = forja_math::Aabb::EMPTY;
    for (i, n) in scene.nodes.iter().enumerate() {
        let Some(m) = n.mesh else { continue };
        let mut b = forja_math::Aabb::EMPTY;
        let mut pts = Vec::new();
        for p in &scene.meshes[m] {
            for v in &p.positions {
                let q = world[i].transform_point3(*v);
                b.grow_point(q);
                pts.push(q);
            }
        }
        if b.is_empty() {
            continue;
        }
        let (wheelish, front) = ancestry(i);
        if wheelish {
            // Brake pads / calipers steer with the wheel but do not spin.
            let l = n.name.to_ascii_lowercase();
            let caliper = l.contains("pad") || l.contains("caliper");
            parts.push((i, b, front, pts, caliper));
        } else {
            body = body.union(&b);
        }
    }
    // Cluster wheel parts whose centers are close.
    struct Wheel {
        nodes: Vec<usize>,
        calipers: Vec<usize>,
        bounds: forja_math::Aabb,
        front: bool,
        points: Vec<Vec3>,
        /// Tyre and rim points only: off-centre brake pads would skew the
        /// axle estimate by several degrees.
        axle_points: Vec<Vec3>,
    }
    let mut wheels: Vec<Wheel> = Vec::new();
    for (node, b, front, pts, caliper) in parts {
        let w = match wheels.iter_mut().find(|w| (w.bounds.center() - b.center()).length() < 0.35 * b.half_extents().max_element().max(0.2) * 4.0) {
            Some(w) => {
                w.bounds = w.bounds.union(&b);
                w.front |= front;
                w
            }
            None => {
                wheels.push(Wheel { nodes: Vec::new(), calipers: Vec::new(), bounds: b, front, points: Vec::new(), axle_points: Vec::new() });
                wheels.last_mut().expect("just pushed")
            }
        };
        if caliper {
            w.calipers.push(node);
        } else {
            w.nodes.push(node);
            w.axle_points.extend(pts.iter().copied());
        }
        w.points.extend(pts);
    }
    // A spare wheel (the SUV's, on its tailgate) does not touch the ground:
    // it stays part of the body.
    if let Some(ground) = wheels.iter().map(|w| w.bounds.min.y).reduce(f32::min) {
        let (road, spare): (Vec<Wheel>, Vec<Wheel>) = wheels.into_iter().partition(|w| w.bounds.min.y < ground + 0.5 * w.bounds.half_extents().y);
        wheels = road;
        for w in spare {
            body = body.union(&w.bounds);
        }
    }
    let identity = BoundVehicle { config: config.clone(), model_to_body: forja_math::Mat4::IDENTITY, wheel_nodes: Vec::new(), wheel_centers: Vec::new(), wheel_rest: Vec::new(), wheel_calipers: Vec::new() };
    if wheels.len() < 2 || body.is_empty() {
        return identity;
    }
    // Rest pose straightening: some models are posed with the front wheels
    // steered (the CarConcept's are turned 30 degrees). The axle is the
    // direction of least spread of the wheel's vertices in the ground
    // plane; the yaw that brings it back onto X is undone before the
    // physics' steer and spin are applied.
    let mut rest_yaw = Vec::with_capacity(wheels.len());
    for w in &mut wheels {
        let q = straighten_yaw(if w.axle_points.len() >= 8 { &w.axle_points } else { &w.points });
        let c = w.bounds.center();
        if q != forja_math::Quat::IDENTITY {
            let mut b = forja_math::Aabb::EMPTY;
            for p in &w.points {
                b.grow_point(c + q * (*p - c));
            }
            w.bounds = b;
        }
        w.points = Vec::new();
        w.axle_points = Vec::new();
        rest_yaw.push(q);
    }
    let all = wheels.iter().fold(body, |acc, w| acc.union(&w.bounds));
    // Front axle: named "front", else the larger Z.
    let named_front = wheels.iter().any(|w| w.front);
    let max_z = wheels.iter().map(|w| w.bounds.center().z).fold(f32::MIN, f32::max);
    let is_front = |w: &Wheel| if named_front { w.front } else { (w.bounds.center().z - max_z).abs() < 0.2 };
    let front_z: f32 = wheels.iter().filter(|w| is_front(w)).map(|w| w.bounds.center().z).sum::<f32>() / wheels.iter().filter(|w| is_front(w)).count().max(1) as f32;
    let rear_z: f32 = wheels.iter().filter(|w| !is_front(w)).map(|w| w.bounds.center().z).sum::<f32>() / wheels.iter().filter(|w| !is_front(w)).count().max(1) as f32;
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
    let mut wheel_calipers = Vec::new();
    let wheel_rest = rest_yaw.iter().map(|q| pre.inverse() * *q * pre).collect();
    config.wheels = wheels
        .iter()
        .map(|w| {
            let t = if is_front(w) { &template_front } else { &template_rear };
            let h = w.bounds.half_extents() * scale;
            wheel_nodes.push(w.nodes.clone());
            wheel_calipers.push(w.calipers.clone());
            wheel_centers.push(pre.inverse() * w.bounds.center());
            WheelConfig { position: to_body(w.bounds.center()) + Vec3::Y * (t.suspension_length - sag), radius: h.y.max(h.z), ..t.clone() }
        })
        .collect();
    config.com = Vec3::new(0.0, -config.chassis_half.y * 0.6, 0.0);
    BoundVehicle { config, model_to_body, wheel_nodes, wheel_centers, wheel_rest, wheel_calipers }
}

/// Rotation about Y that turns a wheel's axle (the horizontal direction in
/// which its vertices spread least) onto the X axis; identity when it is
/// already within 2 degrees or the points do not show a clear axle.
pub fn straighten_yaw(points: &[Vec3]) -> forja_math::Quat {
    if points.len() < 8 {
        return forja_math::Quat::IDENTITY;
    }
    let n = points.len() as f64;
    let (mut mx, mut mz) = (0.0f64, 0.0f64);
    for p in points {
        mx += p.x as f64;
        mz += p.z as f64;
    }
    mx /= n;
    mz /= n;
    let (mut cxx, mut cxz, mut czz) = (0.0f64, 0.0f64, 0.0f64);
    for p in points {
        let (dx, dz) = (p.x as f64 - mx, p.z as f64 - mz);
        cxx += dx * dx;
        cxz += dx * dz;
        czz += dz * dz;
    }
    // Eigenvalues of the 2x2 covariance; the axle is the smaller one.
    let tr = cxx + czz;
    let det = cxx * czz - cxz * cxz;
    let disc = (tr * tr * 0.25 - det).max(0.0).sqrt();
    let (big, small) = (tr * 0.5 + disc, tr * 0.5 - disc);
    // A wheel is a disc: much wider (diameter) than thick (tread).
    if big <= 0.0 || small > big * 0.7 {
        return forja_math::Quat::IDENTITY;
    }
    // Eigenvector of `small`: (cxz, small - cxx) or (small - czz, cxz).
    let (mut ax, mut az) = if cxz.abs() > 1e-12 { (cxz, small - cxx) } else if cxx <= czz { (1.0, 0.0) } else { (0.0, 1.0) };
    if ax < 0.0 {
        (ax, az) = (-ax, -az);
    }
    // R_y(b) * X = (cos b, 0, -sin b): the axle is R_y(b) * X.
    let b = (-az).atan2(ax) as f32;
    if b.abs() < 2f32.to_radians() || b.abs() > 89f32.to_radians() {
        return forja_math::Quat::IDENTITY;
    }
    forja_math::Quat::from_rotation_y(-b)
}
