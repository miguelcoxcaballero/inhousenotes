//! The contract between the city layout and the building generator.
//!
//! The layout (`fabric`) cuts any street layout into lots and the building
//! generator (`building`) builds on them. A `Lot` is a simple
//! counter-clockwise ring in (x, z) with one `EdgeKind` per edge and, for
//! every street-facing edge, the `Front` that edge faces: what the public
//! space in front of the facade allows (projections, awnings, shops,
//! height, lighting).
//!
//! Both sides are developed and tested against this file alone, so it stays
//! small and changes only by agreement. `Lot::check` is the contract in
//! code: the layout only hands out lots that pass it, and the building
//! generator builds every lot that passes it.

use forja_math::Vec2;
use serde::Deserialize;

/// Shortest edge of a lot ring (m).
pub const MIN_EDGE: f32 = 0.05;
/// Smallest interior angle at a vertex of a street-facing edge (degrees).
pub const MIN_STREET_ANGLE: f32 = 25.0;
/// Smallest interior angle between two street-facing edges (degrees). The
/// layout chamfers sharper street corners.
pub const MIN_CORNER_ANGLE: f32 = 70.0;
/// Longest chord of a curved frontage (m): curves arrive as chains of
/// chords at most this long, with every vertex on the curve.
pub const MAX_CHORD: f32 = 3.0;
/// Walls start this far below `Lot::base` (a hidden foundation skirt), so
/// the ground in front of a facade may be up to this much lower than `base`.
pub const SKIRT: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// Facade on a street, alley or plaza.
    Street,
    /// Street facade on a chamfered or rounded block corner.
    Corner,
    /// Facade on an enclosed court (patio de manzana).
    Courtyard,
    /// Wall shared with the neighbour (medianera), the short return of a
    /// neighbour's setback, or a cut at the map edge.
    Party,
}

impl EdgeKind {
    pub fn is_street(self) -> bool {
        matches!(self, EdgeKind::Street | EdgeKind::Corner)
    }
}

/// Street lighting carried by facades: a lantern at every arc length
/// `phase + k * pitch` along the way, on the pier nearest to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rhythm {
    pub pitch: f32,
    pub phase: f32,
}

/// What the public space in front of one street-facing edge allows its
/// facade. Filled by the layout from the street profile and the measured
/// clear width; read by the building generator as limits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Front {
    /// Clear width to the facade opposite, measured (m). On a plaza, twice
    /// the plaza's inscribed radius.
    pub clear: f32,
    /// Walkable width at the facade foot before any carriageway (m): the
    /// raised sidewalk, or half the clear width on shared surfaces.
    pub walk: f32,
    /// Farthest anything above the ground floor may project from the wall
    /// plane: balconies, miradores, cornices, eaves (m).
    pub reach: f32,
    /// Glazed miradores allowed.
    pub miradores: bool,
    /// Shop and balcony awnings allowed.
    pub awnings: bool,
    /// Multiplier on the style's shop chance (0 = homes only).
    pub shops: f32,
    /// Widest street door (portal) that fits (m).
    pub portal: f32,
    /// Highest facade (top of the last floor above `Lot::base`) the street
    /// supports (m).
    pub max_height: f32,
    /// Depth of the lot behind this facade, to its far side (m).
    pub depth: f32,
    /// Faces a plaza or other open space rather than a street.
    pub open: bool,
    /// Arc length along the way at `ring[i]` and at `ring[i + 1]` (m). The
    /// second is smaller when the edge runs against the way.
    pub along: [f32; 2],
    /// Wall-mounted street lighting, if the street has it.
    pub lamps: Option<Rhythm>,
    /// Stable id of the way the edge faces, `Front::NO_WAY` if none.
    pub way: u32,
}

impl Front {
    pub const NO_WAY: u32 = u32::MAX;

    /// A wide street that constrains nothing: today's buildings.
    pub const UNCONSTRAINED: Front = Front {
        clear: 20.0,
        walk: 4.5,
        reach: f32::INFINITY,
        miradores: true,
        awnings: true,
        shops: 1.0,
        portal: 1.9,
        max_height: f32::INFINITY,
        depth: f32::INFINITY,
        open: false,
        along: [0.0, 0.0],
        lamps: None,
        way: Front::NO_WAY,
    };

    /// Arc length along the way at `t` in [0, 1] from `ring[i]` to `ring[i + 1]`.
    pub fn along_at(&self, t: f32) -> f32 {
        self.along[0] + (self.along[1] - self.along[0]) * t
    }
}

impl Default for Front {
    fn default() -> Self {
        Front::UNCONSTRAINED
    }
}

/// How a street's width turns into facade limits (`[front]` table of a
/// street profile). Modelled on municipal ordinances that tie projections
/// (vuelos) and heights to the width of the street.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FrontRules {
    /// Projection allowed per metre of clear width.
    pub reach_frac: f32,
    pub reach_max: f32,
    /// Smallest reach that takes a mirador.
    pub mirador_reach: f32,
    /// Awnings need this much walk and this much clear width.
    pub awning_walk: f32,
    pub awning_clear: f32,
    pub shops: f32,
    /// Below this clear width the street door is the narrow one.
    pub narrow: f32,
    /// Portal width `[narrow street, wide street]`.
    pub portal: [f32; 2],
    /// Facade height allowed per metre of clear width.
    pub height_ratio: f32,
}

impl Default for FrontRules {
    fn default() -> Self {
        FrontRules {
            reach_frac: 0.12,
            reach_max: 1.2,
            mirador_reach: 0.8,
            awning_walk: 1.8,
            awning_clear: 6.0,
            shops: 1.0,
            narrow: 5.0,
            portal: [1.4, 1.9],
            height_ratio: 4.5,
        }
    }
}

impl FrontRules {
    /// The limits of a facade facing `clear` metres of street with `walk`
    /// metres to walk on. `depth`, `open`, `along`, `lamps` and `way` keep
    /// their `UNCONSTRAINED` values for the caller to fill in.
    pub fn front(&self, clear: f32, walk: f32) -> Front {
        let reach = (self.reach_frac * clear).clamp(0.0, self.reach_max);
        Front {
            clear,
            walk,
            reach,
            miradores: reach >= self.mirador_reach,
            awnings: walk >= self.awning_walk && clear >= self.awning_clear,
            shops: self.shops,
            portal: if clear < self.narrow { self.portal[0] } else { self.portal[1] },
            max_height: self.height_ratio * clear,
            ..Front::UNCONSTRAINED
        }
    }
}

/// A building plot: counter-clockwise ring in (x, z), one kind per edge
/// (`edges[i]` runs from `ring[i]` to `ring[i + 1]`) and, when known, one
/// `Front` per edge (only street-facing entries are read).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lot {
    pub ring: Vec<Vec2>,
    pub edges: Vec<EdgeKind>,
    /// Ground level at the facades (the highest ground in front of them).
    pub base: f32,
    /// Empty (every edge `Front::UNCONSTRAINED`) or one per edge.
    pub fronts: Vec<Front>,
    /// Storeys including the ground floor, when known (OSM building:levels).
    pub levels: Option<u32>,
    /// Storeys in the roof (OSM roof:levels): mansard or dormers when > 0.
    pub roof_levels: Option<u32>,
}

impl Lot {
    pub fn new(ring: Vec<Vec2>, edges: Vec<EdgeKind>, base: f32) -> Self {
        Lot { ring, edges, base, ..Default::default() }
    }

    pub fn front(&self, i: usize) -> Front {
        self.fronts.get(i).copied().unwrap_or_default()
    }

    /// Endpoints of edge `i`.
    pub fn edge(&self, i: usize) -> (Vec2, Vec2) {
        (self.ring[i], self.ring[(i + 1) % self.ring.len()])
    }

    /// Signed area (positive for counter-clockwise rings).
    pub fn area(&self) -> f32 {
        let n = self.ring.len();
        let mut a = 0.0f64;
        for i in 0..n {
            let (p, q) = self.edge(i);
            a += p.x as f64 * q.y as f64 - q.x as f64 * p.y as f64;
        }
        (a * 0.5) as f32
    }

    /// Interior angle at vertex `i`, between edges `i - 1` and `i` (radians,
    /// in (0, 2π); above π at reflex vertices).
    pub fn interior_angle(&self, i: usize) -> f32 {
        let n = self.ring.len();
        let p = self.ring[(i + n - 1) % n];
        let (q, r) = self.edge(i);
        let (din, dout) = (q - p, r - q);
        std::f32::consts::PI - din.perp_dot(dout).atan2(din.dot(dout))
    }

    /// The contract: what the layout guarantees and the building generator
    /// relies on.
    pub fn check(&self) -> Result<(), String> {
        let n = self.ring.len();
        if n < 3 {
            return Err(format!("{n} vertices"));
        }
        if self.edges.len() != n {
            return Err(format!("{} edge kinds for {n} edges", self.edges.len()));
        }
        if !self.fronts.is_empty() && self.fronts.len() != n {
            return Err(format!("{} fronts for {n} edges", self.fronts.len()));
        }
        if !self.base.is_finite() || self.ring.iter().any(|p| !p.is_finite()) {
            return Err("non-finite coordinates".into());
        }
        for (i, f) in self.fronts.iter().enumerate() {
            let v = [f.clear, f.walk, f.reach, f.shops, f.portal, f.max_height, f.depth, f.along[0], f.along[1]];
            if v.iter().any(|x| x.is_nan() || *x < 0.0) || !(f.clear.is_finite() && f.walk.is_finite() && f.along[0].is_finite() && f.along[1].is_finite()) {
                return Err(format!("front {i} is not finite and non-negative: {f:?}"));
            }
            if f.lamps.is_some_and(|r| !(r.pitch > 0.0 && r.pitch.is_finite() && r.phase.is_finite())) {
                return Err(format!("front {i} has a bad lamp rhythm"));
            }
        }
        if self.area() <= 0.0 {
            return Err("ring is not counter-clockwise".into());
        }
        for i in 0..n {
            let (a, b) = self.edge(i);
            if a.distance(b) < MIN_EDGE {
                return Err(format!("edge {i} is {:.3} m long", a.distance(b)));
            }
        }
        for i in 0..n {
            for j in i + 1..n {
                if j == i + 1 || (i == 0 && j == n - 1) {
                    continue;
                }
                let (a, b) = self.edge(i);
                let (c, d) = self.edge(j);
                if segments_touch(a, b, c, d) {
                    return Err(format!("edges {i} and {j} touch: the ring is not simple"));
                }
            }
        }
        for i in 0..n {
            let ang = self.interior_angle(i).to_degrees();
            let (e0, e1) = (self.edges[(i + n - 1) % n], self.edges[i]);
            let min = if e0.is_street() && e1.is_street() {
                MIN_CORNER_ANGLE
            } else if e0.is_street() || e1.is_street() {
                MIN_STREET_ANGLE
            } else {
                0.5
            };
            if ang < min || ang > 359.5 {
                return Err(format!("interior angle {ang:.1}° at vertex {i} (minimum {min}°)"));
            }
        }
        Ok(())
    }
}

/// Whether two closed segments share any point (f64 orientation tests).
fn segments_touch(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let o = |p: Vec2, q: Vec2, r: Vec2| {
        let v = (q.x as f64 - p.x as f64) * (r.y as f64 - p.y as f64) - (q.y as f64 - p.y as f64) * (r.x as f64 - p.x as f64);
        if v.abs() < 1e-9 { 0.0 } else { v.signum() }
    };
    let on = |p: Vec2, q: Vec2, r: Vec2| r.x >= p.x.min(q.x) && r.x <= p.x.max(q.x) && r.y >= p.y.min(q.y) && r.y <= p.y.max(q.y);
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return true;
    }
    (d1 == 0.0 && on(c, d, a)) || (d2 == 0.0 && on(c, d, b)) || (d3 == 0.0 && on(a, b, c)) || (d4 == 0.0 && on(a, b, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Lot {
        // A 6 m front on a 3 m alley along z = 0, 12 m deep, CCW in (x, z).
        let ring = vec![Vec2::new(0.0, 0.0), Vec2::new(6.0, 0.0), Vec2::new(6.0, 12.0), Vec2::new(0.0, 12.0)];
        let mut lot = Lot::new(ring, vec![EdgeKind::Street, EdgeKind::Party, EdgeKind::Courtyard, EdgeKind::Party], 0.15);
        lot.fronts = vec![FrontRules::default().front(3.0, 1.5), Front::UNCONSTRAINED, Front::UNCONSTRAINED, Front::UNCONSTRAINED];
        lot
    }

    #[test]
    fn plain_lot_passes_and_defaults_are_unconstrained() {
        let lot = plain();
        lot.check().unwrap();
        assert!(lot.area() > 0.0);
        assert!((lot.interior_angle(1).to_degrees() - 90.0).abs() < 1e-3);
        assert_eq!(Lot::new(lot.ring.clone(), lot.edges.clone(), 0.0).front(0), Front::UNCONSTRAINED);
        let mut cw = lot.clone();
        cw.ring.reverse();
        assert!(cw.check().is_err());
    }

    #[test]
    fn alley_front_is_tight_and_wide_street_is_loose() {
        let r = FrontRules::default();
        let alley = r.front(3.0, 1.5);
        assert!((alley.reach - 0.36).abs() < 1e-4 && !alley.miradores && !alley.awnings);
        assert_eq!(alley.portal, 1.4);
        assert!((alley.max_height - 13.5).abs() < 1e-4);
        let ensanche = r.front(20.0, 4.5);
        assert!(ensanche.reach >= 1.2 && ensanche.miradores && ensanche.awnings);
        let rules: FrontRules = toml::from_str("reach_frac = 0.1\nheight_ratio = 3.0").unwrap();
        assert_eq!(rules.reach_max, 1.2);
    }

    #[test]
    fn check_rejects_spikes_bowties_and_acute_street_corners() {
        let sq = |pts: &[(f32, f32)], e: &[EdgeKind]| Lot::new(pts.iter().map(|p| Vec2::new(p.0, p.1)).collect(), e.to_vec(), 0.0);
        use EdgeKind::*;
        let bowtie = sq(&[(0.0, 0.0), (4.0, 4.0), (4.0, 0.0), (0.0, 4.0)], &[Party; 4]);
        assert!(bowtie.check().is_err());
        // A 40° street corner must have been chamfered.
        let t = 40f32.to_radians();
        let wedge = sq(&[(0.0, 0.0), (10.0, 0.0), (10.0 * t.cos(), 10.0 * t.sin())], &[Street, Party, Street]);
        assert!(wedge.check().unwrap_err().contains("angle"));
        let ok = sq(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], &[Street, Corner, Street, Party]);
        ok.check().unwrap();
    }
}
