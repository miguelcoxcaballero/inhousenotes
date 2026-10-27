//! What traffic AI needs to know about the streets, exported by the layout
//! from the very data it paints the markings and places the signs with:
//! street pieces with their lanes and parking strips, every sign plate and
//! every traffic light (with the module variants for each aspect, so the
//! game can show the state the drivers obey).
//!
//! It is deliberately a plain description (no junction topology): the AI
//! builds its own road graph from it (`forja_ai::adapter`), so a different
//! street generator only has to fill these same records.

use forja_math::Vec2;

use crate::texgen::SignKind;

/// One driving lane: offset from the centreline (left of `a -> b`
/// positive), direction and width.
#[derive(Clone, Copy, Debug)]
pub struct TrafficLane {
    pub offset: f32,
    pub forward: bool,
    pub width: f32,
}

/// A straight street piece between building lines (`a`, `b` on the
/// centreline). Long avenues may cross several junctions.
#[derive(Clone, Debug)]
pub struct TrafficStreet {
    pub a: Vec2,
    pub b: Vec2,
    /// Building line to centreline.
    pub half_width: f32,
    /// Kerb to centreline.
    pub road_half: f32,
    pub sidewalk: f32,
    /// Half width of the raised median (0 if none).
    pub median_half: f32,
    pub lanes: Vec<TrafficLane>,
    /// Centre offsets of the parking strips.
    pub parking: Vec<f32>,
    /// Default limit before speed signs (km/h).
    pub speed_kmh: f32,
    /// Zebra centre and stop line, distance from each street end.
    pub crosswalk: Option<f32>,
    pub stop_line: f32,
}

/// A sign plate: `facing` is its normal (towards the drivers it addresses).
#[derive(Clone, Copy, Debug)]
pub struct TrafficSign {
    pub kind: SignKind,
    pub pos: Vec2,
    pub facing: Vec2,
}

/// A traffic light placement (index into `CityOut::placements`) facing
/// the traffic it controls, with the module scene of every state:
/// `variants[aspect][walk]`, aspect 0 red, 1 amber, 2 green; `walk` is the
/// pedestrian head, which serves the zebra across this same street.
#[derive(Clone, Copy, Debug)]
pub struct SignalPost {
    pub pos: Vec2,
    pub facing: Vec2,
    pub placement: usize,
    pub variants: [[usize; 2]; 3],
}

#[derive(Clone, Debug, Default)]
pub struct TrafficInfo {
    pub streets: Vec<TrafficStreet>,
    pub signs: Vec<TrafficSign>,
    pub signals: Vec<SignalPost>,
    /// Plazas `(min, max)`.
    pub plazas: Vec<(Vec2, Vec2)>,
    /// Benches: position and the yaw they face.
    pub benches: Vec<(Vec2, f32)>,
    /// Centres of the openings in raised medians (turns across allowed).
    pub median_openings: Vec<Vec2>,
}

/// Plate normal of a module placed with `yaw` (furniture faces +Z).
pub fn facing_of_yaw(yaw: f32) -> Vec2 {
    Vec2::new(yaw.sin(), yaw.cos())
}
