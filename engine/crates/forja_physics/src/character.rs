//! Kinematic capsule character controller: collide-and-slide with
//! depenetration, step climbing, slope limits, ground snapping and pushing
//! dynamic props.

use forja_math::{Iso, Ray, Vec3};

use crate::body::{BodyHandle, ColliderDesc, CollisionGroups, RigidBodyDesc, layers};
use crate::shape::Shape;
use crate::world::{PhysicsWorld, QueryFilter};

#[derive(Clone, Debug)]
pub struct CharacterConfig {
    pub radius: f32,
    /// Half length of the capsule's inner segment. Total height is
    /// `2 * (half_height + radius)`.
    pub half_height: f32,
    pub step_height: f32,
    pub max_slope_deg: f32,
    pub skin: f32,
    pub jump_speed: f32,
    /// Force per m/s of approach applied to dynamic bodies we walk into.
    pub push_force: f32,
    pub snap_distance: f32,
}

impl Default for CharacterConfig {
    fn default() -> Self {
        Self {
            radius: 0.35,
            half_height: 0.55,
            step_height: 0.4,
            max_slope_deg: 50.0,
            skin: 0.02,
            jump_speed: 5.5,
            push_force: 600.0,
            snap_distance: 0.3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharacterState {
    pub grounded: bool,
    pub ground_normal: Vec3,
    pub hit_wall: bool,
}

pub struct CharacterController {
    pub config: CharacterConfig,
    /// Capsule center.
    pub position: Vec3,
    pub velocity: Vec3,
    pub state: CharacterState,
    /// Kinematic proxy so other bodies and ray casts see the character.
    pub body: BodyHandle,
}

impl CharacterController {
    pub fn new(world: &mut PhysicsWorld, config: CharacterConfig, position: Vec3) -> Self {
        let body = world.add_body(RigidBodyDesc::kinematic().position(position));
        world.add_collider(
            ColliderDesc::new(Shape::capsule(config.half_height, config.radius))
                .groups(CollisionGroups::new(layers::CHARACTER, u32::MAX)),
            Some(body),
        );
        Self { config, position, velocity: Vec3::ZERO, state: CharacterState::default(), body }
    }

    fn shape(&self) -> Shape {
        Shape::capsule(self.config.half_height, self.config.radius)
    }

    fn filter(&self) -> QueryFilter {
        QueryFilter::exclude(self.body).with_groups(CollisionGroups::new(layers::CHARACTER, !(layers::DEBRIS | layers::TRIGGER)))
    }

    /// Feet position (bottom of the capsule).
    pub fn feet(&self) -> Vec3 {
        self.position - Vec3::Y * (self.config.half_height + self.config.radius)
    }

    pub fn teleport(&mut self, world: &mut PhysicsWorld, position: Vec3) {
        self.position = position;
        self.velocity = Vec3::ZERO;
        world.set_body_iso(self.body, Iso::from_pos(position));
    }

    /// Resolves overlaps; returns (grounded, ground normal, hit wall).
    fn depenetrate(&mut self, world: &mut PhysicsWorld, horizontal: &mut Vec3) -> (bool, Vec3, bool) {
        let shape = self.shape();
        let cos_slope = self.config.max_slope_deg.to_radians().cos();
        let mut grounded = false;
        let mut ground_n = Vec3::Y;
        let mut wall = false;
        for _ in 0..4 {
            let contacts = world.shape_contacts(&shape, &Iso::from_pos(self.position), self.config.skin, self.filter());
            let mut moved = false;
            for (ch, m) in contacts {
                let sep = m.min_separation() - self.config.skin;
                // Normal points from the capsule into the obstacle.
                let push = -m.normal;
                if push.y > cos_slope {
                    grounded = true;
                    ground_n = push;
                }
                if sep >= 0.0 {
                    continue;
                }
                let pushed_body = world.collider(ch).map(|c| c.body());
                if let Some(bh) = pushed_body
                    && let Some(b) = world.body_mut(bh)
                    && b.is_dynamic()
                {
                    let approach = horizontal.dot(m.normal).max(0.0);
                    let point = m.points()[0].point;
                    b.apply_force_at_point(m.normal * approach * self.config.push_force, point);
                    // Light props should not stop the character dead.
                    if b.mass() < 60.0 {
                        continue;
                    }
                }
                if push.y > cos_slope {
                    // Walkable: resolve vertically so we do not slide downhill.
                    self.position.y += -sep / push.y.max(0.2);
                    self.velocity.y = self.velocity.y.max(0.0);
                } else {
                    self.position += push * -sep;
                    let into = horizontal.dot(-push);
                    if into > 0.0 {
                        let flat = Vec3::new(push.x, 0.0, push.z).normalize_or_zero();
                        *horizontal += flat * horizontal.dot(-flat).max(0.0);
                    }
                    if push.y < -0.3 {
                        self.velocity.y = self.velocity.y.min(0.0); // head bump
                    }
                    wall = true;
                }
                moved = true;
            }
            if !moved {
                break;
            }
        }
        (grounded, ground_n, wall)
    }

    /// Moves the character. `desired` is the horizontal velocity (m/s).
    pub fn update(&mut self, world: &mut PhysicsWorld, desired: Vec3, jump: bool, dt: f32) -> CharacterState {
        let g = world.config.gravity.y;
        let mut horizontal = Vec3::new(desired.x, 0.0, desired.z);
        if self.state.grounded && jump {
            self.velocity.y = self.config.jump_speed;
            self.state.grounded = false;
        }
        if self.state.grounded {
            self.velocity.y = self.velocity.y.max(-1.0).min(0.0);
        } else {
            self.velocity.y += g * dt;
        }
        let start = self.position;
        let motion = horizontal * dt + Vec3::Y * self.velocity.y * dt;
        let steps = ((motion.length() / (self.config.radius * 0.5)).ceil() as usize).clamp(1, 16);
        let mut state = CharacterState { grounded: false, ground_normal: Vec3::Y, hit_wall: false };
        for _ in 0..steps {
            let before = self.position;
            let hz = horizontal;
            self.position += (horizontal * dt + Vec3::Y * self.velocity.y * dt) / steps as f32;
            let (gr, n, wall) = self.depenetrate(world, &mut horizontal);
            state.grounded |= gr;
            if gr {
                state.ground_normal = n;
            }
            if wall && self.state.grounded {
                // Try to climb a step: lift, move, drop.
                let saved = self.position;
                let mut probe = before + Vec3::Y * self.config.step_height;
                let lifted_clear = world
                    .shape_contacts(&self.shape(), &Iso::from_pos(probe), 0.0, self.filter())
                    .iter()
                    .all(|(_, m)| m.min_separation() >= 0.0);
                if lifted_clear {
                    probe += hz * dt / steps as f32;
                    self.position = probe;
                    let mut h2 = hz;
                    let (_, _, wall2) = self.depenetrate(world, &mut h2);
                    if !wall2 && let Some(drop) = self.ground_distance(world, self.config.step_height + self.config.skin) {
                        self.position.y -= drop;
                        horizontal = hz;
                        state.grounded = true;
                        state.hit_wall = false;
                        continue;
                    }
                }
                self.position = saved;
            }
            state.hit_wall |= wall;
        }
        // Ground snapping while walking down slopes and stairs.
        if self.state.grounded && !state.grounded && self.velocity.y <= 0.0
            && let Some(d) = self.ground_distance(world, self.config.snap_distance)
        {
            self.position.y -= d;
            state.grounded = true;
        }
        if state.grounded && self.velocity.y < 0.0 {
            self.velocity.y = 0.0;
        }
        self.velocity = Vec3::new((self.position.x - start.x) / dt, self.velocity.y, (self.position.z - start.z) / dt);
        self.state = state;
        world.set_body_iso(self.body, Iso::from_pos(self.position));
        if let Some(b) = world.body_mut(self.body) {
            b.linvel = self.velocity;
        }
        state
    }

    /// Distance the capsule can drop before touching walkable ground.
    fn ground_distance(&self, world: &PhysicsWorld, max: f32) -> Option<f32> {
        let cos_slope = self.config.max_slope_deg.to_radians().cos();
        let bottom = self.config.half_height + self.config.radius;
        // Sample the center and a ring so we stand on ledges.
        let r = self.config.radius * 0.7;
        let offsets = [Vec3::ZERO, Vec3::X * r, -Vec3::X * r, Vec3::Z * r, -Vec3::Z * r];
        let mut best: Option<f32> = None;
        for o in offsets {
            let origin = self.position + o;
            let reach = bottom + max - if o == Vec3::ZERO { 0.0 } else { self.config.radius - (self.config.radius * self.config.radius - r * r).sqrt() };
            if let Some(hit) = world.raycast(&Ray::new(origin, Vec3::NEG_Y), reach + self.config.skin, self.filter())
                && hit.normal.y > cos_slope
            {
                let d = (hit.t - (reach - max)).max(0.0) - self.config.skin;
                best = Some(best.map_or(d, |b: f32| b.min(d)).max(0.0));
            }
        }
        best
    }
}
