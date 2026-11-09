//! The packed island map (`data/maps/isla/isla.bin.gz`, written by
//! `tools/isla/isla_pack.py`): every polygon, line and point of the Isla
//! Levante in world metres (x east, z south, origin at the Plaça de
//! l'Ajuntament), and its height map.
//!
//! Input is data: it is parsed, bounds-checked and never trusted (a short
//! or corrupt file is an error, not a panic).

use std::io::Read;
use std::path::Path;

use forja_math::Vec2;

use crate::WorldError;

/// Polygon kinds (the packer's numbering).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PolyKind {
    Coast = 0,
    Building = 1,
    Water = 2,
    River = 3,
    Lake = 4,
    Reservoir = 5,
    Green = 6,
    Park = 7,
    Agriculture = 8,
    Forest = 9,
    Beach = 10,
    Wetland = 11,
    Rock = 12,
    Industrial = 13,
    Sports = 14,
    Urban = 15,
    Airfield = 16,
    FloodBed = 17,
    FloodBank = 18,
    Apron = 19,
}

impl PolyKind {
    fn from_u8(v: u64) -> Option<Self> {
        use PolyKind::*;
        const ALL: [PolyKind; 20] = [Coast, Building, Water, River, Lake, Reservoir, Green, Park, Agriculture, Forest, Beach, Wetland, Rock, Industrial, Sports, Urban, Airfield, FloodBed, FloodBank, Apron];
        ALL.get(v as usize).copied()
    }

    pub fn is_water(self) -> bool {
        matches!(self, PolyKind::Water | PolyKind::River | PolyKind::Lake | PolyKind::Reservoir)
    }
}

/// Road classes (the packer's numbering, OSM `highway` values plus
/// railways and the airport's runway and taxiways).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum RoadClass {
    Motorway = 0,
    MotorwayLink,
    Trunk,
    TrunkLink,
    Primary,
    PrimaryLink,
    Secondary,
    SecondaryLink,
    Tertiary,
    TertiaryLink,
    Residential,
    Unclassified,
    LivingStreet,
    Service,
    Pedestrian,
    Footway,
    Path,
    Steps,
    Cycleway,
    Track,
    Busway,
    Rail,
    Tram,
    Subway,
    Runway,
    Taxiway,
    Promenade,
    Raceway,
}

impl RoadClass {
    fn from_u8(v: u64) -> Option<Self> {
        use RoadClass::*;
        const ALL: [RoadClass; 28] = [
            Motorway, MotorwayLink, Trunk, TrunkLink, Primary, PrimaryLink, Secondary, SecondaryLink, Tertiary, TertiaryLink, Residential, Unclassified, LivingStreet, Service, Pedestrian, Footway, Path, Steps, Cycleway, Track, Busway, Rail,
            Tram, Subway, Runway, Taxiway, Promenade, Raceway,
        ];
        ALL.get(v as usize).copied()
    }

    /// Cars drive on it.
    pub fn drivable(self) -> bool {
        (self as u8) <= RoadClass::Service as u8 || matches!(self, RoadClass::Busway | RoadClass::Raceway)
    }

    pub fn is_rail(self) -> bool {
        matches!(self, RoadClass::Rail | RoadClass::Tram | RoadClass::Subway)
    }

    /// Paved width (m) when the data gives none.
    pub fn width(self) -> f32 {
        use RoadClass::*;
        match self {
            Motorway => 11.0,
            Trunk => 10.0,
            MotorwayLink | TrunkLink => 6.5,
            Primary => 12.0,
            Secondary => 10.0,
            PrimaryLink | SecondaryLink | TertiaryLink => 6.0,
            Tertiary => 8.0,
            Residential | Unclassified | Busway | Raceway => 6.5,
            LivingStreet => 5.0,
            Service => 4.0,
            Pedestrian => 5.0,
            Promenade => 8.0,
            Footway | Path | Steps => 2.2,
            Cycleway => 2.2,
            Track => 3.0,
            Rail | Subway => 3.2,
            Tram => 2.8,
            Runway => 45.0,
            Taxiway => 18.0,
        }
    }
}

pub const LINE_BRIDGE: u8 = 1;
pub const LINE_TUNNEL: u8 = 2;
pub const LINE_ONEWAY: u8 = 4;
pub const LINE_PASSAGE: u8 = 8;

#[derive(Clone, Debug)]
pub struct Poly {
    pub kind: PolyKind,
    /// Agriculture: crop (1 orchard, 2 arable, 3 fallow). Buildings: bit 0
    /// at the airport, bit 1 height measured (IGN LiDAR), bit 2 height taken
    /// from measured neighbours.
    pub flags: u8,
    /// Buildings: height above the ground (m).
    pub height: f32,
    pub name: u32,
    /// OSM id: `way * 2`, `relation * 2 + 1` (0: none).
    pub osm: u64,
    /// Outer ring first, then holes; `(x, z)`.
    pub rings: Vec<Vec<Vec2>>,
}

impl Poly {
    pub fn bounds(&self) -> (Vec2, Vec2) {
        let mut lo = Vec2::splat(f32::MAX);
        let mut hi = Vec2::splat(f32::MIN);
        for p in &self.rings[0] {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        (lo, hi)
    }

    /// Even-odd test over every ring.
    pub fn contains(&self, p: Vec2) -> bool {
        let mut inside = false;
        for r in &self.rings {
            if point_in_ring(r, p) {
                inside = !inside;
            }
        }
        inside
    }
}

pub fn point_in_ring(ring: &[Vec2], p: Vec2) -> bool {
    let mut inside = false;
    let n = ring.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn ring_area(ring: &[Vec2]) -> f32 {
    let n = ring.len();
    (0..n).map(|i| ring[i].perp_dot(ring[(i + 1) % n])).sum::<f32>() * 0.5
}

pub fn ring_centroid(ring: &[Vec2]) -> Vec2 {
    let n = ring.len();
    let (mut a, mut c) = (0.0f32, Vec2::ZERO);
    let o = ring[0];
    for i in 0..n {
        let (p, q) = (ring[i] - o, ring[(i + 1) % n] - o);
        let w = p.perp_dot(q);
        a += w;
        c += (p + q) * w;
    }
    if a.abs() < 1e-6 { ring.iter().copied().sum::<Vec2>() / n as f32 } else { o + c / (3.0 * a) }
}

#[derive(Clone, Debug)]
pub struct Line {
    pub class: RoadClass,
    pub flags: u8,
    pub layer: i8,
    /// Paved width (m).
    pub width: f32,
    pub name: u32,
    pub pts: Vec<Vec2>,
}

impl Line {
    pub fn bridge(&self) -> bool {
        self.flags & LINE_BRIDGE != 0
    }
    pub fn tunnel(&self) -> bool {
        self.flags & LINE_TUNNEL != 0
    }
    pub fn oneway(&self) -> bool {
        self.flags & LINE_ONEWAY != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    Poi,
    Landmark,
    Station,
}

#[derive(Clone, Debug)]
pub struct Point {
    pub kind: PointKind,
    pub name: u32,
    pub category: u32,
    pub osm: u64,
    pub pos: Vec2,
}

/// The source height map: decimetres, row 0 at the north edge.
#[derive(Clone, Debug, Default)]
pub struct HeightMap {
    /// World `(west, north (min z), east, south)`.
    pub extent: [f32; 4],
    pub size: usize,
    pub dm: Vec<u16>,
}

/// Real terrain where the island is real geography (Valencia): IGN MDT25,
/// centimetres above sea level, row 0 at the north edge
/// (`dem_valencia.png`, written by `tools/isla/fetch_dem.py`).
#[derive(Clone, Debug, Default)]
pub struct RealDem {
    /// World `(west, north (min z), east, south)` of the sample centres.
    pub extent: [f32; 4],
    pub width: usize,
    pub height: usize,
    pub cm: Vec<u16>,
}

impl RealDem {
    /// Height above sea level (m) at a world position and how much to
    /// trust it there (1 inside, fading to 0 over the last 1.5 km to the
    /// edge of the data).
    pub fn sample(&self, x: f32, z: f32) -> (f32, f32) {
        let (w, h) = (self.width, self.height);
        if w < 2 || h < 2 {
            return (0.0, 0.0);
        }
        let edge = (x - self.extent[0]).min(self.extent[2] - x).min(z - self.extent[1]).min(self.extent[3] - z);
        if edge <= 0.0 {
            return (0.0, 0.0);
        }
        let fx = ((x - self.extent[0]) / (self.extent[2] - self.extent[0]) * (w - 1) as f32).clamp(0.0, (w - 1) as f32);
        let fz = ((z - self.extent[1]) / (self.extent[3] - self.extent[1]) * (h - 1) as f32).clamp(0.0, (h - 1) as f32);
        let (c, r) = ((fx as usize).min(w - 2), (fz as usize).min(h - 2));
        let (u, v) = (fx - c as f32, fz - r as f32);
        let s = |r: usize, c: usize| self.cm[r * w + c] as f32 * 0.01;
        let a = s(r, c) + (s(r, c + 1) - s(r, c)) * u;
        let b = s(r + 1, c) + (s(r + 1, c + 1) - s(r + 1, c)) * u;
        let t = (edge / 1500.0).clamp(0.0, 1.0);
        (a + (b - a) * v, t * t * (3.0 - 2.0 * t))
    }
}

#[derive(Clone, Debug, Default)]
pub struct IslandData {
    pub strings: Vec<String>,
    pub polys: Vec<Poly>,
    pub lines: Vec<Line>,
    pub points: Vec<Point>,
    pub height: HeightMap,
    /// Real terrain of the Valencia area (`None` without the file: the
    /// designed plain is used).
    pub dem: Option<RealDem>,
}

struct Cur<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cur<'_> {
    fn u(&mut self) -> Option<u64> {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let byte = *self.b.get(self.at)?;
            self.at += 1;
            v |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }

    fn i(&mut self) -> Option<i64> {
        let v = self.u()?;
        Some(((v >> 1) as i64) ^ -((v & 1) as i64))
    }

    fn pts(&mut self) -> Option<Vec<Vec2>> {
        let n = self.u()? as usize;
        if n > self.b.len() {
            return None;
        }
        let (mut x, mut y) = (0i64, 0i64);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            x += self.i()?;
            y += self.i()?;
            out.push(Vec2::new(x as f32 * 0.01, y as f32 * 0.01));
        }
        Some(out)
    }
}

impl IslandData {
    pub fn name(&self, i: u32) -> &str {
        self.strings.get(i as usize).map_or("", String::as_str)
    }

    /// Loads `isla.bin.gz` and `height.png` from `dir`.
    pub fn load(dir: &Path) -> Result<Self, WorldError> {
        let path = dir.join("isla.bin.gz");
        let gz = std::fs::read(&path).map_err(|e| WorldError::Io(format!("{}: {e}", path.display())))?;
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(&gz[..]).take(256 << 20).read_to_end(&mut raw).map_err(|e| WorldError::Parse(format!("{}: {e}", path.display())))?;
        let mut d = Self::parse(&raw).ok_or_else(|| WorldError::Parse(format!("{}: not an island map", path.display())))?;
        let hp = dir.join("height.png");
        let img = image::open(&hp).map_err(|e| WorldError::Io(format!("{}: {e}", hp.display())))?.into_luma16();
        if img.width() as usize != d.height.size || img.height() as usize != d.height.size {
            return Err(WorldError::Parse(format!("{}: expected {} px", hp.display(), d.height.size)));
        }
        d.height.dm = img.into_raw();
        d.dem = Self::load_dem(dir);
        Ok(d)
    }

    fn load_dem(dir: &Path) -> Option<RealDem> {
        let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("dem_valencia.json")).ok()?).ok()?;
        let img = image::open(dir.join("dem_valencia.png")).ok()?.into_luma16();
        let e = meta.get("extent")?.as_array()?;
        let mut extent = [0.0f32; 4];
        for (k, v) in extent.iter_mut().enumerate() {
            *v = e.get(k)?.as_f64()? as f32;
        }
        if !(extent[2] > extent[0] && extent[3] > extent[1]) || extent.iter().any(|v| !v.is_finite()) {
            return None;
        }
        Some(RealDem { extent, width: img.width() as usize, height: img.height() as usize, cm: img.into_raw() })
    }

    pub fn parse(raw: &[u8]) -> Option<Self> {
        if raw.get(..4)? != b"ISLA" {
            return None;
        }
        let mut c = Cur { b: raw, at: 4 };
        if c.u()? != 1 {
            return None;
        }
        let mut d = IslandData::default();
        for k in 0..4 {
            d.height.extent[k] = c.i()? as f32 * 0.01;
        }
        d.height.size = c.u()? as usize;
        if !(2..=8192).contains(&d.height.size) {
            return None;
        }
        let n = c.u()? as usize;
        for _ in 0..n {
            let len = c.u()? as usize;
            let s = raw.get(c.at..c.at.checked_add(len)?)?;
            c.at += len;
            d.strings.push(String::from_utf8_lossy(s).into_owned());
        }
        let n = c.u()? as usize;
        for _ in 0..n {
            let kind = PolyKind::from_u8(c.u()?)?;
            let flags = c.u()? as u8;
            let height = c.u()? as f32 * 0.1;
            let name = c.u()? as u32;
            let osm = c.u()?;
            let nr = c.u()? as usize;
            let mut rings = Vec::with_capacity(nr.min(64));
            for _ in 0..nr {
                rings.push(c.pts()?);
            }
            if rings.first().is_some_and(|r| r.len() >= 3) {
                d.polys.push(Poly { kind, flags, height, name, osm, rings });
            }
        }
        let n = c.u()? as usize;
        for _ in 0..n {
            let class = RoadClass::from_u8(c.u()?)?;
            let flags = c.u()? as u8;
            let layer = c.i()?.clamp(-8, 8) as i8;
            let w = c.u()? as f32 * 0.1;
            let name = c.u()? as u32;
            let pts = c.pts()?;
            if pts.len() >= 2 {
                d.lines.push(Line { class, flags, layer, width: if w > 0.5 { w } else { class.width() }, name, pts });
            }
        }
        let n = c.u()? as usize;
        for _ in 0..n {
            let kind = match c.u()? {
                1 => PointKind::Landmark,
                2 => PointKind::Station,
                _ => PointKind::Poi,
            };
            let name = c.u()? as u32;
            let category = c.u()? as u32;
            let osm = c.u()?;
            let (x, y) = (c.i()?, c.i()?);
            d.points.push(Point { kind, name, category, osm, pos: Vec2::new(x as f32 * 0.01, y as f32 * 0.01) });
        }
        Some(d)
    }
}
