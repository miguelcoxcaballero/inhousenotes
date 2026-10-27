//! Physical camera exposure.
//!
//! Light in the renderer is linear and photometric up to one constant:
//! one unit of illuminance is [`LUX_PER_UNIT`] lux, and one unit of
//! radiance is [`LUX_PER_UNIT`] cd/m² (a white Lambertian surface under
//! `E` units of illuminance has a luminance of `E / pi` units). The HDRI
//! sun is normalised to 14 units, about 105 klx: a clear summer sun
//! 50-60 degrees above Madrid or Valencia.
//!
//! The camera follows the photographic exposure equations (ISO 2720 /
//! ISO 12232, as used by Frostbite and Unreal):
//!
//! * a reflected-light meter set to `EV100` maps the luminance
//!   `K * 2^EV100 / 100` (K = 12.5) to scene-referred mid grey 0.18;
//! * an incident-light meter reads `EV100 = log2(E * 100 / C)` from the
//!   illuminance `E` on its hemispherical (dome) receptor (C = 330).
//!
//! The renderer's incident reading points the dome at the sky in the open:
//! for the street HDRI it reads EV100 ~15.1, the "sunny 16" rule, and the
//! tone curve then puts a sunlit 18% grey card near sRGB 130 and a sunlit
//! white wall near 0.9, as a camera would. Unlike a flat receptor the dome
//! still sees a low sun, so sunsets are not exposed as if it were dusk. It
//! costs nothing per pixel: two numbers the CPU already has. A
//! center-weighted average meter on the GPU then refines it per view
//! (`Environment::auto_exposure_range`).

use forja_math::Vec3;

/// Lux per renderer illuminance unit (and cd/m² per radiance unit).
pub const LUX_PER_UNIT: f32 = 7500.0;
/// Reflected-light meter calibration constant (cd s / m², ISO 2720).
pub const METER_K: f32 = 12.5;
/// Incident-light meter calibration constant for a hemispherical receptor
/// (lux s; ISO 2720 allows 320-540).
pub const INCIDENT_C: f32 = 330.0;
/// Scene-referred value the meter's reference luminance maps to.
pub const MID_GREY: f32 = 0.18;

/// Linear exposure (multiplier on renderer radiance units) for a camera at
/// `ev100`.
pub fn exposure_from_ev100(ev100: f32) -> f32 {
    MID_GREY * LUX_PER_UNIT * 100.0 / (METER_K * ev100.exp2())
}

/// EV100 an incident-light meter reads for `illuminance` renderer units on
/// its dome.
pub fn incident_ev100(illuminance: f32) -> f32 {
    (illuminance.max(1e-6) * LUX_PER_UNIT * 100.0 / INCIDENT_C).log2()
}

/// Illuminance (RGB) on an upward dome receptor (flux over its projected
/// area) from a sun of normal irradiance `sun` at elevation sine `sun_y` and
/// a sky giving `sky_horizontal` on flat ground: a hemisphere's projected
/// area toward the sun is `(1 + sin(elevation)) / 2` of its base, and a
/// uniform sky lights it 1.5 times as much as the flat base.
pub fn dome_illuminance(sun: Vec3, sun_y: f32, sky_horizontal: Vec3) -> Vec3 {
    // The horizon hides a sun below it (faded over ~1 degree).
    let visible = (sun_y / 0.02).clamp(0.0, 1.0);
    sun * ((1.0 + sun_y.min(1.0)) * 0.5 * visible) + sky_horizontal * 1.5
}

/// Camera white balance gains for the metered light colour `light`:
/// von Kries scaling toward neutral by `amount` (0..1), limited to the
/// +-25% per channel a camera's AWB range allows (a sunset stays warm),
/// at constant luminance.
pub fn white_balance(light: Vec3, amount: f32) -> Vec3 {
    let l = luminance(light);
    if l <= 1e-6 || amount <= 0.0 {
        return Vec3::ONE;
    }
    let g = (Vec3::splat(l) / light.max(Vec3::splat(l * 1e-3))).powf(amount.min(1.0)).clamp(Vec3::splat(0.8), Vec3::splat(1.25));
    g * (l / luminance(light * g))
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sunny_sixteen() {
        // A 105 klx sun 52 degrees up plus a 14 klx sky: the dome reads EV
        // ~15 (f/16, 1/125 s at ISO 100) and a sunlit 18% card lands just
        // over mid grey.
        let (sun, y, sky) = (105_000.0 / LUX_PER_UNIT, 52f32.to_radians().sin(), 14_000.0 / LUX_PER_UNIT);
        let ev = incident_ev100(luminance(dome_illuminance(Vec3::splat(sun), y, Vec3::splat(sky))));
        assert!((ev - 15.05).abs() < 0.1, "{ev}");
        let card = 0.18 * (sun * y + sky) / std::f32::consts::PI * exposure_from_ev100(ev);
        assert!(card > 0.18 && card < 0.26, "{card}");
        // A low sun still counts: a sunset is not metered like dusk.
        assert!(dome_illuminance(Vec3::splat(sun), 0.05, Vec3::ZERO).x > 0.5 * sun);
        // One stop more light halves the exposure.
        assert!((exposure_from_ev100(14.0) / exposure_from_ev100(15.0) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn white_balance_is_partial_and_keeps_luminance() {
        let noon = Vec3::new(1.0, 0.886, 0.79);
        let g = white_balance(noon, 0.6);
        assert!((luminance(noon * g) - luminance(noon)).abs() < 1e-4);
        let w = noon * g;
        // Warmer than neutral still, but cooler than the light.
        assert!(w.x > w.z && w.z / w.x > noon.z / noon.x, "{w}");
        // A red sunset keeps most of its colour.
        let sunset = Vec3::new(1.0, 0.5, 0.2);
        let s = sunset * white_balance(sunset, 0.6);
        assert!(s.x > 2.0 * s.z, "{s}");
        assert_eq!(white_balance(noon, 0.0), Vec3::ONE);
    }
}
