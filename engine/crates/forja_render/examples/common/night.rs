#![allow(dead_code)]
//! Time of day and weather for the render examples:
//! `NIGHT=1`, `DUSK=1`, `RAIN=1` (rainy night) and `WET=1` (wet streets
//! with any of them; `WET=0.5` for damp), plus test lights built from the
//! city's light positions until the content team's lamps land.

use forja_math::Vec3;
use forja_render::{Environment, Light, PointLight, kelvin};

/// The night of the engine before clustered lighting (what the sandbox
/// does): the sun flipped above the horizon as a dim moon, a fixed exposure
/// of 5.5 and the 40 lamps nearest the camera as legacy point lights.
pub fn legacy_night() -> Environment {
    Environment { use_ibl: false, sun_dir: Vec3::new(-0.35, 0.55, -0.45).normalize(), sun_illuminance: 0.35, exposure: 5.5, bloom: 0.1, ..Environment::default() }
}

pub fn legacy_lights(points: &[(Vec3, Vec3)], cam: Vec3) -> Vec<PointLight> {
    let mut near: Vec<(Vec3, Vec3)> = points.iter().filter(|(p, _)| (*p - cam).length_squared() < 150.0 * 150.0).copied().collect();
    near.sort_by(|a, b| (a.0 - cam).length_squared().total_cmp(&(b.0 - cam).length_squared()));
    near.into_iter().take(40).map(|(p, c)| PointLight { position: p, radius: 22.0, color: c, intensity: 60.0 }).collect()
}

/// The environment selected by the environment variables, with a suffix for
/// file names (`None`: the daytime HDRI look).
pub fn environment() -> Option<(&'static str, Environment)> {
    let on = |k: &str| std::env::var(k).is_ok_and(|v| v != "0");
    let (name, mut env) = if on("LEGACY") {
        ("before", legacy_night())
    } else if on("RAIN") {
        ("rain", Environment::rainy_night())
    } else if on("DUSK") {
        ("dusk", Environment::dusk())
    } else if on("NIGHT") {
        ("night", Environment::night())
    } else {
        return None;
    };
    if let Some(w) = std::env::var("WET").ok().and_then(|v| v.parse::<f32>().ok()).filter(|w| *w > 0.0) {
        env = env.with_wetness(w.min(1.0), if w >= 0.99 { 0.55 } else { 0.0 });
    }
    if let Some(ev) = std::env::var("EV").ok().and_then(|v| v.parse::<f32>().ok())
        && let Some(a) = env.auto_exposure.as_mut()
    {
        a.compensation_ev = ev;
    }
    let name = match (name, env.wetness > 0.0) {
        ("night", true) => "night_wet",
        ("dusk", true) => "dusk_wet",
        (n, _) => n,
    };
    Some((name, env))
}

/// Physical lamps for the city's light points `(position, colour)`:
/// avenue road lights (~9 m) become 2000 K sodium cobra heads aimed down,
/// plaza candelabras and street lanterns (~4 m) 2700-3000 K lanterns, wall
/// lanterns warm 2400 K globes, and coloured points (pharmacy crosses)
/// small LED signs.
pub fn city_lights(points: &[(Vec3, Vec3)]) -> Vec<Light> {
    points
        .iter()
        .map(|&(p, c)| {
            let saturated = c.max_element() - c.min_element() > 0.6 && c.x < 0.5;
            if saturated {
                // Green pharmacy cross and other coloured signs.
                let lum = c.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-3);
                Light::point(p, c / lum, 600.0).with_source_radius(0.3).with_range(9.0)
            } else if p.y > 7.0 {
                Light::spot(p, Vec3::NEG_Y, kelvin(2050.0), 16000.0, 0.75, 1.25).with_source_radius(0.2)
            } else if p.y > 3.6 {
                Light::point(p, kelvin(2800.0), 4500.0).with_source_radius(0.18)
            } else {
                Light::point(p, kelvin(2400.0), 1800.0).with_source_radius(0.12)
            }
        })
        .collect()
}

/// Warm light spilling from a lit shop window or doorway onto the
/// pavement: a wide spot just inside the facade aimed out and down.
pub fn shop_spill(at: Vec3, outward: Vec3, lumens: f32) -> Light {
    let dir = (outward.normalize() + Vec3::new(0.0, -0.9, 0.0)).normalize();
    Light::spot(at, dir, kelvin(3000.0), lumens, 0.7, 1.35).with_source_radius(0.4).with_range(14.0).with_scatter(0.5)
}

/// Floodlight aimed up a facade (monument uplighting).
pub fn floodlight(at: Vec3, target: Vec3, lumens: f32) -> Light {
    Light::spot(at, target - at, kelvin(2900.0), lumens, 0.25, 0.45).with_source_radius(0.15).with_range(((target - at).length() * 2.5).max(20.0))
}
