//! Camera and cascaded shadow map fitting.

use forja_math::{Mat4, Quat, Vec3, Vec4};

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub position: Vec3,
    pub rotation: Quat,
    pub fov_y: f32,
    pub near: f32,
    pub aspect: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { position: Vec3::new(0.0, 2.0, 8.0), rotation: Quat::IDENTITY, fov_y: 60f32.to_radians(), near: 0.1, aspect: 16.0 / 9.0 }
    }
}

impl Camera {
    pub fn look_at(position: Vec3, target: Vec3) -> Self {
        let mut c = Self { position, ..Default::default() };
        c.rotation = Quat::from_mat4(&Mat4::look_at_rh(position, target, Vec3::Y).inverse());
        c
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn view(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation, self.position).inverse()
    }

    /// Infinite reverse-Z projection (depth 1 at near, 0 at infinity).
    pub fn proj(&self) -> Mat4 {
        Mat4::perspective_infinite_reverse_rh(self.fov_y, self.aspect, self.near)
    }

    pub fn view_proj(&self) -> Mat4 {
        self.proj() * self.view()
    }

    /// World-space corners of the view frustum slice between two distances.
    pub fn slice_corners(&self, near: f32, far: f32) -> [Vec3; 8] {
        let ty = (self.fov_y * 0.5).tan();
        let tx = ty * self.aspect;
        let m = Mat4::from_rotation_translation(self.rotation, self.position);
        let mut out = [Vec3::ZERO; 8];
        for (i, d) in [near, far].iter().enumerate() {
            for (j, (sx, sy)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().enumerate() {
                out[i * 4 + j] = m.transform_point3(Vec3::new(sx * tx * d, sy * ty * d, -d));
            }
        }
        out
    }
}

/// Practical split scheme (log/linear blend) for `n` cascades.
pub fn cascade_splits(near: f32, far: f32, lambda: f32) -> [f32; 4] {
    let mut s = [0.0; 4];
    for (i, v) in s.iter_mut().enumerate() {
        let p = (i + 1) as f32 / 4.0;
        let log = near * (far / near).powf(p);
        let lin = near + (far - near) * p;
        *v = lambda * log + (1.0 - lambda) * lin;
    }
    s
}

/// Stable (rotation and translation invariant) cascade matrix: bounding
/// sphere of the slice, texel-snapped in light space.
pub fn cascade_matrix(camera: &Camera, near: f32, far: f32, sun_dir: Vec3, resolution: u32, caster_reach: f32) -> Mat4 {
    cascade_fit(camera, near, far, sun_dir, resolution, caster_reach).0
}

/// [`cascade_matrix`] plus the radius of the fitted bounding sphere (world
/// size of a shadow texel = 2 radius / resolution; light-space depth range =
/// 2 radius + caster reach).
pub fn cascade_fit(camera: &Camera, near: f32, far: f32, sun_dir: Vec3, resolution: u32, caster_reach: f32) -> (Mat4, f32) {
    let corners = camera.slice_corners(near, far);
    let center = corners.iter().copied().sum::<Vec3>() / 8.0;
    let radius = corners.iter().map(|c| (*c - center).length()).fold(0.0, f32::max);
    let radius = (radius * 16.0).ceil() / 16.0;
    let up = if sun_dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let light_view = Mat4::look_at_rh(Vec3::ZERO, -sun_dir, up);
    // Snap the center to whole shadow texels.
    let texel = 2.0 * radius / resolution as f32;
    let mut c = light_view.transform_point3(center);
    c.x = (c.x / texel).floor() * texel;
    c.y = (c.y / texel).floor() * texel;
    let snapped = light_view.inverse().transform_point3(c);
    let eye = snapped + sun_dir * (radius + caster_reach);
    let view = Mat4::look_at_rh(eye, snapped, up);
    let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, 2.0 * radius + caster_reach);
    (proj * view, radius)
}

/// Planes of an orthographic or perspective matrix for culling (standard Z).
pub fn frustum_planes_standard(m: &Mat4) -> [Vec4; 6] {
    let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
    let n = |p: Vec4| p / p.truncate().length();
    [n(r3 + r0), n(r3 - r0), n(r3 + r1), n(r3 - r1), n(r2), n(r3 - r2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_increase() {
        let s = cascade_splits(0.1, 300.0, 0.75);
        assert!(s[0] > 0.1 && s[0] < s[1] && s[1] < s[2] && (s[3] - 300.0).abs() < 1e-3);
    }

    #[test]
    fn cascade_contains_slice() {
        let cam = Camera::look_at(Vec3::new(0.0, 2.0, 0.0), Vec3::new(10.0, 1.0, 10.0));
        let sun = Vec3::new(0.3, 0.8, 0.2).normalize();
        let m = cascade_matrix(&cam, 0.1, 20.0, sun, 2048, 200.0);
        for c in cam.slice_corners(0.1, 20.0) {
            let p = m.project_point3(c);
            assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0 && (0.0..=1.0).contains(&p.z), "{p}");
        }
    }
}
