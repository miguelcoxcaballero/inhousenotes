//! Raycast vehicles (cars, trucks) and two-wheelers.
//!
//! The chassis is an ordinary dynamic body. Each wheel casts a ray along the
//! suspension axis; the suspension pushes the chassis and the tire model adds
//! longitudinal and lateral forces at the contact point. The model follows the
//! "arcade-sim" balance of open-world games: grip curves with a peak and a
//! slide plateau (like `handling.meta` traction curves), a friction circle, an
//! engine torque curve with an automatic gearbox, anti-roll bars, drag and
//! downforce. Two-wheel configs get a lean controller.
//!
//! Chassis frame convention (glTF): +Y up, +Z forward, +X left.

use forja_math::{Iso, Quat, Ray, Vec3};
use serde::{Deserialize, Serialize};

use crate::body::{BodyHandle, ColliderDesc, CollisionGroups, RigidBodyDesc, layers};
use crate::shape::Shape;
use crate::world::{PhysicsWorld, QueryFilter};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WheelConfig {
    /// Top of the suspension in the chassis frame.
    pub position: Vec3,
    pub radius: f32,
    /// Suspension length from `position` to the wheel center at full extension.
    pub suspension_length: f32,
    pub steer: bool,
    pub drive: bool,
    pub handbrake: bool,
}

impl Default for WheelConfig {
    fn default() -> Self {
        Self { position: Vec3::ZERO, radius: 0.35, suspension_length: 0.3, steer: false, drive: false, handbrake: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BikeConfig {
    pub max_lean_deg: f32,
    /// Natural frequency of the lean controller (rad/s).
    pub lean_frequency: f32,
    /// Damping ratio of the lean controller (1 = critical).
    pub lean_damping_ratio: f32,
}

impl Default for BikeConfig {
    fn default() -> Self {
        Self { max_lean_deg: 45.0, lean_frequency: 7.0, lean_damping_ratio: 1.0 }
    }
}

/// Handling data. Loadable from TOML (see `data/handling/*.toml`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct VehicleConfig {
    pub name: String,
    pub mass: f32,
    pub chassis_half: Vec3,
    /// Center of mass in the chassis frame (lower = more stable).
    pub com: Vec3,
    pub wheels: Vec<WheelConfig>,
    /// Spring rate per wheel (N/m).
    pub suspension_stiffness: f32,
    pub suspension_compression_damping: f32,
    pub suspension_rebound_damping: f32,
    /// Anti-roll bar rate (N/m of compression difference).
    pub anti_roll: f32,
    pub peak_torque: f32,
    pub peak_torque_rpm: f32,
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    pub gear_ratios: Vec<f32>,
    pub reverse_ratio: f32,
    pub final_drive: f32,
    pub drivetrain_efficiency: f32,
    pub shift_time: f32,
    pub max_steer_deg: f32,
    /// Steering lock reduction with speed: `max * 1/(1 + k v)`.
    pub steer_speed_falloff: f32,
    pub steer_rate: f32,
    /// Peak friction coefficient.
    pub grip: f32,
    /// Friction once fully sliding, as a fraction of `grip`.
    pub slide_grip: f32,
    pub peak_slip_deg: f32,
    pub brake_force: f32,
    pub handbrake_force: f32,
    /// Lateral grip multiplier on handbrake wheels while pulled.
    pub handbrake_grip: f32,
    pub drag: f32,
    pub rolling_resistance: f32,
    pub downforce: f32,
    /// Where tire forces act: 0 = contact patch, 1 = center-of-mass height.
    /// Higher values reduce body roll, pitch and wheelies (arcade feel).
    pub force_height: f32,
    pub bike: Option<BikeConfig>,
}

impl Default for VehicleConfig {
    fn default() -> Self {
        Self::sedan()
    }
}

impl VehicleConfig {
    /// A balanced rear-wheel-drive sedan.
    pub fn sedan() -> Self {
        let (x, z, y) = (0.8, 1.35, -0.1);
        let wheel = |px: f32, pz: f32, front: bool| WheelConfig {
            position: Vec3::new(px, y, pz),
            radius: 0.34,
            suspension_length: 0.35,
            steer: front,
            drive: !front,
            handbrake: !front,
        };
        Self {
            name: "sedan".into(),
            mass: 1400.0,
            chassis_half: Vec3::new(0.9, 0.45, 2.2),
            com: Vec3::new(0.0, -0.25, 0.05),
            wheels: vec![wheel(x, z, true), wheel(-x, z, true), wheel(x, -z, false), wheel(-x, -z, false)],
            suspension_stiffness: 38_000.0,
            suspension_compression_damping: 3_200.0,
            suspension_rebound_damping: 4_200.0,
            anti_roll: 9_000.0,
            peak_torque: 420.0,
            peak_torque_rpm: 4200.0,
            idle_rpm: 900.0,
            redline_rpm: 6800.0,
            gear_ratios: vec![3.4, 2.2, 1.5, 1.15, 0.92, 0.76],
            reverse_ratio: 3.2,
            final_drive: 3.6,
            drivetrain_efficiency: 0.85,
            shift_time: 0.25,
            max_steer_deg: 35.0,
            steer_speed_falloff: 0.025,
            steer_rate: 4.0,
            grip: 1.25,
            slide_grip: 0.75,
            peak_slip_deg: 7.0,
            brake_force: 14_000.0,
            handbrake_force: 6_000.0,
            handbrake_grip: 0.45,
            drag: 0.38,
            rolling_resistance: 12.0,
            downforce: 0.6,
            force_height: 0.55,
            bike: None,
        }
    }

    /// A sport motorcycle.
    pub fn motorcycle() -> Self {
        let mut c = Self::sedan();
        c.name = "motorcycle".into();
        c.mass = 220.0;
        c.chassis_half = Vec3::new(0.2, 0.35, 0.9);
        c.com = Vec3::new(0.0, -0.1, 0.0);
        c.wheels = vec![
            WheelConfig { position: Vec3::new(0.0, -0.15, 0.72), radius: 0.31, suspension_length: 0.3, steer: true, drive: false, handbrake: false },
            WheelConfig { position: Vec3::new(0.0, -0.15, -0.72), radius: 0.31, suspension_length: 0.3, steer: false, drive: true, handbrake: true },
        ];
        c.suspension_stiffness = 16_000.0;
        c.suspension_compression_damping = 1_000.0;
        c.suspension_rebound_damping = 1_300.0;
        c.anti_roll = 0.0;
        c.peak_torque = 110.0;
        c.peak_torque_rpm = 9000.0;
        c.idle_rpm = 1300.0;
        c.redline_rpm = 12_500.0;
        c.gear_ratios = vec![2.6, 1.9, 1.5, 1.25, 1.08, 0.96];
        c.reverse_ratio = 0.0;
        c.final_drive = 5.6;
        c.max_steer_deg = 28.0;
        c.steer_speed_falloff = 0.06;
        c.grip = 1.4;
        c.brake_force = 3_200.0;
        c.handbrake_force = 900.0;
        c.drag = 0.25;
        c.rolling_resistance = 3.0;
        c.downforce = 0.0;
        c.force_height = 0.9;
        c.bike = Some(BikeConfig::default());
        c
    }
}

/// Driver inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleControls {
    /// -1 (brake / reverse) to 1 (full throttle).
    pub throttle: f32,
    /// 0..1, independent of throttle.
    pub brake: f32,
    /// -1 (left) to 1 (right).
    pub steer: f32,
    pub handbrake: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WheelState {
    pub grounded: bool,
    pub compression: f32,
    pub contact_point: Vec3,
    pub contact_normal: Vec3,
    pub load: f32,
    pub steer_angle: f32,
    /// Visual spin angle (rad) and rate (rad/s).
    pub spin: f32,
    pub spin_rate: f32,
    pub slip_angle: f32,
    pub sliding: bool,
}

pub struct Vehicle {
    pub body: BodyHandle,
    pub config: VehicleConfig,
    pub controls: VehicleControls,
    pub wheels: Vec<WheelState>,
    pub gear: i32,
    pub rpm: f32,
    shift_timer: f32,
    anti_roll_pairs: Vec<(usize, usize)>,
}

impl Vehicle {
    /// Creates the chassis body and collider at `iso`.
    pub fn spawn(world: &mut PhysicsWorld, config: VehicleConfig, iso: Iso) -> Self {
        let body = world.add_body(
            RigidBodyDesc::dynamic().position(iso.pos).rotation(iso.rot).com(config.com).damping(0.02, 0.4).can_sleep(true),
        );
        world.add_collider(
            ColliderDesc::new(Shape::cuboid(config.chassis_half))
                .mass(config.mass)
                .friction(0.4)
                .groups(CollisionGroups::new(layers::VEHICLE, u32::MAX)),
            Some(body),
        );
        Self::from_body(body, config)
    }

    pub fn from_body(body: BodyHandle, config: VehicleConfig) -> Self {
        let mut pairs = Vec::new();
        for i in 0..config.wheels.len() {
            for j in i + 1..config.wheels.len() {
                let (a, b) = (config.wheels[i].position, config.wheels[j].position);
                if (a.z - b.z).abs() < 0.1 && (a.x + b.x).abs() < 0.1 && a.x.abs() > 0.05 {
                    pairs.push((i, j));
                }
            }
        }
        let wheels = vec![WheelState::default(); config.wheels.len()];
        Self { body, config, controls: VehicleControls::default(), wheels, gear: 1, rpm: 0.0, shift_timer: 0.0, anti_roll_pairs: pairs }
    }

    pub fn speed(&self, world: &PhysicsWorld) -> f32 {
        world.body(self.body).map(|b| b.linvel.dot(b.rotation() * Vec3::Z)).unwrap_or(0.0)
    }

    pub fn is_bike(&self) -> bool {
        self.config.bike.is_some()
    }

    /// Wheel transform for rendering.
    pub fn wheel_iso(&self, world: &PhysicsWorld, i: usize) -> Iso {
        let Some(b) = world.body(self.body) else { return Iso::IDENTITY };
        let iso = b.iso();
        let wc = &self.config.wheels[i];
        let ws = &self.wheels[i];
        let center = wc.position - Vec3::Y * (wc.suspension_length - ws.compression);
        let rot = iso.rot * Quat::from_rotation_y(ws.steer_angle) * Quat::from_rotation_x(ws.spin);
        Iso::new(iso.transform_point(center), rot)
    }

    fn torque_at(&self, rpm: f32) -> f32 {
        let c = &self.config;
        let x = (rpm - c.peak_torque_rpm) / (c.redline_rpm - c.idle_rpm).max(1.0);
        (c.peak_torque * (1.0 - 1.6 * x * x)).max(c.peak_torque * 0.3)
    }

    /// Applies suspension, tire, engine and aero forces. Call before each
    /// physics step.
    pub fn update(&mut self, world: &mut PhysicsWorld, dt: f32) {
        let Some(body) = world.body(self.body) else { return };
        let iso = body.iso();
        let mass = body.mass();
        let linvel = body.linvel;
        let up = iso.rot * Vec3::Y;
        let fwd = iso.rot * Vec3::Z;
        let speed = linvel.dot(fwd);
        let c = self.config.clone();
        let ctl = self.controls;
        let filter = QueryFilter::exclude(self.body).with_groups(CollisionGroups::new(layers::VEHICLE, !layers::DEBRIS & !layers::TRIGGER));

        // Steering.
        let max_steer = c.max_steer_deg.to_radians() / (1.0 + c.steer_speed_falloff * speed.abs());
        let target = -ctl.steer.clamp(-1.0, 1.0) * max_steer;

        // Gearbox.
        self.shift_timer = (self.shift_timer - dt).max(0.0);
        let wants_reverse = ctl.throttle < -0.05 && speed < 1.0 && c.reverse_ratio > 0.0;
        if wants_reverse && self.gear >= 0 && speed > -0.5 {
            self.gear = -1;
        } else if !wants_reverse && self.gear < 0 && ctl.throttle > 0.05 {
            self.gear = 1;
        }
        let ratio = match self.gear {
            g if g < 0 => c.reverse_ratio,
            0 => 0.0,
            g => c.gear_ratios[(g as usize - 1).min(c.gear_ratios.len() - 1)],
        };
        let avg_radius = c.wheels.iter().map(|w| w.radius).sum::<f32>() / c.wheels.len().max(1) as f32;
        let wheel_rpm = speed.abs() / avg_radius * 60.0 / std::f32::consts::TAU;
        self.rpm = (wheel_rpm * ratio * c.final_drive).clamp(c.idle_rpm, c.redline_rpm);
        if self.gear > 0 && self.shift_timer == 0.0 {
            if self.rpm > 0.92 * c.redline_rpm && (self.gear as usize) < c.gear_ratios.len() {
                self.gear += 1;
                self.shift_timer = c.shift_time;
            } else if self.rpm < 0.45 * c.redline_rpm && self.gear > 1 {
                self.gear -= 1;
                self.shift_timer = c.shift_time;
            }
        }
        let drive_input = if self.gear < 0 { (-ctl.throttle).max(0.0) } else { ctl.throttle.max(0.0) };
        let braking = (ctl.brake + if self.gear >= 0 && ctl.throttle < 0.0 && speed > 1.0 { -ctl.throttle } else { 0.0 }).min(1.0);
        let engine_torque = if self.shift_timer > 0.0 || self.rpm >= c.redline_rpm { 0.0 } else { self.torque_at(self.rpm) * drive_input };
        let wheel_torque = engine_torque * ratio * c.final_drive * c.drivetrain_efficiency * if self.gear < 0 { -1.0 } else { 1.0 };
        let driven = c.wheels.iter().filter(|w| w.drive).count().max(1) as f32;

        // Suspension rays.
        let mut grounded = 0;
        for (i, wc) in c.wheels.iter().enumerate() {
            let origin = iso.transform_point(wc.position);
            let max_len = wc.suspension_length + wc.radius;
            let ws = &mut self.wheels[i];
            ws.steer_angle += (if wc.steer { target } else { 0.0 } - ws.steer_angle) * (c.steer_rate * dt).min(1.0);
            match world.raycast(&Ray::new(origin, -up), max_len, filter) {
                Some(hit) => {
                    let prev = if ws.grounded { ws.compression } else { 0.0 };
                    ws.compression = (max_len - hit.t).clamp(0.0, wc.suspension_length);
                    let comp_vel = (ws.compression - prev) / dt;
                    let damping = if comp_vel > 0.0 { c.suspension_compression_damping } else { c.suspension_rebound_damping };
                    ws.load = (c.suspension_stiffness * ws.compression + damping * comp_vel).max(0.0);
                    ws.grounded = true;
                    ws.contact_point = hit.point;
                    ws.contact_normal = hit.normal;
                    grounded += 1;
                }
                None => {
                    ws.grounded = false;
                    ws.compression = 0.0;
                    ws.load = 0.0;
                }
            }
        }
        for &(l, r) in &self.anti_roll_pairs {
            let (wl, wr) = (self.wheels[l], self.wheels[r]);
            if wl.grounded && wr.grounded {
                let f = (wl.compression - wr.compression) * c.anti_roll;
                self.wheels[l].load = (wl.load + f).max(0.0);
                self.wheels[r].load = (wr.load - f).max(0.0);
            }
        }

        let share = mass / grounded.max(1) as f32;
        let peak_slip = c.peak_slip_deg.to_radians();
        let body = world.body_mut(self.body).expect("checked above");
        for (i, wc) in c.wheels.iter().enumerate() {
            let ws = &mut self.wheels[i];
            if !ws.grounded {
                ws.spin_rate = ws.spin_rate * (1.0 - dt) + if wc.drive { drive_input * 40.0 * dt } else { 0.0 };
                ws.spin += ws.spin_rate * dt;
                ws.sliding = false;
                continue;
            }
            let n = ws.contact_normal;
            body.apply_force_at_point(up * ws.load, ws.contact_point);
            let wheel_fwd = iso.rot * (Quat::from_rotation_y(ws.steer_angle) * Vec3::Z);
            let f = (wheel_fwd - n * wheel_fwd.dot(n)).normalize_or(fwd);
            let s = f.cross(n);
            let v = body.velocity_at_point(ws.contact_point);
            let (v_long, v_lat) = (v.dot(f), v.dot(s));
            let handbrake = ctl.handbrake && wc.handbrake;
            let grip_scale = if handbrake { c.handbrake_grip } else { 1.0 };

            // Lateral: grip curve, limited to what stops the slide this step.
            let slip = v_lat.atan2(v_long.abs().max(0.5));
            ws.slip_angle = slip;
            let a = slip.abs();
            let mu = if a < peak_slip {
                c.grip * a / peak_slip
            } else {
                c.grip * (1.0 + (c.slide_grip - 1.0) * ((a - peak_slip) / (peak_slip * 3.0)).min(1.0))
            } * grip_scale;
            let stop_lat = share * v_lat.abs() / dt;
            let f_lat = -v_lat.signum() * (mu * ws.load).min(stop_lat);

            // Longitudinal: drive, brakes, rolling resistance.
            let mut f_long = if wc.drive { wheel_torque / driven / wc.radius } else { 0.0 };
            let brake = braking * c.brake_force / c.wheels.len() as f32 + if handbrake { c.handbrake_force } else { 0.0 };
            let stop_long = share * v_long.abs() / dt;
            f_long -= v_long.signum() * brake.min(stop_long);
            f_long -= v_long * c.rolling_resistance;

            // Friction circle.
            let limit = c.grip * grip_scale * ws.load;
            let total = (f_long * f_long + f_lat * f_lat).sqrt();
            let (mut fl, mut ft) = (f_long, f_lat);
            ws.sliding = total > limit * 0.98 || a > peak_slip * 1.5;
            if total > limit && total > 0.0 {
                let k = limit / total;
                fl *= k;
                ft *= k;
            }
            let com = body.com();
            let at = ws.contact_point + up * (up.dot(com - ws.contact_point) * c.force_height);
            body.apply_force_at_point(f * fl + s * ft, at);

            let roll_rate = v_long / wc.radius;
            // Wheelspin when the drive force exceeds traction.
            ws.spin_rate = if wc.drive && total > limit && drive_input > 0.1 { roll_rate + drive_input * 30.0 } else { roll_rate };
            if handbrake {
                ws.spin_rate = 0.0;
            }
            ws.spin += ws.spin_rate * dt;
        }

        // Aero.
        let v2 = linvel.length_squared();
        body.apply_force(-linvel * linvel.length() * c.drag - up * c.downforce * v2);

        // Lean controller for two-wheelers.
        if let Some(bike) = &c.bike {
            let right = -(iso.rot * Vec3::X);
            let roll = -right.dot(Vec3::Y).clamp(-1.0, 1.0).asin();
            let wheelbase = (c.wheels[0].position.z - c.wheels[c.wheels.len() - 1].position.z).abs().max(0.5);
            let steer = -self.wheels.iter().map(|w| w.steer_angle).fold(0.0, |a: f32, b| if b.abs() > a.abs() { b } else { a });
            let lat_acc = speed * speed * steer.tan() / wheelbase;
            let max_lean = bike.max_lean_deg.to_radians();
            let target_roll = if speed.abs() > 1.5 { (lat_acc / 9.81).atan().clamp(-max_lean, max_lean) } else { 0.0 };
            let roll_rate = body.angvel.dot(fwd);
            let inertia = body.inertia_world();
            let roll_inertia = fwd.dot(inertia * fwd);
            let w = bike.lean_frequency;
            let torque = (w * w * (target_roll - roll) - 2.0 * bike.lean_damping_ratio * w * roll_rate) * roll_inertia;
            body.apply_torque(fwd * torque);
            // Damp pitch so the bike neither wheelies nor tumbles in the air.
            let pitch_rate = body.angvel.dot(right);
            let pitch_gain = if grounded == 0 { 2.0 } else { 6.0 };
            body.apply_torque(-right * pitch_rate * right.dot(inertia * right) * pitch_gain);
        }
    }
}
