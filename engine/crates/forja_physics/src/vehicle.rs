//! Raycast vehicles (cars, trucks) and two-wheelers.
//!
//! The chassis is an ordinary dynamic body. Each wheel casts a ray along the
//! suspension axis; the suspension pushes the chassis and the tyre model adds
//! longitudinal and lateral forces at the contact point.
//!
//! * **Tyres**: Pacejka's Magic Formula (Pacejka & Bakker 1992; Pacejka,
//!   *Tyre and Vehicle Dynamics*, 2012) on the normalised combined slip
//!   (slip ratio over its peak, tan of the slip angle over its peak), so
//!   braking or wheelspin reduce cornering grip and vice versa. The curve
//!   coefficients come from the handling data: `grip` is the peak friction,
//!   `slide_grip` the sliding plateau and `peak_slip_deg` / `peak_slip_ratio`
//!   where the peaks sit.
//! * **Wheels** have spin state: drive, brake and tyre torques integrate each
//!   wheel's angular velocity with implicit sub-steps (stable for stiff
//!   tyres at 60 Hz); slip ratio, wheelspin, locking and the engine's rpm
//!   follow from it.
//! * An engine torque curve with an automatic gearbox, anti-roll bars, drag
//!   and downforce complete the "arcade-sim" balance of open-world games.
//! * **Two-wheelers** have a lean model: while ridden, the rider's steering
//!   sets a target lean (`tan φ = v² κ / g`), the bike countersteers to get
//!   there and the front wheel then holds the balance steer for the current
//!   lean, so it leans into turns and never lies down; a velocity-level
//!   upright constraint keeps it stable (Sharp 1971; Limebeer & Sharp 2006
//!   for the steer-lean coupling). Parked, it drops its kickstand and rests
//!   on it; released while moving it falls over like a real bike.
//! * A vehicle at rest with idle controls falls asleep with its island.
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
    /// Front wheel steer per radian of lean error used to initiate a lean
    /// (countersteering); 0 steers straight into the turn.
    pub countersteer: f32,
    /// Fastest roll rate the rider produces (rad/s).
    pub max_roll_rate: f32,
    /// Below this speed (m/s) the rider puts a foot down and holds it upright.
    pub feet_down_speed: f32,
    /// Deploy a kickstand when parked.
    pub kickstand: bool,
    /// Lean onto the kickstand (degrees, to the left).
    pub kickstand_lean_deg: f32,
    /// Crown radius of the tyre profile (m): a leaning tyre rolls on it.
    pub tyre_crown: f32,
}

impl Default for BikeConfig {
    fn default() -> Self {
        Self {
            max_lean_deg: 45.0,
            lean_frequency: 7.0,
            lean_damping_ratio: 1.0,
            countersteer: 0.35,
            max_roll_rate: 2.5,
            feet_down_speed: 2.5,
            kickstand: true,
            kickstand_lean_deg: 11.0,
            tyre_crown: 0.09,
        }
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
    /// Slip ratio at peak longitudinal grip.
    pub peak_slip_ratio: f32,
    /// Spin inertia of one wheel with its share of the drivetrain (kg m²).
    pub wheel_inertia: f32,
    /// Traction control and ABS (0 = off, 1 = keeps the tyres near their
    /// peak slip): arcade-friendly launches and stops.
    pub traction_control: f32,
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
            peak_slip_ratio: 0.12,
            wheel_inertia: 1.4,
            traction_control: 1.0,
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
        c.wheel_inertia = 0.7;
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
    /// Longitudinal slip ratio `(ω r - v) / |v|`.
    pub slip_ratio: f32,
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
    /// Someone rides it. Bikes balance only while ridden; parked ones use
    /// their kickstand. Cars ignore it.
    pub rider: bool,
    /// Bike lean target (rad, + = right).
    pub lean_target: f32,
    stand: Option<crate::body::ColliderHandle>,
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
        Self {
            body,
            config,
            controls: VehicleControls::default(),
            wheels,
            gear: 1,
            rpm: 0.0,
            shift_timer: 0.0,
            anti_roll_pairs: pairs,
            rider: true,
            lean_target: 0.0,
            stand: None,
        }
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
        self.wheel_iso_at(b.iso(), i)
    }

    /// Wheel transform for a given chassis pose (e.g. the interpolated one).
    pub fn wheel_iso_at(&self, iso: Iso, i: usize) -> Iso {
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

    /// Magic Formula on a slip normalised so the peak sits at 1: returns the
    /// fraction of the peak force. `c` is the shape factor; the plateau at
    /// large slip is `sin(c π / 2)`.
    #[inline]
    fn magic(s: f32, c: f32) -> f32 {
        let b = (std::f32::consts::PI / (2.0 * c)).tan();
        (c * (b * s).atan()).sin()
    }

    /// Shape factor that puts the sliding plateau at `slide_grip`.
    fn shape_factor(slide_grip: f32) -> f32 {
        (2.0 - 2.0 / std::f32::consts::PI * slide_grip.clamp(0.05, 0.999).asin()).clamp(1.01, 1.99)
    }

    /// Combined-slip tyre forces (longitudinal, lateral) for a load.
    fn tyre_forces(c: &VehicleConfig, load: f32, kappa: f32, tan_alpha: f32, lat_scale: f32) -> (f32, f32) {
        let sx = kappa / c.peak_slip_ratio.max(1e-3);
        let sy = tan_alpha / c.peak_slip_deg.to_radians().tan().max(1e-3);
        let s = (sx * sx + sy * sy).sqrt();
        if s < 1e-6 {
            return (0.0, 0.0);
        }
        let f = c.grip * load * Self::magic(s, Self::shape_factor(c.slide_grip));
        (f * sx / s, -f * sy / s * lat_scale)
    }

    /// Lean (roll) angle of the chassis about its forward axis, + = right.
    pub fn lean(&self, world: &PhysicsWorld) -> f32 {
        let Some(b) = world.body(self.body) else { return 0.0 };
        let right = -(b.rotation() * Vec3::X);
        -right.y.clamp(-1.0, 1.0).asin()
    }

    fn set_kickstand(&mut self, world: &mut PhysicsWorld, down: bool) {
        match (down, self.stand) {
            (true, None) => {
                let c = &self.config;
                let Some(bike) = &c.bike else { return };
                // Foot of the stand where the ground is once the bike leans
                // `kickstand_lean_deg` to the left (+X) about the tyre contacts.
                let w = &c.wheels[c.wheels.len() - 1];
                let ground = w.position.y - w.suspension_length - w.radius + 0.06;
                let x = c.chassis_half.x + 0.12;
                let r = 0.04;
                let y = ground + x * bike.kickstand_lean_deg.to_radians().tan() + r;
                let z = (c.wheels[0].position.z + w.position.z) * 0.5 - 0.1;
                let h = world.add_collider(
                    ColliderDesc::new(Shape::sphere(r)).translation(Vec3::new(x, y, z)).density(0.0).friction(1.0).groups(CollisionGroups::new(layers::VEHICLE, u32::MAX)),
                    Some(self.body),
                );
                self.stand = Some(h);
                // Tip it towards the stand, as the rider does when parking.
                if let Some(b) = world.body_mut(self.body) {
                    let fwd = b.rotation() * Vec3::Z;
                    b.angvel -= fwd * 0.5;
                }
            }
            (false, Some(h)) => {
                world.remove_collider(h);
                self.stand = None;
            }
            _ => {}
        }
    }

    /// Applies suspension, tyre, engine and aero forces. Call before each
    /// physics step.
    pub fn update(&mut self, world: &mut PhysicsWorld, dt: f32) {
        let ctl = self.controls;
        let idle = ctl.throttle.abs() < 0.01 && ctl.steer.abs() < 0.01;
        // Bikes: kickstand down when parked and slow.
        if let Some(bike) = &self.config.bike {
            let speed = world.body(self.body).map_or(0.0, |b| b.linvel.length());
            // Down when parked slowly; stays down while it settles.
            let want = bike.kickstand && !self.rider && (speed < 1.0 || (self.stand.is_some() && speed < 4.0));
            if want != self.stand.is_some() {
                self.set_kickstand(world, want);
                if let Some(b) = world.body_mut(self.body) {
                    b.wake();
                }
            }
        }
        let Some(body) = world.body(self.body) else { return };
        if body.is_sleeping() {
            if idle {
                return;
            }
            world.body_mut(self.body).expect("checked").wake();
        }
        let body = world.body(self.body).expect("checked");
        let iso = body.iso();
        let mass = body.mass();
        let linvel = body.linvel;
        let up = iso.rot * Vec3::Y;
        let fwd = iso.rot * Vec3::Z;
        let speed = linvel.dot(fwd);
        let c = self.config.clone();
        let filter = QueryFilter::exclude(self.body).with_groups(CollisionGroups::new(layers::VEHICLE, !layers::DEBRIS & !layers::TRIGGER));
        let bike_mode = c.bike.as_ref().filter(|_| c.wheels.len() >= 2);
        let ridden_bike = bike_mode.is_some() && self.rider;

        // Steering.
        let max_steer = c.max_steer_deg.to_radians() / (1.0 + c.steer_speed_falloff * speed.abs());
        let mut target = -ctl.steer.clamp(-1.0, 1.0) * max_steer;
        let wheelbase = (c.wheels[0].position.z - c.wheels[c.wheels.len() - 1].position.z).abs().max(0.5);
        let lean = {
            let right = -(iso.rot * Vec3::X);
            -right.y.clamp(-1.0, 1.0).asin()
        };
        if let Some(bike) = bike_mode {
            // The rider's input asks for a turn; the lean that balances it is
            // the target. The front wheel holds the balance steer for the
            // current lean plus a countersteer on the lean error.
            let max_lean = bike.max_lean_deg.to_radians();
            let v = speed.abs();
            let fast = ((v - bike.feet_down_speed) / 2.0).clamp(0.0, 1.0);
            // Full input leans to `max_lean` once the bike is fast enough
            // for it to be comfortable (~12 m/s).
            let ramp = (v / 12.0).clamp(0.0, 1.0);
            self.lean_target = if self.rider { ctl.steer.clamp(-1.0, 1.0) * max_lean * ramp * fast } else { 0.0 };
            if self.rider && fast > 0.0 {
                let balance = -(wheelbase * 9.81 * lean.tan() / (v * v).max(1.0)).atan();
                // Countersteer: steer away from the turn by the balance steer
                // of the lean error (it shrinks with v², as on a real bike).
                let err = (self.lean_target - lean).clamp(-0.6, 0.6);
                let counter = bike.countersteer * (wheelbase * 9.81 * err.tan() / (v * v).max(1.0)).atan();
                let geom = (balance + counter).clamp(-max_steer, max_steer);
                target = target * (1.0 - fast) + geom * fast;
            }
        }
        let steer_follow = if bike_mode.is_some() && self.rider { 1.0 } else { (c.steer_rate * dt).min(1.0) };

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
        let driven = c.wheels.iter().filter(|w| w.drive).count().max(1) as f32;
        // Engine speed follows the driven wheels' spin (wheelspin revs it);
        // the gearbox shifts on road speed, like an automatic's output shaft.
        let wheel_w = c.wheels.iter().zip(&self.wheels).filter(|(wc, _)| wc.drive).map(|(_, ws)| ws.spin_rate.abs()).sum::<f32>() / driven;
        let avg_radius = c.wheels.iter().map(|w| w.radius).sum::<f32>() / c.wheels.len().max(1) as f32;
        let to_rpm = 60.0 / std::f32::consts::TAU * ratio.abs() * c.final_drive;
        let road_rpm = speed.abs() / avg_radius * to_rpm;
        self.rpm = (wheel_w * to_rpm).max(road_rpm).clamp(c.idle_rpm, c.redline_rpm);
        if self.gear > 0 && self.shift_timer == 0.0 {
            let next_rpm = |g: i32| speed.abs() / avg_radius * 60.0 / std::f32::consts::TAU * c.gear_ratios[(g as usize - 1).min(c.gear_ratios.len() - 1)] * c.final_drive;
            if road_rpm > 0.92 * c.redline_rpm && (self.gear as usize) < c.gear_ratios.len() {
                self.gear += 1;
                self.shift_timer = c.shift_time;
            } else if self.gear > 1 && road_rpm < 0.45 * c.redline_rpm && next_rpm(self.gear - 1) < 0.8 * c.redline_rpm {
                self.gear -= 1;
                self.shift_timer = c.shift_time;
            }
        }
        let drive_input = if self.gear < 0 { (-ctl.throttle).max(0.0) } else { ctl.throttle.max(0.0) };
        let braking = (ctl.brake + if self.gear >= 0 && ctl.throttle < 0.0 && speed > 1.0 { -ctl.throttle } else { 0.0 }).min(1.0);
        let engine_torque = if self.shift_timer > 0.0 || self.rpm >= c.redline_rpm { 0.0 } else { self.torque_at(self.rpm) * drive_input };
        let wheel_torque = engine_torque * ratio * c.final_drive * c.drivetrain_efficiency * if self.gear < 0 { -1.0 } else { 1.0 };

        // Suspension rays. Bikes cast straight down and roll on the tyre's
        // crown, so a leaning bike keeps its contact patch.
        let mut grounded = 0;
        for (i, wc) in c.wheels.iter().enumerate() {
            let origin = iso.transform_point(wc.position);
            let ws = &mut self.wheels[i];
            ws.steer_angle += (if wc.steer { target } else { 0.0 } - ws.steer_angle) * steer_follow;
            let (dir, crown) = match bike_mode {
                Some(b) => (Vec3::NEG_Y, b.tyre_crown.min(wc.radius)),
                None => (-up, wc.radius),
            };
            let cos_l = up.dot(-dir).max(0.2);
            let max_len = (wc.suspension_length * cos_l + (wc.radius - crown) * cos_l + crown) * 1.0 + 0.02;
            match world.raycast(&Ray::new(origin, dir), max_len, filter) {
                Some(hit) => {
                    let prev = if ws.grounded { ws.compression } else { 0.0 };
                    // Height of the wheel centre above the ground at this lean.
                    let need = (wc.radius - crown) * cos_l + crown;
                    ws.compression = (wc.suspension_length - (hit.t - need) / cos_l).clamp(0.0, wc.suspension_length);
                    let comp_vel = (ws.compression - prev) / dt;
                    let damping = if comp_vel > 0.0 { c.suspension_compression_damping } else { c.suspension_rebound_damping };
                    ws.load = (c.suspension_stiffness * ws.compression + damping * comp_vel).max(0.0);
                    ws.grounded = ws.compression > 0.0 || hit.t <= need + wc.suspension_length * cos_l;
                    ws.contact_point = hit.point;
                    if bike_mode.is_some() {
                        // The tyre touches the road under its crown, in the
                        // wheel plane: a leaning bike pivots about that line
                        // (gravity tips it), not about the point under the
                        // suspension mount.
                        let centre = origin - up * (wc.suspension_length - ws.compression);
                        let mut p = centre - up * (wc.radius - crown);
                        p.y = hit.point.y;
                        ws.contact_point = p;
                    }
                    ws.contact_normal = hit.normal;
                    grounded += ws.grounded as usize;
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
        let n_wheels = c.wheels.len() as f32;
        let gravity = world.config.gravity;
        let n_braked = c.wheels.iter().zip(&self.wheels).filter(|(wc, ws)| ws.grounded && (braking > 0.0 || (ctl.handbrake && wc.handbrake))).count().max(1);
        let hold_share = mass / n_braked as f32;
        let body = world.body_mut(self.body).expect("checked above");
        for (i, wc) in c.wheels.iter().enumerate() {
            let ws = &mut self.wheels[i];
            let handbrake = ctl.handbrake && wc.handbrake;
            // Traction control and ABS act on last step's slip ratio.
            // It watches the combined slip, so it also backs off the throttle
            // when the tyre is busy cornering (no power oversteer).
            let sx = ws.slip_ratio / c.peak_slip_ratio.max(1e-3);
            let sy = ws.slip_angle.tan() / c.peak_slip_deg.to_radians().tan().max(1e-3);
            let combined = (sx * sx + sy * sy).sqrt();
            let over = ((combined - 0.9) / 0.6).clamp(0.0, 0.95) * c.traction_control.clamp(0.0, 1.0);
            let tc = if ws.slip_ratio > 0.0 { 1.0 - over } else { 1.0 };
            let abs = if ws.slip_ratio < 0.0 && !handbrake { 1.0 - over } else { 1.0 };
            let brake_torque = (braking * c.brake_force / n_wheels * abs + if handbrake { c.handbrake_force * 2.0 } else { 0.0 }) * wc.radius;
            let drive_torque = if wc.drive { wheel_torque / driven * tc } else { 0.0 };
            let inertia = c.wheel_inertia.max(0.05);
            if !ws.grounded {
                // Free wheel: drive and brakes only, capped at the redline.
                let mut w = ws.spin_rate + drive_torque / inertia * dt;
                let db = brake_torque / inertia * dt;
                w = if w.abs() <= db { 0.0 } else { w - w.signum() * db };
                let max_w = c.redline_rpm / (ratio.abs() * c.final_drive).max(0.1) * std::f32::consts::TAU / 60.0;
                ws.spin_rate = w.clamp(-max_w, max_w) * (1.0 - 0.5 * dt);
                ws.spin += ws.spin_rate * dt;
                ws.sliding = false;
                ws.slip_ratio = 0.0;
                continue;
            }
            let n = ws.contact_normal;
            let support = if bike_mode.is_some() { n } else { up };
            body.add_force_at_point(support * ws.load, ws.contact_point);
            let wheel_fwd = iso.rot * (Quat::from_rotation_y(ws.steer_angle) * Vec3::Z);
            let f = (wheel_fwd - n * wheel_fwd.dot(n)).normalize_or(fwd);
            let s = f.cross(n).normalize_or(-(iso.rot * Vec3::X));
            let v = body.velocity_at_point(ws.contact_point);
            let (v_long, v_lat) = (v.dot(f), v.dot(s));
            let lat_scale = if handbrake { c.handbrake_grip } else { 1.0 };
            // Slip quantities with a low-speed floor (no singularity at rest).
            let denom = v_long.abs().max(1.5);
            let tan_alpha = v_lat / v_long.abs().max(1.5);
            ws.slip_angle = tan_alpha.atan();
            let a = ws.slip_angle.abs();

            // Wheel spin: implicit sub-steps of I dω/dt = T_drive - T_brake - Fx r.
            let subs = 4;
            let h = dt / subs as f32;
            let mut w = ws.spin_rate;
            let mut fx_sum = 0.0;
            let mut fy = 0.0;
            for _ in 0..subs {
                let kappa = (w * wc.radius - v_long) / denom;
                let (fx, fy_now) = Self::tyre_forces(&c, ws.load, kappa, tan_alpha, lat_scale);
                let eps = 0.01;
                let (fx2, _) = Self::tyre_forces(&c, ws.load, kappa + eps * wc.radius / denom, tan_alpha, lat_scale);
                let k = ((fx2 - fx) / eps).max(0.0); // dFx/dω
                let w0 = w;
                w += h * (drive_torque - fx * wc.radius) / (inertia + h * k * wc.radius);
                // The road force consistent with the implicit spin update
                // (what actually changed the wheel's angular momentum).
                let fx = (drive_torque - inertia * (w - w0) / h) / wc.radius;
                let db = brake_torque / inertia * h;
                if brake_torque > 0.0 {
                    w = if w.abs() <= db { 0.0 } else { w - w.signum() * db };
                }
                fx_sum += fx;
                fy = fy_now;
            }
            ws.spin_rate = w;
            ws.slip_ratio = (w * wc.radius - v_long) / denom;
            let mut f_long = fx_sum / subs as f32 - v_long * c.rolling_resistance;
            let mut f_lat = fy;
            // The velocity part uses the chassis' effective mass at the
            // contact (with its rotational inertia) and is relaxed, so the
            // wheels, solved one by one, do not over-correct each other.
            let limit = c.grip * ws.load;
            let low = (1.0 - v_long.abs() / 2.0).clamp(0.0, 1.0);
            let inv_i = body.inv_inertia_world();
            let r = ws.contact_point - body.com();
            let eff = |d: Vec3| {
                let rd = r.cross(d);
                let k = body.inv_mass() + rd.dot(inv_i * rd);
                // Jacobi over the grounded wheels: they share common modes
                // (roll, sway), so each corrects its part.
                if k > 0.0 { 0.8 / (k * grounded.max(1) as f32) } else { 0.0 }
            };
            // Friction never reverses the motion within a step (the slip
            // curves are stiff at low speed; explicit forces would ring).
            if f_lat * v_lat < 0.0 {
                f_lat = f_lat.signum() * f_lat.abs().min(eff(s) * v_lat.abs() / dt);
            }
            if f_long * v_long < 0.0 && drive_torque.abs() < 1.0 {
                f_long = f_long.signum() * f_long.abs().min(eff(f) * v_long.abs() / dt);
            }
            // Static grip at walking pace: the slip curves are viscous near
            // rest and would let a parked car creep down a camber or a
            // hill. Below ~2 m/s blend into a friction constraint that also
            // cancels this wheel's share of gravity, bounded by the tyre's
            // grip.
            if low > 0.0 {
                let stop = (-eff(s) * v_lat / dt - share * gravity.dot(s)).clamp(-limit, limit);
                f_lat = f_lat * (1.0 - low) + stop * lat_scale.max(0.5) * low;
            }
            // Parked and braked: hold still without creeping on slopes.
            if brake_torque > 0.0 && v_long.abs() < 0.3 && w == 0.0 {
                f_long = (-eff(f) * v_long / dt - hold_share * gravity.dot(f)).clamp(-limit, limit);
            }
            ws.sliding = ws.slip_ratio.abs() > c.peak_slip_ratio * 1.5 || a > c.peak_slip_deg.to_radians() * 1.5;
            let com = body.com();
            // Longitudinal forces act at `force_height` (arcade: no
            // wheelies or nose dives). Bikes get their cornering force at the
            // road: that is what balances gravity at the right lean
            // (tan φ = a/g), and a riderless bike pivots about its tyres (it
            // falls, or rests on its stand) instead of sliding out from
            // under itself.
            let at = ws.contact_point + up * (up.dot(com - ws.contact_point) * c.force_height);
            if bike_mode.is_some() {
                body.add_force_at_point(f * f_long, at);
                body.add_force_at_point(s * f_lat, ws.contact_point);
            } else {
                body.add_force_at_point(f * f_long + s * f_lat, at);
            }
            ws.spin += ws.spin_rate * dt;
        }

        // Aero.
        let v2 = linvel.length_squared();
        body.add_force_at_point(-linvel * linvel.length() * c.drag - up * c.downforce * v2, body.com());

        // Two-wheelers.
        if let Some(bike) = bike_mode {
            let right = -(iso.rot * Vec3::X);
            if ridden_bike {
                // Velocity-level upright constraint: drive the roll rate
                // towards the rate that closes the lean error, never faster
                // than the rider can. Unconditionally stable, unlike a stiff
                // PD torque, and it never lets a ridden bike lie down.
                let w_lean = bike.lean_frequency;
                let err = self.lean_target - lean;
                // d(lean)/dt = ω·fwd (rolling about +Z lifts the left side).
                let lean_rate = body.angvel.dot(fwd);
                let desired = (err * w_lean / (2.0 * bike.lean_damping_ratio.max(0.1))).clamp(-bike.max_roll_rate, bike.max_roll_rate);
                // The rider's strength bounds the roll acceleration (beats
                // gravity at full lean, ~8 rad/s², with margin).
                let max_dw = 30.0 * dt;
                let dw = (desired - lean_rate).clamp(-max_dw, max_dw);
                // Roll about the tyre contact line, not the centre of mass:
                // the contact patches stay put instead of being swung
                // sideways (which reads as tyre slip and spins the bike).
                let contacts: Vec<Vec3> = self.wheels.iter().filter(|w| w.grounded).map(|w| w.contact_point).collect();
                let com = body.com();
                let pivot = if contacts.is_empty() { com } else { contacts.iter().copied().sum::<Vec3>() / contacts.len() as f32 };
                body.angvel += fwd * dw;
                body.linvel += (fwd * dw).cross(com - pivot);
                // Damp pitch so the bike neither wheelies nor tumbles. The
                // pitch axis is horizontal: the body's lateral axis tilts with
                // the lean and would also damp the yaw of the turn, leaving
                // the heading behind the velocity (a spin-out).
                let pitch_axis = fwd.cross(Vec3::Y).normalize_or(right);
                let pitch_rate = body.angvel.dot(pitch_axis);
                let pitch_gain = if grounded == 0 { 2.0 } else { 6.0 };
                body.angvel -= pitch_axis * pitch_rate * (pitch_gain * dt).min(1.0);
            }
        }
    }
}
