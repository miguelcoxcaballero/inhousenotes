//! Raycast vehicles (cars, trucks) and two-wheelers.
//!
//! The chassis is an ordinary dynamic body. Each wheel casts rays along the
//! suspension axis (a fan across the tyre's circumference for cars, so it
//! rolls over steps and kerbs); the suspension pushes the chassis and the
//! tyre model adds longitudinal and lateral forces at the contact point.
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
//! * **Steering** is speed sensitive: full input asks for the lock the turn
//!   needs at the limit of grip plus the front tyres' peak slip angle, and
//!   the wheels follow the direction of travel (caster), so a key press
//!   cannot throw the fronts past their peak and a slide is countersteered.
//! * **Assists**: ABS, traction control and stability control (yaw-rate
//!   target from the bicycle model, single-wheel braking, throttle cut),
//!   brake bias, a limited-slip differential, tyre load sensitivity and a
//!   front/rear roll stiffness split set a nose-led balance per class.
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
    /// Anti-roll bar rate (N/m of compression difference), mean of the axles.
    pub anti_roll: f32,
    /// Share of the roll stiffness on the front axle (0.5 = even). More at
    /// the front loads the outer front tyre harder: understeer.
    pub anti_roll_front: f32,
    pub peak_torque: f32,
    pub peak_torque_rpm: f32,
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    pub gear_ratios: Vec<f32>,
    pub reverse_ratio: f32,
    pub final_drive: f32,
    pub drivetrain_efficiency: f32,
    pub shift_time: f32,
    /// Steering lock at parking speed.
    pub max_steer_deg: f32,
    /// Two-wheelers only: lock reduction with speed, `max / (1 + k v)`.
    /// Cars derive their lock from the grip (see `steer_slip`).
    pub steer_speed_falloff: f32,
    /// How fast a held key winds on lock: full input after `1 / steer_rate`
    /// seconds at a standstill, slower at speed, faster back to centre.
    pub steer_rate: f32,
    /// Front slip angle full input asks for at speed, as a multiple of
    /// `peak_slip_deg`: 1 puts the front tyres at their peak and no further,
    /// whatever the speed (speed-sensitive steering).
    pub steer_slip: f32,
    /// 0..1: share of the front axle's drift angle that is steered out for
    /// the driver, like the caster of a real car pulling the wheel towards
    /// the direction of travel. The input then commands front slip rather
    /// than a wheel angle: it self-centres and countersteers a slide.
    pub countersteer_assist: f32,
    /// Peak friction coefficient.
    pub grip: f32,
    /// Grip of the rear axle relative to the front (wider rear tyres: above
    /// 1 the car pushes its nose wide before the tail lets go).
    pub rear_grip: f32,
    /// Tyre load sensitivity: fraction of grip lost when a tyre carries
    /// twice its static load. Makes weight transfer cost grip, so the roll
    /// stiffness split sets the understeer balance.
    pub load_sensitivity: f32,
    /// Friction once fully sliding, as a fraction of `grip`.
    pub slide_grip: f32,
    pub peak_slip_deg: f32,
    /// Slip ratio at peak longitudinal grip.
    pub peak_slip_ratio: f32,
    /// Spin inertia of one wheel with its share of the drivetrain (kg m²).
    pub wheel_inertia: f32,
    /// Traction control (0 = off, 1 = keeps the driven tyres at their peak
    /// slip and leaves room for cornering).
    pub traction_control: f32,
    /// Anti-lock brakes (0 = off, 1 = no wheel locks).
    pub abs: f32,
    /// Stability control (0 = off, 1 = early): brakes single wheels and cuts
    /// the throttle when the yaw rate leaves what the steering asks for or
    /// the rear axle slides.
    pub stability_control: f32,
    /// Differential lock of the driven axles (0 = open, 1 = spool).
    pub diff_lock: f32,
    /// Total brake force at the tyres (N).
    pub brake_force: f32,
    /// Share of the brake force on the front axle.
    pub brake_bias: f32,
    pub handbrake_force: f32,
    /// Cornering grip the handbrake wheels keep while it is pulled, as a
    /// fraction of their free-rolling grip. A locked tyre has almost none;
    /// keeping some makes the handbrake start a slide the car comes back
    /// from instead of a spin.
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
            anti_roll_front: 0.62,
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
            steer_slip: 1.0,
            countersteer_assist: 0.85,
            grip: 1.0,
            rear_grip: 1.06,
            load_sensitivity: 0.12,
            slide_grip: 0.85,
            peak_slip_deg: 7.0,
            peak_slip_ratio: 0.12,
            wheel_inertia: 1.4,
            traction_control: 1.0,
            abs: 1.0,
            stability_control: 1.0,
            diff_lock: 0.25,
            brake_force: 20_000.0,
            brake_bias: 0.65,
            handbrake_force: 6_000.0,
            handbrake_grip: 0.45,
            drag: 0.38,
            rolling_resistance: 12.0,
            downforce: 0.6,
            force_height: 0.3,
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
        c.rear_grip = 1.0;
        c.load_sensitivity = 0.0;
        c.stability_control = 0.0;
        c.diff_lock = 0.0;
        c.brake_bias = 0.6;
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
    /// Steering input after the wind-on rate limit (-1..1).
    steer_input: f32,
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
            steer_input: 0.0,
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
    fn tyre_forces(c: &VehicleConfig, mu: f32, load: f32, kappa: f32, tan_alpha: f32, lat_scale: f32) -> (f32, f32) {
        let sx = kappa / c.peak_slip_ratio.max(1e-3);
        let sy = tan_alpha / c.peak_slip_deg.to_radians().tan().max(1e-3);
        let s = (sx * sx + sy * sy).sqrt();
        if s < 1e-6 {
            return (0.0, 0.0);
        }
        let f = mu * load * Self::magic(s, Self::shape_factor(c.slide_grip));
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
        // Axles: wheels ahead of the middle of the wheel set are the front.
        let mid_z = c.wheels.iter().map(|w| w.position.z).sum::<f32>() / c.wheels.len().max(1) as f32;
        let is_front = |w: &WheelConfig| w.position.z > mid_z;
        let n_front = c.wheels.iter().filter(|w| is_front(w)).count().max(1) as f32;
        let n_rear = (c.wheels.len() as f32 - n_front).max(1.0);
        let front_z = c.wheels.iter().filter(|w| is_front(w)).map(|w| w.position.z).sum::<f32>() / n_front;
        let rear_z = c.wheels.iter().filter(|w| !is_front(w)).map(|w| w.position.z).sum::<f32>() / n_rear;
        let wheelbase = (front_z - rear_z).abs().max(0.5);
        let left = iso.rot * Vec3::X;
        if bike_mode.is_none() {
            // Cars. A key is all or nothing, so the lock a full input gives
            // has to fit the speed: what the turn needs geometrically at the
            // limit of grip (`L a / v²`) plus the slip angle that puts the
            // front tyres at their peak, and no more. The input winds on at
            // a finite rate, slower at speed, and unwinds faster.
            let v = speed.abs();
            let s_in = ctl.steer.clamp(-1.0, 1.0);
            let centring = s_in.abs() < self.steer_input.abs() || s_in * self.steer_input < 0.0;
            let rate = c.steer_rate.max(0.1) / (1.0 + v / 45.0) * if centring { 1.8 } else { 1.0 };
            self.steer_input += (s_in - self.steer_input).clamp(-rate * dt, rate * dt);
            let lock = c.max_steer_deg.to_radians();
            let slip = (c.peak_slip_deg * c.steer_slip).to_radians().min(lock);
            let geometric = (wheelbase * c.grip * 9.81 / (v * v).max(1.0)).atan().min(lock - slip);
            // Caster: the front wheels follow the direction the front axle
            // travels in, so the input commands front slip. Hands off, the
            // wheels point down the road (self-centring); in a slide they
            // countersteer on their own.
            let t = ((speed - 4.0) / 8.0).clamp(0.0, 1.0);
            let assist = c.countersteer_assist.clamp(0.0, 1.0) * t * t * (3.0 - 2.0 * t);
            let vf = body.velocity_at_point(iso.transform_point(Vec3::new(0.0, c.com.y, front_z)));
            let drift = vf.dot(left).atan2(vf.dot(fwd).abs().max(1.0));
            // Keyboard countersteer: once the tail is out past the tyres'
            // peak, a key still held into the slide stops adding front grip
            // to the rotation (a driver would be unwinding the wheel).
            let vr = body.velocity_at_point(iso.transform_point(Vec3::new(0.0, c.com.y, rear_z)));
            let rear_slip = vr.dot(left).atan2(vr.dot(fwd).abs().max(1.0));
            let peak = c.peak_slip_deg.to_radians().max(0.01);
            let sliding = if self.steer_input * rear_slip > 0.0 { ((rear_slip.abs() - 1.4 * peak) / (2.0 * peak)).clamp(0.0, 1.0) } else { 0.0 };
            let input = self.steer_input * (1.0 - sliding * assist);
            // The slip curve is steep at first (half the peak slip already
            // gives nine tenths of the grip), so the slip asked for is the
            // curve's inverse of the input: half a stick is half the
            // cornering force, not a switch.
            let shape = Self::shape_factor(c.slide_grip);
            let slip_input = input.signum() * (input.abs().min(1.0).asin() / shape).tan() / (std::f32::consts::PI / (2.0 * shape)).tan();
            target = (-slip_input * slip - input * (1.0 - assist) * geometric + assist * drift).clamp(-lock, lock);
        }
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
            // Never more lean than the tyres can hold (`tan φ = a / g`).
            let max_lean = max_lean.min((0.85 * c.grip).atan());
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
        let steer_follow = if bike_mode.is_none() || self.rider { 1.0 } else { (c.steer_rate * dt).min(1.0) };

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
        // Pulling away, the clutch (or torque converter) slips and lets the
        // engine rev towards its torque instead of lugging at idle.
        let launching = self.gear == 1 || self.gear < 0;
        let stall_rpm = if launching { c.idle_rpm + ctl.throttle.abs().min(1.0) * (0.6 * c.peak_torque_rpm - c.idle_rpm).max(0.0) } else { c.idle_rpm };
        self.rpm = (wheel_w * to_rpm).max(road_rpm).max(stall_rpm).clamp(c.idle_rpm, c.redline_rpm);
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
        let static_load = mass * world.config.gravity.length() / c.wheels.len().max(1) as f32;
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
            // Height of the wheel centre above the ground at this lean.
            let need = (wc.radius - crown) * cos_l + crown;
            // Cars sample the tyre's circumference: rays ahead of and behind
            // the axle, each shortened by the tyre's chord there, so the
            // wheel rolls onto a kerb or a step with its leading edge like a
            // disc instead of dropping into it and meeting it as a wall.
            let samples: &[f32] = if bike_mode.is_some() { &[0.0] } else { &[0.0, 0.5, -0.5, 0.82, -0.82] };
            let roll = iso.rot * (Quat::from_rotation_y(ws.steer_angle) * Vec3::Z);
            // (distance of the tyre's lowest point below the mount, hit, under the axle)
            let mut best: Option<(f32, crate::world::RayHit, bool)> = None;
            for &k in samples {
                let dx = k * wc.radius;
                let chord = wc.radius - (wc.radius * wc.radius - dx * dx).sqrt();
                let max_len = wc.suspension_length * cos_l + need + 0.02 - chord;
                if let Some(hit) = world.raycast(&Ray::new(origin + roll * dx, dir), max_len, filter) {
                    let t = hit.t + chord;
                    if best.is_none_or(|(b, ..)| t < b - 1e-4) {
                        best = Some((t, hit, k == 0.0));
                    }
                }
            }
            match best {
                Some((t, hit, centre)) => {
                    let prev = if ws.grounded { ws.compression } else { 0.0 };
                    let raw = wc.suspension_length - (t - need) / cos_l;
                    ws.compression = raw.clamp(0.0, wc.suspension_length);
                    let comp_vel = (ws.compression - prev) / dt;
                    let damping = if comp_vel > 0.0 { c.suspension_compression_damping } else { c.suspension_rebound_damping };
                    // Dampers blow off on sharp hits (a kerb compresses the
                    // wheel within one step), and a bump stop takes over at
                    // the end of the travel so the chassis does not drop
                    // onto the road.
                    let damper = (damping * comp_vel).clamp(-2.0 * static_load, 2.5 * static_load);
                    let bump = ((raw - wc.suspension_length).max(0.0) * c.suspension_stiffness * 6.0).min(4.0 * static_load);
                    ws.load = (c.suspension_stiffness * ws.compression + damper + bump).max(0.0);
                    ws.grounded = ws.compression > 0.0 || t <= need + wc.suspension_length * cos_l;
                    // Forces act under the axle. The road's own slope counts
                    // as the contact normal; the face of a kerb or the edge
                    // the tyre is climbing does not (it would turn grip into
                    // a force against the direction of travel).
                    ws.contact_point = origin + dir * t;
                    ws.contact_normal = if centre && hit.normal.dot(up) > 0.6 { hit.normal } else { up };
                    if bike_mode.is_some() {
                        ws.contact_point = hit.point;
                        ws.contact_normal = hit.normal;
                        // The tyre touches the road under its crown, in the
                        // wheel plane: a leaning bike pivots about that line
                        // (gravity tips it), not about the point under the
                        // suspension mount.
                        let centre = origin - up * (wc.suspension_length - ws.compression);
                        let mut p = centre - up * (wc.radius - crown);
                        p.y = hit.point.y;
                        ws.contact_point = p;
                    }
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
                let share = if is_front(&c.wheels[l]) { c.anti_roll_front } else { 1.0 - c.anti_roll_front };
                let f = (wl.compression - wr.compression) * c.anti_roll * 2.0 * share.clamp(0.0, 1.0);
                self.wheels[l].load = (wl.load + f).max(0.0);
                self.wheels[r].load = (wr.load - f).max(0.0);
            }
        }

        let share = mass / grounded.max(1) as f32;
        let gravity = world.config.gravity;
        let n_braked = c.wheels.iter().zip(&self.wheels).filter(|(wc, ws)| ws.grounded && (braking > 0.0 || (ctl.handbrake && wc.handbrake))).count().max(1);
        let hold_share = mass / n_braked as f32;
        let body = world.body_mut(self.body).expect("checked above");
        let inertia = c.wheel_inertia.max(0.05);

        // Stability control (ESC). The yaw rate the steering asks for comes
        // from the bicycle model, `r = v δ / (L (1 + v²/v_ch²))`, bounded by
        // the grip (`μ g / v`). When the car yaws more than that, or its
        // rear axle slides past the tyres' peak slip, the outer front wheel
        // is braked and the throttle is cut; when it yaws less (understeer)
        // the inner rear is braked lightly. The brake torques go through
        // the tyres like the driver's, so the correction is bounded by grip.
        let mut esc_brake = vec![0.0f32; c.wheels.len()];
        let mut esc_cut = 0.0f32;
        let sc = c.stability_control.clamp(0.0, 1.0);
        if bike_mode.is_none() && sc > 0.0 && speed > 7.0 && grounded >= 3 {
            let dead_zone = |x: f32, d: f32| if x.abs() <= d { 0.0 } else { x - x.signum() * d };
            let yaw = body.angvel.dot(up);
            let steer = c.wheels.iter().zip(&self.wheels).filter(|(wc, _)| wc.steer).map(|(_, ws)| ws.steer_angle).sum::<f32>() / c.wheels.iter().filter(|w| w.steer).count().max(1) as f32;
            let r_max = c.grip * 9.81 / speed;
            let r_ref = (speed * steer.tan() / wheelbase / (1.0 + (speed / 40.0).powi(2))).clamp(-r_max, r_max);
            let err = yaw - r_ref;
            let dead = (4.0 + 8.0 * (1.0 - sc)).to_radians() + 0.1 * r_ref.abs();
            let vr = body.velocity_at_point(iso.transform_point(Vec3::new(0.0, c.com.y, rear_z)));
            let rear_slip = vr.dot(left).atan2(vr.dot(fwd).max(1.0));
            // The handbrake asks for a slide: it gets one, held at an angle
            // the car comes back from.
            let slip_ok = c.peak_slip_deg.to_radians() * (1.1 + 0.8 * (1.0 - sc) + if ctl.handbrake { 2.0 } else { 0.0 });
            let iz = up.dot(body.inertia_world() * up);
            let oversteer = err * yaw > 0.0;
            // Yaw moment wanted (about +Y, + = to the left).
            let yaw_gain = if ctl.handbrake { 0.0 } else if oversteer { 7.0 } else { 2.5 };
            let m = iz * (-yaw_gain * dead_zone(err, dead) + 30.0 * dead_zone(rear_slip, slip_ok)) * ((speed - 7.0) / 5.0).clamp(0.0, 1.0);
            if m != 0.0 {
                // Against the rotation: outer front. With it: inner rear.
                let front = m * yaw < 0.0;
                let pick = c.wheels.iter().enumerate().find(|(i, w)| is_front(w) == front && (w.position.x > 0.0) == (m > 0.0) && self.wheels[*i].grounded);
                if let Some((i, wc)) = pick {
                    let cap = c.grip * self.wheels[i].load * wc.radius * if front { 0.9 } else { 0.45 };
                    esc_brake[i] = (m.abs() / wc.position.x.abs().max(0.3) * wc.radius).min(cap);
                }
                esc_cut = (m.abs() / (iz * 2.5)).clamp(0.0, 1.0) * if front { 0.95 } else { 0.6 };
            }
        }
        let wheel_torque = wheel_torque * (1.0 - esc_cut);

        // Limited-slip differential: a clutch torque from the faster to the
        // slower wheel of each driven axle, up to a preload plus a share of
        // the drive torque (0 = open: the wheels only share the torque).
        let mut diff = vec![0.0f32; c.wheels.len()];
        if c.diff_lock > 0.0 {
            for &(l, r) in &self.anti_roll_pairs {
                if c.wheels[l].drive && c.wheels[r].drive {
                    let dw = self.wheels[l].spin_rate - self.wheels[r].spin_rate;
                    let t_max = c.diff_lock.min(1.0) * (60.0 + (wheel_torque / driven).abs());
                    let t = (0.3 * inertia * dw / dt).clamp(-t_max, t_max);
                    diff[l] -= t;
                    diff[r] += t;
                }
            }
        }

        for (i, wc) in c.wheels.iter().enumerate() {
            let ws = &mut self.wheels[i];
            let handbrake = ctl.handbrake && wc.handbrake;
            let front = is_front(wc);
            // Tyre grip: axle balance and load sensitivity.
            let mu = c.grip * if front { 1.0 } else { c.rear_grip } * (1.0 - c.load_sensitivity * (ws.load / static_load.max(1.0) - 1.0)).clamp(0.75, 1.1);
            let cap = mu * ws.load * wc.radius;
            // Last step's slip, normalised by the peaks.
            let sx = ws.slip_ratio / c.peak_slip_ratio.max(1e-3);
            let sy = ws.slip_angle.tan() / c.peak_slip_deg.to_radians().tan().max(1e-3);
            // Brakes with their front/rear bias. ABS: no more torque than the
            // tyre can hold, and less while the wheel is past its peak slip.
            let bias = if front { c.brake_bias / n_front } else { (1.0 - c.brake_bias) / n_rear };
            let mut brake_torque = braking * c.brake_force * bias.max(0.0) * wc.radius + esc_brake[i];
            let abs = if ws.grounded && speed.abs() > 2.0 { c.abs.clamp(0.0, 1.0) } else { 0.0 };
            if abs > 0.0 && brake_torque > 0.0 {
                let held = brake_torque.min(cap * (1.05 - 0.5 * (-sx - 0.9).clamp(0.0, 1.0)) + 20.0);
                brake_torque += (held - brake_torque) * abs;
            }
            if handbrake {
                brake_torque += c.handbrake_force * 2.0 * wc.radius;
            }
            // Traction control: the same on the drive side, leaving room in
            // the friction circle for the cornering the tyre is doing.
            let mut drive_torque = if wc.drive { wheel_torque / driven + diff[i] } else { 0.0 };
            let tc = if ws.grounded { c.traction_control.clamp(0.0, 1.0) } else { 0.0 };
            if tc > 0.0 && wc.drive && drive_torque != 0.0 {
                let room = (1.0 - 0.8 * sy.abs().min(1.0).powi(2)).sqrt();
                let spin = sx * drive_torque.signum();
                let held = drive_torque.abs().min(cap * room * (1.04 - 0.6 * (spin - 0.9).clamp(0.0, 1.0)) + 15.0);
                drive_torque = drive_torque.signum() * (drive_torque.abs() + (held - drive_torque.abs()) * tc);
            }
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
                let (fx, fy_now) = Self::tyre_forces(&c, mu, ws.load, kappa, tan_alpha, lat_scale);
                let eps = 0.01;
                let (fx2, _) = Self::tyre_forces(&c, mu, ws.load, kappa + eps * wc.radius / denom, tan_alpha, lat_scale);
                // dFx/dω: the tangent, but never less than the secant to
                // the free-rolling speed. Past the peak the tangent is flat
                // or negative, and a plain step would throw a light wheel
                // from one side of the slip curve to the other at low speed
                // (a limit cycle between wheelspin and lock that eats the
                // tyre's cornering grip). With the secant the road torque
                // alone can bring the wheel to free rolling, never past it.
                let slip_w = w - v_long / wc.radius;
                let secant = if slip_w.abs() > 1e-4 { (fx / slip_w).max(0.0) } else { 0.0 };
                let k = ((fx2 - fx) / eps).max(secant);
                // Drive, brake and tyre torque in one implicit step. The
                // brake opposes the rotation and can stop the wheel, not
                // reverse it; a stopped wheel stays locked while the brake
                // holds what the road and the engine put on it.
                let w0 = w;
                let free = drive_torque - fx * wc.radius;
                let against = if w0.abs() > 1e-3 { w0.signum() } else { free.signum() };
                w = w0 + h * (free - against * brake_torque) / (inertia + h * k * wc.radius);
                if brake_torque > 0.0 && (w * w0 < 0.0 || (w0.abs() <= 1e-3 && free.abs() <= brake_torque)) {
                    w = 0.0;
                }
                // The road force after the step, on the same linearisation.
                let fx = (fx + k * (w - w0)).clamp(-mu * ws.load, mu * ws.load);
                fx_sum += fx;
                fy = fy_now;
            }
            if handbrake {
                // See `handbrake_grip`: part of the free-rolling cornering
                // force, the rest of the friction circle brakes.
                let keep = c.handbrake_grip.clamp(0.0, 1.0);
                fy = keep * Self::tyre_forces(&c, mu, ws.load, 0.0, tan_alpha, 1.0).1;
                fx_sum *= (1.0 - keep * keep).sqrt();
            }
            ws.spin_rate = w;
            ws.slip_ratio = (w * wc.radius - v_long) / denom;
            let mut f_long = fx_sum / subs as f32 - v_long * c.rolling_resistance;
            let mut f_lat = fy;
            // The velocity part uses the chassis' effective mass at the
            // contact (with its rotational inertia) and is relaxed, so the
            // wheels, solved one by one, do not over-correct each other.
            let limit = mu * ws.load;
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
