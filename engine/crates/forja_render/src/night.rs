//! Night, dusk and rain: eye adaptation, the night sky and the
//! [`Environment`] presets that combine them with the clustered lights,
//! wet surfaces, fog glow and light sprites.
//!
//! All radiances are in renderer units (1 = 10^4 cd/m², see
//! [`crate::lights`]), so the presets use real-world magnitudes: an urban
//! night sky of ~0.01 cd/m², a light-pollution dome of ~0.1 cd/m² at the
//! horizon, streets lit to 5-30 lux. Eye adaptation then exposes them.

use forja_math::Vec3;

use crate::Environment;

/// Automatic exposure from a luminance histogram of the frame (GPU only, no
/// readback). The target follows a night-aware key value (Krawczyk et al.
/// 2005): bright scenes are exposed toward mid grey, dark ones keep a low
/// key so night stays night while lamp-lit areas stay readable.
#[derive(Clone, Copy, Debug)]
pub struct EyeAdaptation {
    /// Exposure compensation (EV, +1 = twice as bright).
    pub compensation_ev: f32,
    /// Exposure clamps (multipliers of scene radiance).
    pub min_exposure: f32,
    pub max_exposure: f32,
    /// Adaptation speed toward a brighter scene (1/s, exponential).
    pub brighten_rate: f32,
    /// Adaptation speed toward a darker scene (1/s; the eye is slower).
    pub darken_rate: f32,
    /// Histogram window averaged for the adaptation luminance: the darkest
    /// `low_percentile` (black sky, deep shadow) and the brightest
    /// `1 - high_percentile` (the lamps themselves) are ignored.
    pub low_percentile: f32,
    pub high_percentile: f32,
}

impl Default for EyeAdaptation {
    fn default() -> Self {
        Self { compensation_ev: 0.0, min_exposure: 0.02, max_exposure: 2.0e5, brighten_rate: 3.0, darken_rate: 1.0, low_percentile: 0.5, high_percentile: 0.97 }
    }
}

/// Analytic night sky added to the atmosphere (radiance units).
#[derive(Clone, Copy, Debug)]
pub struct NightSky {
    /// Sky radiance away from the horizon.
    pub zenith: Vec3,
    /// Light-pollution glow added at the horizon, fading with elevation.
    pub glow: Vec3,
    /// Height of the glow (sine of the elevation where it falls to 1/e).
    pub glow_height: f32,
    /// Peak radiance of the brightest stars (0 = none).
    pub stars: f32,
}

impl Default for NightSky {
    fn default() -> Self {
        Self { zenith: Vec3::ZERO, glow: Vec3::ZERO, glow_height: 0.15, stars: 0.0 }
    }
}

/// The city kit authors emission bright enough to read in sunlight (a lamp
/// globe at ~5x10^5 cd/m², neon at ~5x10^5, lit interiors at ~1200); the
/// night presets scale it to physical night luminances (globes ~2x10^4,
/// shop windows ~1200, room interiors ~60 cd/m²). Content authored in
/// physical units should set `emissive_scale = 1`.
pub const AUTHORED_EMISSIVE_TO_NIGHT: f32 = 0.05;

/// Radiance in renderer units from cd/m².
pub fn nits(cd_m2: f32) -> f32 {
    cd_m2 / crate::lights::NITS_PER_UNIT
}

impl Environment {
    /// Clear night in a lit city: moonlit sky with a sodium light-pollution
    /// dome, a little bounce light, faint stars, eye adaptation, lamp glow in
    /// a light haze and distant light sprites. No sun shadows (the moon is
    /// ~100x dimmer than the street lighting).
    pub fn night() -> Self {
        let pollution = crate::lights::kelvin(2300.0);
        Self {
            // The "sun" is the moon: the same atmosphere scattering at
            // 1/400000 of the sunlight gives the blue moonlit sky.
            sun_dir: Vec3::new(-0.35, 0.55, -0.45).normalize(),
            sun_illuminance: 14.0 * 2.5e-6,
            exposure: 1500.0,
            fog_density: 0.0009,
            fog_falloff: 0.01,
            bloom: 0.05,
            vignette: 0.3,
            shadow_distance: 0.0,
            use_ibl: false,
            auto_exposure: Some(EyeAdaptation::default()),
            night_sky: NightSky { zenith: Vec3::new(0.55, 0.7, 1.0) * nits(0.004), glow: pollution * nits(0.06), glow_height: 0.12, stars: nits(0.5) },
            ambient_fill: crate::lights::kelvin(2600.0) * nits(0.03),
            light_scatter: 1.0,
            light_sprites: 1.0,
            bloom_radius: 0.72,
            bloom_limit: 48.0,
            scotopic: 0.5,
            emissive_scale: AUTHORED_EMISSIVE_TO_NIGHT,
            ..Self::default()
        }
    }

    /// Blue hour: the sun just below the horizon (twilight sky from the
    /// atmosphere model), lamps on, a hazier air.
    pub fn dusk() -> Self {
        Self {
            sun_dir: Vec3::new(0.82, -0.035, 0.57).normalize(),
            sun_illuminance: 14.0,
            exposure: 60.0,
            fog_density: 0.0022,
            fog_falloff: 0.012,
            bloom: 0.045,
            vignette: 0.3,
            shadow_distance: 0.0,
            use_ibl: false,
            auto_exposure: Some(EyeAdaptation::default()),
            night_sky: NightSky { zenith: Vec3::ZERO, glow: crate::lights::kelvin(2300.0) * nits(0.05), glow_height: 0.1, stars: 0.0 },
            ambient_fill: crate::lights::kelvin(2600.0) * nits(0.03),
            light_scatter: 1.0,
            light_sprites: 1.0,
            bloom_radius: 0.75,
            bloom_limit: 48.0,
            scotopic: 0.25,
            emissive_scale: AUTHORED_EMISSIVE_TO_NIGHT,
            ..Self::default()
        }
    }

    /// Overcast rainy night: wet streets with puddles, low clouds lit orange
    /// by the city, no moon or stars, a misty air that halos every lamp.
    pub fn rainy_night() -> Self {
        let cloud = crate::lights::kelvin(2400.0);
        Self {
            sun_illuminance: 14.0 * 3.0e-7,
            fog_density: 0.0028,
            fog_falloff: 0.008,
            night_sky: NightSky { zenith: cloud * nits(0.012), glow: cloud * nits(0.08), glow_height: 0.2, stars: 0.0 },
            ambient_fill: crate::lights::kelvin(2500.0) * nits(0.045),
            wetness: 1.0,
            puddles: 0.6,
            bloom: 0.055,
            ..Self::night()
        }
        .with_wetness(1.0, 0.6)
    }

    /// Rain-wet surfaces: `wetness` 0..1 darkens porous materials and
    /// smooths everything; `puddles` 0..1 covers flat ground with mirrors.
    pub fn with_wetness(mut self, wetness: f32, puddles: f32) -> Self {
        self.wetness = wetness.clamp(0.0, 1.0);
        self.puddles = puddles.clamp(0.0, 1.0);
        self
    }

    /// True when any night feature is on (local light glow, sprites, auto
    /// exposure or wetness).
    pub fn is_night_aware(&self) -> bool {
        self.auto_exposure.is_some() || self.light_scatter > 0.0 || self.light_sprites > 0.0 || self.wetness > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_dark_and_analytic() {
        for e in [Environment::night(), Environment::dusk(), Environment::rainy_night()] {
            assert!(!e.use_ibl, "night presets must not use the daytime HDRI");
            assert!(e.auto_exposure.is_some());
            assert!(e.light_scatter > 0.0 && e.light_sprites > 0.0);
        }
        let n = Environment::night();
        let sky = crate::atmosphere::sky_lighting(n.sun_dir, n.sun_illuminance, Vec3::splat(0.25));
        // Moonlight ~0.25 lux, i.e. 2.5e-5 units of illuminance at most.
        assert!(sky.sun_color.length() < 5e-5, "moon too bright: {}", sky.sun_color);
        let r = Environment::rainy_night();
        assert!(r.wetness > 0.9 && r.puddles > 0.0 && r.fog_density > n.fog_density);
        // Daytime defaults keep every night feature off.
        let d = Environment::default();
        assert!(!d.is_night_aware() && d.emissive_scale == 1.0 && d.bloom_radius == 1.0);
    }
}
