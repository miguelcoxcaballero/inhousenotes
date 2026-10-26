//! Data-driven hitscan weapons (`data/weapons/*.toml`, `[[weapon]]` tables).

use std::collections::HashMap;
use std::path::Path;

use forja_math::{Quat, Ray, Vec3, orthonormal_basis};
use forja_physics::{BodyHandle, ColliderHandle, CollisionGroups, PhysicsWorld, QueryFilter, layers};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponCategory {
    Melee,
    Pistol,
    Smg,
    Shotgun,
    Rifle,
    Sniper,
    Heavy,
    Thrown,
}

impl WeaponCategory {
    /// Order of the weapon wheel.
    pub const WHEEL: [WeaponCategory; 8] = [
        WeaponCategory::Melee,
        WeaponCategory::Pistol,
        WeaponCategory::Smg,
        WeaponCategory::Shotgun,
        WeaponCategory::Rifle,
        WeaponCategory::Sniper,
        WeaponCategory::Heavy,
        WeaponCategory::Thrown,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireMode {
    Semi,
    Burst,
    Auto,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeaponDef {
    pub id: String,
    pub name: String,
    pub category: WeaponCategory,
    pub mode: FireMode,
    #[serde(default = "three")]
    pub burst: u32,
    pub damage: f32,
    /// Rounds per minute.
    pub rpm: f32,
    pub magazine: u32,
    pub reload_time: f32,
    /// Ammo item consumed on reload (`None` = infinite, e.g. melee).
    #[serde(default)]
    pub ammo: Option<String>,
    pub range: f32,
    /// Damage starts falling off at this distance, down to `falloff_min`.
    pub falloff_start: f32,
    #[serde(default = "half")]
    pub falloff_min: f32,
    /// Cone half-angle in degrees when hip firing / aiming.
    pub spread_hip_deg: f32,
    pub spread_aim_deg: f32,
    /// Extra spread per shot and recovery per second.
    #[serde(default)]
    pub bloom_deg: f32,
    #[serde(default = "five")]
    pub bloom_recovery: f32,
    /// Camera kick per shot (degrees up).
    #[serde(default)]
    pub recoil_deg: f32,
    #[serde(default = "one_u")]
    pub pellets: u32,
    /// Impulse applied to dynamic bodies per hit (N·s).
    #[serde(default = "impulse")]
    pub impulse: f32,
    #[serde(default)]
    pub model: Option<String>,
}

fn three() -> u32 {
    3
}
fn half() -> f32 {
    0.5
}
fn five() -> f32 {
    5.0
}
fn one_u() -> u32 {
    1
}
fn impulse() -> f32 {
    40.0
}

#[derive(Clone, Debug, Default)]
pub struct WeaponRegistry {
    defs: HashMap<String, WeaponDef>,
}

impl WeaponRegistry {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let defs: Vec<WeaponDef> = crate::load_dir(dir, "weapon")?;
        for d in &defs {
            if d.rpm <= 0.0 || d.magazine == 0 || d.range <= 0.0 {
                return Err(format!("weapon {}: rpm, magazine and range must be positive", d.id));
            }
        }
        Ok(Self::from_defs(defs))
    }

    pub fn from_defs(defs: Vec<WeaponDef>) -> Self {
        Self { defs: defs.into_iter().map(|d| (d.id.clone(), d)).collect() }
    }

    pub fn get(&self, id: &str) -> Option<&WeaponDef> {
        self.defs.get(id)
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub collider: ColliderHandle,
    pub body: BodyHandle,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
    pub damage: f32,
}

/// Runtime state of an equipped weapon.
#[derive(Clone, Debug)]
pub struct WeaponState {
    pub def: WeaponDef,
    pub in_mag: u32,
    cooldown: f32,
    reloading: f32,
    burst_left: u32,
    trigger_was_down: bool,
    bloom: f32,
    rng: u64,
}

impl WeaponState {
    pub fn new(def: WeaponDef) -> Self {
        let in_mag = def.magazine;
        Self { def, in_mag, cooldown: 0.0, reloading: 0.0, burst_left: 0, trigger_was_down: false, bloom: 0.0, rng: 0x9E37_79B9_7F4A_7C15 }
    }

    pub fn is_reloading(&self) -> bool {
        self.reloading > 0.0
    }

    pub fn reload_progress(&self) -> f32 {
        if self.reloading > 0.0 { 1.0 - self.reloading / self.def.reload_time } else { 1.0 }
    }

    /// Starts a reload if the magazine is not full and ammo is available.
    pub fn start_reload(&mut self, reserve: u32) -> bool {
        if self.reloading > 0.0 || self.in_mag >= self.def.magazine || (self.def.ammo.is_some() && reserve == 0) {
            return false;
        }
        self.reloading = self.def.reload_time;
        true
    }

    /// Current cone half-angle in radians.
    pub fn spread(&self, aiming: bool) -> f32 {
        (if aiming { self.def.spread_aim_deg } else { self.def.spread_hip_deg } + self.bloom).to_radians()
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Random direction inside a cone (uniform over the cap).
    fn spread_dir(&mut self, dir: Vec3, half_angle: f32) -> Vec3 {
        let cos_max = half_angle.cos();
        let z = 1.0 - self.rand() * (1.0 - cos_max);
        let phi = self.rand() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        let (t, b) = orthonormal_basis(dir);
        (t * (r * phi.cos()) + b * (r * phi.sin()) + dir * z).normalize()
    }

    /// Advances timers. `reserve` is the ammo available for a finishing
    /// reload; returns rounds taken from the reserve.
    pub fn tick(&mut self, dt: f32, reserve: u32) -> u32 {
        // Keep the remainder (bounded by one tick) so fire rates are exact
        // instead of rounding up to whole ticks.
        self.cooldown = (self.cooldown - dt).max(-dt);
        self.bloom = (self.bloom - self.def.bloom_recovery * dt).max(0.0);
        if self.reloading > 0.0 {
            self.reloading -= dt;
            if self.reloading <= 0.0 {
                self.reloading = 0.0;
                let need = self.def.magazine - self.in_mag;
                let take = if self.def.ammo.is_some() { need.min(reserve) } else { need };
                self.in_mag += take;
                return if self.def.ammo.is_some() { take } else { 0 };
            }
        }
        0
    }

    /// Handles the trigger and fires due shots through the physics world.
    /// Returns hits and the number of shots fired this call.
    pub fn trigger(
        &mut self,
        physics: &mut PhysicsWorld,
        trigger_down: bool,
        aiming: bool,
        origin: Vec3,
        dir: Vec3,
        shooter: Option<BodyHandle>,
    ) -> (Vec<Hit>, u32) {
        let pressed = trigger_down && !self.trigger_was_down;
        self.trigger_was_down = trigger_down;
        let mut hits = Vec::new();
        let mut shots = 0u32;
        if pressed && self.def.mode == FireMode::Burst {
            self.burst_left = self.def.burst;
        }
        let wants = match self.def.mode {
            FireMode::Semi => pressed,
            FireMode::Burst => self.burst_left > 0,
            FireMode::Auto => trigger_down,
        };
        let interval = 60.0 / self.def.rpm;
        if !wants && self.cooldown < 0.0 {
            self.cooldown = 0.0;
        }
        // Several shots per call when the fire rate exceeds the tick rate.
        let mut fired = false;
        while wants && self.cooldown <= 1e-6 && self.reloading <= 0.0 && self.in_mag > 0 && shots < 8 {
            if self.def.mode == FireMode::Semi && fired {
                break;
            }
            if self.def.mode == FireMode::Burst && self.burst_left == 0 {
                break;
            }
            fired = true;
            self.cooldown += interval;
            self.in_mag -= 1;
            self.burst_left = self.burst_left.saturating_sub(1);
            shots += 1;
            let spread = self.spread(aiming);
            let filter = QueryFilter {
                exclude_body: shooter,
                groups: CollisionGroups::new(u32::MAX, !layers::TRIGGER),
                ..Default::default()
            };
            for _ in 0..self.def.pellets.max(1) {
                let d = self.spread_dir(dir.normalize_or(Vec3::Z), spread);
                if let Some(h) = physics.raycast(&Ray::new(origin, d), self.def.range, filter) {
                    let k = ((h.t - self.def.falloff_start) / (self.def.range - self.def.falloff_start).max(1e-3)).clamp(0.0, 1.0);
                    let damage = self.def.damage * (1.0 + (self.def.falloff_min - 1.0) * k);
                    if let Some(b) = physics.body_mut(h.body)
                        && b.is_dynamic()
                    {
                        b.apply_impulse_at_point(d * self.def.impulse / self.def.pellets.max(1) as f32, h.point);
                    }
                    hits.push(Hit { collider: h.collider, body: h.body, point: h.point, normal: h.normal, distance: h.t, damage });
                }
            }
            self.bloom += self.def.bloom_deg;
        }
        if self.def.mode == FireMode::Burst && self.in_mag == 0 {
            self.burst_left = 0;
        }
        (hits, shots)
    }

    /// Recoil rotation to apply to the aim (pitch up), randomised sideways.
    pub fn recoil_kick(&mut self) -> Quat {
        let side = (self.rand() - 0.5) * 0.4;
        Quat::from_euler(forja_math::EulerRot::YXZ, (self.def.recoil_deg * side).to_radians(), self.def.recoil_deg.to_radians(), 0.0)
    }
}
