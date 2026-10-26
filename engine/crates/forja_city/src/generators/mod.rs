//! Generators: from a `CityPlan` (data in `data/city/plans/*.toml`) to a
//! `Draft`. Generators only write street centrelines, plazas, masks and
//! footprints; they never build blocks, lots or geometry. One generic
//! function (`fabric::build`) turns any draft into lots.

pub mod carve;
pub mod grid;
pub mod measure;
pub mod organic;

use std::collections::BTreeMap;
use std::path::Path;

use forja_math::Vec2;
use serde::Deserialize;

use crate::plan::{Draft, Mask, PlazaKind, Region, Way};
use crate::profile::ProfileLibrary;

pub use grid::GridSpec;
pub use organic::OrganicSpec;

/// How a zone cuts its blocks into lots.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LotRules {
    /// Frontage width range (m).
    pub width: [f32; 2],
    /// Lot depth from the street (m).
    pub depth: f32,
    pub min_width: f32,
    pub min_area: f32,
    /// Smallest patio de manzana kept open (m²).
    pub court_min: f32,
    /// Land necks thinner than twice this become public space (m).
    pub neck: f32,
    /// Narrowest acute street corner after chamfering (m).
    pub min_tip: f32,
    /// Setback jitter of each facade end (m): 0 in planned grids.
    pub jitter: f32,
    /// Amplitude of wavy party walls (m).
    pub wobble: f32,
}

impl Default for LotRules {
    fn default() -> Self {
        LotRules { width: [8.0, 14.0], depth: 13.0, min_width: 3.5, min_area: 30.0, court_min: 80.0, neck: 2.5, min_tip: 2.5, jitter: 0.0, wobble: 0.0 }
    }
}

/// Where a zone lies.
#[derive(Clone, Debug, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MaskSpec {
    /// Explicit outline.
    pub ring: Vec<Vec2>,
    /// Or next to another zone: its id and side (east, west, north, south).
    pub beside: Option<String>,
    pub side: String,
    /// Extent across and along the shared side (m).
    pub size: [f32; 2],
    /// Wobble of the free sides (m).
    pub wobble: f32,
}

/// One district of the city.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub id: String,
    /// `madrid` or `valencia`: furniture and plaza dressing.
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub mask: MaskSpec,
    #[serde(default)]
    pub lots: LotRules,
    /// Style pools by key (`street`, `avenue`, `outer`, `plaza`, `alley`):
    /// `(style id, weight)`.
    #[serde(default)]
    pub styles: BTreeMap<String, Vec<(String, f32)>>,
    #[serde(default)]
    pub plaza_kind: PlazaKind,
    /// Ring road around the zone (old wall line), a profile id.
    #[serde(default)]
    pub ronda: Option<String>,
    #[serde(default)]
    pub grid: Option<GridSpec>,
    #[serde(default)]
    pub organic: Option<OrganicSpec>,
}

impl Zone {
    pub fn valencia(&self) -> bool {
        self.city == "valencia"
    }

    /// The pool for `key`, falling back to `street`.
    pub fn pool(&self, key: &str) -> &[(String, f32)] {
        self.styles.get(key).filter(|p| !p.is_empty()).or_else(|| self.styles.get("street")).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// A street that belongs to no zone (a boulevard, a ring road).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalWay {
    pub profile: String,
    pub line: Vec<Vec2>,
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub name: Option<String>,
    /// Trim the way to the zones' extent along it (rows then end on it).
    #[serde(default)]
    pub clip_to_zones: bool,
}

/// A whole city: zones in order plus global ways.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CityPlan {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "zone")]
    pub zones: Vec<Zone>,
    #[serde(default, rename = "way")]
    pub ways: Vec<GlobalWay>,
    /// A draft given as data (tests, OSM): used as is.
    #[serde(skip)]
    pub given: Option<Draft>,
}

pub const PLAN_CLASSIC: &str = include_str!("../../../../data/city/plans/classic.toml");
pub const PLAN_DEFAULT: &str = include_str!("../../../../data/city/plans/default.toml");
pub const PLAN_CARMEN: &str = include_str!("../../../../data/city/plans/carmen.toml");
pub const PLAN_AUSTRIAS: &str = include_str!("../../../../data/city/plans/austrias.toml");
pub const PLAN_ORGANIC: &str = include_str!("../../../../data/city/plans/organic.toml");

impl CityPlan {
    pub fn parse(text: &str) -> Result<Self, String> {
        let p: CityPlan = toml::from_str(text).map_err(|e| e.to_string())?;
        for z in &p.zones {
            if z.grid.is_some() && z.organic.is_some() {
                return Err(format!("zone {} has two generators", z.id));
            }
        }
        Ok(p)
    }

    /// A built-in plan by name.
    pub fn builtin(name: &str) -> Option<Self> {
        let t = match name {
            "classic" => PLAN_CLASSIC,
            "default" => PLAN_DEFAULT,
            "carmen" => PLAN_CARMEN,
            "austrias" => PLAN_AUSTRIAS,
            "organic" => PLAN_ORGANIC,
            _ => return None,
        };
        Some(Self::parse(t).expect("built-in plan parses"))
    }

    /// `data/city/plans/<name>.toml` from disk, else the built-in copy.
    pub fn load(dir: Option<&Path>, name: &str) -> Option<Self> {
        dir.and_then(|d| std::fs::read_to_string(d.join(format!("{name}.toml"))).ok())
            .and_then(|t| Self::parse(&t).map_err(|e| log::warn!("plan {name}: {e}")).ok())
            .or_else(|| Self::builtin(name))
    }

    /// Zone of a draft zone index (`index + 1`; 0 is no zone).
    pub fn zone(&self, z: u16) -> Option<&Zone> {
        (z as usize).checked_sub(1).and_then(|i| self.zones.get(i))
    }

    /// A plan around a given draft, with one zone of `city`.
    pub fn from_draft(d: Draft, city: &str) -> Self {
        let (street, alley, plaza) = if city == "madrid" {
            (vec![("madrid_centro".to_string(), 0.5), ("madrid_popular".to_string(), 0.3), ("madrid_ladrillo".to_string(), 0.2)], vec![("madrid_popular".to_string(), 0.6), ("madrid_austrias".to_string(), 0.4)], vec![("madrid_austrias".to_string(), 1.0)])
        } else {
            (vec![("valencia_carmen".to_string(), 0.5), ("valencia_ruzafa".to_string(), 0.3), ("valencia_ensanche".to_string(), 0.2)], vec![("valencia_carmen".to_string(), 0.7), ("valencia_ruzafa".to_string(), 0.3)], vec![("valencia_modernista".to_string(), 0.5), ("valencia_carmen".to_string(), 0.5)])
        };
        let mut styles = BTreeMap::new();
        styles.insert("street".to_string(), street);
        styles.insert("alley".to_string(), alley);
        styles.insert("plaza".to_string(), plaza);
        let zone = Zone {
            id: "given".into(),
            city: city.into(),
            mask: MaskSpec::default(),
            lots: LotRules { width: [4.5, 9.0], depth: 12.0, min_width: 3.0, min_area: 25.0, court_min: 60.0, neck: 1.6, min_tip: 2.5, jitter: 0.15, wobble: 0.3 },
            styles,
            plaza_kind: if city == "madrid" { PlazaKind::Parterre } else { PlazaKind::Fountain },
            ronda: None,
            grid: None,
            organic: None,
        };
        CityPlan { name: "given".into(), zones: vec![zone], ways: Vec::new(), given: Some(d) }
    }
}

/// Mixes a seed with a stage and a key (SplitMix64).
pub fn keyed(seed: u64, stage: u64, key: u64) -> crate::geom::Rng {
    crate::geom::Rng::keyed(seed, stage, key)
}

/// Runs every zone's generator in plan order, then the global ways.
pub fn draft(plan: &CityPlan, lib: &ProfileLibrary, seed: u64) -> Draft {
    draft_report(plan, lib, seed).0
}

/// How an organic zone's carving ended.
#[derive(Clone, Debug, PartialEq)]
pub struct Carved {
    pub zone: u16,
    /// Accessibility depth of the last tier: no land is deeper than this.
    pub d_max: f32,
    /// Carving iterations over all tiers.
    pub iterations: usize,
    /// A tier stopped at its safety cap (the termination bound failed).
    pub capped: bool,
}

/// `draft`, plus how each organic zone's carving ended.
pub fn draft_report(plan: &CityPlan, lib: &ProfileLibrary, seed: u64) -> (Draft, Vec<Carved>) {
    let mut report = Vec::new();
    if let Some(d) = &plan.given {
        return (d.clone(), report);
    }
    let mut d = Draft { snap: 0.05, ..Default::default() };
    // Zone masks first (organic masks are placed beside earlier zones).
    let mut masks: Vec<Vec<Vec2>> = Vec::new();
    for (i, z) in plan.zones.iter().enumerate() {
        let zi = (i + 1) as u16;
        let ring = if let Some(g) = &z.grid {
            let (dd, ring) = grid::generate(g, z, zi, lib, seed);
            d.extend(dd);
            ring
        } else {
            mask_ring(&z.mask, plan, &masks, seed ^ zi as u64)
        };
        masks.push(ring.clone());
        d.masks.push(Mask { zone: zi, ring });
    }
    // Ronda loops along organic zone boundaries.
    for (i, z) in plan.zones.iter().enumerate() {
        if let (Some(r), true) = (&z.ronda, z.grid.is_none()) {
            let mut line = masks[i].clone();
            line.push(line[0]);
            d.ways.push(Way::new(line, r).zone(0));
        }
    }
    // Global ways, trimmed to the zones' extent.
    let (lo, hi) = masks.iter().flatten().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    for gw in &plan.ways {
        let mut line = gw.line.clone();
        if gw.clip_to_zones && masks.iter().any(|m| !m.is_empty()) {
            line = line.iter().map(|p| p.clamp(lo, hi)).collect();
            line.dedup();
        }
        let mut w = Way::new(line, &gw.profile);
        if let Some(width) = gw.width {
            w = w.width(width);
        }
        w.name = gw.name.clone();
        d.ways.push(w);
    }
    // Organic zones carve last: they need every street around them. Each
    // carves only its own land (earlier zones win where masks overlap).
    for (i, z) in plan.zones.iter().enumerate() {
        if let Some(o) = &z.organic {
            let zi = (i + 1) as u16;
            let earlier = Region::rings(masks[..i].iter().map(|m| m.as_slice()));
            let own = Region::ring(&masks[i]).minus(&earlier);
            let (dd, carved) = organic::generate(o, z, zi, &own, &d, lib, seed);
            d.extend(dd);
            report.push(carved);
        }
    }
    (d, report)
}

/// The outline of a zone without a grid.
fn mask_ring(m: &MaskSpec, plan: &CityPlan, done: &[Vec<Vec2>], seed: u64) -> Vec<Vec2> {
    if m.ring.len() >= 3 {
        return m.ring.clone();
    }
    let Some(other) = m.beside.as_ref().and_then(|b| plan.zones.iter().position(|z| &z.id == b)).and_then(|i| done.get(i)) else {
        let [w, h] = m.size;
        return wobbly_rect(Vec2::new(-w * 0.5, -h * 0.5), Vec2::new(w * 0.5, h * 0.5), m.wobble, None, seed);
    };
    let (lo, hi) = other.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let c = (lo + hi) * 0.5;
    let [across, along] = m.size;
    let (min, max, shared) = match m.side.as_str() {
        "west" => (Vec2::new(lo.x - across, c.y - along * 0.5), Vec2::new(lo.x, c.y + along * 0.5), 1),
        "north" => (Vec2::new(c.x - along * 0.5, lo.y - across), Vec2::new(c.x + along * 0.5, lo.y), 2),
        "south" => (Vec2::new(c.x - along * 0.5, hi.y), Vec2::new(c.x + along * 0.5, hi.y + across), 0),
        _ => (Vec2::new(hi.x, c.y - along * 0.5), Vec2::new(hi.x + across, c.y + along * 0.5), 3),
    };
    wobbly_rect(min, max, m.wobble, Some(shared), seed)
}

/// A rectangle (CCW) whose sides bulge and dent smoothly; side `straight`
/// (0 bottom, 1 right, 2 top, 3 left) stays straight to meet a neighbour.
pub fn wobbly_rect(min: Vec2, max: Vec2, wobble: f32, straight: Option<usize>, seed: u64) -> Vec<Vec2> {
    let v = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
    let mut ring = Vec::new();
    for k in 0..4 {
        let (a, b) = (v[k], v[(k + 1) % 4]);
        let len = a.distance(b);
        let n = (len / 12.0).ceil().max(1.0) as usize;
        let out = crate::plan::line::left(b - a).normalize_or(Vec2::Y) * -1.0;
        for i in 0..n {
            let t = i as f32 / n as f32;
            let p = a.lerp(b, t);
            let w = if Some(k) == straight || wobble <= 0.0 {
                0.0
            } else {
                // Zero at the corners, smooth in between.
                let env = (t * std::f32::consts::PI).sin();
                wobble * env * crate::plan::noise1(seed ^ (k as u64) << 8, t * len / 60.0)
            };
            ring.push(p + out * w);
        }
    }
    ring
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_plans_parse() {
        for n in ["classic", "default", "carmen", "austrias", "organic"] {
            let p = CityPlan::builtin(n).unwrap();
            assert!(!p.zones.is_empty(), "{n}");
        }
    }

    #[test]
    fn wobbly_rect_keeps_the_shared_side_straight() {
        let r = wobbly_rect(Vec2::ZERO, Vec2::new(100.0, 80.0), 10.0, Some(3), 4);
        assert!(crate::geom::signed_area(&r) > 0.0);
        let left: Vec<&Vec2> = r.iter().filter(|p| p.x.abs() < 1e-4).collect();
        assert!(left.len() >= 4);
    }
}
