//! Time of day: sun path, camera and night lighting.
//!
//! By day the physical camera meters the HDR photo or the analytic sky.
//! Once the sun is down the renderer's night pipeline takes over
//! ([`Environment::dusk`] blending into [`Environment::night`]: eye
//! adaptation, night sky, fog glow, light sprites, night grade), lit by the
//! city's own night content (`WorldDescription::night_lights`: lamps, lit
//! windows, shops, signs, floodlights) through the clustered lights. The
//! procedural city bakes its lit windows and lanterns for the hour it is
//! built at, so the game rebuilds it when the clock crosses dusk or dawn
//! (see [`TimeOfDay::night_content`]).

use forja_core::{App, Plugin, Stage, Time};
use forja_math::Vec3;
use forja_render::{Environment, Light, Spot, kelvin};
use forja_world::{CityLight, LightKind};

use super::RenderScene;
use super::world::GameWorld;

pub struct EnvPlugin;

pub struct TimeOfDay {
    /// Light the world with the HDR photo (fixed time of day) when loaded.
    pub photo_sky: bool,
    /// 0..24.
    pub hours: f32,
    /// Game hours per real second (0 freezes time).
    pub speed: f32,
}

/// Sun elevation (sine) where the night camera takes over from the daylight
/// one, and where dusk has fully become night.
const DUSK_START: f32 = 0.02;
const NIGHT_FULL: f32 = -0.16;

impl TimeOfDay {
    pub fn sun_dir(&self) -> Vec3 {
        let day = (self.hours - 6.0) / 12.0 * std::f32::consts::PI;
        let elevation = day.sin() * 62f32.to_radians();
        let azimuth = day.cos() * 1.3 + 0.4;
        Vec3::new(elevation.cos() * azimuth.cos(), elevation.sin(), elevation.cos() * azimuth.sin()).normalize()
    }

    /// 0 at full day, 1 at night.
    pub fn night(&self) -> f32 {
        let y = self.sun_dir().y;
        1.0 - smoothstep(-0.08, 0.12, y)
    }

    /// `None` by day; `Some(t)` once the sun is down: 0 at the start of
    /// dusk, 1 in full night.
    pub fn dusk_to_night(&self) -> Option<f32> {
        if self.photo_sky {
            return None;
        }
        let y = self.sun_dir().y;
        (y < DUSK_START).then(|| smoothstep(DUSK_START, NIGHT_FULL, y))
    }

    /// The city's night content (lit windows, lamps on) belongs to this
    /// time of day.
    pub fn night_content(&self) -> bool {
        self.dusk_to_night().is_some()
    }
}

/// Clock hour to build the city content for at game hour `h`: `None` by
/// day. The game's sun sets at 18 h and rises at 6 h, the content's lamp
/// schedule (`forja_city::NightState`) runs an hour later in the evening
/// and two later in the morning (late-October Spain), so night hours are
/// moved into the content's full night.
pub fn content_hour(h: f32) -> Option<f32> {
    let h = h.rem_euclid(24.0);
    TimeOfDay { photo_sky: false, hours: h, speed: 0.0 }.night_content().then(|| if h >= 12.0 { h.max(20.0) } else { h.min(6.0) })
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_log(a: f32, b: f32, t: f32) -> f32 {
    (a.max(1e-12).ln() * (1.0 - t) + b.max(1e-12).ln() * t).exp()
}

/// Blue hour (`t` = 0) to full night (`t` = 1) with the sun at `sun`: the
/// twilight sky comes from the atmosphere while the sun is just below the
/// horizon, then the moon of [`Environment::night`] takes over.
pub fn night_environment(t: f32, sun: Vec3) -> Environment {
    let (a, b) = (Environment::dusk(), Environment::night());
    let mut e = b.clone();
    let moon = t > 0.9;
    e.sun_dir = if moon { b.sun_dir } else { sun };
    e.sun_illuminance = lerp_log(a.sun_illuminance, b.sun_illuminance, t);
    e.exposure = lerp_log(a.exposure, b.exposure, t);
    e.fog_density = lerp(a.fog_density, b.fog_density, t);
    e.fog_falloff = lerp(a.fog_falloff, b.fog_falloff, t);
    e.vignette = lerp(a.vignette, b.vignette, t);
    if let (Some(ea), Some(eb)) = (a.auto_exposure, b.auto_exposure) {
        e.auto_exposure = Some(forja_render::EyeAdaptation { compensation_ev: lerp(ea.compensation_ev, eb.compensation_ev, t), ..eb });
    }
    e.night_sky.zenith = a.night_sky.zenith.lerp(b.night_sky.zenith, t);
    e.night_sky.glow = a.night_sky.glow.lerp(b.night_sky.glow, t);
    e.night_sky.glow_height = lerp(a.night_sky.glow_height, b.night_sky.glow_height, t);
    e.night_sky.stars = b.night_sky.stars * t;
    e.ambient_fill = a.ambient_fill.lerp(b.ambient_fill, t);
    e.bloom = lerp(a.bloom, b.bloom, t);
    e.bloom_radius = lerp(a.bloom_radius, b.bloom_radius, t);
    e.bloom_limit = lerp(a.bloom_limit, b.bloom_limit, t);
    e.scotopic = lerp(a.scotopic, b.scotopic, t);
    e.night_grade = lerp(a.night_grade, b.night_grade, t);
    e
}

/// The kit's light intensities are ~candela / 20; renderer units are
/// 10^4 cd (as `forja_render::night::AUTHORED_EMISSIVE_TO_NIGHT`).
const CONTENT_TO_UNITS: f32 = 20.0 / 1.0e4;

/// A content light of the night city as a clustered renderer light.
/// Traffic signals are skipped: their lenses follow the AI's live state
/// (`traffic.rs`), the content's are a frozen snapshot.
pub fn from_city(l: &CityLight) -> Option<Light> {
    if l.kind == LightKind::Signal {
        return None;
    }
    let lum = l.color.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-3);
    let mut out = Light::point(l.pos, l.color / lum, 0.0);
    out.intensity = l.intensity * lum * CONTENT_TO_UNITS;
    out.range = l.range;
    out.source_radius = (l.size * 0.5).clamp(0.02, 1.5);
    out.spot = l.spot.map(|s| Spot { direction: s.dir, inner_angle: s.inner, outer_angle: s.outer });
    out.scatter = match l.kind {
        LightKind::Window => 0.0,
        LightKind::Flood | LightKind::Accent => 0.3,
        _ => 1.0,
    };
    Some(out)
}

/// Physical lamps for maps without night content (`(position, colour)`
/// lamp points): high road lights become sodium spots aimed down, street
/// lanterns warm points.
pub fn lamp_light(p: Vec3, c: Vec3) -> Light {
    let saturated = c.max_element() - c.min_element() > 0.6 && c.x < 0.5;
    if saturated {
        let lum = c.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-3);
        Light::point(p, c / lum, 600.0).with_source_radius(0.3).with_range(9.0)
    } else if p.y > 7.0 {
        Light::spot(p, Vec3::NEG_Y, kelvin(2050.0), 16000.0, 0.75, 1.25).with_source_radius(0.2)
    } else if p.y > 3.6 {
        Light::point(p, kelvin(2800.0), 4500.0).with_source_radius(0.18)
    } else {
        Light::point(p, kelvin(2400.0), 1800.0).with_source_radius(0.12)
    }
}

/// The map's night lights, converted once.
#[derive(Default)]
pub struct NightLights {
    pub lights: Vec<Light>,
    built: bool,
}

/// Lights farther than this from the camera are not submitted (the
/// renderer culls the rest by frustum and its per-frame budget).
const NIGHT_LIGHT_RADIUS: f32 = 600.0;

impl Plugin for EnvPlugin {
    fn name(&self) -> &'static str {
        "env"
    }

    fn build(&self, app: &mut App) {
        app.insert_resource(TimeOfDay { photo_sky: true, hours: 16.5, speed: 0.0 });
        app.insert_resource(NightLights::default());
        app.add_system(Stage::Update, "time_of_day", |w| {
            let dt = w.res::<Time>().delta;
            let tod = w.res_mut::<TimeOfDay>();
            tod.hours = (tod.hours + tod.speed * dt).rem_euclid(24.0);
            let sun = tod.sun_dir();
            let night = tod.night();
            let photo = tod.photo_sky;
            let dusk = tod.dusk_to_night();
            let hdri = w.res::<RenderScene>().hdri.is_some();
            let env = &mut w.res_mut::<RenderScene>().env;
            if let Some(t) = dusk {
                // The night pipeline: dusk blending into night.
                *env = night_environment(t, sun);
                return;
            }
            // Daylight: the physical camera meters the sun and sky
            // (Environment::ev100 = None), so the photo sky and every time
            // of day expose themselves; the end of the afternoon is kept a
            // little darker than the meter's mid grey.
            let defaults = Environment::default();
            if env.camera != defaults.camera {
                *env = Environment { use_ibl: hdri, ..defaults.clone() };
            }
            env.use_ibl = photo;
            if photo {
                env.exposure_compensation = defaults.exposure_compensation;
                env.bloom = defaults.bloom;
                return;
            }
            env.sun_dir = sun;
            env.sun_illuminance = defaults.sun_illuminance;
            env.exposure_compensation = defaults.exposure_compensation - night * 2.0;
            env.bloom = defaults.bloom + night * 0.05;
        });
        // City lights at night: the map's night content (or its lamp
        // points), dimmed in through dusk.
        app.add_system_at(Stage::PostUpdate, "street_lights", 50, |w| {
            let Some(t) = w.res::<TimeOfDay>().dusk_to_night() else { return };
            if !w.res::<NightLights>().built {
                let desc = &w.res::<GameWorld>().desc;
                let lights: Vec<Light> = if desc.night_lights.is_empty() { desc.lights.iter().map(|&(p, c)| lamp_light(p, c)).collect() } else { desc.night_lights.iter().filter_map(from_city).collect() };
                log::info!("night: {} city lights", lights.len());
                *w.res_mut::<NightLights>() = NightLights { lights, built: true };
            }
            // Photocells trip early in the blue hour.
            let level = smoothstep(0.0, 0.35, t);
            let cam = w.res::<RenderScene>().camera.position;
            let r2 = NIGHT_LIGHT_RADIUS * NIGHT_LIGHT_RADIUS;
            let near: Vec<Light> = w.res::<NightLights>().lights.iter().filter(|l| (l.position - cam).length_squared() < r2).map(|l| Light { intensity: l.intensity * level, ..*l }).collect();
            w.res_mut::<RenderScene>().local_lights.extend(near);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_is_high_and_midnight_is_dark() {
        let noon = TimeOfDay { photo_sky: false, hours: 12.0, speed: 0.0 };
        assert!(noon.sun_dir().y > 0.8 && noon.night() < 0.01);
        let midnight = TimeOfDay { photo_sky: false, hours: 0.0, speed: 0.0 };
        assert!(midnight.sun_dir().y < 0.0 && midnight.night() > 0.99);
    }

    #[test]
    fn dusk_blends_into_night() {
        let day = TimeOfDay { photo_sky: false, hours: 15.0, speed: 0.0 };
        assert!(day.dusk_to_night().is_none() && !day.night_content());
        let photo = TimeOfDay { photo_sky: true, hours: 23.0, speed: 0.0 };
        assert!(photo.dusk_to_night().is_none());
        let midnight = TimeOfDay { photo_sky: false, hours: 0.0, speed: 0.0 };
        assert_eq!(midnight.dusk_to_night(), Some(1.0));
        // The blend is continuous: exposure moves monotonically from the
        // dusk preset to the night one.
        let mut last = Environment::dusk().exposure;
        for i in 0..=10 {
            let e = night_environment(i as f32 / 10.0, Vec3::new(0.8, -0.03, 0.5).normalize());
            assert!(e.exposure >= last * 0.999);
            last = e.exposure;
        }
        assert!((last - Environment::night().exposure).abs() < 1.0);
        // Night hours build lit content; day hours the daytime city.
        assert_eq!(content_hour(15.0), None);
        assert!(content_hour(22.0).is_some_and(|h| forja_world::NightState::at(h).dark > 0.99));
        assert!(content_hour(3.0).is_some_and(|h| forja_world::NightState::at(h).dark > 0.99));
    }
}
