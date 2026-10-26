//! CPU twin of the sky shader, used to light the scene with the same sun
//! color and sky ambient the player sees in the sky.

use forja_math::{Vec3, vec3};

const EARTH_R: f32 = 6360.0;
const ATMO_R: f32 = 6460.0;
const RAYLEIGH: Vec3 = Vec3::new(5.802e-3, 13.558e-3, 33.1e-3);
const MIE: f32 = 3.996e-3;
const MIE_EXT: f32 = 4.4e-3;
const OZONE: Vec3 = Vec3::new(0.65e-3, 1.881e-3, 0.085e-3);

fn ray_sphere(o: Vec3, d: Vec3, r: f32) -> f32 {
    let b = o.dot(d);
    let c = o.dot(o) - r * r;
    let disc = b * b - c;
    if disc < 0.0 {
        return -1.0;
    }
    let s = disc.sqrt();
    if -b - s > 0.0 { -b - s } else { -b + s }
}

fn density(h: f32) -> Vec3 {
    vec3((-h / 8.0).exp(), (-h / 1.2).exp(), (1.0 - (h - 25.0).abs() / 15.0).max(0.0))
}

fn extinction(d: Vec3) -> Vec3 {
    RAYLEIGH * d.x + Vec3::splat(MIE_EXT * d.y) + OZONE * d.z
}

/// Fraction of sunlight reaching point `p` (km, planet-centered) along `l`.
pub fn sun_transmittance(p: Vec3, l: Vec3) -> Vec3 {
    let b = p.dot(l);
    if b < 0.0 && p.dot(p) - b * b < EARTH_R * EARTH_R {
        return Vec3::ZERO;
    }
    let t = ray_sphere(p, l, ATMO_R);
    let n = 12;
    let dt = t / n as f32;
    let mut od = Vec3::ZERO;
    for i in 0..n {
        let q = p + l * ((i as f32 + 0.5) * dt);
        od += extinction(density(q.length() - EARTH_R)) * dt;
    }
    (-od).exp()
}

fn phase_rayleigh(c: f32) -> f32 {
    3.0 / (16.0 * std::f32::consts::PI) * (1.0 + c * c)
}

fn phase_mie(c: f32) -> f32 {
    let g = 0.8f32;
    let k = 3.0 / (8.0 * std::f32::consts::PI) * (1.0 - g * g) / (2.0 + g * g);
    k * (1.0 + c * c) / (1.0 + g * g - 2.0 * g * c).powf(1.5)
}

pub fn sky_radiance(d: Vec3, l: Vec3, alt_km: f32) -> Vec3 {
    let o = vec3(0.0, EARTH_R + alt_km.max(0.001), 0.0);
    let mut t_max = ray_sphere(o, d, ATMO_R);
    let tg = ray_sphere(o, d, EARTH_R);
    if tg > 0.0 {
        t_max = t_max.min(tg);
    }
    let n = 24;
    let dt = t_max / n as f32;
    let c = d.dot(l);
    let (pr, pm) = (phase_rayleigh(c), phase_mie(c));
    let mut lum = Vec3::ZERO;
    let mut trans = Vec3::ONE;
    for i in 0..n {
        let p = o + d * ((i as f32 + 0.5) * dt);
        let dens = density(p.length() - EARTH_R);
        let ext = extinction(dens);
        let ts = sun_transmittance(p, l);
        let scatter = (RAYLEIGH * dens.x * pr + Vec3::splat(MIE * dens.y * pm)) * ts;
        let step = (-ext * dt).exp();
        lum += trans * (scatter - scatter * step) / ext.max(Vec3::splat(1e-6));
        trans *= step;
    }
    lum
}

/// Lighting terms derived from the atmosphere for a sun direction.
#[derive(Clone, Copy, Debug)]
pub struct SkyLighting {
    pub sun_color: Vec3,
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub ground: Vec3,
}

/// `sun_illuminance` scales both sun and sky (physical-ish units where a
/// noon sun gives ~ 1-3 after exposure).
pub fn sky_lighting(sun_dir: Vec3, sun_illuminance: f32, ground_albedo: Vec3) -> SkyLighting {
    let ground_p = vec3(0.0, EARTH_R + 0.001, 0.0);
    let ts = sun_transmittance(ground_p, sun_dir);
    let sun_color = ts * sun_illuminance;
    // Hemisphere average around the zenith and the horizon ring.
    let mut zenith = Vec3::ZERO;
    let mut horizon = Vec3::ZERO;
    let k = 8;
    for i in 0..k {
        let a = i as f32 / k as f32 * std::f32::consts::TAU;
        zenith += sky_radiance(vec3(a.cos() * 0.35, 0.94, a.sin() * 0.35).normalize(), sun_dir, 0.0);
        horizon += sky_radiance(vec3(a.cos(), 0.08, a.sin()).normalize(), sun_dir, 0.0);
    }
    // pi * average radiance approximates hemispherical irradiance.
    let scale = sun_illuminance * std::f32::consts::PI / k as f32;
    let zenith = zenith * scale;
    let horizon = horizon * scale;
    let sun_on_ground = sun_color * sun_dir.y.max(0.0) / std::f32::consts::PI;
    let ground = ground_albedo * (sun_on_ground + (zenith + horizon) * 0.25);
    SkyLighting { sun_color, zenith, horizon, ground }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_sky_is_blue_and_sunset_is_red() {
        let noon = sky_lighting(vec3(0.2, 0.95, 0.1).normalize(), 1.0, Vec3::splat(0.3));
        assert!(noon.zenith.z > noon.zenith.x, "noon sky should be blue: {:?}", noon.zenith);
        let sunset = sky_lighting(vec3(1.0, 0.03, 0.0).normalize(), 1.0, Vec3::splat(0.3));
        assert!(sunset.sun_color.x > sunset.sun_color.z * 2.0, "sunset sun should be red: {:?}", sunset.sun_color);
        assert!(sunset.sun_color.length() < noon.sun_color.length());
        let night = sky_lighting(vec3(0.0, -0.3, 1.0).normalize(), 1.0, Vec3::splat(0.3));
        assert!(night.sun_color.length() < 1e-4);
    }
}
