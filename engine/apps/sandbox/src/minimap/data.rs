//! What the radar and the pause map draw: land, water, green areas,
//! building blocks, roads by class and points of interest, in game world
//! metres on the ground plane (`x` east, `y` = world Z, south).
//!
//! [`MapData`] is the one thing a world has to provide (see
//! [`MapSource`]). Two sources exist today: the packed vector file of the
//! whole Isla Levante (`data/maps/isla/minimap.bin`, written by
//! `tools/isla/isla_minimap.py`), and any [`WorldDescription`] (the
//! streets and plazas of its traffic description).

use std::io::Read;
use std::path::Path;

use forja_math::Vec2;
use forja_world::WorldDescription;

/// Area kinds, in drawing order of [`AREA_ORDER`].
pub mod area {
    pub const LAND: u8 = 0;
    pub const WATER: u8 = 1;
    pub const GREEN: u8 = 2;
    pub const FOREST: u8 = 3;
    pub const FARM: u8 = 4;
    pub const BEACH: u8 = 5;
    pub const URBAN: u8 = 6;
    pub const INDUSTRIAL: u8 = 7;
    pub const AIRFIELD: u8 = 8;
    pub const WETLAND: u8 = 9;
    pub const ROCK: u8 = 10;
    pub const BUILDING: u8 = 11;
    pub const LANDMARK: u8 = 12;
    pub const RIVERBED: u8 = 13;
    pub const APRON: u8 = 14;
    pub const PLAZA: u8 = 15;
    /// Built-up ground of a world that only knows its streets.
    pub const BLOCK: u8 = 16;
    pub const COUNT: usize = 17;
}

/// Bottom to top.
pub const AREA_ORDER: [u8; area::COUNT] = [area::LAND, area::BLOCK, area::URBAN, area::INDUSTRIAL, area::AIRFIELD, area::FARM, area::ROCK, area::FOREST, area::GREEN, area::BEACH, area::RIVERBED, area::WETLAND, area::WATER, area::APRON, area::PLAZA, area::BUILDING, area::LANDMARK];

/// Road classes, most important first.
pub mod road {
    pub const MOTORWAY: u8 = 0;
    pub const TRUNK: u8 = 1;
    pub const PRIMARY: u8 = 2;
    pub const SECONDARY: u8 = 3;
    pub const TERTIARY: u8 = 4;
    pub const RESIDENTIAL: u8 = 5;
    pub const SERVICE: u8 = 6;
    pub const PEDESTRIAN: u8 = 7;
    pub const RAIL: u8 = 8;
    pub const RUNWAY: u8 = 9;
    /// A street's ground between its building lines (not a roadway).
    pub const GROUND: u8 = 10;
    pub const COUNT: usize = 11;

    /// Drawn width in metres.
    pub fn width(class: u8) -> f32 {
        [22.0, 18.0, 15.0, 12.0, 10.0, 8.0, 5.0, 5.0, 2.5, 45.0, 12.0][class.min(10) as usize]
    }

    /// Cruise speed the GPS assumes (m/s); 0: not for cars.
    pub fn speed(class: u8) -> f32 {
        [30.0, 25.0, 15.0, 13.0, 11.0, 8.0, 4.0, 0.0, 0.0, 0.0, 0.0][class.min(10) as usize]
    }
}

pub const ONEWAY: u8 = 1;
pub const BRIDGE: u8 = 2;
pub const TUNNEL: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PoiKind {
    Landmark,
    Hospital,
    Police,
    Shop,
    Fuel,
    Station,
    Airport,
    Church,
    Museum,
    School,
    TownHall,
    Stadium,
    Culture,
}

impl PoiKind {
    pub const ALL: [PoiKind; 13] = [Self::Landmark, Self::Hospital, Self::Police, Self::Shop, Self::Fuel, Self::Station, Self::Airport, Self::Church, Self::Museum, Self::School, Self::TownHall, Self::Stadium, Self::Culture];

    pub fn from_u8(k: u8) -> Option<Self> {
        Self::ALL.get(k as usize).copied()
    }

    /// Name in the pause map's legend.
    pub fn label(self) -> &'static str {
        match self {
            Self::Landmark => "Monumento",
            Self::Hospital => "Hospital",
            Self::Police => "Policía",
            Self::Shop => "Mercado / supermercado",
            Self::Fuel => "Gasolinera",
            Self::Station => "Estación",
            Self::Airport => "Aeropuerto",
            Self::Church => "Iglesia",
            Self::Museum => "Museo",
            Self::School => "Universidad",
            Self::TownHall => "Ayuntamiento",
            Self::Stadium => "Estadio",
            Self::Culture => "Teatro / auditorio",
        }
    }

    /// Services the radar always shows; the rest only appear on the big
    /// radar and the pause map.
    pub fn on_radar(self) -> bool {
        matches!(self, Self::Landmark | Self::Hospital | Self::Police | Self::Shop | Self::Fuel | Self::Station | Self::Airport)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub kind: u8,
    /// Rings `first_ring..first_ring + rings` of [`MapData::rings`]: the
    /// outline, then its holes.
    pub first_ring: u32,
    pub rings: u32,
    pub min: Vec2,
    pub max: Vec2,
}

#[derive(Clone, Copy, Debug)]
pub struct Road {
    pub class: u8,
    /// [`ONEWAY`] (along the points), [`BRIDGE`], [`TUNNEL`].
    pub flags: u8,
    /// Points `start..start + len` of [`MapData::pts`].
    pub start: u32,
    pub len: u32,
    /// Roadway width in metres (0: the class width).
    pub width: f32,
    pub min: Vec2,
    pub max: Vec2,
}

impl Road {
    pub fn width(&self) -> f32 {
        if self.width > 0.0 { self.width } else { road::width(self.class) }
    }
}

#[derive(Clone, Debug)]
pub struct Poi {
    pub kind: PoiKind,
    pub pos: Vec2,
    /// Height, when known (blips show an arrow when it is above or below).
    pub height: Option<f32>,
    pub name: String,
}

/// The map of one world.
#[derive(Clone, Debug, Default)]
pub struct MapData {
    pub name: String,
    pub min: Vec2,
    pub max: Vec2,
    /// Outside the land areas is sea (else plain ground, on maps with no
    /// coast).
    pub sea: bool,
    pub pts: Vec<Vec2>,
    /// `(start, len)` in `pts`.
    pub rings: Vec<(u32, u32)>,
    pub areas: Vec<Area>,
    pub roads: Vec<Road>,
    pub pois: Vec<Poi>,
}

/// Anything that can describe itself to the radar. The streamed island
/// world implements this (or simply builds a [`MapData`]).
pub trait MapSource {
    fn map_data(&self) -> MapData;
}

/// Where the island's metric frame (`+x` east, `+y` north) sits in the game
/// world: `origin` is the island point at the world origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IslandFrame {
    pub origin: Vec2,
}

impl IslandFrame {
    /// The window `data/maps/isla_valencia.json` was cut around
    /// (`tools/isla/isla_to_osm.py ... 3520 3170 230`).
    pub const VALENCIA: IslandFrame = IslandFrame { origin: Vec2::new(3520.0, 3170.0) };

    pub fn to_world(&self, island: Vec2) -> Vec2 {
        Vec2::new(island.x - self.origin.x, -(island.y - self.origin.y))
    }

    pub fn to_island(&self, world: Vec2) -> Vec2 {
        Vec2::new(world.x + self.origin.x, self.origin.y - world.y)
    }
}

fn bounds(pts: &[Vec2]) -> (Vec2, Vec2) {
    pts.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
}

impl MapData {
    pub fn ring(&self, i: u32) -> &[Vec2] {
        let (s, n) = self.rings[i as usize];
        &self.pts[s as usize..(s + n) as usize]
    }

    pub fn road_pts(&self, r: &Road) -> &[Vec2] {
        &self.pts[r.start as usize..(r.start + r.len) as usize]
    }

    /// Adds an area: its outline, then its holes.
    pub fn add_area(&mut self, kind: u8, rings: &[&[Vec2]]) {
        let Some(outline) = rings.first().filter(|r| r.len() >= 3) else { return };
        let (min, max) = bounds(outline);
        let first_ring = self.rings.len() as u32;
        for r in rings.iter().filter(|r| r.len() >= 3) {
            self.rings.push((self.pts.len() as u32, r.len() as u32));
            self.pts.extend_from_slice(r);
        }
        self.areas.push(Area { kind, first_ring, rings: self.rings.len() as u32 - first_ring, min, max });
    }

    pub fn add_road(&mut self, class: u8, flags: u8, pts: &[Vec2], width: f32) {
        if pts.len() < 2 {
            return;
        }
        let (min, max) = bounds(pts);
        self.roads.push(Road { class, flags, start: self.pts.len() as u32, len: pts.len() as u32, width, min, max });
        self.pts.extend_from_slice(pts);
    }

    /// Recomputes the extent from the areas and roads (plus `margin`).
    pub fn fit(&mut self, margin: f32) {
        let (lo, hi) = bounds(&self.pts);
        if lo.x <= hi.x {
            self.min = lo - Vec2::splat(margin);
            self.max = hi + Vec2::splat(margin);
        }
    }

    /// The packed island file (see `tools/isla/isla_minimap.py`), placed
    /// in the world by `frame`.
    pub fn from_island_file(path: &Path, frame: IslandFrame) -> Result<MapData, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(std::io::BufReader::new(file)).read_to_end(&mut raw).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_island_bytes(&raw, frame).ok_or_else(|| format!("{}: not a minimap file", path.display()))
    }

    /// [`Self::from_island_file`] from the unpacked bytes; `None` when they
    /// are truncated or of another version.
    pub fn from_island_bytes(raw: &[u8], frame: IslandFrame) -> Option<MapData> {
        struct Rd<'a>(&'a [u8]);
        impl Rd<'_> {
            fn take(&mut self, n: usize) -> Option<&[u8]> {
                let (a, b) = self.0.split_at_checked(n)?;
                self.0 = b;
                Some(a)
            }
            fn u8(&mut self) -> Option<u8> {
                self.take(1).map(|b| b[0])
            }
            fn u16(&mut self) -> Option<u16> {
                self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
            }
            fn u32(&mut self) -> Option<u32> {
                self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            fn f32(&mut self) -> Option<f32> {
                self.u32().map(f32::from_bits)
            }
        }
        let mut r = Rd(raw);
        if r.take(4)? != b"FJMM" || r.u32()? != 1 {
            return None;
        }
        let (x0, y0, scale) = (r.f32()?, r.f32()?, r.f32()?);
        let mut m = MapData { name: "Isla Levante".into(), sea: true, ..Default::default() };
        let point = |r: &mut Rd| -> Option<Vec2> {
            let (x, y) = (r.u16()?, r.u16()?);
            Some(frame.to_world(Vec2::new(x0 + x as f32 * scale, y0 + y as f32 * scale)))
        };
        let line = |r: &mut Rd, pts: &mut Vec<Vec2>| -> Option<(u32, u32)> {
            let n = r.u16()? as usize;
            let start = pts.len() as u32;
            for _ in 0..n {
                pts.push(point(r)?);
            }
            Some((start, n as u32))
        };
        for _ in 0..r.u32()? {
            let (kind, rings) = (r.u8()?, r.u8()? as u32);
            let first_ring = m.rings.len() as u32;
            for _ in 0..rings {
                let ring = line(&mut r, &mut m.pts)?;
                m.rings.push(ring);
            }
            if rings == 0 || (kind as usize) >= area::COUNT {
                continue;
            }
            let (min, max) = bounds(m.ring(first_ring));
            m.areas.push(Area { kind, first_ring, rings, min, max });
        }
        for _ in 0..r.u32()? {
            let (class, flags) = (r.u8()?, r.u8()?);
            let (start, len) = line(&mut r, &mut m.pts)?;
            if len < 2 || (class as usize) >= road::COUNT {
                continue;
            }
            let (min, max) = bounds(&m.pts[start as usize..(start + len) as usize]);
            m.roads.push(Road { class, flags, start, len, width: 0.0, min, max });
        }
        for _ in 0..r.u32()? {
            let kind = r.u8()?;
            let pos = point(&mut r)?;
            let n = r.u8()? as usize;
            let name = String::from_utf8_lossy(r.take(n)?).into_owned();
            if let Some(kind) = PoiKind::from_u8(kind) {
                m.pois.push(Poi { kind, pos, height: None, name });
            }
        }
        m.fit(0.0);
        Some(m)
    }

    /// The map of a built world: its streets (the traffic description),
    /// with their ground and its plazas; the rest is built-up blocks.
    pub fn from_world(desc: &WorldDescription) -> MapData {
        let mut m = MapData { name: desc.name.clone(), ..Default::default() };
        let e = desc.extent;
        if e.x > 0.0 && e.y > 0.0 {
            m.add_area(area::BLOCK, &[&[Vec2::new(-e.x, -e.y), Vec2::new(e.x, -e.y), Vec2::new(e.x, e.y), Vec2::new(-e.x, e.y)]]);
        }
        if let Some(t) = &desc.traffic {
            for (lo, hi) in &t.plazas {
                m.add_area(area::PLAZA, &[&[*lo, Vec2::new(hi.x, lo.y), *hi, Vec2::new(lo.x, hi.y)]]);
            }
        }
        // Everything that is not street is a block; each street lays its
        // ground from building line to building line, then its roadway.
        // The pieces stop at the building lines of the streets they meet:
        // they are drawn on to the junction.
        if let Some(t) = &desc.traffic {
            let mut roadways = Vec::new();
            for s in &t.streets {
                let mut pts = s.centreline();
                let n = pts.len();
                let reach = |end: Vec2, next: Vec2, node: Option<usize>| match node.and_then(|i| t.nodes.get(i)).filter(|c| c.distance(end) < 60.0) {
                    Some(c) => *c,
                    None => end + (end - next).normalize_or_zero() * s.half_width,
                };
                let (first, last) = (reach(pts[0], pts[1], s.nodes.map(|k| k[0])), reach(pts[n - 1], pts[n - 2], s.nodes.map(|k| k[1])));
                if first.distance(pts[0]) > 0.3 {
                    pts.insert(0, first);
                }
                if last.distance(pts[pts.len() - 1]) > 0.3 {
                    pts.push(last);
                }
                m.add_road(road::GROUND, 0, &pts, s.half_width * 2.0);
                let fwd = s.lanes.iter().filter(|l| l.forward).count();
                let oneway = !s.lanes.is_empty() && (fwd == 0 || fwd == s.lanes.len());
                if oneway && fwd == 0 {
                    pts.reverse();
                }
                let class = if s.lanes.is_empty() {
                    road::PEDESTRIAN
                } else if s.road_half >= 8.0 {
                    road::PRIMARY
                } else if s.road_half >= 5.5 {
                    road::SECONDARY
                } else if s.road_half >= 3.6 {
                    road::TERTIARY
                } else {
                    road::RESIDENTIAL
                };
                roadways.push((class, if oneway { ONEWAY } else { 0 }, pts, (s.road_half * 2.0).max(4.0)));
            }
            for (class, flags, pts, width) in roadways {
                m.add_road(class, flags, &pts, width);
            }
        }
        m.fit(0.0);
        if e.x > 0.0 {
            m.min = m.min.min(-e);
            m.max = m.max.max(e);
        }
        m
    }
}

impl MapSource for WorldDescription {
    fn map_data(&self) -> MapData {
        MapData::from_world(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn island_frame_round_trips() {
        let f = IslandFrame::VALENCIA;
        assert_eq!(f.to_world(Vec2::new(3520.0, 3170.0)), Vec2::ZERO);
        // North of the origin is -Z.
        assert_eq!(f.to_world(Vec2::new(3520.0, 3270.0)), Vec2::new(0.0, -100.0));
        let p = Vec2::new(-812.5, 406.25);
        assert_eq!(f.to_world(f.to_island(p)), p);
    }

    #[test]
    fn packed_file_is_read_and_truncations_are_refused() {
        let mut b = Vec::new();
        b.extend_from_slice(b"FJMM");
        b.extend_from_slice(&1u32.to_le_bytes());
        for v in [100.0f32, 200.0, 0.5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // One land triangle.
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[area::LAND, 1]);
        b.extend_from_slice(&3u16.to_le_bytes());
        for v in [0u16, 0, 200, 0, 0, 200] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // One one-way primary road.
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[road::PRIMARY, ONEWAY]);
        b.extend_from_slice(&2u16.to_le_bytes());
        for v in [0u16, 0, 100, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // One hospital.
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(1);
        for v in [20u16, 40] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.push(3);
        b.extend_from_slice(b"H 1");
        let frame = IslandFrame { origin: Vec2::new(100.0, 200.0) };
        let m = MapData::from_island_bytes(&b, frame).expect("valid");
        assert_eq!((m.areas.len(), m.roads.len(), m.pois.len()), (1, 1, 1));
        assert_eq!(m.ring(m.areas[0].first_ring), &[Vec2::ZERO, Vec2::new(100.0, 0.0), Vec2::new(0.0, -100.0)]);
        assert_eq!(m.road_pts(&m.roads[0]), &[Vec2::ZERO, Vec2::new(50.0, 0.0)]);
        assert_eq!(m.roads[0].flags & ONEWAY, ONEWAY);
        assert_eq!((m.pois[0].kind, m.pois[0].pos, m.pois[0].name.as_str()), (PoiKind::Hospital, Vec2::new(10.0, -20.0), "H 1"));
        assert!(m.sea);
        for n in 0..b.len() {
            assert!(MapData::from_island_bytes(&b[..n], frame).is_none(), "truncated at {n}");
        }
    }
}
