//! The map painted into square tiles, once: a small software rasteriser
//! (filled polygons and thick lines, 2 x 2 supersampled) run on a worker
//! thread, and the cache of the resulting textures. The radar and the
//! pause map only draw a handful of textured quads per frame.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use forja_math::Vec2;

use super::data::{AREA_ORDER, MapData, TUNNEL, area, road};

/// Texels along a tile image; the outer ring of texels overlaps the
/// neighbours, so bilinear filtering has no seams.
pub const TILE: usize = 256;
/// Texels of a tile that are its own.
pub const INNER: f32 = (TILE - 2) as f32;
/// Metres per texel of level 0; each level doubles it.
pub const BASE_MPP: f32 = 0.5;
pub const LEVELS: u8 = 8;

pub fn level_mpp(level: u8) -> f32 {
    BASE_MPP * (1u32 << level) as f32
}

/// The level whose texels are closest to `mpp` metres per screen pixel
/// (never coarser than about 1.6 pixels per texel).
pub fn level_for(mpp: f32) -> u8 {
    ((mpp / BASE_MPP).max(1.0).log2() + 0.3).floor().clamp(0.0, (LEVELS - 1) as f32) as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub level: u8,
    pub x: i32,
    pub y: i32,
}

impl TileKey {
    /// Ground size of a tile of this level.
    pub fn size(&self) -> f32 {
        INNER * level_mpp(self.level)
    }

    /// Ground corner with the smallest coordinates.
    pub fn origin(&self) -> Vec2 {
        Vec2::new(self.x as f32, self.y as f32) * self.size()
    }

    /// The tile of the level above that contains this one.
    pub fn parent(&self) -> TileKey {
        TileKey { level: self.level + 1, x: self.x.div_euclid(2), y: self.y.div_euclid(2) }
    }
}

pub const SEA: [u8; 3] = [70, 98, 124];
pub const VOID: [u8; 3] = [44, 46, 45];
pub const LAND: [u8; 3] = [60, 62, 58];

fn area_color(kind: u8) -> [u8; 3] {
    match kind {
        area::LAND => LAND,
        area::WATER => SEA,
        area::GREEN => [58, 84, 56],
        area::FOREST => [48, 70, 48],
        area::FARM => [64, 71, 55],
        area::BEACH => [146, 138, 108],
        area::URBAN => [70, 70, 67],
        area::INDUSTRIAL => [76, 74, 71],
        area::AIRFIELD => [68, 70, 71],
        area::WETLAND => [62, 90, 88],
        area::ROCK => [82, 80, 76],
        area::BUILDING | area::BLOCK => [91, 90, 85],
        area::LANDMARK => [150, 126, 82],
        area::RIVERBED => [84, 92, 74],
        area::APRON => [92, 94, 96],
        _ => [96, 94, 88],
    }
}

/// Colour of a road class (the GTA V radar's light greys, warmer and
/// brighter the more important the road).
pub fn road_color(class: u8) -> [u8; 3] {
    match class {
        road::MOTORWAY => [240, 226, 178],
        road::TRUNK => [238, 232, 208],
        road::PRIMARY => [226, 226, 221],
        road::SECONDARY => [198, 198, 193],
        road::TERTIARY => [172, 172, 168],
        road::RESIDENTIAL => [146, 146, 142],
        road::SERVICE => [116, 116, 112],
        road::PEDESTRIAN => [126, 121, 110],
        road::RAIL => [28, 28, 30],
        road::GROUND => [62, 63, 60],
        _ => [118, 120, 124],
    }
}

/// Coarsest metres per texel at which a road class is still drawn, and its
/// least width in texels.
fn road_lod(class: u8) -> (f32, f32) {
    match class {
        road::MOTORWAY => (f32::MAX, 2.4),
        road::TRUNK => (f32::MAX, 2.0),
        road::PRIMARY => (f32::MAX, 1.6),
        road::SECONDARY => (40.0, 1.3),
        road::TERTIARY => (20.0, 1.1),
        road::RESIDENTIAL => (9.0, 0.9),
        road::RAIL => (9.0, 0.7),
        road::RUNWAY => (40.0, 1.5),
        road::GROUND => (20.0, 1.0),
        _ => (2.5, 0.8),
    }
}

/// Features by cell, to find those of a tile.
pub struct MapIndex {
    pub map: Arc<MapData>,
    origin: Vec2,
    cell: f32,
    nx: i32,
    ny: i32,
    areas: Vec<Vec<u32>>,
    roads: Vec<Vec<u32>>,
    /// Areas too large for the cells (the coast, lakes): always tested.
    big: Vec<u32>,
}

impl MapIndex {
    pub fn new(map: Arc<MapData>) -> MapIndex {
        let cell = 256.0;
        let origin = map.min;
        let n = ((map.max - map.min) / cell).ceil().max(Vec2::ONE);
        let (nx, ny) = ((n.x as i32).clamp(1, 1024), (n.y as i32).clamp(1, 1024));
        let mut areas = vec![Vec::new(); (nx * ny) as usize];
        let mut roads = vec![Vec::new(); (nx * ny) as usize];
        let mut big = Vec::new();
        let range = |lo: Vec2, hi: Vec2| {
            let a = ((lo - origin) / cell).floor();
            let b = ((hi - origin) / cell).floor();
            ((a.x as i32).clamp(0, nx - 1), (a.y as i32).clamp(0, ny - 1), (b.x as i32).clamp(0, nx - 1), (b.y as i32).clamp(0, ny - 1))
        };
        for (i, a) in map.areas.iter().enumerate() {
            let (x0, y0, x1, y1) = range(a.min, a.max);
            if (x1 - x0 + 1) * (y1 - y0 + 1) > 400 {
                big.push(i as u32);
                continue;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    areas[(y * nx + x) as usize].push(i as u32);
                }
            }
        }
        for (i, r) in map.roads.iter().enumerate() {
            // By segment: a long road only lands in the cells it crosses.
            let mut last = usize::MAX;
            for w in map.road_pts(r).windows(2) {
                let pad = Vec2::splat(r.width());
                let (x0, y0, x1, y1) = range(w[0].min(w[1]) - pad, w[0].max(w[1]) + pad);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let c = (y * nx + x) as usize;
                        if c != last && roads[c].last() != Some(&(i as u32)) {
                            roads[c].push(i as u32);
                        }
                        last = c;
                    }
                }
            }
        }
        MapIndex { map, origin, cell, nx, ny, areas, roads, big }
    }

    fn cells(&self, lo: Vec2, hi: Vec2) -> impl Iterator<Item = usize> + use<> {
        let a = ((lo - self.origin) / self.cell).floor();
        let b = ((hi - self.origin) / self.cell).floor();
        let (nx, ny) = (self.nx, self.ny);
        let (x0, y0, x1, y1) = ((a.x as i32).max(0), (a.y as i32).max(0), (b.x as i32).min(nx - 1), (b.y as i32).min(ny - 1));
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (y * nx + x) as usize))
    }

    /// Paints the ground square from `lo`, `size` metres wide, into an
    /// image of `texels` x `texels`.
    pub fn paint(&self, lo: Vec2, size: f32, texels: usize) -> Vec<egui::Color32> {
        const SS: usize = 2;
        let n = texels * SS;
        let mpp = size / texels as f32;
        let k = n as f32 / size;
        let hi = lo + Vec2::splat(size);
        let map = &*self.map;
        let mut c = Canvas { n, px: vec![if map.sea { SEA } else { VOID }; n * n], cross: Vec::new() };
        let mut ids: Vec<u32> = self.big.clone();
        for cell in self.cells(lo, hi) {
            ids.extend_from_slice(&self.areas[cell]);
        }
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|&i| {
            let a = &map.areas[i as usize];
            let d = a.max - a.min;
            a.max.x >= lo.x && a.max.y >= lo.y && a.min.x <= hi.x && a.min.y <= hi.y && (a.kind == area::LAND || (d.x.max(d.y) >= mpp * 0.8 && (a.kind != area::BUILDING || mpp <= 3.5)))
        });
        let order = |kind: u8| AREA_ORDER.iter().position(|k| *k == kind).unwrap_or(0);
        ids.sort_by_key(|&i| order(map.areas[i as usize].kind));
        let mut ring: Vec<Vec<Vec2>> = Vec::new();
        for &i in &ids {
            let a = &map.areas[i as usize];
            ring.clear();
            for r in a.first_ring..a.first_ring + a.rings {
                ring.push(map.ring(r).iter().map(|p| (*p - lo) * k).collect());
            }
            c.fill(&ring, area_color(a.kind));
        }
        // Roads: the least important first.
        let mut ids: Vec<u32> = Vec::new();
        for cell in self.cells(lo - Vec2::splat(50.0), hi + Vec2::splat(50.0)) {
            ids.extend_from_slice(&self.roads[cell]);
        }
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|&i| {
            let r = &map.roads[i as usize];
            let pad = r.width();
            mpp <= road_lod(r.class).0 && r.max.x + pad >= lo.x && r.max.y + pad >= lo.y && r.min.x - pad <= hi.x && r.min.y - pad <= hi.y
        });
        let rank = |class: u8| match class {
            road::GROUND => -1,
            road::RAIL => 0,
            road::RUNWAY => 1,
            c => 12 - c as i32,
        };
        ids.sort_by_key(|&i| rank(map.roads[i as usize].class));
        let mut line: Vec<Vec2> = Vec::new();
        for &i in &ids {
            let r = &map.roads[i as usize];
            let width = (r.width() * k).max(road_lod(r.class).1 * SS as f32);
            let mut col = road_color(r.class);
            if r.flags & TUNNEL != 0 {
                for (c, l) in col.iter_mut().zip(LAND) {
                    *c = ((*c as u16 + l as u16 * 2) / 3) as u8;
                }
            }
            line.clear();
            line.extend(map.road_pts(r).iter().map(|p| (*p - lo) * k));
            c.line(&line, width, col);
        }
        // Box filter down to the tile.
        let mut out = Vec::with_capacity(texels * texels);
        for y in 0..texels {
            for x in 0..texels {
                let mut s = [0u32; 3];
                for dy in 0..SS {
                    for dx in 0..SS {
                        let p = c.px[(y * SS + dy) * n + x * SS + dx];
                        s[0] += p[0] as u32;
                        s[1] += p[1] as u32;
                        s[2] += p[2] as u32;
                    }
                }
                let d = (SS * SS) as u32;
                out.push(egui::Color32::from_rgb((s[0] / d) as u8, (s[1] / d) as u8, (s[2] / d) as u8));
            }
        }
        out
    }

    /// The image of a tile (with its overlap ring).
    pub fn paint_tile(&self, key: TileKey) -> egui::ColorImage {
        let mpp = level_mpp(key.level);
        egui::ColorImage::new([TILE, TILE], self.paint(key.origin() - Vec2::splat(mpp), TILE as f32 * mpp, TILE))
    }
}

struct Canvas {
    n: usize,
    px: Vec<[u8; 3]>,
    /// Scratch: `(row, x)` crossings of the polygon being filled.
    cross: Vec<(u32, f32)>,
}

impl Canvas {
    /// Even-odd fill of the rings (canvas pixels).
    fn fill(&mut self, rings: &[Vec<Vec2>], color: [u8; 3]) {
        let n = self.n as f32;
        self.cross.clear();
        for ring in rings {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                let (a, b) = if a.y <= b.y { (a, b) } else { (b, a) };
                if b.y <= 0.0 || a.y >= n || a.y == b.y {
                    continue;
                }
                // Rows whose centre (y + 0.5) lies in [a.y, b.y).
                let y0 = (a.y - 0.5).ceil().max(0.0) as u32;
                let y1 = ((b.y - 0.5).ceil().min(n)) as u32;
                let slope = (b.x - a.x) / (b.y - a.y);
                for y in y0..y1 {
                    self.cross.push((y, a.x + (y as f32 + 0.5 - a.y) * slope));
                }
            }
        }
        if self.cross.is_empty() {
            return;
        }
        self.cross.sort_unstable_by(|p, q| p.0.cmp(&q.0).then(p.1.total_cmp(&q.1)));
        let mut i = 0;
        while i + 1 < self.cross.len() {
            let (y, xa) = self.cross[i];
            let (y2, xb) = self.cross[i + 1];
            if y != y2 {
                i += 1;
                continue;
            }
            // Pixels whose centre lies in [xa, xb).
            let x0 = (xa - 0.5).ceil().clamp(0.0, n) as usize;
            let x1 = (xb - 0.5).ceil().clamp(0.0, n) as usize;
            if x1 > x0 {
                let row = y as usize * self.n;
                self.px[row + x0..row + x1].fill(color);
            }
            i += 2;
        }
    }

    fn disc(&mut self, c: Vec2, r: f32, color: [u8; 3]) {
        let n = self.n as f32;
        let y0 = (c.y - r - 0.5).ceil().max(0.0) as usize;
        let y1 = ((c.y + r - 0.5).floor() + 1.0).clamp(0.0, n) as usize;
        for y in y0..y1 {
            let dy = y as f32 + 0.5 - c.y;
            let h = (r * r - dy * dy).max(0.0).sqrt();
            let x0 = (c.x - h - 0.5).ceil().clamp(0.0, n) as usize;
            let x1 = (c.x + h - 0.5).ceil().clamp(0.0, n) as usize;
            if x1 > x0 {
                self.px[y * self.n + x0..y * self.n + x1].fill(color);
            }
        }
    }

    /// A polyline `width` pixels wide with round joints.
    fn line(&mut self, pts: &[Vec2], width: f32, color: [u8; 3]) {
        let n = self.n as f32;
        let h = width * 0.5;
        let mut quad = [Vec::with_capacity(4)];
        for (i, w) in pts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            if a.x.max(b.x) < -h || a.y.max(b.y) < -h || a.x.min(b.x) > n + h || a.y.min(b.y) > n + h {
                continue;
            }
            let d = b - a;
            let l = d.length();
            if l > 0.0 {
                let o = Vec2::new(-d.y, d.x) / l * h;
                quad[0].clear();
                quad[0].extend_from_slice(&[a + o, b + o, b - o, a - o]);
                self.fill(&quad, color);
            }
            if width >= 2.5 && (i > 0 || pts.len() == 2) {
                self.disc(a, h, color);
            }
            if width >= 2.5 && i + 2 == pts.len() {
                self.disc(b, h, color);
            }
        }
    }
}

struct Shared {
    /// Tiles asked for, the most recent last (painted first).
    queue: Mutex<(Vec<TileKey>, bool)>,
    wake: Condvar,
}

struct Tile {
    texture: egui::TextureHandle,
    used: u64,
}

/// The tile textures: painted on demand by a worker thread, kept while
/// they are in use.
pub struct Tiles {
    pub index: Arc<MapIndex>,
    cache: HashMap<TileKey, Tile>,
    pending: HashSet<TileKey>,
    shared: Arc<Shared>,
    done: Receiver<(TileKey, egui::ColorImage)>,
    /// Paint on the calling thread (headless captures and tests).
    pub sync: bool,
    frame: u64,
    /// Tiles kept at most.
    pub capacity: usize,
    /// Tiles painted so far and the time it took (statistics).
    pub painted: (usize, f32),
}

impl Tiles {
    pub fn new(index: Arc<MapIndex>) -> Tiles {
        let shared = Arc::new(Shared { queue: Mutex::new((Vec::new(), false)), wake: Condvar::new() });
        let (tx, done): (Sender<(TileKey, egui::ColorImage)>, _) = channel();
        let (sh, ix) = (Arc::clone(&shared), Arc::clone(&index));
        let spawned = std::thread::Builder::new().name("minimap-tiles".into()).spawn(move || {
            loop {
                let key = {
                    let mut q = sh.queue.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        if q.1 {
                            return;
                        }
                        if let Some(k) = q.0.pop() {
                            break k;
                        }
                        q = sh.wake.wait(q).unwrap_or_else(|e| e.into_inner());
                    }
                };
                if tx.send((key, ix.paint_tile(key))).is_err() {
                    return;
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("minimap: no tile thread ({e}); tiles are painted on the frame");
        }
        Tiles { index, cache: HashMap::new(), pending: HashSet::new(), shared, done, sync: false, frame: 0, capacity: 160, painted: (0, 0.0) }
    }

    /// Takes finished tiles from the worker (a few per frame: each is a
    /// texture upload) and drops the least recently used beyond capacity.
    pub fn pump(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        for _ in 0..3 {
            let Ok((key, image)) = self.done.try_recv() else { break };
            self.pending.remove(&key);
            self.insert(ctx, key, image);
        }
        if self.cache.len() > self.capacity {
            let mut by_age: Vec<(u64, TileKey)> = self.cache.iter().map(|(k, t)| (t.used, *k)).collect();
            by_age.sort_unstable_by_key(|e| e.0);
            for (used, key) in by_age.into_iter().take(self.cache.len() - self.capacity) {
                if used + 2 < self.frame {
                    self.cache.remove(&key);
                }
            }
        }
    }

    fn insert(&mut self, ctx: &egui::Context, key: TileKey, image: egui::ColorImage) {
        let options = egui::TextureOptions { magnification: egui::TextureFilter::Linear, minification: egui::TextureFilter::Linear, wrap_mode: egui::TextureWrapMode::ClampToEdge, mipmap_mode: None };
        let texture = ctx.load_texture(format!("minimap {} {} {}", key.level, key.x, key.y), image, options);
        self.cache.insert(key, Tile { texture, used: self.frame });
    }

    /// The texture of a tile, asking for it when it is not there yet.
    pub fn get(&mut self, ctx: &egui::Context, key: TileKey) -> Option<egui::TextureId> {
        if let Some(t) = self.cache.get_mut(&key) {
            t.used = self.frame;
            return Some(t.texture.id());
        }
        if self.sync {
            let t = std::time::Instant::now();
            let image = self.index.paint_tile(key);
            self.painted = (self.painted.0 + 1, self.painted.1 + t.elapsed().as_secs_f32() * 1000.0);
            self.insert(ctx, key, image);
            return self.cache.get(&key).map(|t| t.texture.id());
        }
        if self.pending.len() < 96 && self.pending.insert(key) {
            let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            q.0.push(key);
            self.shared.wake.notify_one();
        }
        None
    }

    /// A loaded tile that covers `key` from a coarser level: the texture
    /// and the part of it (`uv` min and max) that is `key`'s ground.
    pub fn fallback(&mut self, key: TileKey) -> Option<(egui::TextureId, [f32; 4])> {
        let mut k = key;
        for _ in 0..4 {
            if k.level + 1 >= LEVELS {
                return None;
            }
            k = k.parent();
            if let Some(t) = self.cache.get_mut(&k) {
                t.used = self.frame;
                let rel = (key.origin() - k.origin()) / k.size();
                let f = key.size() / k.size();
                let uv = |v: f32| (1.0 + v * INNER) / TILE as f32;
                return Some((t.texture.id(), [uv(rel.x), uv(rel.y), uv(rel.x + f), uv(rel.y + f)]));
            }
        }
        None
    }

    pub fn loaded(&self) -> usize {
        self.cache.len()
    }

    pub fn waiting(&self) -> usize {
        self.pending.len()
    }

    /// Forgets the requests the worker has not started (the view moved on).
    pub fn forget_waiting(&mut self) {
        let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        for k in q.0.drain(..) {
            self.pending.remove(&k);
        }
    }
}

impl Drop for Tiles {
    fn drop(&mut self) {
        let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.1 = true;
        q.0.clear();
        self.shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> Arc<MapData> {
        let mut m = MapData { sea: true, ..Default::default() };
        // An island 200 m square with a lake and a north-south avenue.
        m.add_area(area::LAND, &[&[Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0), Vec2::new(200.0, 200.0), Vec2::new(0.0, 200.0)]]);
        m.add_area(area::GREEN, &[&[Vec2::new(20.0, 20.0), Vec2::new(60.0, 20.0), Vec2::new(60.0, 60.0), Vec2::new(20.0, 60.0)], &[Vec2::new(30.0, 30.0), Vec2::new(50.0, 30.0), Vec2::new(50.0, 50.0), Vec2::new(30.0, 50.0)]]);
        m.add_road(road::PRIMARY, 0, &[Vec2::new(100.0, -50.0), Vec2::new(100.0, 250.0)], 0.0);
        m.fit(50.0);
        Arc::new(m)
    }

    fn rgb(c: egui::Color32) -> [u8; 3] {
        [c.r(), c.g(), c.b()]
    }

    #[test]
    fn levels_follow_the_zoom() {
        assert_eq!(level_for(0.2), 0);
        assert_eq!(level_for(0.5), 0);
        assert_eq!(level_for(1.0), 1);
        assert_eq!(level_for(7.0), 4);
        assert_eq!(level_for(1.0e6), LEVELS - 1);
        let k = TileKey { level: 2, x: -3, y: 5 };
        assert_eq!(k.size(), 254.0 * 2.0);
        assert_eq!(k.parent(), TileKey { level: 3, x: -2, y: 2 });
        // A tile lies inside its parent.
        let (p, o) = (k.parent(), k.origin());
        assert!(o.x >= p.origin().x && o.x + k.size() <= p.origin().x + p.size() + 1e-3);
    }

    #[test]
    fn paints_land_sea_holes_and_roads() {
        let ix = MapIndex::new(map());
        // 1 m per texel over (-28, -28)..(228, 228).
        let img = ix.paint(Vec2::splat(-28.0), 256.0, 256);
        let at = |x: f32, y: f32| rgb(img[(y + 28.0) as usize * 256 + (x + 28.0) as usize]);
        assert_eq!(at(-20.0, -20.0), SEA);
        assert_eq!(at(150.0, 150.0), LAND);
        assert_eq!(at(25.0, 40.0), area_color(area::GREEN));
        // The hole of the park shows the land under it.
        assert_eq!(at(40.0, 40.0), LAND);
        // The avenue crosses land and sea.
        assert_eq!(at(100.0, 120.0), road_color(road::PRIMARY));
        assert_eq!(at(100.0, -20.0), road_color(road::PRIMARY));
        assert_eq!(at(100.0 + road::width(road::PRIMARY), 120.0), LAND);
    }

    #[test]
    fn far_levels_keep_main_roads_visible() {
        let ix = MapIndex::new(map());
        // 32 m per texel: the avenue is thinner than a texel but drawn.
        let img = ix.paint(Vec2::new(-1948.0, -1948.0), 4096.0, 128);
        let col = ((100.0 + 1948.0) / 32.0) as usize;
        let row = ((100.0 + 1948.0) / 32.0) as usize;
        let c = rgb(img[row * 128 + col]);
        assert!(c[0] > LAND[0] + 40, "{c:?}");
    }
}
