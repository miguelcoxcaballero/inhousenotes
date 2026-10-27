//! Local lights (point and spot) for the clustered forward pass.
//!
//! ## Units
//! The renderer works in scaled photometric units: one unit of illuminance
//! is [`LUX_PER_UNIT`] lux (the analytic noon sun is ~10 units, ~100 klx)
//! and one unit of radiance is [`NITS_PER_UNIT`] cd/m². A light's
//! `intensity` is a luminous intensity in units of 10^4 cd, so a surface at
//! distance `d` facing it receives `intensity / d²` units of illuminance.
//! [`candela`], [`Light::point`] and [`Light::spot`] convert from
//! datasheet values (lumens, candela).
//!
//! ## GPU path
//! Every frame the renderer frustum-culls the lights on the CPU, sorts the
//! survivors by distance (so a cluster that overflows drops the farthest
//! ones), packs them into a storage buffer and a compute pass bins them into
//! a froxel grid of 64 px tiles x 32 exponential depth slices, plus one
//! whole-column bucket per tile used for fog in-scattering (see
//! `shaders/cluster.wgsl`). Shading walks only the lights of its cluster.

use bytemuck::{Pod, Zeroable};
use forja_math::{Frustum, Mat4, Vec3};

/// Lux in one renderer illuminance unit.
pub const LUX_PER_UNIT: f32 = 1.0e4;
/// cd/m² in one renderer radiance unit.
pub const NITS_PER_UNIT: f32 = 1.0e4;
/// Most lights the GPU buffer holds per frame (indices are packed as u16).
pub const MAX_LOCAL_LIGHTS: usize = 65535;
/// Screen tile of the light clusters (pixels).
pub const CLUSTER_TILE: u32 = 64;
/// Exponential depth slices of the light clusters.
pub const CLUSTER_SLICES: u32 = 32;
/// Most lights one cluster keeps (the closest ones win).
pub const CLUSTER_CAP: u32 = 96;
/// Depth range covered by the slices (m); nearer pixels use slice 0,
/// farther ones the last slice.
pub const CLUSTER_NEAR: f32 = 2.0;
pub const CLUSTER_FAR: f32 = 3000.0;
/// u32 words per cluster: a count plus `CLUSTER_CAP` u16 indices.
pub const CLUSTER_STRIDE: u32 = 1 + CLUSTER_CAP / 2;
/// Most spot lights that cast shadows in one frame.
pub const MAX_SPOT_SHADOWS: usize = 4;

/// Luminous intensity in candela to renderer units.
pub fn candela(cd: f32) -> f32 {
    cd / LUX_PER_UNIT
}

/// Cone of a spot light (half angles, radians).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    /// Direction the light points to (normalized on upload).
    pub direction: Vec3,
    /// Full intensity inside this half angle.
    pub inner_angle: f32,
    /// No light outside this half angle.
    pub outer_angle: f32,
}

/// A point or spot light with physical inverse-square falloff and a smooth
/// window to zero at `range`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub position: Vec3,
    /// Linear RGB of luminance ~1 ([`kelvin`], [`LightColor`]); the
    /// brightness is in `intensity`.
    pub color: Vec3,
    /// Luminous intensity in renderer units (1 = 10^4 cd, see [`candela`]).
    pub intensity: f32,
    /// Distance where the light reaches zero (m).
    pub range: f32,
    /// Radius of the emitting bulb or globe (m): softens the falloff near the
    /// light (1 / (d² + r²)) and widens its highlight so glossy and wet
    /// surfaces show a lamp-sized reflection instead of a pin prick.
    pub source_radius: f32,
    pub spot: Option<Spot>,
    /// Shadow-mapped (spot lights only; the [`MAX_SPOT_SHADOWS`] closest
    /// flagged spots get a map).
    pub cast_shadows: bool,
    /// Scale of this light's glow in fog (0 = none, 1 = physical).
    pub scatter: f32,
    /// Drawn as a point sprite when far away (aerial views keep distant city
    /// lights as points after their geometry is culled).
    pub sprite: bool,
}

impl Light {
    /// Omnidirectional light emitting `lumens`.
    pub fn point(position: Vec3, color: Vec3, lumens: f32) -> Self {
        let intensity = candela(lumens / (4.0 * std::f32::consts::PI));
        Self { position, color, intensity, range: auto_range(intensity), source_radius: 0.05, spot: None, cast_shadows: false, scatter: 1.0, sprite: true }
    }

    /// Spot light emitting `lumens` into its cone (Filament convention:
    /// the flux fills the outer cone, so narrowing it brightens the beam).
    pub fn spot(position: Vec3, direction: Vec3, color: Vec3, lumens: f32, inner_angle: f32, outer_angle: f32) -> Self {
        let outer = outer_angle.clamp(0.01, std::f32::consts::FRAC_PI_2 * 0.99);
        let inner = inner_angle.clamp(0.0, outer);
        let intensity = candela(lumens / (std::f32::consts::TAU * (1.0 - outer.cos())));
        Self {
            position,
            color,
            intensity,
            range: auto_range(intensity),
            source_radius: 0.05,
            spot: Some(Spot { direction, inner_angle: inner, outer_angle: outer }),
            cast_shadows: false,
            scatter: 1.0,
            sprite: true,
        }
    }

    pub fn with_range(mut self, range: f32) -> Self {
        self.range = range;
        self
    }

    pub fn with_source_radius(mut self, r: f32) -> Self {
        self.source_radius = r;
        self
    }

    pub fn with_shadows(mut self) -> Self {
        self.cast_shadows = true;
        self
    }

    pub fn with_scatter(mut self, s: f32) -> Self {
        self.scatter = s;
        self
    }
}

/// Range where a light of `intensity` falls to ~0.5 lux (the smooth window
/// hides the cut), clamped to 1..400 m.
pub fn auto_range(intensity: f32) -> f32 {
    (intensity / candela(0.5)).sqrt().clamp(1.0, 400.0)
}

impl From<crate::PointLight> for Light {
    /// Legacy point lights keep their arbitrary intensity and their
    /// `1 / (d² + 1)` falloff (a 1 m source radius).
    fn from(p: crate::PointLight) -> Self {
        Self {
            position: p.position,
            color: p.color,
            intensity: p.intensity,
            range: p.radius,
            source_radius: 1.0,
            spot: None,
            cast_shadows: false,
            scatter: 1.0,
            sprite: true,
        }
    }
}

/// Colour of a black body at `k` kelvin (1667..25000 K) as linear sRGB
/// normalized to luminance 1 (Kang et al. 2002 fit of the Planckian locus).
pub fn kelvin(k: f32) -> Vec3 {
    let t = k.clamp(1667.0, 25000.0) as f64;
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    let (cx, cz) = (x / y, (1.0 - x - y) / y);
    let r = 3.2406 * cx - 1.5372 - 0.4986 * cz;
    let g = -0.9689 * cx + 1.8758 + 0.0415 * cz;
    let b = 0.0557 * cx - 0.2040 + 1.0570 * cz;
    let c = Vec3::new(r.max(0.0) as f32, g.max(0.0) as f32, b.max(0.0) as f32);
    c / c.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-6)
}

/// Common lamp colours (luminance 1).
pub struct LightColor;

impl LightColor {
    /// High-pressure sodium street lamp (~2000 K, orange).
    pub fn sodium() -> Vec3 {
        kelvin(2000.0)
    }
    /// Low-pressure sodium (589 nm, nearly monochromatic amber).
    pub fn low_pressure_sodium() -> Vec3 {
        Vec3::new(1.0, 0.55, 0.0) / Vec3::new(1.0, 0.55, 0.0).dot(Vec3::new(0.2126, 0.7152, 0.0722))
    }
    /// Warm white incandescent / warm LED (2700 K).
    pub fn warm() -> Vec3 {
        kelvin(2700.0)
    }
    /// Neutral LED street light (4000 K).
    pub fn led() -> Vec3 {
        kelvin(4000.0)
    }
    /// Cool LED / metal halide (5600 K).
    pub fn cool() -> Vec3 {
        kelvin(5600.0)
    }
}

/// GPU layout of one light (`LocalLight` in common.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuLight {
    /// xyz world position, w range.
    pub pos_range: [f32; 4],
    /// rgb colour x intensity, w source radius.
    pub color: [f32; 4],
    /// xyz spot direction, w cosine of the outer angle (-2 for points).
    pub dir: [f32; 4],
    /// x cone scale, y cone offset (cone = sat(cos * x + y)²), z shadow
    /// layer (-1: none), w fog scatter weight (negative: no sprite).
    pub params: [f32; 4],
}

impl GpuLight {
    pub fn pack(l: &Light, shadow_layer: i32) -> Self {
        let c = l.color * l.intensity;
        let (dir, cos_outer, scale, offset) = match l.spot {
            Some(s) => {
                let co = s.outer_angle.cos();
                let ci = s.inner_angle.cos().max(co + 1e-3);
                let scale = 1.0 / (ci - co);
                (s.direction.normalize_or(Vec3::NEG_Y), co, scale, -co * scale)
            }
            None => (Vec3::NEG_Y, -2.0, 0.0, 1.0),
        };
        let scatter = l.scatter.max(0.0) + 1e-6;
        Self {
            pos_range: [l.position.x, l.position.y, l.position.z, l.range.max(0.01)],
            color: [c.x, c.y, c.z, l.source_radius.max(0.0)],
            dir: [dir.x, dir.y, dir.z, cos_outer],
            params: [scale, offset, shadow_layer as f32, if l.sprite { scatter } else { -scatter }],
        }
    }
}

/// Sphere bounding the lit volume of a light (tight for narrow spots).
pub fn light_bounds(l: &Light) -> (Vec3, f32) {
    match l.spot {
        Some(s) => {
            let d = s.direction.normalize_or(Vec3::NEG_Y);
            let a = s.outer_angle;
            if a > std::f32::consts::FRAC_PI_4 {
                (l.position + d * (a.cos() * l.range), a.sin() * l.range)
            } else {
                let r = l.range / (2.0 * a.cos());
                (l.position + d * r, r)
            }
        }
        None => (l.position, l.range),
    }
}

/// Lights that can affect the view, closest first (capped at
/// [`MAX_LOCAL_LIGHTS`]).
pub(crate) fn visible_lights<'a>(lights: impl Iterator<Item = Light> + 'a, frustum: &Frustum, eye: Vec3) -> Vec<Light> {
    let mut v: Vec<(f32, Light)> = lights
        .filter(|l| l.intensity > 0.0 && l.range > 0.0)
        .filter(|l| {
            let (c, r) = light_bounds(l);
            frustum.intersects_sphere(c, r) || (l.sprite && frustum.intersects_sphere(l.position, 0.0))
        })
        .map(|l| ((l.position - eye).length_squared(), l))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.truncate(MAX_LOCAL_LIGHTS);
    v.into_iter().map(|(_, l)| l).collect()
}

/// View-projection of a shadowed spot light (standard 0..1 depth).
pub fn spot_view_proj(l: &Light) -> Option<Mat4> {
    let s = l.spot?;
    let d = s.direction.normalize_or(Vec3::NEG_Y);
    let up = if d.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_at_rh(l.position, l.position + d, up);
    let near = (l.range * 0.002).clamp(0.05, 0.5);
    let proj = Mat4::perspective_rh((s.outer_angle * 2.0 + 0.05).min(3.0), 1.0, near, l.range);
    Some(proj * view)
}

/// Cluster grid size for a viewport.
pub fn cluster_grid(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(CLUSTER_TILE), height.div_ceil(CLUSTER_TILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_temperature_runs_from_orange_to_blue() {
        let sodium = kelvin(2000.0);
        let warm = kelvin(2700.0);
        let led = kelvin(4000.0);
        let d65 = kelvin(6504.0);
        let lum = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        for c in [sodium, warm, led, d65] {
            assert!((lum(c) - 1.0).abs() < 1e-3, "{c}");
        }
        assert!(sodium.x / sodium.z > warm.x / warm.z && warm.x / warm.z > led.x / led.z);
        assert!(sodium.z < 0.1 && sodium.x > 2.0, "sodium should be deep orange: {sodium}");
        // 6504 K is the sRGB white point.
        assert!((d65.x - d65.z).abs() < 0.08 && (d65.x - d65.y).abs() < 0.08, "{d65}");
    }

    #[test]
    fn photometric_constructors() {
        // A 4π-sr 12566 lm bulb is 1000 cd: 10 lux at 10 m.
        let p = Light::point(Vec3::ZERO, Vec3::ONE, 4000.0 * std::f32::consts::PI);
        assert!((p.intensity * LUX_PER_UNIT - 1000.0).abs() < 0.5);
        assert!((p.intensity / 100.0 * LUX_PER_UNIT - 10.0).abs() < 0.01);
        // Narrowing a spot of the same flux brightens it.
        let wide = Light::spot(Vec3::ZERO, Vec3::NEG_Y, Vec3::ONE, 1000.0, 0.5, 1.0);
        let narrow = Light::spot(Vec3::ZERO, Vec3::NEG_Y, Vec3::ONE, 1000.0, 0.1, 0.3);
        assert!(narrow.intensity > wide.intensity * 4.0);
        // Range reaches 0.5 lux.
        assert!((p.intensity / (p.range * p.range) * LUX_PER_UNIT - 0.5).abs() < 0.01);
    }

    #[test]
    fn packing_and_bounds() {
        let s = Light::spot(Vec3::new(0.0, 8.0, 0.0), Vec3::NEG_Y, Vec3::ONE, 5000.0, 0.4, 0.6).with_range(20.0);
        let g = GpuLight::pack(&s, -1);
        // Cone factor is 1 on the axis and 0 at the outer angle.
        let cone = |cosv: f32| (cosv * g.params[0] + g.params[1]).clamp(0.0, 1.0);
        assert!((cone(1.0) - 1.0).abs() < 1e-6);
        assert!(cone(0.6f32.cos()).abs() < 1e-5);
        let p = GpuLight::pack(&Light::point(Vec3::ZERO, Vec3::ONE, 100.0), 0);
        assert_eq!((p.params[0], p.params[1]), (0.0, 1.0));
        // The cone's bounding sphere contains its apex and its rim.
        let (c, r) = light_bounds(&s);
        assert!((c - s.position).length() <= r + 1e-4);
        let rim = s.position + Vec3::new(0.6f32.sin(), -0.6f32.cos(), 0.0) * 20.0;
        assert!((c - rim).length() <= r + 1e-3);
        assert!(r < 20.0);
        assert_eq!(std::mem::size_of::<GpuLight>(), 64);
    }

    #[test]
    fn culling_sorts_closest_first() {
        let cam = crate::Camera::look_at(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 2.0, -10.0));
        let fr = Frustum::from_view_proj(&cam.view_proj());
        let lights = [
            Light::point(Vec3::new(0.0, 3.0, -50.0), Vec3::ONE, 1000.0).with_range(5.0),
            Light::point(Vec3::new(0.0, 3.0, -10.0), Vec3::ONE, 1000.0).with_range(5.0),
            // Behind the camera, out of range of the view.
            Light::point(Vec3::new(0.0, 3.0, 50.0), Vec3::ONE, 1000.0).with_range(5.0),
        ];
        let v = visible_lights(lights.into_iter(), &fr, cam.position);
        assert_eq!(v.len(), 2);
        assert!(v[0].position.z > v[1].position.z);
    }
}
