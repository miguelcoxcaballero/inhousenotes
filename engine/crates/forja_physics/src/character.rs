//! Kinematic capsule character controller.
//!
//! The capsule moves by swept collide-and-slide (Fauerby 2003, "Improved
//! Collision detection and Response"; the same scheme as PhysX's and Jolt's
//! `CharacterVirtual`): every move is a shape cast, the remaining motion is
//! projected onto the contact plane, and creases between two planes slide
//! along their intersection. On top of that:
//!
//! * **Slope limit**: walkable ground (normal within `max_slope_deg` of up)
//!   carries the character with the walking speed along the slope; steeper
//!   surfaces act as walls while walking and make it slide when airborne.
//! * **Step-up**: a wall hit while grounded tries "up, forward, down" by at
//!   most `step_height` (kerbs, stairs).
//! * **Snap-down**: while grounded and not jumping, the capsule is swept down
//!   by `snap_distance` (plus what the slope needs at the current speed) so it
//!   stays on stairs and downhill streets instead of hopping off.
//! * **Moving platforms**: standing on a kinematic or dynamic body carries
//!   the character with that body's velocity at the feet.
//! * **Step smoothing**: the vertical pops of step-up and snap-down go into a
//!   decaying visual offset ([`CharacterController::smoothed_feet`]), so the
//!   camera and the avatar glide up stairs while the collision stays exact.
//! * Dynamic props in the way receive a push proportional to the approach
//!   speed.

use forja_math::{Iso, Vec3};

use crate::body::{BodyHandle, BodyType, ColliderDesc, CollisionGroups, RigidBodyDesc, layers};
use crate::shape::Shape;
use crate::world::{PhysicsWorld, QueryFilter, ShapeHit};

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
    /// Decay rate (1/s) of the visual offset that smooths step-ups and
    /// snap-downs; 0 disables smoothing.
    pub step_smoothing: f32,
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
            step_smoothing: 14.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharacterState {
    pub grounded: bool,
    pub ground_normal: Vec3,
    pub hit_wall: bool,
    /// Body the character stands on (the city's ground body for scenery).
    pub ground_body: Option<BodyHandle>,
    /// Velocity of the ground under the feet (moving platforms).
    pub ground_velocity: Vec3,
    /// Standing on a surface steeper than the slope limit (sliding).
    pub on_steep: bool,
    /// Climbed a step this update.
    pub stepped: bool,
}

pub struct CharacterController {
    pub config: CharacterConfig,
    /// Capsule center.
    pub position: Vec3,
    pub velocity: Vec3,
    pub state: CharacterState,
    /// Kinematic proxy so other bodies and ray casts see the character.
    pub body: BodyHandle,
    /// Capsule center before the last update (render interpolation).
    pub prev_position: Vec3,
    /// Visual vertical offset absorbing step pops (see `step_smoothing`).
    pub visual_offset: f32,
    prev_visual_offset: f32,
}

impl CharacterController {
    pub fn new(world: &mut PhysicsWorld, config: CharacterConfig, position: Vec3) -> Self {
        let body = world.add_body(RigidBodyDesc::kinematic().position(position));
        world.add_collider(
            ColliderDesc::new(Shape::capsule(config.half_height, config.radius))
                .groups(CollisionGroups::new(layers::CHARACTER, u32::MAX)),
            Some(body),
        );
        Self {
            config,
            position,
            velocity: Vec3::ZERO,
            state: CharacterState { ground_normal: Vec3::Y, ..Default::default() },
            body,
            prev_position: position,
            visual_offset: 0.0,
            prev_visual_offset: 0.0,
        }
    }

    fn shape(&self) -> Shape {
        Shape::capsule(self.config.half_height, self.config.radius)
    }

    fn filter(&self) -> QueryFilter {
        QueryFilter::exclude(self.body).with_groups(CollisionGroups::new(layers::CHARACTER, !(layers::DEBRIS | layers::TRIGGER)))
    }

    fn half_total(&self) -> f32 {
        self.config.half_height + self.config.radius
    }

    /// Feet position (bottom of the capsule).
    pub fn feet(&self) -> Vec3 {
        self.position - Vec3::Y * self.half_total()
    }

    /// Feet with the step smoothing applied (for the avatar and camera).
    pub fn smoothed_feet(&self) -> Vec3 {
        self.feet() + Vec3::Y * self.visual_offset
    }

    /// Smoothed feet interpolated between the last two fixed updates
    /// (`alpha` is `Time::alpha`).
    pub fn interpolated_feet(&self, alpha: f32) -> Vec3 {
        let a = alpha.clamp(0.0, 1.0);
        let prev = self.prev_position - Vec3::Y * (self.half_total() - self.prev_visual_offset);
        prev.lerp(self.smoothed_feet(), a)
    }

    /// Capsule centre for rendering: interpolated and step-smoothed.
    pub fn interpolated_position(&self, alpha: f32) -> Vec3 {
        self.interpolated_feet(alpha) + Vec3::Y * self.half_total()
    }

    /// Places the capsule without collision or interpolation (riding in a
    /// vehicle); the caller moves the kinematic proxy.
    pub fn carry_to(&mut self, position: Vec3) {
        self.position = position;
        self.prev_position = position;
        self.visual_offset = 0.0;
        self.prev_visual_offset = 0.0;
    }

    pub fn teleport(&mut self, world: &mut PhysicsWorld, position: Vec3) {
        self.position = position;
        self.prev_position = position;
        self.velocity = Vec3::ZERO;
        self.visual_offset = 0.0;
        self.prev_visual_offset = 0.0;
        self.state = CharacterState { ground_normal: Vec3::Y, ..Default::default() };
        world.set_body_iso(self.body, Iso::from_pos(position));
    }

    fn cast(&self, world: &PhysicsWorld, from: Vec3, dir: Vec3, dist: f32) -> Option<ShapeHit> {
        world.cast_shape(&self.shape(), &Iso::from_pos(from), dir, dist, self.filter())
    }

    fn walkable(&self, n: Vec3) -> bool {
        n.y >= self.config.max_slope_deg.to_radians().cos()
    }

    /// Pushes a dynamic body we walked into.
    fn push(&self, world: &mut PhysicsWorld, hit: &ShapeHit, motion: Vec3, dt: f32) {
        if let Some(b) = world.body_mut(hit.body)
            && b.is_dynamic()
        {
            let approach = (motion / dt).dot(-hit.normal).max(0.0);
            b.apply_force_at_point(-hit.normal * approach * self.config.push_force, hit.point);
        }
    }

    /// Swept move with sliding. Returns the end position and what was hit.
    fn slide(&self, world: &mut PhysicsWorld, mut pos: Vec3, motion: Vec3, grounded: bool, dt: f32, st: &mut CharacterState, vy: &mut f32) -> Vec3 {
        let skin = self.config.skin;
        let mut remaining = motion;
        let mut planes: [Vec3; 3] = [Vec3::ZERO; 3];
        let mut np = 0;
        let mut tried_step = false;
        for _ in 0..4 {
            let len = remaining.length();
            if len < 1e-5 {
                break;
            }
            let dir = remaining / len;
            let Some(hit) = self.cast(world, pos, dir, len + skin) else {
                pos += remaining;
                break;
            };
            let travel = (hit.toi - skin).max(0.0).min(len);
            pos += dir * travel;
            remaining = dir * (len - travel);
            self.push(world, &hit, motion, dt);
            let mut n = hit.normal;
            let support = if self.walkable(n) { Some(n) } else if grounded { self.support(world, &hit, pos) } else { None };
            if support.is_none() && grounded && !tried_step && n.y < 0.7 {
                tried_step = true;
                let horiz = Vec3::new(remaining.x, 0.0, remaining.z);
                if let Some(p) = self.step_up(world, pos, horiz) {
                    st.stepped = true;
                    pos = p;
                    break;
                }
            }
            if let Some(ground) = support {
                st.grounded = true;
                st.ground_normal = ground;
                st.ground_body = Some(hit.body);
                *vy = vy.max(0.0);
                // Roll over supported edges (kerbs, stair noses) keeping the
                // walking speed: project, then restore the horizontal length.
                let h0 = Vec3::new(remaining.x, 0.0, remaining.z).length();
                remaining -= n * remaining.dot(n);
                let h1 = Vec3::new(remaining.x, 0.0, remaining.z).length();
                if h1 > 1e-6 && n.dot(ground) < 0.999 {
                    remaining *= (h0 / h1).min(3.0);
                }
                if np < planes.len() {
                    planes[np] = n;
                    np += 1;
                }
                continue;
            }
            st.hit_wall = true;
            if grounded && n.y > 0.0 {
                // Steep slope while walking: a wall, not a ramp.
                n = Vec3::new(n.x, 0.0, n.z).normalize_or(n);
            }
            if n.y < -0.3 {
                *vy = vy.min(0.0); // head bump
            }
            remaining -= n * remaining.dot(n);
            for p in &planes[..np] {
                if remaining.dot(*p) < -1e-6 {
                    // Crease: slide along the line where both planes meet.
                    let crease = p.cross(n).normalize_or_zero();
                    remaining = crease * remaining.dot(crease);
                }
            }
            if np < planes.len() {
                planes[np] = n;
                np += 1;
            }
        }
        pos
    }

    /// Ground support for a contact that is too steep by its normal: the
    /// rounded bottom of the capsule resting on an edge (kerb, stair nose)
    /// whose top face is walkable. Probes the face just beyond the contact
    /// point and returns its normal.
    fn support(&self, world: &PhysicsWorld, hit: &ShapeHit, pos: Vec3) -> Option<Vec3> {
        let feet = pos.y - self.half_total();
        let rel = hit.point - pos;
        if hit.point.y > feet + self.config.step_height + 0.02 || rel.y > -self.config.half_height {
            return None;
        }
        let out = Vec3::new(rel.x, 0.0, rel.z).normalize_or_zero();
        if out == Vec3::ZERO {
            return None;
        }
        let from = hit.point + out * 0.04 + Vec3::Y * 0.06;
        let ray = forja_math::Ray::new(from, Vec3::NEG_Y);
        let r = world.raycast(&ray, 0.12, self.filter())?;
        (r.t > 0.005 && self.walkable(r.normal) && (r.point.y - hit.point.y).abs() < 0.05).then_some(r.normal)
    }

    /// Up, forward, down by at most `step_height`. Returns the new position
    /// when it lands on walkable ground higher than `pos`.
    fn step_up(&self, world: &PhysicsWorld, pos: Vec3, horiz: Vec3) -> Option<Vec3> {
        let skin = self.config.skin;
        let h = self.config.step_height;
        let up = self.cast(world, pos, Vec3::Y, h + skin).map_or(h, |hit| (hit.toi - skin).max(0.0));
        if up < 0.05 {
            return None;
        }
        let p1 = pos + Vec3::Y * up;
        // Always probe at least a fraction of the radius forward so slow
        // walking still gets onto the tread.
        let len = horiz.length();
        if len < 1e-5 {
            return None;
        }
        let dir = horiz / len;
        let fwd = len.max(self.config.radius * 0.3);
        let moved = self.cast(world, p1, dir, fwd + skin).map_or(fwd, |hit| (hit.toi - skin).max(0.0));
        if moved < 0.01 {
            return None;
        }
        let p2 = p1 + dir * moved.min(len.max(0.01));
        let down = self.cast(world, p2, Vec3::NEG_Y, up + skin * 2.0)?;
        if !self.walkable(down.normal) && self.support(world, &down, p2 - Vec3::Y * (down.toi - skin).max(0.0)).is_none() {
            return None;
        }
        let p3 = p2 - Vec3::Y * (down.toi - skin).max(0.0);
        (p3.y > pos.y + 0.005).then_some(p3)
    }

    /// Moves the character. `desired` is the horizontal velocity (m/s).
    pub fn update(&mut self, world: &mut PhysicsWorld, desired: Vec3, jump: bool, dt: f32) -> CharacterState {
        let cfg = self.config.clone();
        let g = world.config.gravity.y;
        self.prev_position = self.position;
        self.prev_visual_offset = self.visual_offset;
        let was_grounded = self.state.grounded;
        let mut st = CharacterState { ground_normal: Vec3::Y, ..Default::default() };
        // Ride the platform we stand on.
        let mut carry = Vec3::ZERO;
        if was_grounded
            && let Some(gb) = self.state.ground_body
            && let Some(b) = world.body(gb)
            && b.body_type != BodyType::Static
        {
            carry = b.velocity_at_point(self.feet());
        }
        let mut vy = self.velocity.y;
        let jumping = was_grounded && jump;
        if jumping {
            vy = cfg.jump_speed + carry.y.max(0.0);
        } else if was_grounded {
            vy = 0.0;
        } else {
            vy += g * dt;
        }
        let horizontal = Vec3::new(desired.x, 0.0, desired.z);
        let start = self.position;
        let mut pos = self.position;
        // Platform motion first (it is not ours to slide).
        if carry != Vec3::ZERO {
            let c = Vec3::new(carry.x, if jumping { 0.0 } else { carry.y }, carry.z) * dt;
            pos = self.slide(world, pos, c, false, dt, &mut CharacterState::default(), &mut 0.0);
        }
        // Walking motion follows walkable ground; gravity only in the air.
        let mut walk = horizontal * dt;
        if was_grounded && !jumping && self.walkable(self.state.ground_normal) {
            let n = self.state.ground_normal;
            let along = walk - n * walk.dot(n);
            walk = along.normalize_or_zero() * walk.length();
        }
        let motion = walk + Vec3::Y * vy * dt;
        // Long moves are split so a fall or a sprint never skips a step check.
        let n_sub = ((motion.length() / (cfg.radius * 1.5)).ceil() as usize).clamp(1, 8);
        for _ in 0..n_sub {
            pos = self.slide(world, pos, motion / n_sub as f32, was_grounded && !jumping, dt, &mut st, &mut vy);
        }
        // Snap down / ground probe.
        if vy <= 0.0 && !jumping {
            let reach = if was_grounded || st.stepped {
                cfg.snap_distance + horizontal.length() * dt * cfg.max_slope_deg.to_radians().tan()
            } else {
                cfg.skin * 2.0
            };
            if let Some(hit) = self.cast(world, pos, Vec3::NEG_Y, reach + cfg.skin) {
                let landed = pos - Vec3::Y * (hit.toi - cfg.skin).max(0.0);
                let support = if self.walkable(hit.normal) { Some(hit.normal) } else { self.support(world, &hit, landed) };
                if let Some(ground) = support {
                    pos = landed;
                    st.grounded = true;
                    st.ground_normal = ground;
                    st.ground_body = Some(hit.body);
                    vy = 0.0;
                } else if hit.toi <= cfg.skin * 3.0 {
                    st.on_steep = true;
                    st.grounded = false;
                }
            }
        }
        if st.grounded && !self.walkable(st.ground_normal) {
            st.grounded = false;
            st.on_steep = true;
        }
        // Residual overlaps (other bodies moved into us).
        pos = self.depenetrate(world, pos);
        if let Some(gb) = st.ground_body
            && let Some(b) = world.body(gb)
            && b.body_type != BodyType::Static
        {
            st.ground_velocity = b.velocity_at_point(pos - Vec3::Y * self.half_total());
        }
        // Step smoothing: hide the vertical pops of step-ups and snaps.
        if cfg.step_smoothing > 0.0 && was_grounded && st.grounded && !jumping {
            // Vertical motion beyond what walking along the ground explains.
            let pop = (pos.y - start.y) - walk.y - carry.y * dt;
            if pop.abs() > 0.02 && pop.abs() <= cfg.step_height + cfg.snap_distance {
                self.visual_offset -= pop;
            }
        }
        self.visual_offset *= (-cfg.step_smoothing * dt).exp();
        self.visual_offset = self.visual_offset.clamp(-cfg.step_height, cfg.step_height);
        if !st.grounded || cfg.step_smoothing == 0.0 {
            self.visual_offset = 0.0;
        }
        self.position = pos;
        self.velocity = Vec3::new((pos.x - start.x) / dt, if st.grounded { 0.0 } else { vy }, (pos.z - start.z) / dt);
        self.state = st;
        world.set_body_iso(self.body, Iso::from_pos(self.position));
        if let Some(b) = world.body_mut(self.body) {
            b.linvel = self.velocity;
        }
        st
    }

    /// Pushes the capsule out of residual overlaps.
    fn depenetrate(&self, world: &PhysicsWorld, mut pos: Vec3) -> Vec3 {
        let shape = self.shape();
        for _ in 0..3 {
            let contacts = world.shape_contacts(&shape, &Iso::from_pos(pos), 0.0, self.filter());
            let mut deepest: Option<(f32, Vec3)> = None;
            for (ch, m) in &contacts {
                let sep = m.min_separation();
                if sep >= -1e-4 {
                    continue;
                }
                // Light dynamic props are pushed, not stood off.
                if world.collider(*ch).and_then(|c| world.body(c.body())).is_some_and(|b| b.is_dynamic() && b.mass() < 60.0) {
                    continue;
                }
                if deepest.is_none_or(|d| sep < d.0) {
                    deepest = Some((sep, -m.normal));
                }
            }
            let Some((sep, n)) = deepest else { break };
            if self.walkable(n) {
                pos.y += -sep / n.y.max(0.2);
            } else {
                pos += n * -sep;
            }
        }
        pos
    }
}
