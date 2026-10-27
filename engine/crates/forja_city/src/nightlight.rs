//! Night lighting content: the time of day that switches the city's lamps,
//! windows, shops and signs on, the light records the kit emits
//! ([`CityLight`], point or spot), reusable fixtures (facade floodlights,
//! accent spots, fountain lights, vehicle lamps) and the per-frame light
//! budget ([`select`]) with far-light glow points ([`glow_parts`]) so a
//! renderer with a bounded light list still shows a carpet of lights from
//! the air.
//!
//! Units follow the renderer's point lights: `intensity` is roughly
//! candela / 20 and emissive radiance roughly nits / 20 (the shader
//! multiplies material emissive by 4), so a night exposure of 3-6 shows a
//! 15 lux street at mid grey.
//!
//! Daytime (`NightState::dark == 0`) changes nothing in the generated
//! city: every night decision uses position hashes, never the layout's
//! random stream.

use forja_math::{Mat4, Vec2, Vec3};

use crate::geom::Parts;
use crate::kit::{Mat, Rgb};
use crate::noise::{hashf, smoothstep};

/// Time of day as the city content sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NightState {
    /// Local solar-ish clock time, 0..24.
    pub hour: f32,
    /// 0 by day, 1 in full night (lamps on), ramping through dusk/dawn.
    pub dark: f32,
}

impl Default for NightState {
    fn default() -> Self {
        Self::DAY
    }
}

/// Late-October Madrid/Valencia: sunset ~19:00, sunrise ~8:00 (CET).
pub const SUNSET: f32 = 19.0;
pub const SUNRISE: f32 = 8.0;
/// Street lighting switches on during civil twilight.
const TWILIGHT: f32 = 0.75;

impl NightState {
    pub const DAY: NightState = NightState { hour: 12.0, dark: 0.0 };

    pub fn at(hour: f32) -> Self {
        let h = hour.rem_euclid(24.0);
        let evening = smoothstep(SUNSET - 0.1, SUNSET + TWILIGHT, h);
        let morning = 1.0 - smoothstep(SUNRISE - TWILIGHT, SUNRISE + 0.1, h);
        let dark = if h > 12.0 { evening } else { morning };
        NightState { hour: h, dark }
    }

    pub fn is_night(&self) -> bool {
        self.dark > 0.0
    }

    /// Street lamps are on (photocells trip early in dusk).
    pub fn lamps_on(&self) -> bool {
        self.dark > 0.15
    }

    /// Fraction of flats with someone awake and a light on. Spanish
    /// evenings run late: dinner at 21-22 h, most lights out by 1-2 h.
    pub fn lit_flats(&self) -> f32 {
        const CURVE: [(f32, f32); 14] = [(0.0, 0.34), (1.0, 0.2), (2.0, 0.1), (3.0, 0.06), (4.0, 0.05), (5.0, 0.07), (6.0, 0.18), (7.0, 0.32), (8.0, 0.2), (18.0, 0.2), (19.0, 0.32), (20.0, 0.5), (22.0, 0.62), (23.0, 0.5)];
        piecewise(&CURVE, self.hour, 24.0)
    }

    /// Chance that premises of `kind` are open at this hour.
    pub fn open_chance(&self, kind: Premises) -> f32 {
        let h = self.hour;
        match kind {
            // Pharmacies near you are the 24 h duty one or close at 22 h.
            Premises::Pharmacy => {
                if (9.0..22.0).contains(&h) {
                    1.0
                } else {
                    0.5
                }
            }
            // Bars: breakfast from 7, busiest at 21-24, last ones shut ~2:30.
            Premises::Bar => piecewise(&[(0.0, 0.7), (1.0, 0.45), (2.0, 0.2), (3.0, 0.06), (6.0, 0.06), (7.0, 0.5), (8.0, 0.85), (23.0, 0.85)], h, 24.0),
            // Shops: 10-14 and 17-21 h, a few supermarkets until 22 h.
            Premises::Shop => piecewise(&[(0.0, 0.06), (2.0, 0.03), (9.0, 0.03), (10.0, 0.9), (20.5, 0.85), (21.25, 0.4), (22.0, 0.25), (23.0, 0.12)], h, 24.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Premises {
    Shop,
    Bar,
    Pharmacy,
}

/// Linear interpolation over `(x, y)` keys sorted by x, wrapping at `period`.
fn piecewise(keys: &[(f32, f32)], x: f32, period: f32) -> f32 {
    let n = keys.len();
    for i in 0..n {
        let (x0, y0) = keys[i];
        let (x1, y1) = if i + 1 < n { keys[i + 1] } else { (keys[0].0 + period, keys[0].1) };
        if x >= x0 && x < x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    // Before the first key: wrap from the last one.
    let (xl, yl) = keys[n - 1];
    let (x0, y0) = keys[0];
    let t = (x + period - xl) / (x0 + period - xl);
    yl + (y0 - yl) * t
}

/// Normalized color of a black body at `k` kelvin (max channel 1, linear),
/// Tanner Helland's fit.
pub fn kelvin(k: f32) -> Vec3 {
    let t = k / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.7 * (t - 60.0).powf(-0.1332) };
    let g = if t <= 66.0 { 99.47 * t.ln() - 161.12 } else { 288.12 * (t - 60.0).powf(-0.0755) };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.52 * (t - 10.0).ln() - 305.04
    };
    let s = |v: f32| crate::noise::srgb_to_linear((v / 255.0).clamp(0.0, 1.0));
    let c = Vec3::new(s(r), s(g), s(b));
    c / c.max_element().max(1e-6)
}

/// High-pressure sodium: the orange of old Spanish streets (~2050 K, with
/// the lamp's yellow-orange spectral peak).
pub fn sodium() -> Vec3 {
    Vec3::new(1.0, 0.43, 0.09)
}

/// Converts a linear color to the kit's sRGB key.
pub fn to_rgb(c: Vec3) -> Rgb {
    let f = |v: f32| (crate::noise::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
    [f(c.x), f(c.y), f(c.z)]
}

/// Spot cone of a [`CityLight`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    /// Unit direction the light points to.
    pub dir: Vec3,
    /// Half angles (radians): full intensity inside `inner`, none past `outer`.
    pub inner: f32,
    pub outer: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LightKind {
    /// Cast-iron lantern or candelabra on a sidewalk or plaza.
    Street,
    /// Tall road light (báculo) over a carriageway.
    Road,
    /// Facade lantern on a bracket (narrow streets).
    Wall,
    /// Light spilling out of an open shop or bar.
    Shop,
    /// Lit sign, neon or pharmacy cross.
    Sign,
    /// Lit window of a flat.
    Window,
    /// Ground floodlight washing a facade.
    Flood,
    /// Accent spot on a cornice, tower or statue.
    Accent,
    /// Underwater fountain light.
    Fountain,
    /// Traffic or pedestrian signal lens.
    Signal,
    /// Vehicle head, tail or brake lamp.
    Vehicle,
}

impl LightKind {
    /// How much a light of this kind matters when the budget is tight
    /// (the lights that shape the street come first).
    pub fn weight(self) -> f32 {
        match self {
            LightKind::Road | LightKind::Street | LightKind::Flood => 1.0,
            LightKind::Wall => 0.9,
            LightKind::Vehicle => 0.85,
            LightKind::Shop | LightKind::Accent | LightKind::Fountain => 0.7,
            LightKind::Sign => 0.5,
            LightKind::Signal => 0.3,
            LightKind::Window => 0.25,
        }
    }
}

/// One light of the city.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CityLight {
    pub pos: Vec3,
    /// Linear color, max channel 1.
    pub color: Vec3,
    /// Renderer point-light intensity (~ candela / 20).
    pub intensity: f32,
    /// Distance where the light fades out.
    pub range: f32,
    /// `None`: omnidirectional point light.
    pub spot: Option<Spot>,
    pub kind: LightKind,
    /// World size of the glowing part (for far glow points and flares).
    pub size: f32,
}

impl CityLight {
    pub fn point(kind: LightKind, pos: Vec3, color: Vec3, intensity: f32, range: f32) -> Self {
        CityLight { pos, color, intensity, range, spot: None, kind, size: 0.2 }
    }

    pub fn spot(kind: LightKind, pos: Vec3, dir: Vec3, outer_deg: f32, color: Vec3, intensity: f32, range: f32) -> Self {
        let outer = outer_deg.to_radians();
        CityLight { pos, color, intensity, range, spot: Some(Spot { dir: dir.normalize_or(Vec3::NEG_Y), inner: outer * 0.6, outer }), kind, size: 0.15 }
    }

    pub fn with_size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }

    /// Rough importance for a viewer at `eye`: illuminance scale of the
    /// light where its range reaches toward the viewer.
    pub fn importance(&self, eye: Vec3) -> f32 {
        let d = (self.pos - eye).length();
        let reach = (d - self.range).max(0.0);
        self.kind.weight() * self.intensity * self.range * self.range / (1.0 + reach * reach) / (1.0 + d * 0.02)
    }

    /// The light as a point for renderers without spot lights: a spot is
    /// moved along its axis to where its cone lands, so a ground
    /// floodlight still lights the facade it washes.
    pub fn as_point(&self) -> (Vec3, Vec3, f32, f32) {
        match self.spot {
            None => (self.pos, self.color, self.intensity, self.range),
            Some(s) => {
                let shift = (self.range * 0.3).min(6.0);
                let cone = (1.0 - s.outer.cos()).clamp(0.05, 1.0) * 2.0;
                (self.pos + s.dir * shift, self.color, self.intensity * cone.min(1.0) * 0.8, self.range * 0.75)
            }
        }
    }
}

/// Deterministic per-feature randomness for night decisions (so daytime
/// generation, which uses the layout's random stream, is untouched).
pub fn hash01(p: Vec3, salt: u32) -> f32 {
    let q = (p * 4.0).round();
    let a = hashf(q.x as i32, q.z as i32, salt.wrapping_mul(0x9e37_79b9) ^ (q.y as i32 as u32).wrapping_mul(0x85eb_ca6b));
    a
}

// ------------------------------------------------------------------ fixtures

/// Street lamp of a district: old sodium (Madrid keeps more of them) or a
/// newer warm-white LED retrofit, chosen per lamp position.
pub fn street_lamp_color(pos: Vec3, madrid: bool) -> (Vec3, bool) {
    let h = hash01(pos, 11);
    let sodium_share = if madrid { 0.7 } else { 0.45 };
    // A few quantized tones so lamps share their modules: older sodium
    // lamps drift between ~1950 and 2200 K, LED retrofits are 2700-3000 K.
    if h < sodium_share {
        let t = (hash01(pos, 12) * 3.0).floor() / 2.0;
        (sodium().lerp(kelvin(2200.0), t * 0.35), true)
    } else {
        (kelvin(if hash01(pos, 13) < 0.5 { 2700.0 } else { 3000.0 }), false)
    }
}

/// Outer half angle of the full cut-off street luminaires (degrees from
/// the nadir): nothing is emitted above ~70 degrees, so lamps light the
/// ground, facades only get the low spill and the sky none.
pub const CUTOFF_DEG: f32 = 68.0;

/// Light of a lantern-style street lamp whose bulb is at `bulb`: a full
/// cut-off cone straight down (modern lanterns hide the source above a
/// reflector; Madrid and Valencia retrofitted them for light pollution).
pub fn street_lamp(bulb: Vec3, color: Vec3, candelabra: bool) -> CityLight {
    let (i, r) = if candelabra { (60.0, 16.0) } else { (40.0, 14.0) };
    CityLight::spot(LightKind::Street, bulb, Vec3::NEG_Y, CUTOFF_DEG, color, i, r).with_size(if candelabra { 0.9 } else { 0.35 })
}

/// Facade lantern on a bracket: a cut-off cone aimed down and a little away
/// from the wall it hangs on (`out`, the facade normal), dimmer than a
/// street lamp so the wall above stays dark.
pub fn wall_lantern(bulb: Vec3, color: Vec3, out: Vec3) -> CityLight {
    let dir = (Vec3::NEG_Y + out.normalize_or(Vec3::ZERO) * 0.3).normalize();
    CityLight::spot(LightKind::Wall, bulb, dir, CUTOFF_DEG - 4.0, color, 14.0, 10.0).with_size(0.3)
}

/// Tall LED road light aimed down at the carriageway.
pub fn road_light(head: Vec3, color: Vec3) -> CityLight {
    CityLight::spot(LightKind::Road, head, Vec3::NEG_Y, 70.0, color, 160.0, 34.0).with_size(0.5)
}

/// Options of [`facade_floodlights`].
#[derive(Clone, Copy, Debug)]
pub struct FloodOptions {
    /// Spacing of the ground fixtures along the facade.
    pub spacing: f32,
    /// Distance of the fixtures in front of the wall.
    pub setback: f32,
    /// Color temperature (gold sodium-like washes are ~2400-2800 K).
    pub kelvin: f32,
    /// Overall strength multiplier.
    pub strength: f32,
    /// Add accent spots along the cornice aimed at the parapet/attic.
    pub cornice_accents: bool,
}

impl Default for FloodOptions {
    fn default() -> Self {
        FloodOptions { spacing: 6.0, setback: 1.6, kelvin: 2600.0, strength: 1.0, cornice_accents: true }
    }
}

/// Ground-mounted spots washing the facade from `a` to `b` (on the wall
/// plane at ground level; `out` the wall normal) up to `height` above the
/// ground, the way Madrid's and Valencia's civic buildings are lit: warm
/// grazing light from below, brightest at the plinth, fading toward the
/// cornice, with accents along the cornice itself.
pub fn facade_floodlights(a: Vec3, b: Vec3, out: Vec3, height: f32, o: &FloodOptions) -> Vec<CityLight> {
    let len = (b - a).length();
    let mut lights = Vec::new();
    if len < 1.0 || height < 2.0 {
        return lights;
    }
    let t = (b - a) / len;
    let out = Vec3::new(out.x, 0.0, out.z).normalize_or(Vec3::Z);
    let n = (len / o.spacing).round().max(1.0) as usize;
    let color = kelvin(o.kelvin);
    // Aim at two thirds of the height on the wall; the cone wide enough
    // to cover one bay to each side.
    let range = (height * 1.35 + o.setback).max(8.0);
    for k in 0..n {
        let u = (k as f32 + 0.5) * len / n as f32;
        let foot = a + t * u + out * o.setback + Vec3::Y * 0.25;
        let target = a + t * u + Vec3::Y * height * 0.62;
        let outer = (((o.spacing * 0.7) / height.max(3.0)).atan().to_degrees() + 12.0).clamp(18.0, 40.0);
        lights.push(CityLight::spot(LightKind::Flood, foot, target - foot, outer, color, 220.0 * o.strength * (height / 15.0).clamp(0.6, 2.5), range).with_size(0.25));
    }
    if o.cornice_accents && height > 8.0 {
        // Small spots on the cornice ledge aimed up at the attic.
        let m = ((len / (o.spacing * 1.5)).round() as usize).max(1);
        let warm = kelvin(o.kelvin - 200.0);
        for k in 0..m {
            let u = (k as f32 + 0.5) * len / m as f32;
            let p = a + t * u + out * 0.6 + Vec3::Y * (height - 1.2);
            lights.push(CityLight::spot(LightKind::Accent, p, Vec3::Y * 1.0 - out * 0.35, 30.0, warm, 35.0 * o.strength, 6.0).with_size(0.1));
        }
    }
    lights
}

/// Accent spot at `pos` aimed at `target` (towers, domes, statues).
pub fn accent(pos: Vec3, target: Vec3, kelvin_k: f32, intensity: f32) -> CityLight {
    let d = target - pos;
    CityLight::spot(LightKind::Accent, pos, d, 22.0, kelvin(kelvin_k), intensity, d.length() * 1.4 + 2.0).with_size(0.1)
}

/// Cyan underwater lights around a fountain basin of `radius` centered at
/// `center` (water surface height), aimed up at the jets as at Cibeles.
pub fn fountain_lights(center: Vec3, radius: f32) -> Vec<CityLight> {
    let cyan = Vec3::new(0.25, 0.85, 1.0);
    let n = ((radius * 1.2) as usize).clamp(3, 8);
    let mut v: Vec<CityLight> = (0..n)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / n as f32;
            let p = center + Vec3::new(a.cos(), 0.0, a.sin()) * radius * 0.75 - Vec3::Y * 0.1;
            CityLight::spot(LightKind::Fountain, p, Vec3::Y * 2.0 - (p - center).normalize_or(Vec3::X), 35.0, cyan, 30.0, radius + 6.0).with_size(0.2)
        })
        .collect();
    v.push(CityLight::point(LightKind::Fountain, center + Vec3::Y * 1.2, cyan, 25.0, radius * 1.6 + 4.0).with_size(radius * 0.4));
    v
}

/// Head, tail and (optionally) brake lamps of a vehicle with transform
/// `m` (+Z forward, origin on the ground at the centre), `half` its half
/// extents (x width, y height, z length).
pub fn vehicle_lights(m: &Mat4, half: Vec3, headlights: bool, braking: bool) -> Vec<CityLight> {
    let mut v = Vec::new();
    let fwd = m.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
    let y = (half.y * 0.55).max(0.55);
    for side in [-1.0f32, 1.0] {
        let x = side * (half.x - 0.25);
        if headlights {
            let p = m.transform_point3(Vec3::new(x, y, half.z + 0.05));
            let dir = (fwd - Vec3::Y * 0.06).normalize();
            v.push(CityLight::spot(LightKind::Vehicle, p, dir, 28.0, kelvin(5200.0), 260.0, 40.0).with_size(0.12));
        }
        let p = m.transform_point3(Vec3::new(x, y + 0.1, -half.z - 0.05));
        let (i, r) = if braking { (14.0, 7.0) } else { (4.0, 4.0) };
        v.push(CityLight::spot(LightKind::Vehicle, p, -fwd, 70.0, Vec3::new(1.0, 0.02, 0.01), i, r).with_size(0.1));
    }
    v
}

// --------------------------------------------------------------- light budget

/// Picks at most `budget` lights for a viewer at `eye` looking along
/// `fwd`: the most important ones (by intensity, range and distance),
/// with lights behind the camera discounted unless their range reaches
/// it. Returns indices into `lights`, most important first.
pub fn select(lights: &[CityLight], eye: Vec3, fwd: Vec3, budget: usize) -> Vec<usize> {
    let fwd = fwd.normalize_or(Vec3::Z);
    let mut scored: Vec<(f32, usize)> = lights
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let to = l.pos - eye;
            let d = to.length();
            // Lights reach what the camera sees up to their range.
            let ahead = to.dot(fwd) + l.range;
            if ahead < 0.0 {
                return None;
            }
            let facing = if d < l.range { 1.0 } else { (0.35 + 0.65 * (to.dot(fwd) / d.max(1e-3)).max(0.0)).min(1.0) };
            let s = l.importance(eye) * facing;
            (s > 0.0).then_some((s, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(budget).map(|(_, i)| i).collect()
}

/// Far lights as small emissive octahedra so distant lamps stay visible
/// points when their own glass is sub-pixel and they are out of the light
/// budget. Lights closer than `near` are skipped (their fixtures show
/// them); the size grows with distance to cover ~`px` pixels of
/// `pixel_angle` radians each. Rebuild when the camera moves far.
pub fn glow_parts(lights: &[CityLight], eye: Vec3, near: f32, pixel_angle: f32, px: f32) -> Parts<Mat> {
    let mut p: Parts<Mat> = Parts::new();
    for l in lights {
        let d = (l.pos - eye).length();
        // Floodlit walls and fountains are large lit surfaces; windows,
        // shopfronts and signs too until they shrink to a pixel or two, then they
        // become dimmer points of the carpet of lights.
        let (min_d, dim) = match l.kind {
            LightKind::Flood | LightKind::Accent | LightKind::Fountain => continue,
            LightKind::Window | LightKind::Shop | LightKind::Sign => (near * 4.0, 0.35),
            _ => (near, 1.0),
        };
        if d < min_d {
            continue;
        }
        // A point, never a blob: at most a pixel or two, or the lamp head.
        let r = (d * pixel_angle * px * 0.5).max((l.size * 0.5).min(0.25));
        let key = Mat::Glow(to_rgb(l.color * dim));
        octahedron(p.get(key), l.pos, r);
    }
    p
}

fn octahedron(prim: &mut crate::geom::Prim, c: Vec3, r: f32) {
    let ax = [Vec3::X * r, Vec3::Z * r, -Vec3::X * r, -Vec3::Z * r];
    let (top, bot) = (c + Vec3::Y * r, c - Vec3::Y * r);
    for i in 0..4 {
        let (a, b) = (c + ax[i], c + ax[(i + 1) % 4]);
        prim.tri(a, b, top, [Vec2::ZERO, Vec2::X, Vec2::Y]);
        prim.tri(b, a, bot, [Vec2::ZERO, Vec2::X, Vec2::Y]);
    }
}

/// Tones of lit rooms: incandescent, warm LED, neutral, fluorescent,
/// television. `(color, brightness)`.
pub const ROOM_TONES: usize = 5;

pub fn room_tone(t: u8) -> (Vec3, f32) {
    match t % ROOM_TONES as u8 {
        0 => (kelvin(2600.0), 1.0),
        1 => (kelvin(3000.0), 1.15),
        2 => (kelvin(4000.0), 1.2),
        3 => (Vec3::new(0.82, 1.0, 0.9), 1.25),
        _ => (Vec3::new(0.42, 0.58, 1.0), 0.45),
    }
}

/// Picks a room tone for a flat: warm light dominates Spanish homes,
/// some cool LED and fluorescent kitchens, a few TV-only rooms.
pub fn pick_room_tone(h: f32) -> u8 {
    match h {
        x if x < 0.34 => 0,
        x if x < 0.62 => 1,
        x if x < 0.8 => 2,
        x if x < 0.9 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darkness_follows_the_clock() {
        assert_eq!(NightState::at(12.0).dark, 0.0);
        assert_eq!(NightState::at(15.5).dark, 0.0);
        assert!(NightState::at(22.0).dark > 0.99);
        assert!(NightState::at(3.0).dark > 0.99);
        let dusk = NightState::at(SUNSET + 0.3).dark;
        assert!(dusk > 0.05 && dusk < 0.95, "{dusk}");
        assert!(NightState::at(SUNRISE + 0.5).dark == 0.0);
    }

    #[test]
    fn evenings_are_busier_than_small_hours() {
        let at = |h| NightState::at(h).lit_flats();
        assert!(at(22.0) > 0.5 && at(3.5) < 0.1 && at(0.5) > at(2.5));
        assert!(NightState::at(23.0).open_chance(Premises::Bar) > NightState::at(23.0).open_chance(Premises::Shop) * 5.0);
        assert!(NightState::at(4.0).open_chance(Premises::Pharmacy) > 0.0);
    }

    #[test]
    fn kelvin_and_sodium_are_warm() {
        let k2 = kelvin(2000.0);
        let k6 = kelvin(6500.0);
        assert!(k2.x > k2.y && k2.y > k2.z);
        assert!((k6.x - k6.z).abs() < 0.15, "{k6:?}");
        assert!(sodium().z < 0.2);
    }

    #[test]
    fn street_luminaires_are_full_cut_off() {
        let c = kelvin(2700.0);
        for l in [street_lamp(Vec3::Y * 4.0, c, true), street_lamp(Vec3::Y * 3.5, c, false), wall_lantern(Vec3::Y * 3.0, c, Vec3::Z), road_light(Vec3::Y * 9.0, c)] {
            let s = l.spot.expect("street luminaires are downward cones");
            // No light at or above the horizontal: the cone's upper edge
            // stays below it.
            let nadir = s.dir.dot(Vec3::NEG_Y).clamp(-1.0, 1.0).acos();
            assert!(nadir + s.outer < std::f32::consts::FRAC_PI_2, "{:?}: tilt {nadir} + cone {}", l.kind, s.outer);
        }
        // A wall lantern leans away from its wall.
        assert!(wall_lantern(Vec3::Y * 3.0, c, Vec3::Z).spot.unwrap().dir.z > 0.1);
    }

    #[test]
    fn floodlights_wash_the_wall_upward() {
        let ls = facade_floodlights(Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0), Vec3::Z, 18.0, &FloodOptions::default());
        let floods: Vec<_> = ls.iter().filter(|l| l.kind == LightKind::Flood).collect();
        assert!(floods.len() >= 3);
        for l in floods {
            let s = l.spot.unwrap();
            assert!(s.dir.y > 0.8 && s.dir.z < 0.0, "aim {:?}", s.dir);
            assert!(l.pos.z > 1.0 && l.pos.y < 0.5);
        }
    }

    #[test]
    fn selection_is_bounded_and_prefers_near_bright_lights() {
        let mut ls = Vec::new();
        for i in 0..2000 {
            let p = Vec3::new((i % 50) as f32 * 10.0, 4.0, (i / 50) as f32 * 10.0);
            ls.push(CityLight::point(if i % 3 == 0 { LightKind::Window } else { LightKind::Street }, p, Vec3::ONE, 50.0, 15.0));
        }
        let eye = Vec3::new(100.0, 1.7, 100.0);
        let sel = select(&ls, eye, Vec3::Z, 64);
        assert_eq!(sel.len(), 64);
        let far = sel.iter().filter(|&&i| (ls[i].pos - eye).length() > 120.0).count();
        assert!(far < 4, "{far} far lights selected");
        // Nothing well behind the camera.
        assert!(sel.iter().all(|&i| (ls[i].pos - eye).z > -20.0));
    }

    #[test]
    fn vehicle_lights_point_forward_and_back() {
        let m = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
        let v = vehicle_lights(&m, Vec3::new(0.9, 0.7, 2.2), true, true);
        assert_eq!(v.len(), 4);
        assert!(v.iter().any(|l| l.spot.unwrap().dir.z > 0.9));
        assert!(v.iter().any(|l| l.spot.unwrap().dir.z < -0.9 && l.color.x > l.color.y * 10.0));
    }
}
