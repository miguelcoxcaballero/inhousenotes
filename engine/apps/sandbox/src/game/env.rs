//! Time of day: sun path, exposure and night lighting.

use forja_core::{App, Plugin, Stage, Time};
use forja_math::Vec3;
use forja_render::PointLight;

use super::world::GameWorld;
use super::RenderScene;

pub struct EnvPlugin;

pub struct TimeOfDay {
    /// Light the world with the HDR photo (fixed time of day) when loaded.
    pub photo_sky: bool,
    /// 0..24.
    pub hours: f32,
    /// Game hours per real second (0 freezes time).
    pub speed: f32,
}

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
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Plugin for EnvPlugin {
    fn name(&self) -> &'static str {
        "env"
    }

    fn build(&self, app: &mut App) {
        app.insert_resource(TimeOfDay { photo_sky: true, hours: 16.5, speed: 0.0 });
        app.add_system(Stage::Update, "time_of_day", |w| {
            let dt = w.res::<Time>().delta;
            let tod = w.res_mut::<TimeOfDay>();
            tod.hours = (tod.hours + tod.speed * dt).rem_euclid(24.0);
            let sun = tod.sun_dir();
            let night = tod.night();
            let photo = tod.photo_sky;
            let env = &mut w.res_mut::<RenderScene>().env;
            env.use_ibl = photo;
            // The physical camera meters the sun and sky (Environment::ev100
            // = None), so the photo sky and every time of day expose
            // themselves; night is kept darker than the meter's mid grey.
            let defaults = forja_render::Environment::default();
            if photo {
                env.exposure_compensation = defaults.exposure_compensation;
                env.bloom = defaults.bloom;
                return;
            }
            // Below the horizon the "sun" becomes a dim moon opposite to it.
            env.sun_dir = if sun.y > -0.05 { sun } else { Vec3::new(-sun.x, (-sun.y).max(0.25), -sun.z).normalize() };
            env.sun_illuminance = if sun.y > -0.05 { 14.0 } else { 0.35 };
            env.exposure_compensation = defaults.exposure_compensation - night * 2.0;
            env.bloom = defaults.bloom + night * 0.05;
        });
        // Street lamps near the camera at night.
        app.add_system_at(Stage::PostUpdate, "street_lights", 50, |w| {
            let tod = w.res::<TimeOfDay>();
            let night = if tod.photo_sky { 0.0 } else { tod.night() };
            if night < 0.05 {
                return;
            }
            let cam = w.res::<RenderScene>().camera.position;
            let mut near: Vec<(Vec3, Vec3)> = w.res::<GameWorld>().desc.lights.iter().filter(|(p, _)| (*p - cam).length_squared() < 150.0 * 150.0).copied().collect();
            near.sort_by(|a, b| (a.0 - cam).length_squared().total_cmp(&(b.0 - cam).length_squared()));
            let rs = w.res_mut::<RenderScene>();
            for (p, c) in near.into_iter().take(40) {
                rs.lights.push(PointLight { position: p, radius: 22.0, color: c, intensity: 60.0 * night });
            }
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
}
