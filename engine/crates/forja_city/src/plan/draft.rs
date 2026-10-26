//! `Draft`: everything a generator produces, and the only thing the fabric
//! reads. A draft is plain data (serde), so any failing case of the
//! property tests is written out as a TOML fixture and replayed.

use forja_math::Vec2;
use serde::{Deserialize, Serialize};

/// A street centreline with its profile and per-vertex width (building
/// line to building line).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Way {
    pub line: Vec<Vec2>,
    /// One width per vertex, or a single value for the whole way; empty
    /// means the profile's typical width.
    #[serde(default)]
    pub width: Vec<f32>,
    pub profile: String,
    /// Roadway override (OSM measurements); `Some(0.0)` makes it shared.
    #[serde(default)]
    pub carriageway: Option<f32>,
    /// +1: traffic along `line` only, -1: against it, 0: both ways.
    #[serde(default)]
    pub oneway: i8,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub zone: u16,
    #[serde(default)]
    pub osm: Option<i64>,
}

impl Way {
    pub fn new(line: Vec<Vec2>, profile: &str) -> Self {
        Way { line, width: Vec::new(), profile: profile.into(), carriageway: None, oneway: 0, name: None, zone: 0, osm: None }
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = vec![w];
        self
    }

    pub fn zone(mut self, z: u16) -> Self {
        self.zone = z;
        self
    }

    pub fn oneway(mut self, o: i8) -> Self {
        self.oneway = o;
        self
    }

    /// Width at vertex `i`, or `fallback`.
    pub fn width_at(&self, i: usize, fallback: f32) -> f32 {
        match self.width.len() {
            0 => fallback,
            1 => self.width[0],
            n => self.width[i.min(n - 1)],
        }
    }
}

/// A tagged point: traffic signals, or a real dead end that must not be
/// pruned.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeMark {
    pub p: Vec2,
    #[serde(default)]
    pub signals: bool,
    #[serde(default)]
    pub end: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlazaKind {
    #[default]
    Paved,
    Garden,
    Fountain,
    Parterre,
}

/// An open square of any polygon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plaza {
    pub ring: Vec<Vec2>,
    #[serde(default)]
    pub zone: u16,
    #[serde(default)]
    pub kind: PlazaKind,
}

/// The land a zone may build on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub zone: u16,
    pub ring: Vec<Vec2>,
}

/// A real building outline (OSM), with patios as holes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Footprint {
    pub ring: Vec<Vec2>,
    #[serde(default)]
    pub holes: Vec<Vec<Vec2>>,
    #[serde(default)]
    pub levels: Option<u32>,
    #[serde(default)]
    pub roof_levels: Option<u32>,
    #[serde(default)]
    pub osm: i64,
    #[serde(default)]
    pub zone: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub ways: Vec<Way>,
    #[serde(default)]
    pub plazas: Vec<Plaza>,
    #[serde(default)]
    pub masks: Vec<Mask>,
    #[serde(default)]
    pub footprints: Vec<Footprint>,
    #[serde(default)]
    pub nodes: Vec<NodeMark>,
    /// Node merge distance (m): 0.05 for procedural plans, 0.3 for OSM.
    #[serde(default = "d_snap")]
    pub snap: f32,
}

fn d_snap() -> f32 {
    0.05
}

impl Draft {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn from_toml(s: &str) -> Result<Draft, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    /// Appends another draft (ways, plazas, masks, footprints, nodes).
    pub fn extend(&mut self, o: Draft) {
        self.ways.extend(o.ways);
        self.plazas.extend(o.plazas);
        self.masks.extend(o.masks);
        self.footprints.extend(o.footprints);
        self.nodes.extend(o.nodes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_round_trips_through_toml() {
        let mut d = Draft { snap: 0.05, ..Default::default() };
        d.ways.push(Way::new(vec![Vec2::new(0.0, 0.0), Vec2::new(10.5, -3.25)], "callejon").width(2.8));
        d.masks.push(Mask { zone: 1, ring: vec![Vec2::ZERO, Vec2::X, Vec2::Y] });
        d.plazas.push(Plaza { ring: vec![Vec2::ZERO, Vec2::X, Vec2::ONE], zone: 1, kind: PlazaKind::Fountain });
        d.nodes.push(NodeMark { p: Vec2::new(1.0, 2.0), signals: true, end: false });
        let t = d.to_toml();
        assert_eq!(Draft::from_toml(&t).unwrap(), d, "{t}");
    }
}
