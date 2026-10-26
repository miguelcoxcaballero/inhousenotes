//! Rigid bodies and colliders.

use forja_math::{Aabb, Iso, Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::arena::Handle;
use crate::shape::Shape;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BodyHandle(pub Handle);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ColliderHandle(pub Handle);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyType {
    /// Moved by forces and contacts.
    Dynamic,
    /// Moved by its velocity only; pushes dynamic bodies, ignores contacts.
    Kinematic,
    /// Never moves.
    Static,
}

#[derive(Clone, Debug)]
pub struct RigidBody {
    pub body_type: BodyType,
    pub(crate) iso: Iso,
    pub linvel: Vec3,
    pub angvel: Vec3,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_scale: f32,
    pub can_sleep: bool,
    pub user_data: u64,
    pub(crate) local_com: Vec3,
    pub(crate) com_override: Option<Vec3>,
    pub(crate) mass: f32,
    pub(crate) inv_mass: f32,
    pub(crate) inertia_local: Mat3,
    pub(crate) inv_inertia_local: Mat3,
    pub(crate) force: Vec3,
    pub(crate) torque: Vec3,
    pub(crate) sleeping: bool,
    pub(crate) sleep_time: f32,
    pub(crate) colliders: Vec<ColliderHandle>,
}

impl RigidBody {
    pub fn iso(&self) -> Iso {
        self.iso
    }

    pub fn position(&self) -> Vec3 {
        self.iso.pos
    }

    pub fn rotation(&self) -> Quat {
        self.iso.rot
    }

    pub fn mass(&self) -> f32 {
        self.mass
    }

    pub fn inv_mass(&self) -> f32 {
        self.inv_mass
    }

    pub fn local_com(&self) -> Vec3 {
        self.local_com
    }

    pub fn com(&self) -> Vec3 {
        self.iso.transform_point(self.local_com)
    }

    pub fn is_sleeping(&self) -> bool {
        self.sleeping
    }

    pub fn is_dynamic(&self) -> bool {
        self.body_type == BodyType::Dynamic
    }

    pub fn colliders(&self) -> &[ColliderHandle] {
        &self.colliders
    }

    /// World-space inverse inertia tensor.
    pub fn inv_inertia_world(&self) -> Mat3 {
        let r = Mat3::from_quat(self.iso.rot);
        r * self.inv_inertia_local * r.transpose()
    }

    pub fn inertia_world(&self) -> Mat3 {
        let r = Mat3::from_quat(self.iso.rot);
        r * self.inertia_local * r.transpose()
    }

    #[inline]
    pub fn velocity_at_point(&self, p: Vec3) -> Vec3 {
        self.linvel + self.angvel.cross(p - self.com())
    }

    pub fn wake(&mut self) {
        if self.body_type != BodyType::Static {
            self.sleeping = false;
            self.sleep_time = 0.0;
        }
    }

    pub fn set_iso(&mut self, iso: Iso) {
        self.iso = iso;
        self.wake();
    }

    pub fn set_linvel(&mut self, v: Vec3) {
        self.linvel = v;
        self.wake();
    }

    pub fn set_angvel(&mut self, w: Vec3) {
        self.angvel = w;
        self.wake();
    }

    /// Overrides the center of mass (body frame). Vehicles use this to lower it.
    pub fn set_com_override(&mut self, com: Option<Vec3>) {
        self.com_override = com;
    }

    pub fn apply_force(&mut self, f: Vec3) {
        self.force += f;
        self.wake();
    }

    pub fn apply_torque(&mut self, t: Vec3) {
        self.torque += t;
        self.wake();
    }

    pub fn apply_force_at_point(&mut self, f: Vec3, p: Vec3) {
        self.force += f;
        self.torque += (p - self.com()).cross(f);
        self.wake();
    }

    pub fn apply_impulse(&mut self, j: Vec3) {
        self.linvel += j * self.inv_mass;
        self.wake();
    }

    pub fn apply_impulse_at_point(&mut self, j: Vec3, p: Vec3) {
        self.linvel += j * self.inv_mass;
        self.angvel += self.inv_inertia_world() * (p - self.com()).cross(j);
        self.wake();
    }
}

/// Builder for [`RigidBody`].
#[derive(Clone, Debug)]
pub struct RigidBodyDesc {
    pub body_type: BodyType,
    pub iso: Iso,
    pub linvel: Vec3,
    pub angvel: Vec3,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_scale: f32,
    pub can_sleep: bool,
    pub com_override: Option<Vec3>,
    pub user_data: u64,
}

impl RigidBodyDesc {
    pub fn new(body_type: BodyType) -> Self {
        Self {
            body_type,
            iso: Iso::IDENTITY,
            linvel: Vec3::ZERO,
            angvel: Vec3::ZERO,
            linear_damping: 0.0,
            angular_damping: 0.05,
            gravity_scale: 1.0,
            can_sleep: true,
            com_override: None,
            user_data: 0,
        }
    }

    pub fn dynamic() -> Self {
        Self::new(BodyType::Dynamic)
    }

    pub fn kinematic() -> Self {
        Self::new(BodyType::Kinematic)
    }

    pub fn fixed() -> Self {
        Self::new(BodyType::Static)
    }

    pub fn position(mut self, p: Vec3) -> Self {
        self.iso.pos = p;
        self
    }

    pub fn rotation(mut self, q: Quat) -> Self {
        self.iso.rot = q;
        self
    }

    pub fn linvel(mut self, v: Vec3) -> Self {
        self.linvel = v;
        self
    }

    pub fn angvel(mut self, w: Vec3) -> Self {
        self.angvel = w;
        self
    }

    pub fn damping(mut self, linear: f32, angular: f32) -> Self {
        self.linear_damping = linear;
        self.angular_damping = angular;
        self
    }

    pub fn can_sleep(mut self, v: bool) -> Self {
        self.can_sleep = v;
        self
    }

    pub fn com(mut self, com: Vec3) -> Self {
        self.com_override = Some(com);
        self
    }

    pub fn user_data(mut self, d: u64) -> Self {
        self.user_data = d;
        self
    }

    pub(crate) fn build(self) -> RigidBody {
        RigidBody {
            body_type: self.body_type,
            iso: self.iso,
            linvel: self.linvel,
            angvel: self.angvel,
            linear_damping: self.linear_damping,
            angular_damping: self.angular_damping,
            gravity_scale: self.gravity_scale,
            can_sleep: self.can_sleep,
            user_data: self.user_data,
            local_com: self.com_override.unwrap_or(Vec3::ZERO),
            com_override: self.com_override,
            mass: 0.0,
            inv_mass: 0.0,
            inertia_local: Mat3::ZERO,
            inv_inertia_local: Mat3::ZERO,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            sleeping: false,
            sleep_time: 0.0,
            colliders: Vec::new(),
        }
    }
}

/// Collision layers. Two colliders interact when each one's `memberships`
/// intersects the other's `filter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollisionGroups {
    pub memberships: u32,
    pub filter: u32,
}

impl Default for CollisionGroups {
    fn default() -> Self {
        Self::ALL
    }
}

impl CollisionGroups {
    pub const ALL: Self = Self { memberships: u32::MAX, filter: u32::MAX };

    pub fn new(memberships: u32, filter: u32) -> Self {
        Self { memberships, filter }
    }

    #[inline]
    pub fn test(&self, o: &CollisionGroups) -> bool {
        self.memberships & o.filter != 0 && o.memberships & self.filter != 0
    }
}

/// Common layer bits used by the gameplay crates.
pub mod layers {
    pub const WORLD: u32 = 1 << 0;
    pub const DYNAMIC: u32 = 1 << 1;
    pub const VEHICLE: u32 = 1 << 2;
    pub const CHARACTER: u32 = 1 << 3;
    pub const DEBRIS: u32 = 1 << 4;
    pub const TRIGGER: u32 = 1 << 5;
}

#[derive(Clone, Debug)]
pub struct Collider {
    pub shape: Shape,
    /// Pose relative to the parent body.
    pub local: Iso,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub groups: CollisionGroups,
    pub sensor: bool,
    pub user_data: u64,
    pub(crate) body: BodyHandle,
    pub(crate) proxy: u32,
    pub(crate) world_iso: Iso,
    pub(crate) aabb: Aabb,
}

impl Collider {
    pub fn body(&self) -> BodyHandle {
        self.body
    }

    pub fn world_iso(&self) -> Iso {
        self.world_iso
    }

    pub fn aabb(&self) -> Aabb {
        self.aabb
    }
}

/// Builder for [`Collider`].
#[derive(Clone, Debug)]
pub struct ColliderDesc {
    pub shape: Shape,
    pub local: Iso,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub groups: CollisionGroups,
    pub sensor: bool,
    pub user_data: u64,
}

impl ColliderDesc {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            local: Iso::IDENTITY,
            density: 1000.0,
            friction: 0.6,
            restitution: 0.0,
            groups: CollisionGroups::ALL,
            sensor: false,
            user_data: 0,
        }
    }

    pub fn local(mut self, iso: Iso) -> Self {
        self.local = iso;
        self
    }

    pub fn translation(mut self, p: Vec3) -> Self {
        self.local.pos = p;
        self
    }

    pub fn density(mut self, d: f32) -> Self {
        self.density = d;
        self
    }

    /// Sets density so the collider weighs `mass` kilograms.
    pub fn mass(mut self, mass: f32) -> Self {
        let unit = self.shape.mass_properties(1.0).mass;
        self.density = if unit > 0.0 { mass / unit } else { 0.0 };
        self
    }

    pub fn friction(mut self, f: f32) -> Self {
        self.friction = f;
        self
    }

    pub fn restitution(mut self, r: f32) -> Self {
        self.restitution = r;
        self
    }

    pub fn groups(mut self, g: CollisionGroups) -> Self {
        self.groups = g;
        self
    }

    pub fn sensor(mut self, s: bool) -> Self {
        self.sensor = s;
        self
    }

    pub fn user_data(mut self, d: u64) -> Self {
        self.user_data = d;
        self
    }
}
