//! Math primitives shared by every Forja crate.
//!
//! Everything is `f32` and right-handed with +Y up, matching glTF.

pub use glam::{self, EulerRot, Mat3, Mat4, Quat, Vec2, Vec3, Vec3A, Vec4, vec2, vec3, vec4};
use serde::{Deserialize, Serialize};

/// Rigid transform: rotation followed by translation (no scale).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Iso {
    pub pos: Vec3,
    pub rot: Quat,
}

impl Default for Iso {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Iso {
    pub const IDENTITY: Self = Self { pos: Vec3::ZERO, rot: Quat::IDENTITY };

    pub fn new(pos: Vec3, rot: Quat) -> Self {
        Self { pos, rot }
    }

    pub fn from_pos(pos: Vec3) -> Self {
        Self { pos, rot: Quat::IDENTITY }
    }

    #[inline]
    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        self.rot * p + self.pos
    }

    #[inline]
    pub fn transform_vector(&self, v: Vec3) -> Vec3 {
        self.rot * v
    }

    #[inline]
    pub fn inverse_transform_point(&self, p: Vec3) -> Vec3 {
        self.rot.conjugate() * (p - self.pos)
    }

    #[inline]
    pub fn inverse_transform_vector(&self, v: Vec3) -> Vec3 {
        self.rot.conjugate() * v
    }

    /// `self * other`: apply `other` first, then `self`.
    #[inline]
    pub fn mul(&self, other: &Iso) -> Iso {
        Iso { pos: self.transform_point(other.pos), rot: (self.rot * other.rot).normalize() }
    }

    pub fn inverse(&self) -> Iso {
        let rot = self.rot.conjugate();
        Iso { pos: rot * -self.pos, rot }
    }

    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rot, self.pos)
    }
}

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Aabb {
    pub const EMPTY: Self = Self { min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) };

    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn from_center_half(center: Vec3, half: Vec3) -> Self {
        Self { min: center - half, max: center + half }
    }

    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Self {
        let mut aabb = Self::EMPTY;
        for p in points {
            aabb.grow_point(p);
        }
        aabb
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }

    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    #[inline]
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    #[inline]
    pub fn grow_point(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    #[inline]
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }

    #[inline]
    pub fn expanded(&self, margin: f32) -> Aabb {
        Aabb { min: self.min - Vec3::splat(margin), max: self.max + Vec3::splat(margin) }
    }

    #[inline]
    pub fn intersects(&self, o: &Aabb) -> bool {
        self.min.cmple(o.max).all() && self.max.cmpge(o.min).all()
    }

    #[inline]
    pub fn contains(&self, o: &Aabb) -> bool {
        self.min.cmple(o.min).all() && self.max.cmpge(o.max).all()
    }

    #[inline]
    pub fn contains_point(&self, p: Vec3) -> bool {
        self.min.cmple(p).all() && self.max.cmpge(p).all()
    }

    /// Half the surface area; the SAH cost metric used by BVHs.
    #[inline]
    pub fn half_area(&self) -> f32 {
        let d = self.max - self.min;
        d.x * d.y + d.y * d.z + d.z * d.x
    }

    /// AABB of this box after a rigid transform.
    pub fn transformed(&self, iso: &Iso) -> Aabb {
        let m = Mat3::from_quat(iso.rot);
        let c = iso.transform_point(self.center());
        let h = self.half_extents();
        let e = m.x_axis.abs() * h.x + m.y_axis.abs() * h.y + m.z_axis.abs() * h.z;
        Aabb::from_center_half(c, e)
    }

    /// Slab test. Returns the entry distance along the ray in `[0, max_t]`.
    pub fn ray_hit(&self, ray: &Ray, max_t: f32) -> Option<f32> {
        let inv = ray.inv_dir();
        let t0 = (self.min - ray.origin) * inv;
        let t1 = (self.max - ray.origin) * inv;
        let tmin = t0.min(t1).max_element().max(0.0);
        let tmax = t0.max(t1).min_element().min(max_t);
        (tmin <= tmax).then_some(tmin)
    }
}

/// Ray with a normalized direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, dir: Vec3) -> Self {
        Self { origin, dir: dir.normalize_or(Vec3::NEG_Y) }
    }

    #[inline]
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }

    #[inline]
    pub fn inv_dir(&self) -> Vec3 {
        // Division by +-0 yields +-inf, which the slab test handles correctly.
        Vec3::ONE / self.dir
    }
}

/// Plane `dot(normal, p) = d`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    pub normal: Vec3,
    pub d: f32,
}

impl Plane {
    pub fn from_point_normal(p: Vec3, n: Vec3) -> Self {
        Self { normal: n, d: n.dot(p) }
    }

    #[inline]
    pub fn distance(&self, p: Vec3) -> f32 {
        self.normal.dot(p) - self.d
    }
}

/// View frustum built from a view-projection matrix (works with reverse-Z).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    /// Left, right, bottom, top, near (planes point inwards). Far is omitted so
    /// infinite projections work.
    pub planes: [Vec4; 5],
}

impl Frustum {
    pub fn from_view_proj(m: &Mat4) -> Self {
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        let norm = |p: Vec4| p / p.truncate().length();
        // With reverse-Z the near plane is z = w (clip z <= w).
        Self { planes: [norm(r3 + r0), norm(r3 - r0), norm(r3 + r1), norm(r3 - r1), norm(r3 - r2)] }
    }

    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        let c = aabb.center();
        let h = aabb.half_extents();
        self.planes.iter().all(|p| {
            let n = p.truncate();
            n.dot(c) + n.abs().dot(h) + p.w >= 0.0
        })
    }

    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.planes.iter().all(|p| p.truncate().dot(center) + p.w >= -radius)
    }
}

/// Orthonormal basis `(t1, t2)` perpendicular to unit vector `n` (Duff et al. 2017).
#[inline]
pub fn orthonormal_basis(n: Vec3) -> (Vec3, Vec3) {
    let sign = 1.0f32.copysign(n.z);
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    (Vec3::new(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x), Vec3::new(b, sign + n.y * n.y * a, -n.y))
}

/// Skew-symmetric cross product matrix: `skew(a) * b == a.cross(b)`.
#[inline]
pub fn skew(a: Vec3) -> Mat3 {
    Mat3::from_cols(Vec3::new(0.0, a.z, -a.y), Vec3::new(-a.z, 0.0, a.x), Vec3::new(a.y, -a.x, 0.0))
}

/// Exponential smoothing factor that is frame-rate independent.
#[inline]
pub fn smoothing(rate: f32, dt: f32) -> f32 {
    1.0 - (-rate * dt).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_roundtrip() {
        let iso = Iso::new(vec3(1.0, 2.0, 3.0), Quat::from_rotation_y(0.7));
        let p = vec3(-4.0, 5.0, 0.5);
        let back = iso.inverse_transform_point(iso.transform_point(p));
        assert!((back - p).length() < 1e-5);
        let q = iso.inverse().mul(&iso);
        assert!(q.pos.length() < 1e-5);
    }

    #[test]
    fn aabb_ray() {
        let b = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        let r = Ray::new(vec3(-5.0, 0.0, 0.0), Vec3::X);
        assert!((b.ray_hit(&r, 100.0).unwrap() - 4.0).abs() < 1e-5);
        let miss = Ray::new(vec3(-5.0, 3.0, 0.0), Vec3::X);
        assert!(b.ray_hit(&miss, 100.0).is_none());
    }

    #[test]
    fn basis_is_orthonormal() {
        for n in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::Z, vec3(0.3, -0.5, 0.8).normalize()] {
            let (a, b) = orthonormal_basis(n);
            assert!(a.dot(n).abs() < 1e-5 && b.dot(n).abs() < 1e-5 && a.dot(b).abs() < 1e-5);
            assert!((a.length() - 1.0).abs() < 1e-5 && (b.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn frustum_culls() {
        let proj = Mat4::perspective_infinite_reverse_rh(1.0, 1.0, 0.1);
        let view = Mat4::look_at_rh(Vec3::ZERO, -Vec3::Z, Vec3::Y);
        let f = Frustum::from_view_proj(&(proj * view));
        assert!(f.intersects_aabb(&Aabb::from_center_half(vec3(0.0, 0.0, -10.0), Vec3::ONE)));
        assert!(!f.intersects_aabb(&Aabb::from_center_half(vec3(0.0, 0.0, 10.0), Vec3::ONE)));
        assert!(!f.intersects_aabb(&Aabb::from_center_half(vec3(100.0, 0.0, -10.0), Vec3::ONE)));
    }
}
