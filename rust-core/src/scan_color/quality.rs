//! Capture-quality kernels: edge acutance (fine/broad gradient ratio over nine
//! zones plus the page frame), the photographed bullet-lattice preflight and the
//! readiness decision. Pixels are Rec. 709 luma held as f32 (as a Float32Array
//! raster) while every sum is f64 and keeps the original operation order, so the
//! results are bit-identical to the JavaScript they replace.

const PITCH: f64 = 15.0;

// ---- JavaScript numeric semantics ----------------------------------------------------------

/// `Math.max` (propagates NaN, unlike `f64::max`).
fn jmax(a: f64, b: f64) -> f64 { if a.is_nan() || b.is_nan() { f64::NAN } else if a > b { a } else { b } }
/// `Math.min`.
fn jmin(a: f64, b: f64) -> f64 { if a.is_nan() || b.is_nan() { f64::NAN } else if a < b { a } else { b } }
/// `Math.round`: halves go toward +infinity (`f64::round` goes away from zero).
fn jround(x: f64) -> f64 {
    if !x.is_finite() { return x; }
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}
/// `Math.sign`.
fn jsign(x: f64) -> f64 { if x.is_nan() || x == 0.0 { x } else if x > 0.0 { 1.0 } else { -1.0 } }
/// V8's two-argument `Math.hypot` (scaled, Kahan-summed), not libm's `hypot`.
fn hypot(a: f64, b: f64) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    for v in [a, b] {
        if v.is_nan() { nan = true; } else if v.abs() > max { max = v.abs(); }
    }
    if max == f64::INFINITY { return f64::INFINITY; }
    if nan { return f64::NAN; }
    if max == 0.0 { return 0.0; }
    let (mut sum, mut compensation) = (0.0f64, 0.0f64);
    for v in [a.abs(), b.abs()] {
        let n = v / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

// ---- Inputs -------------------------------------------------------------------------------------

pub struct Raster<'a> { pub gray: &'a [f32], pub width: usize, pub height: usize }

impl Raster<'_> {
    #[inline]
    fn luma(&self, x: i64, y: i64) -> f64 {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        self.gray[y * self.width + x] as f64
    }
    /// Fine (+-1) and broad (+-3) squared gradients; None below the noise floor.
    #[inline]
    fn gradient(&self, x: i64, y: i64) -> Option<(f64, f64)> {
        let (w, h) = (self.width as i64, self.height as i64);
        let (dx, dy, bx, by);
        if x >= 3 && x + 3 < w && y >= 3 && y + 3 < h {
            let at = y as usize * self.width + x as usize;
            let g = |i: usize| self.gray[i] as f64;
            let row = self.width;
            dx = g(at + 1) - g(at - 1);
            dy = g(at + row) - g(at - row);
            bx = g(at + 3) - g(at - 3);
            by = g(at + 3 * row) - g(at - 3 * row);
        } else {
            dx = self.luma(x + 1, y) - self.luma(x - 1, y);
            dy = self.luma(x, y + 1) - self.luma(x, y - 1);
            bx = self.luma(x + 3, y) - self.luma(x - 3, y);
            by = self.luma(x, y + 3) - self.luma(x, y - 3);
        }
        let fine = dx * dx + dy * dy;
        let broad = bx * bx + by * by;
        if fine + broad >= 144.0 { Some((fine, broad)) } else { None }
    }
}

/// Frame-edge polylines: flat x,y pairs plus point-count prefix sums (`offsets.len() == edges + 1`).
pub struct Edges<'a> { pub coords: &'a [f64], pub offsets: &'a [u32] }

impl<'a> Edges<'a> {
    pub fn count(&self) -> usize { self.offsets.len().saturating_sub(1) }
    fn edge(&self, index: usize) -> &'a [f64] { &self.coords[self.offsets[index] as usize * 2..self.offsets[index + 1] as usize * 2] }
}

// ---- Acutance -----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Sharpness {
    pub valid: bool,
    pub clarity: f64,
    pub border: f64,
    pub interior: Option<f64>,
    pub zones: usize,
    pub edges: usize,
    pub minimum: Option<f64>,
}

/// `quad` holds the four page corners as normalised x,y pairs.
pub fn inspect(raster: &Raster, quad: &[f64; 8], frame: &Edges) -> Sharpness {
    let width = raster.width as f64;
    let height = raster.height as f64;
    let q: [(f64, f64); 4] = std::array::from_fn(|i| (quad[i * 2] * width, quad[i * 2 + 1] * height));
    let mut zones = [(0.0f64, 0.0f64, 0usize); 9];
    let min_x = jmin(jmin(jmin(q[0].0, q[1].0), q[2].0), q[3].0);
    let max_x = jmax(jmax(jmax(q[0].0, q[1].0), q[2].0), q[3].0);
    let min_y = jmin(jmin(jmin(q[0].1, q[1].1), q[2].1), q[3].1);
    let max_y = jmax(jmax(jmax(q[0].1, q[1].1), q[2].1), q[3].1);
    let left = jmax(4.0, min_x.floor());
    let right = jmin(width - 5.0, max_x.ceil());
    let top = jmax(4.0, min_y.floor());
    let bottom = jmin(height - 5.0, max_y.ceil());
    let inside = |x: f64, y: f64| {
        let mut sign = 0.0f64;
        for i in 0..4 {
            let (a, b) = (q[i], q[(i + 1) % 4]);
            let d = (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
            if d.abs() < 1.0 { continue; }
            if sign != 0.0 && !sign.is_nan() && jsign(d) != sign { return false; }
            sign = jsign(d);
        }
        true
    };
    let bottom_edge = (q[2].0 - q[3].0, q[2].1 - q[3].1);
    let bottom_length = hypot(bottom_edge.0, bottom_edge.1);
    let marker_gap = (hypot(q[3].0 - q[0].0, q[3].1 - q[0].1) + hypot(q[2].0 - q[1].0, q[2].1 - q[1].1)) * 0.045;
    let mut y = top + 6.0;
    while y < bottom - 6.0 {
        let mut x = left + 6.0;
        while x < right - 6.0 {
            let skip = !inside(x, y)
                || ((x - q[3].0) * bottom_edge.1 - (y - q[3].1) * bottom_edge.0).abs() / bottom_length < marker_gap;
            if !skip {
                if let Some((fine, broad)) = raster.gradient(x as i64, y as i64) {
                    let c = jmin(2.0, ((x - left) / (right - left) * 3.0).floor()) as usize;
                    let r = jmin(2.0, ((y - top) / (bottom - top) * 3.0).floor()) as usize;
                    let zone = &mut zones[r * 3 + c];
                    zone.0 += fine;
                    zone.1 += broad;
                    zone.2 += 1;
                }
            }
            x += 2.0;
        }
        y += 2.0;
    }
    let mut measured: Vec<f64> = zones.iter()
        .filter(|z| z.2 >= 12 && z.1 > 0.0 && z.0 > z.1 * 0.02).map(|z| z.0 / z.1).collect();
    measured.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let (mut fine, mut broad, mut edges) = (0.0f64, 0.0f64, 0usize);
    for index in 0..frame.count() {
        for point in frame.edge(index).chunks_exact(2) {
            let px = jround(point[0] * width);
            let py = jround(point[1] * height);
            // A non-finite point never yields a gradient; huge values clamp like any off-raster pixel.
            if px.is_nan() || py.is_nan() { continue; }
            let (px, py) = (px.clamp(-1e9, 1e9) as i64, py.clamp(-1e9, 1e9) as i64);
            for dy in -2..=2 {
                for dx in -2..=2 {
                    if let Some((f, b)) = raster.gradient(px + dx, py + dy) { fine += f; broad += b; edges += 1; }
                }
            }
        }
    }
    let border = if edges >= 60 && broad > 0.0 { fine / broad } else { 0.0 };
    let interior = if measured.len() >= 3 { Some(measured[(measured.len() as f64 * 0.25).floor() as usize]) } else { None };
    let clarity = match interior { None => border, Some(value) => jmin(border, value) };
    Sharpness {
        valid: border >= 0.32 && interior.map_or(true, |value| value >= 0.32),
        clarity, border, interior, zones: measured.len(), edges,
        minimum: measured.first().copied(),
    }
}

// ---- Lattice preflight ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridKind { Full, Empty, Clipped, Texture }

#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub kind: GridKind,
    pub valid: bool,
    pub dots: usize,
    pub matched: usize,
    pub cells: usize,
    pub rows: usize,
    pub columns: usize,
    pub coverage: f64,
    pub confidence: f64,
    pub contrast: f64,
}

impl Grid {
    fn empty(kind: GridKind, dots: usize) -> Grid {
        Grid { kind, valid: false, dots, matched: 0, cells: 0, rows: 0, columns: 0, coverage: 0.0, confidence: 0.0, contrast: 0.0 }
    }
}

struct Dot { x: f64, y: f64, contrast: f64, links: [i32; 4] }

type Point = (f64, f64);
fn mix(a: Point, b: Point, t: f64) -> Point { (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t) }
fn along(points: &[Point], t: f64) -> Point {
    let at = jmax(0.0, jmin(1.0, t)) * (points.len() - 1) as f64;
    let index = at.floor() as usize;
    mix(points[index], points[(index + 1).min(points.len() - 1)], at - index as f64)
}

/// Err(()) means a frame edge that would be used carries a non-finite coordinate.
pub fn inspect_grid(raster: &Raster, quad: &[f64; 8], frame: &Edges) -> Result<Grid, ()> {
    if quad.iter().any(|v| !v.is_finite()) { return Ok(Grid::empty(GridKind::Empty, 0)); }
    let q: [Point; 4] = std::array::from_fn(|i| (quad[i * 2], quad[i * 2 + 1]));
    let (sw, sh) = (raster.width as f64, raster.height as f64);
    let distance = |a: Point, b: Point| hypot((a.0 - b.0) * sw, (a.1 - b.1) * sh);
    let landscape = distance(q[0], q[1]) + distance(q[2], q[3]) > distance(q[1], q[2]) + distance(q[3], q[0]);
    let (width, height) = if landscape { (810usize, 540usize) } else { (540, 810) };
    let use_edges = frame.count() == 4 && (0..4).all(|i| frame.edge(i).len() >= 4);
    let polylines: Vec<Vec<Point>> = if use_edges {
        (0..4).map(|i| frame.edge(i).chunks_exact(2).map(|p| (p[0], p[1])).collect()).collect()
    } else { Vec::new() };
    if use_edges && polylines.iter().flatten().any(|p| !p.0.is_finite() || !p.1.is_finite()) { return Err(()); }
    let sides: Vec<Vec<Point>> = [width, height, width, height].iter().enumerate().map(|(side, &length)| {
        (0..length).map(|index| {
            let t = index as f64 / (length - 1) as f64;
            if use_edges { along(&polylines[side], t) } else { mix(q[side], q[(side + 1) % 4], t) }
        }).collect()
    }).collect();
    let swi = raster.width;
    let mut gray = vec![0.0f32; width * height];
    let stride = width + 1;
    let mut integral = vec![0.0f64; stride * (height + 1)];
    let mut clipped = 0usize;
    for y in 0..height {
        let v = y as f64 / (height - 1) as f64;
        let left = sides[3][height - 1 - y];
        let right = sides[1][y];
        let mut row_sum = 0.0f64;
        for x in 0..width {
            let u = x as f64 / (width - 1) as f64;
            let top = sides[0][x];
            let bottom = sides[2][width - 1 - x];
            let base_x = (q[0].0 * (1.0 - u) + q[1].0 * u) * (1.0 - v) + (q[3].0 * (1.0 - u) + q[2].0 * u) * v;
            let base_y = (q[0].1 * (1.0 - u) + q[1].1 * u) * (1.0 - v) + (q[3].1 * (1.0 - u) + q[2].1 * u) * v;
            let sx = (top.0 * (1.0 - v) + bottom.0 * v + left.0 * (1.0 - u) + right.0 * u - base_x) * sw;
            let sy = (top.1 * (1.0 - v) + bottom.1 * v + left.1 * (1.0 - u) + right.1 * u - base_y) * sh;
            if sx < 1.0 || sy < 1.0 || sx >= sw - 2.0 || sy >= sh - 2.0 { clipped += 1; }
            let ix = jmax(0.0, jmin(sw - 2.0, sx.floor()));
            let iy = jmax(0.0, jmin(sh - 2.0, sy.floor()));
            let value = if ix.is_nan() || iy.is_nan() { f64::NAN } else {
                let fx = jmax(0.0, jmin(1.0, sx - ix));
                let fy = jmax(0.0, jmin(1.0, sy - iy));
                let at = iy as usize * swi + ix as usize;
                let g = |i: usize| raster.gray[i] as f64;
                (g(at) * (1.0 - fx) + g(at + 1) * fx) * (1.0 - fy) + (g(at + swi) * (1.0 - fx) + g(at + swi + 1) * fx) * fy
            };
            gray[y * width + x] = value as f32;
            row_sum += value;
            integral[(y + 1) * stride + x + 1] = integral[y * stride + x + 1] + row_sum;
        }
    }
    if clipped as f64 > (width * height) as f64 * 0.01 { return Ok(Grid::empty(GridKind::Clipped, 0)); }
    let mut mask = vec![0u8; width * height];
    let mut contrast = vec![0.0f32; width * height];
    let (radius, margin) = (4usize, 11usize);
    for y in margin..height - margin {
        for x in margin..width - margin {
            let a = (y - radius) * stride + x - radius;
            let b = (y + radius + 1) * stride + x + radius + 1;
            let mean = (integral[b] - integral[(y - radius) * stride + x + radius + 1]
                - integral[(y + radius + 1) * stride + x - radius] + integral[a]) / 81.0;
            let at = y * width + x;
            let deficit = mean - gray[at] as f64;
            // Relative local contrast retains real dots under dim or tinted light;
            // the absolute floor excludes quantization noise in almost black areas.
            if deficit >= jmax(4.0, mean * 0.022) { mask[at] = 1; contrast[at] = deficit as f32; }
        }
    }
    let mut queue = vec![0i32; width * height];
    let mut dots: Vec<Dot> = Vec::new();
    for start in 0..mask.len() {
        if mask[start] != 1 { continue; }
        let (mut read, mut end) = (0usize, 1usize);
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (width, 0usize, height, 0usize);
        let (mut weight, mut sum_x, mut sum_y, mut peak) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        queue[0] = start as i32;
        mask[start] = 2;
        while read < end {
            let at = queue[read] as usize;
            read += 1;
            let (x, y) = (at % width, at / width);
            let w = contrast[at] as f64;
            min_x = min_x.min(x); max_x = max_x.max(x); min_y = min_y.min(y); max_y = max_y.max(y);
            weight += w; sum_x += x as f64 * w; sum_y += y as f64 * w; peak = jmax(peak, w);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let next = at as i64 + dy * width as i64 + dx;
                    if next >= 0 && (next as usize) < mask.len() && mask[next as usize] == 1 {
                        mask[next as usize] = 2;
                        queue[end] = next as i32;
                        end += 1;
                    }
                }
            }
        }
        let (bw, bh) = ((max_x - min_x + 1) as f64, (max_y - min_y + 1) as f64);
        let (long, short) = (bw.max(bh), bw.min(bh));
        if end > 45 || long > 8.0 || long > short * 2.5 || peak < 6.0 { continue; }
        dots.push(Dot { x: sum_x / weight, y: sum_y / weight, contrast: peak, links: [-1; 4] });
        // Texture/noise is not a recoverable printed lattice. Bound later work
        // independently of the number of dark camera-noise components.
        if dots.len() > 5500 { return Ok(Grid::empty(GridKind::Texture, dots.len())); }
    }
    // Bucket the dots by pitch cell; each bucket keeps ascending dot order (ties keep the first).
    let (bucket_columns, bucket_rows) = ((width as f64 / PITCH) as i64 + 2, (height as f64 / PITCH) as i64 + 2);
    let bucket_of = |dot: &Dot| ((dot.x / PITCH).floor() as i64, (dot.y / PITCH).floor() as i64);
    let mut starts = vec![0usize; (bucket_columns * bucket_rows) as usize + 1];
    for dot in &dots { let (bx, by) = bucket_of(dot); starts[(by * bucket_columns + bx) as usize + 1] += 1; }
    for i in 1..starts.len() { starts[i] += starts[i - 1]; }
    let mut fill = starts.clone();
    let mut members = vec![0usize; dots.len()];
    for (index, dot) in dots.iter().enumerate() {
        let (bx, by) = bucket_of(dot);
        let slot = &mut fill[(by * bucket_columns + bx) as usize];
        members[*slot] = index;
        *slot += 1;
    }
    for index in 0..dots.len() {
        let (dot_x, dot_y) = (dots[index].x, dots[index].y);
        let mut best = [f64::INFINITY; 4];
        let mut links = [-1i32; 4];
        let (bx, by) = bucket_of(&dots[index]);
        for y in by - 2..=by + 2 {
            for x in bx - 2..=bx + 2 {
                if x < 0 || y < 0 || x >= bucket_columns || y >= bucket_rows { continue; }
                let cell = (y * bucket_columns + x) as usize;
                for &other in &members[starts[cell]..starts[cell + 1]] {
                    if other == index { continue; }
                    let dx = dots[other].x - dot_x;
                    let dy = dots[other].y - dot_y;
                    let horizontal = dx.abs() > dy.abs();
                    let (major, minor) = if horizontal { (dx.abs(), dy.abs()) } else { (dy.abs(), dx.abs()) };
                    if major < PITCH * 0.65 || major > PITCH * 1.35 || minor > PITCH * 0.32 { continue; }
                    let direction = if horizontal { if dx > 0.0 { 0 } else { 2 } } else if dy > 0.0 { 1 } else { 3 };
                    let error = (major - PITCH).abs() + minor * 1.5;
                    if error < best[direction] { best[direction] = error; links[direction] = other as i32; }
                }
            }
        }
        dots[index].links = links;
    }
    // Reciprocal links and complete elementary cells reject text baselines,
    // crossing ruled lines, random specks, and one-dimensional dot patterns.
    let mut matched = vec![false; dots.len()];
    let mut cells = 0usize;
    for index in 0..dots.len() {
        let (right, down) = (dots[index].links[0], dots[index].links[1]);
        if right < 0 || down < 0 || dots[right as usize].links[2] != index as i32 || dots[down as usize].links[3] != index as i32 { continue; }
        let (right, down) = (right as usize, down as usize);
        let corner = dots[right].links[1];
        if corner < 0 || corner != dots[down].links[0]
            || dots[corner as usize].links[3] != right as i32 || dots[corner as usize].links[2] != down as i32 { continue; }
        let corner = corner as usize;
        let dot = &dots[index];
        let a = (dots[right].x - dot.x, dots[right].y - dot.y);
        let b = (dots[down].x - dot.x, dots[down].y - dot.y);
        let c = (dots[corner].x - dots[down].x, dots[corner].y - dots[down].y);
        let d = (dots[corner].x - dots[right].x, dots[corner].y - dots[right].y);
        if hypot(a.0 - c.0, a.1 - c.1) > PITCH * 0.25 || hypot(b.0 - d.0, b.1 - d.1) > PITCH * 0.25 { continue; }
        for member in [index, right, down, corner] { matched[member] = true; }
        cells += 1;
    }
    let mut zones = [0usize; 9];
    let (mut row_bands, mut column_bands) = (vec![false; height / 15 + 2], vec![false; width / 15 + 2]);
    let mut contrasts = Vec::new();
    let mut matched_count = 0usize;
    for (index, dot) in dots.iter().enumerate() {
        if !matched[index] { continue; }
        matched_count += 1;
        let zone_row = jmin(2.0, (dot.y / height as f64 * 3.0).floor()) as usize;
        let zone_column = jmin(2.0, (dot.x / width as f64 * 3.0).floor()) as usize;
        zones[zone_row * 3 + zone_column] += 1;
        row_bands[(dot.y / PITCH).floor() as usize] = true;
        column_bands[(dot.x / PITCH).floor() as usize] = true;
        contrasts.push(dot.contrast);
    }
    contrasts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let coverage = zones.iter().filter(|&&count| count >= 12).count() as f64 / 9.0;
    let confidence = jmin(1.0, matched_count as f64 / 500.0) * coverage;
    let rows = row_bands.iter().filter(|&&set| set).count();
    let columns = column_bands.iter().filter(|&&set| set).count();
    let valid = cells >= 160 && matched_count >= 400 && coverage == 1.0
        && rows >= if landscape { 21 } else { 32 } && columns >= if landscape { 32 } else { 21 };
    Ok(Grid {
        kind: GridKind::Full, valid, dots: dots.len(), matched: matched_count, cells, rows, columns, coverage, confidence,
        contrast: if contrasts.is_empty() { 0.0 } else { contrasts[(contrasts.len() as f64 * 0.25).floor() as usize] },
    })
}

// ---- Readiness decision -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Readiness { Blur(Sharpness), Grid(Sharpness, Grid) }

pub fn readiness(raster: &Raster, quad: &[f64; 8], frame: &Edges) -> Result<Readiness, ()> {
    let sharpness = inspect(raster, quad, frame);
    // The capture threshold is deliberately stricter than the focus diagnostic: a barely
    // readable lattice is not a sufficiently sharp scan. Sharp outer zones must not conceal a
    // defocused patch on a curved page, so every measured region must meet the acutance floor.
    if !sharpness.valid || sharpness.clarity < 0.42 || sharpness.minimum.map_or(false, |value| value < 0.32) {
        return Ok(Readiness::Blur(sharpness));
    }
    let grid = inspect_grid(raster, quad, frame)?;
    Ok(Readiness::Grid(sharpness, grid))
}

// ---- C ABI ---------------------------------------------------------------------------------------

/// Number of f64 values written to the result block.
pub const RESULT_LEN: usize = 20;

fn reason_code(sharpness: &Sharpness, readiness: Option<&Readiness>) -> f64 {
    match readiness {
        None => if sharpness.clarity >= 0.32 { 0.0 } else { 1.0 },
        Some(Readiness::Blur(_)) => 1.0,
        Some(Readiness::Grid(_, grid)) => match (grid.valid, grid.kind) {
            (true, _) => 2.0,
            (false, GridKind::Clipped) => 3.0,
            (false, GridKind::Texture) => 4.0,
            _ => 5.0,
        },
    }
}

fn write_sharpness(out: &mut [f64], s: &Sharpness) {
    out[0] = s.valid as u8 as f64; out[1] = s.clarity; out[2] = s.border;
    out[3] = s.interior.unwrap_or(f64::NAN); out[4] = s.zones as f64; out[5] = s.edges as f64;
    out[6] = s.minimum.unwrap_or(f64::NAN);
}

fn write_grid(out: &mut [f64], g: &Grid) {
    out[8] = match g.kind { GridKind::Full => 0.0, GridKind::Empty => 1.0, GridKind::Clipped => 2.0, GridKind::Texture => 3.0 };
    out[9] = g.valid as u8 as f64; out[10] = g.dots as f64; out[11] = g.matched as f64; out[12] = g.cells as f64;
    out[13] = g.rows as f64; out[14] = g.columns as f64; out[15] = g.coverage; out[16] = g.confidence; out[17] = g.contrast;
}

/// Rec. 709 luma of an RGBA raster, rounded to f32. Returns 0 on success, -1 on invalid arguments.
#[no_mangle]
pub unsafe extern "C" fn scan_color_luma(rgba: *const u8, gray: *mut f32, pixels: usize) -> i32 {
    if rgba.is_null() || gray.is_null() || pixels == 0 || pixels > 64 * 1024 * 1024 { return -1; }
    let input = std::slice::from_raw_parts(rgba, pixels * 4);
    let output = std::slice::from_raw_parts_mut(gray, pixels);
    for (value, pixel) in output.iter_mut().zip(input.chunks_exact(4)) {
        *value = (pixel[0] as f64 * 0.2126 + pixel[1] as f64 * 0.7152 + pixel[2] as f64 * 0.0722) as f32;
    }
    0
}

/// mode 0: acutance only (result[0..7], result[19]); 1: lattice only (result[8..18]);
/// 2: full readiness decision. Returns 0 on success, -1 invalid arguments, -2 non-finite frame edge.
/// `quad` is 8 f64 (normalised x,y); `offsets` holds `edge_count + 1` point-count prefix sums
/// ending at `points`; `edges` holds `points` x,y pairs.
#[no_mangle]
pub unsafe extern "C" fn scan_color_quality(
    mode: u32, gray: *const f32, width: usize, height: usize, quad: *const f64,
    edges: *const f64, offsets: *const u32, edge_count: usize, points: usize, out: *mut f64,
) -> i32 {
    let area = match width.checked_mul(height) { Some(area) => area, None => return -1 };
    if mode > 2 || gray.is_null() || quad.is_null() || out.is_null() || offsets.is_null() || edges.is_null()
        || width < 2 || height < 2 || width > 16384 || height > 16384 || area > 64 * 1024 * 1024
        || edge_count > 64 || points > 1 << 20 { return -1; }
    let offsets = std::slice::from_raw_parts(offsets, edge_count + 1);
    if offsets[0] != 0 || offsets[edge_count] as usize != points || offsets.windows(2).any(|pair| pair[0] > pair[1]) { return -1; }
    let raster = Raster { gray: std::slice::from_raw_parts(gray, area), width, height };
    let quad: [f64; 8] = std::array::from_fn(|i| *quad.add(i));
    let frame = Edges { coords: std::slice::from_raw_parts(edges, points * 2), offsets };
    let out = std::slice::from_raw_parts_mut(out, RESULT_LEN);
    out.fill(0.0);
    match mode {
        0 => {
            let sharpness = inspect(&raster, &quad, &frame);
            write_sharpness(out, &sharpness);
            out[19] = reason_code(&sharpness, None);
        }
        1 => match inspect_grid(&raster, &quad, &frame) { Ok(grid) => write_grid(out, &grid), Err(()) => return -2 },
        _ => match readiness(&raster, &quad, &frame) {
            Err(()) => return -2,
            Ok(result) => {
                let (sharpness, grid) = match &result { Readiness::Blur(s) => (s, None), Readiness::Grid(s, g) => (s, Some(g)) };
                write_sharpness(out, sharpness);
                out[7] = if grid.is_some() { 2.0 } else { 1.0 };
                if let Some(grid) = grid { write_grid(out, grid); }
                out[19] = reason_code(sharpness, Some(&result));
            }
        },
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUAD: [f64; 8] = [0.08, 0.05, 0.92, 0.05, 0.92, 0.95, 0.08, 0.95];

    fn frame_edges() -> (Vec<f64>, Vec<u32>) {
        let mut coords = Vec::new();
        let mut offsets = vec![0u32];
        for side in 0..4 {
            let (a, b) = ((QUAD[side * 2], QUAD[side * 2 + 1]), (QUAD[(side + 1) % 4 * 2], QUAD[(side + 1) % 4 * 2 + 1]));
            for step in 0..33 { let t = step as f64 / 32.0; coords.extend([a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t]); }
            offsets.push((side as u32 + 1) * 33);
        }
        (coords, offsets)
    }

    /// White page with a 36 x 54 lattice of 3x3 grey dots, a dark page outline and optional blur.
    fn page(width: usize, height: usize, blur: bool) -> Vec<f32> {
        let mut gray = vec![235.0f32; width * height];
        let (x0, y0) = (width as f64 * 0.08, height as f64 * 0.05);
        let (sx, sy) = (width as f64 * 0.84 / 36.0, height as f64 * 0.9 / 54.0);
        for row in 1..54 { for col in 1..36 {
            let (cx, cy) = ((x0 + col as f64 * sx).round() as i64, (y0 + row as f64 * sy).round() as i64);
            for dy in -1..=1 { for dx in -1..=1 { gray[(cy + dy) as usize * width + (cx + dx) as usize] = 120.0; } }
        } }
        for x in x0 as usize..(width as f64 * 0.92) as usize { for y in [y0 as usize, (height as f64 * 0.95) as usize] { gray[y * width + x] = 60.0; } }
        for y in y0 as usize..(height as f64 * 0.95) as usize { for x in [x0 as usize, (width as f64 * 0.92) as usize] { gray[y * width + x] = 60.0; } }
        if blur {
            let copy = gray.clone();
            for y in 2..height - 2 { for x in 2..width - 2 {
                let mut sum = 0.0; for dy in -2i64..=2 { for dx in -2i64..=2 { sum += copy[(y as i64 + dy) as usize * width + (x as i64 + dx) as usize]; } }
                gray[y * width + x] = sum / 25.0;
            } }
        }
        gray
    }

    #[test]
    fn javascript_numeric_helpers() {
        assert_eq!(jround(-2.5), -2.0);
        assert_eq!(jround(2.5), 3.0);
        assert_eq!(jround(0.49999999999999994), 0.0);
        assert!(jmax(1.0, f64::NAN).is_nan() && jmin(f64::NAN, 1.0).is_nan());
        assert_eq!(jsign(-3.0), -1.0);
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, 0.0), 0.0);
        assert_eq!(hypot(f64::NAN, f64::INFINITY), f64::INFINITY);
        assert!(hypot(f64::NAN, 1.0).is_nan());
    }

    #[test]
    fn luma_matches_weighted_sum_rounded_to_f32() {
        let rgba = [255u8, 255, 255, 255, 0, 0, 0, 255, 17, 200, 90, 3];
        let mut gray = [0.0f32; 3];
        assert_eq!(unsafe { scan_color_luma(rgba.as_ptr(), gray.as_mut_ptr(), 3) }, 0);
        assert_eq!(gray[0], (255.0 * 0.2126 + 255.0 * 0.7152 + 255.0 * 0.0722) as f32);
        assert_eq!(gray[1], 0.0);
        assert_eq!(gray[2], (17.0 * 0.2126 + 200.0 * 0.7152 + 90.0 * 0.0722f64) as f32);
        assert_eq!(unsafe { scan_color_luma(std::ptr::null(), gray.as_mut_ptr(), 3) }, -1);
    }

    #[test]
    fn sharp_lattice_page_is_ready_and_blurred_page_is_not() {
        let (coords, offsets) = frame_edges();
        let frame = Edges { coords: &coords, offsets: &offsets };
        let (width, height) = (720usize, 1080usize);
        let sharp = page(width, height, false);
        match readiness(&Raster { gray: &sharp, width, height }, &QUAD, &frame).unwrap() {
            Readiness::Grid(sharpness, grid) => {
                assert!(sharpness.valid && sharpness.clarity >= 0.42, "{sharpness:?}");
                assert!(grid.valid && grid.matched > 400 && grid.coverage == 1.0, "{grid:?}");
            }
            other => panic!("expected a lattice, got {other:?}"),
        }
        let soft = page(width, height, true);
        assert!(matches!(readiness(&Raster { gray: &soft, width, height }, &QUAD, &frame).unwrap(), Readiness::Blur(_)));
    }

    #[test]
    fn flat_raster_has_no_lattice_and_clipped_quad_is_reported() {
        let (coords, offsets) = frame_edges();
        let frame = Edges { coords: &coords, offsets: &offsets };
        let flat = vec![200.0f32; 720 * 1080];
        let raster = Raster { gray: &flat, width: 720, height: 1080 };
        let grid = inspect_grid(&raster, &QUAD, &frame).unwrap();
        assert!(!grid.valid && grid.kind == GridKind::Full && grid.dots == 0 && grid.matched == 0);
        let off_page = [-0.5, 0.0, 0.5, 0.0, 0.5, 1.0, -0.5, 1.0];
        assert_eq!(inspect_grid(&raster, &off_page, &Edges { coords: &[], offsets: &[0] }).unwrap().kind, GridKind::Clipped);
        let mut bad = QUAD;
        bad[3] = f64::NAN;
        assert_eq!(inspect_grid(&raster, &bad, &frame).unwrap().kind, GridKind::Empty);
    }

    #[test]
    fn non_finite_frame_edge_is_an_error_not_a_panic() {
        let (mut coords, offsets) = frame_edges();
        coords[10] = f64::NAN;
        let flat = vec![200.0f32; 300 * 400];
        let raster = Raster { gray: &flat, width: 300, height: 400 };
        assert!(inspect_grid(&raster, &QUAD, &Edges { coords: &coords, offsets: &offsets }).is_err());
        // Acutance skips the point instead.
        let _ = inspect(&raster, &QUAD, &Edges { coords: &coords, offsets: &offsets });
    }

    #[test]
    fn c_abi_rejects_bad_arguments_and_fills_the_result_block() {
        let (coords, offsets) = frame_edges();
        let gray = page(360, 540, false);
        let mut out = [9.0f64; RESULT_LEN];
        let call = |mode: u32, width: usize, points: usize, out: &mut [f64; RESULT_LEN]| unsafe {
            scan_color_quality(mode, gray.as_ptr(), width, 540, QUAD.as_ptr(), coords.as_ptr(), offsets.as_ptr(), 4, points, out.as_mut_ptr())
        };
        assert_eq!(call(3, 360, 132, &mut out), -1);
        assert_eq!(call(2, 1, 132, &mut out), -1);
        assert_eq!(call(2, 360, 131, &mut out), -1);
        assert_eq!(call(0, 360, 132, &mut out), 0);
        assert!(out[0] == 1.0 || out[0] == 0.0);
        assert_eq!(out[7], 0.0);
        assert_eq!(call(2, 360, 132, &mut out), 0);
        assert!(out[7] == 1.0 || out[7] == 2.0);
    }
}
