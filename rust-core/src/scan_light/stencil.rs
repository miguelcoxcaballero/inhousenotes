//! Removal of the photographed stencil from a colour-corrected page
//! (port of `removePhotographedStencil` and `isYellow`). The pixel buffer is edited in
//! place. The algorithm is a resumable state machine so the host keeps its yield and
//! cancellation points: every `step` does one bounded slice of the original loops.
//!
//! Parity notes (the original is JavaScript, results must be byte-identical):
//! * `Math.round` rounds half up (`js_round`), `Uint8ClampedArray` stores round half even.
//! * `Math.hypot` is V8's scaled Kahan sum (`js_hypot`), not `sqrt(x*x+y*y)`.
//! * Typed-array reads outside the buffer give `undefined` (NaN in arithmetic) and a
//!   column outside the image wraps onto the neighbouring row, because pixel offsets
//!   are linear. `Pixels::at` reproduces both.
use std::f64::consts::FRAC_1_SQRT_2;

const COLUMNS: usize = 35;
const ROWS: usize = 51;
const CELLS: usize = COLUMNS * ROWS;

/// `Math.round`: ties go toward +infinity (f64::round goes away from zero).
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// `Math.hypot(a, b)` as implemented by V8 (scaled sum of squares with Kahan compensation).
fn js_hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    if a == f64::INFINITY || b == f64::INFINITY { return f64::INFINITY; }
    if a.is_nan() || b.is_nan() { return f64::NAN; }
    let max = if a > b { a } else { b };
    if max == 0.0 { return 0.0; }
    let (mut sum, mut compensation) = (0.0f64, 0.0f64);
    for value in [a, b] {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// Uint8ClampedArray store: round half to even, clamp.
fn clamp_byte(value: f64) -> u8 { value.round_ties_even().clamp(0.0, 255.0) as u8 }

/// Printed yellow of the stencil (same predicate as `isYellow` in the scanner).
pub fn is_yellow(r: f64, g: f64, b: f64) -> bool {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max < 82.0 || max == min { return false; }
    let saturation = (max - min) / max;
    let hue = if max == r { 60.0 * (((g - b) / (max - min)) % 6.0) }
        else if max == g { 60.0 * (((b - r) / (max - min)) + 2.0) }
        else { 60.0 * (((r - g) / (max - min)) + 4.0) };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    hue >= 39.0 && hue <= 68.0 && saturation >= 0.16 && g / r.max(1.0) >= 0.70
        && b / g.max(1.0) <= 0.84 && (r + g) * 0.5 - b >= 20.0
}

#[derive(Clone, Copy)]
struct Metrics { luminance: f64, saturation: f64 }

/// One lattice dot observed by the geometry stage, as marshalled by the adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observation { pub column: f64, pub row: f64, pub x: f64, pub y: f64, pub neutral: bool }

#[derive(Clone, Copy, Default)]
struct Centre { x: i64, y: i64, dx: f64, dy: f64, neutral: bool }

#[derive(Clone, Copy)]
struct Phase { x: f64, y: f64, matches: u32, contrast: f64 }

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Bands,
    Observed(usize),
    Interpolate(usize),
    PhaseCoarse(i32),
    Blocks(usize),
    Histogram(usize),
    Isolated(usize),
    Done,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub observed_dots: bool,
    pub observations: u32,
    pub photographed: u32,
    pub confirmed: u32,
    pub isolated_removed: u32,
    pub isolated_rejected: u32,
    pub protected_components: u32,
}

pub struct Stencil {
    width: i64, height: i64, px: f64, target: f64,
    observed: Vec<Observation>, geometric_count: usize,
    ring_offsets: [(i64, i64); 8], stroke_offsets: [(i64, i64); 4],
    stage: Stage,
    centres: Vec<Centre>, measured: Vec<Option<Centre>>,
    measured_columns: u64, measured_rows: u64,
    measured_count: u32, confirmed: u32, geometry_measured: u32,
    use_observed: bool, global_dx: f64, global_dy: f64,
    nominal: Phase, best: Phase, grid: (f64, f64),
    glyph_radius: i64, glyph_side: i64, glyph_offsets: Vec<(i64, i64, usize)>,
    histograms: Vec<u16>, counts: Vec<u16>, darkness: Vec<u8>, repeated_peak: f64,
    seen: Vec<u8>, queue: Vec<usize>, search_radius: i64,
    isolated_removed: u32, isolated_rejected: u32, protected_components: u32,
    protect: bool, halo: i64,
}

/// Linear RGBA view with JavaScript typed-array read semantics.
struct Pixels<'a> { data: &'a mut [u8], width: i64 }
impl Pixels<'_> {
    /// Offset of pixel (x, y) when its three colour bytes exist (`undefined` otherwise).
    fn at(&self, x: i64, y: i64) -> Option<usize> {
        let offset = (y * self.width + x) * 4;
        if offset >= 0 && (offset as usize) + 2 < self.data.len() { Some(offset as usize) } else { None }
    }
    fn metrics(&self, x: i64, y: i64) -> Metrics {
        let Some(o) = self.at(x, y) else { return Metrics { luminance: f64::NAN, saturation: 0.0 }; };
        let (r, g, b) = (self.data[o] as f64, self.data[o + 1] as f64, self.data[o + 2] as f64);
        let maximum = r.max(g).max(b);
        let minimum = r.min(g).min(b);
        Metrics { luminance: r * 0.2126 + g * 0.7152 + b * 0.0722,
            saturation: if maximum != 0.0 { (maximum - minimum) / maximum } else { 0.0 } }
    }
}

impl Stencil {
    pub fn new(width: usize, height: usize, px_per_cm: f64, target_paper: f64, observed: Vec<Observation>, geometric_count: usize) -> Result<Stencil, String> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 { return Err("Invalid stencil image size".into()); }
        if !px_per_cm.is_finite() || px_per_cm <= 0.0 || px_per_cm > 4096.0 { return Err("Invalid pixels per cm".into()); }
        if !target_paper.is_finite() { return Err("Invalid target paper".into()); }
        if geometric_count > observed.len() { return Err("Invalid observation split".into()); }
        let px = px_per_cm;
        let ring_radius = 3.0f64.max((0.095 * px).ceil());
        let stroke_radius = (ring_radius + 1.0).max(js_round(px * 0.14));
        let directions = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0),
            (FRAC_1_SQRT_2, FRAC_1_SQRT_2), (-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
            (FRAC_1_SQRT_2, -FRAC_1_SQRT_2), (-FRAC_1_SQRT_2, -FRAC_1_SQRT_2)];
        let ring_offsets = directions.map(|(ux, uy)| (js_round(ux * ring_radius) as i64, js_round(uy * ring_radius) as i64));
        let strokes = [(1.0, 0.0), (0.0, 1.0), (FRAC_1_SQRT_2, FRAC_1_SQRT_2), (FRAC_1_SQRT_2, -FRAC_1_SQRT_2)];
        let stroke_offsets = strokes.map(|(ux, uy)| (js_round(ux * stroke_radius) as i64, js_round(uy * stroke_radius) as i64));
        let glyph_radius = 4.0f64.max((0.22 * px).ceil()) as i64;
        let glyph_side = glyph_radius * 2 + 1;
        let glyph_pixels = (glyph_side * glyph_side) as usize;
        let mut glyph_offsets = Vec::new();
        for dy in -glyph_radius..=glyph_radius {
            for dx in -glyph_radius..=glyph_radius {
                if js_hypot(dx as f64, dy as f64) <= glyph_radius as f64 {
                    glyph_offsets.push((dx, dy, ((dy + glyph_radius) * glyph_side + dx + glyph_radius) as usize));
                }
            }
        }
        let search_radius = ((glyph_radius - 2) as f64).min(2.0f64.max((px * 0.12).ceil())) as i64;
        let empty = Phase { x: 0.0, y: 0.0, matches: 0, contrast: 0.0 };
        Ok(Stencil {
            width: width as i64, height: height as i64, px, target: target_paper, observed, geometric_count,
            ring_offsets, stroke_offsets, stage: Stage::Bands,
            centres: vec![Centre::default(); CELLS], measured: vec![None; CELLS],
            measured_columns: 0, measured_rows: 0, measured_count: 0, confirmed: 0, geometry_measured: 0,
            use_observed: false, global_dx: 0.0, global_dy: 0.0, nominal: empty, best: empty, grid: (0.0, 0.0),
            glyph_radius, glyph_side, glyph_offsets,
            histograms: vec![0; glyph_pixels * 256], counts: vec![0; glyph_pixels], darkness: vec![0; glyph_pixels],
            repeated_peak: 0.0, seen: vec![0; glyph_pixels], queue: vec![0; glyph_pixels], search_radius,
            isolated_removed: 0, isolated_rejected: 0, protected_components: 0, protect: false, halo: 0,
        })
    }

    pub fn stats(&self) -> Stats {
        Stats {
            observed_dots: self.use_observed && self.geometry_measured >= 600,
            observations: self.geometry_measured,
            photographed: self.measured_count - self.geometry_measured,
            confirmed: self.confirmed,
            isolated_removed: self.isolated_removed,
            isolated_rejected: self.isolated_rejected,
            protected_components: self.protected_components,
        }
    }

    pub fn finished(&self) -> bool { self.stage == Stage::Done }

    /// Runs one slice of work. Returns true while more slices remain.
    pub fn step(&mut self, data: &mut [u8]) -> Result<bool, String> {
        if data.len() as i64 != self.width * self.height * 4 { return Err("Stencil buffer size changed".into()); }
        let mut pixels = Pixels { data, width: self.width };
        match self.stage {
            Stage::Bands => {
                self.paint_frame(&mut pixels);
                if self.observed.len() >= 600 && self.observed.len() <= 6000 { self.stage = Stage::Observed(0); } else { self.after_observed(&pixels); }
            }
            Stage::Observed(next) => self.observe(&pixels, next),
            Stage::Interpolate(column) => {
                self.interpolate_column(column);
                self.stage = if column + 1 < COLUMNS { Stage::Interpolate(column + 1) } else { Stage::Histogram(0) };
            }
            Stage::PhaseCoarse(y_step) => self.phase_coarse(&pixels, y_step),
            Stage::Blocks(index) => {
                self.block(&pixels, index);
                self.stage = if index + 1 < 30 { Stage::Blocks(index + 1) } else { Stage::Histogram(0) };
            }
            Stage::Histogram(column) => {
                self.histogram_column(&pixels, column);
                if column + 1 < COLUMNS { self.stage = Stage::Histogram(column + 1); } else { self.glyph_darkness(); self.stage = Stage::Isolated(0); }
            }
            Stage::Isolated(column) => {
                self.isolated_column(&mut pixels, column);
                self.stage = if column + 1 < COLUMNS { Stage::Isolated(column + 1) } else { Stage::Done };
            }
            Stage::Done => {}
        }
        Ok(self.stage != Stage::Done)
    }

    // ---- frame, footer boxes, reference dots -------------------------------------------------

    fn set_paper(&self, pixels: &mut Pixels, x: i64, y: i64) {
        if x < 0 || x >= self.width || y < 0 || y >= self.height { return; }
        let o = ((y * self.width + x) * 4) as usize;
        let v = clamp_byte(self.target);
        pixels.data[o] = v; pixels.data[o + 1] = v; pixels.data[o + 2] = v;
    }

    fn remove_printed_ink(&self, pixels: &mut Pixels, x: i64, y: i64, force: bool) {
        if force { self.set_paper(pixels, x, y); return; }
        let o = ((y * self.width + x) * 4) as usize;
        let (r, g, b) = (pixels.data[o] as f64, pixels.data[o + 1] as f64, pixels.data[o + 2] as f64);
        let m = pixels.metrics(x, y);
        if is_yellow(r, g, b) || (r >= g * 0.9 && g >= b * 1.08 && r >= b * 1.18)
            || (m.saturation <= 0.19 && m.luminance >= 72.0 && m.luminance < 246.0) {
            self.set_paper(pixels, x, y);
        }
    }

    fn vertical_band(&self, pixels: &mut Pixels, x_cm: f64, y1_cm: f64, y2_cm: f64, radius_cm: f64, force: bool) {
        let center = js_round(x_cm * self.px) as i64;
        let radius = 1.0f64.max((radius_cm * self.px).ceil()) as i64;
        let y1 = 0.0f64.max((y1_cm * self.px).floor()) as i64;
        let y2 = ((self.height - 1) as f64).min((y2_cm * self.px).ceil()) as i64;
        for y in y1..=y2 {
            for x in 0.max(center - radius)..=(self.width - 1).min(center + radius) { self.remove_printed_ink(pixels, x, y, force); }
        }
    }

    fn horizontal_band(&self, pixels: &mut Pixels, y_cm: f64, x1_cm: f64, x2_cm: f64, radius_cm: f64, force: bool) {
        let center = js_round(y_cm * self.px) as i64;
        let radius = 1.0f64.max((radius_cm * self.px).ceil()) as i64;
        let x1 = 0.0f64.max((x1_cm * self.px).floor()) as i64;
        let x2 = ((self.width - 1) as f64).min((x2_cm * self.px).ceil()) as i64;
        for y in 0.max(center - radius)..=(self.height - 1).min(center + radius) {
            for x in x1..=x2 { self.remove_printed_ink(pixels, x, y, force); }
        }
    }

    fn paint_frame(&self, pixels: &mut Pixels) {
        self.vertical_band(pixels, 1.5, 1.25, 28.6, 0.13, true);
        self.vertical_band(pixels, 19.5, 1.25, 28.6, 0.13, true);
        self.horizontal_band(pixels, 1.43, 1.35, 19.65, 0.13, true);
        self.horizontal_band(pixels, 28.43, 1.35, 19.65, 0.13, true);
        for y in [27.43, 27.93] {
            self.horizontal_band(pixels, y, 1.82, 10.18, 0.18, false);
            self.horizontal_band(pixels, y, 10.82, 19.18, 0.18, false);
        }
        let mut x = 2.0f64;
        while x <= 10.0001 { self.vertical_band(pixels, x, 27.24, 28.12, 0.13, false); x += 0.5; }
        let mut x = 11.0f64;
        while x <= 19.0001 { self.vertical_band(pixels, x, 27.24, 28.12, 0.13, false); x += 0.5; }
        for x_cm in [10.125, 10.375, 10.625, 10.875] {
            let cx = x_cm * self.px;
            let cy = 27.68 * self.px;
            let radius = 2.0f64.max((0.19 * self.px).ceil());
            let y_start = 0.0f64.max((cy - radius).floor()) as i64;
            let y_end = ((self.height - 1) as f64).min((cy + radius).ceil()) as i64;
            let x_start = 0.0f64.max((cx - radius).floor()) as i64;
            let x_end = ((self.width - 1) as f64).min((cx + radius).ceil()) as i64;
            for y in y_start..=y_end {
                for x in x_start..=x_end {
                    if js_hypot(x as f64 - cx, y as f64 - cy) <= radius { self.set_paper(pixels, x, y); }
                }
            }
        }
    }

    // ---- dot detection ----------------------------------------------------------------------

    /// Contrast of a printed spot at (x, y): lighter paper on `minimum` of the 8 ring directions.
    fn dot_response(&self, pixels: &Pixels, x: i64, y: i64, minimum: usize) -> Option<f64> {
        let center = pixels.metrics(x, y);
        if center.saturation > 0.38 || center.luminance < 105.0 || center.luminance > 238.0 { return None; }
        let mut ring_sum = 0.0f64;
        let mut supports = 0usize;
        for (dx, dy) in self.ring_offsets {
            let ring = pixels.metrics(x + dx, y + dy);
            if ring.luminance >= center.luminance + 2.0 && ring.saturation <= 0.48 { supports += 1; }
            ring_sum += ring.luminance;
        }
        if supports < minimum { return None; }
        let contrast = ring_sum / 8.0 - center.luminance;
        if contrast >= 5.0 { Some(contrast) } else { None }
    }

    fn score_phase(&self, pixels: &Pixels, offset_x_cm: f64, offset_y_cm: f64) -> Phase {
        let (mut matches, mut contrast) = (0u32, 0.0f64);
        let mut column = 0usize;
        while column < 35 {
            let x = js_round((2.0 + column as f64 * 0.5 + offset_x_cm) * self.px) as i64;
            let mut row = 0usize;
            while row < 51 {
                let y = js_round((1.93 + row as f64 * 0.5 + offset_y_cm) * self.px) as i64;
                if let Some(c) = self.dot_response(pixels, x, y, 6) { matches += 1; contrast += c.min(30.0); }
                row += 2;
            }
            column += 2;
        }
        Phase { x: offset_x_cm, y: offset_y_cm, matches, contrast }
    }

    fn observe(&mut self, pixels: &Pixels, mut index: usize) {
        let px = self.px;
        while index < self.observed.len() {
            let observation_index = index;
            let point = self.observed[observation_index];
            index += 1;
            let (column, row) = (point.column, point.row);
            if column.fract() != 0.0 || !column.is_finite() || row.fract() != 0.0 || !row.is_finite()
                || column < 0.0 || column >= 35.0 || row < 0.0 || row >= 51.0
                || !point.x.is_finite() || !point.y.is_finite() { continue; }
            let (column, row) = (column as usize, row as usize);
            let slot = column * 51 + row;
            if self.measured[slot].is_some() { continue; }
            let nominal_x = (2.0 + column as f64 * 0.5) * px;
            let nominal_y = (1.93 + row as f64 * 0.5) * px;
            if (point.x - nominal_x).abs() > px * 0.24 || (point.y - nominal_y).abs() > px * 0.24 { continue; }
            if observation_index >= self.geometric_count
                && ((point.x - nominal_x).abs() > px * 0.045 || (point.y - nominal_y).abs() > px * 0.045) { continue; }
            let cx = js_round(point.x - 0.5) as i64;
            let cy = js_round(point.y - 0.5) as i64;
            // Only the existence of a confirming spot matters; the search order does not.
            let mut best = false;
            'search: for dy in -2..=2 {
                for dx in -2..=2 {
                    if self.dot_response(pixels, cx + dx, cy + dy, 6).is_some() { best = true; break 'search; }
                }
            }
            if best { self.confirmed += 1; }
            self.measured[slot] = Some(Centre { x: cx, y: cy, dx: cx as f64 - nominal_x, dy: cy as f64 - nominal_y, neutral: point.neutral });
            self.measured_count += 1;
            if observation_index < self.geometric_count { self.geometry_measured += 1; }
            self.measured_columns |= 1u64 << column; self.measured_rows |= 1u64 << row;
            if self.measured_count % 128 == 0 { self.stage = Stage::Observed(index); return; }
        }
        self.after_observed(pixels);
    }

    fn after_observed(&mut self, pixels: &Pixels) {
        self.use_observed = self.measured_count >= 600
            && self.measured_columns.count_ones() >= 25 && self.measured_rows.count_ones() >= 36
            && self.confirmed as f64 >= 180.0f64.min(self.measured_count as f64 * 0.18);
        if self.use_observed {
            let median = |values: Vec<f64>| { let mut v = values; v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)); v[v.len() / 2] };
            let measured: Vec<&Centre> = self.measured.iter().flatten().collect();
            self.global_dx = median(measured.iter().map(|c| c.dx).collect());
            self.global_dy = median(measured.iter().map(|c| c.dy).collect());
            self.stage = Stage::Interpolate(0);
        } else {
            self.nominal = self.score_phase(pixels, 0.0, 0.0);
            self.best = Phase { x: 0.0, y: 0.0, ..self.nominal };
            self.stage = Stage::PhaseCoarse(-10);
        }
    }

    fn interpolate_column(&mut self, column: usize) {
        for row in 0..ROWS {
            let index = column * 51 + row;
            if let Some(centre) = self.measured[index] { self.centres[index] = centre; continue; }
            let (mut weight, mut sum_x, mut sum_y) = (0.0f64, 0.0f64, 0.0f64);
            for dc in -4i64..=4 {
                let c = column as i64 + dc;
                if c < 0 || c >= 35 { continue; }
                for dr in -4i64..=4 {
                    let r = row as i64 + dr;
                    if r < 0 || r >= 51 { continue; }
                    let Some(nearby) = self.measured[(c * 51 + r) as usize] else { continue; };
                    let amount = 1.0 / (0.5 + (dc * dc + dr * dr) as f64);
                    weight += amount; sum_x += nearby.dx * amount; sum_y += nearby.dy * amount;
                }
            }
            self.centres[index] = Centre {
                x: js_round((2.0 + column as f64 * 0.5) * self.px + if weight != 0.0 { sum_x / weight } else { self.global_dx }) as i64,
                y: js_round((1.93 + row as f64 * 0.5) * self.px + if weight != 0.0 { sum_y / weight } else { self.global_dy }) as i64,
                ..Centre::default()
            };
        }
    }

    fn phase_coarse(&mut self, pixels: &Pixels, y_step: i32) {
        for x_step in -10..=10 {
            let (x, y) = (x_step as f64 * 0.02, y_step as f64 * 0.02);
            let score = self.score_phase(pixels, x, y);
            if score.matches > self.best.matches || (score.matches == self.best.matches && score.contrast > self.best.contrast) {
                self.best = score;
            }
        }
        if y_step < 10 { self.stage = Stage::PhaseCoarse(y_step + 1); return; }
        self.grid = if self.best.matches >= self.nominal.matches + 5 { (self.best.x, self.best.y) } else { (0.0, 0.0) };
        self.stage = Stage::Blocks(0);
    }

    fn block(&mut self, pixels: &Pixels, index: usize) {
        let block_column = index / 6 * 7;
        let block_row = index % 6 * 9;
        let end_column = 35.min(block_column + 7);
        let end_row = 51.min(block_row + 9);
        let (grid_x, grid_y) = self.grid;
        let mut best = Phase { x: grid_x, y: grid_y, matches: 0, contrast: 0.0 };
        for y_step in -5..=5 {
            for x_step in -5..=5 {
                let x_phase = grid_x + x_step as f64 * 0.04;
                let y_phase = grid_y + y_step as f64 * 0.04;
                let (mut matches, mut contrast) = (0u32, 0.0f64);
                for column in block_column..end_column {
                    for row in block_row..end_row {
                        let x = js_round((2.0 + column as f64 * 0.5 + x_phase) * self.px) as i64;
                        let y = js_round((1.93 + row as f64 * 0.5 + y_phase) * self.px) as i64;
                        if let Some(c) = self.dot_response(pixels, x, y, 6) { matches += 1; contrast += c.min(30.0); }
                    }
                }
                if matches > best.matches || (matches == best.matches && contrast > best.contrast) {
                    best = Phase { x: x_phase, y: y_phase, matches, contrast };
                }
            }
        }
        let reliable = best.matches as f64 >= 8.0f64.max(((end_column - block_column) * (end_row - block_row)) as f64 * 0.22);
        let phase = if reliable { best } else { Phase { x: grid_x, y: grid_y, matches: 0, contrast: 0.0 } };
        for column in block_column..end_column {
            for row in block_row..end_row {
                let mut cx = js_round((2.0 + column as f64 * 0.5 + phase.x) * self.px) as i64;
                let mut cy = js_round((1.93 + row as f64 * 0.5 + phase.y) * self.px) as i64;
                if reliable {
                    let mut local: Option<(i64, i64, f64)> = None;
                    let search = 2.0f64.max(js_round(self.px * 0.04)) as i64;
                    for dy in -search..=search {
                        for dx in -search..=search {
                            if let Some(c) = self.dot_response(pixels, cx + dx, cy + dy, 6) {
                                if local.map_or(true, |l| c > l.2) { local = Some((cx + dx, cy + dy, c)); }
                            }
                        }
                    }
                    if let Some((x, y, _)) = local { cx = x; cy = y; }
                }
                self.centres[column * 51 + row] = Centre { x: cx, y: cy, ..Centre::default() };
            }
        }
    }

    // ---- learned glyph ----------------------------------------------------------------------

    fn histogram_column(&mut self, pixels: &Pixels, column: usize) {
        for row in 0..ROWS {
            let c = self.centres[column * 51 + row];
            for &(dx, dy, glyph_index) in &self.glyph_offsets {
                let m = pixels.metrics(c.x + dx, c.y + dy);
                if m.saturation > 0.48 { continue; }
                // An unreadable pixel (NaN) is counted but has no histogram bin, as with a typed array.
                if !m.luminance.is_nan() {
                    let luminance = js_round(m.luminance).clamp(0.0, 255.0);
                    self.histograms[glyph_index * 256 + luminance as usize] += 1;
                }
                self.counts[glyph_index] += 1;
            }
        }
    }

    fn glyph_darkness(&mut self) {
        for glyph_index in 0..self.counts.len() {
            let count = self.counts[glyph_index];
            if count == 0 { continue; }
            let target = (count as f64 * 0.12).ceil();
            let mut seen = 0.0f64;
            let mut median = self.target;
            for luminance in 0..256usize {
                seen += self.histograms[glyph_index * 256 + luminance] as f64;
                if seen >= target { median = luminance as f64; break; }
            }
            self.repeated_peak = self.repeated_peak.max(self.target - median);
            // Uint8Array store (truncation), values are within 0..=112.
            self.darkness[glyph_index] = 0.0f64.max(112.0f64.min(self.target - median)) as u8;
        }
    }

    // ---- isolated glyph removal and learned-glyph subtraction ---------------------------------

    fn darkness_at(&self, pixels: &Pixels, cx: i64, cy: i64, dx: i64, dy: i64, neutral: bool) -> f64 {
        let Some(o) = pixels.at(cx + dx, cy + dy) else { return 0.0; };
        let (r, g, b) = (pixels.data[o] as f64, pixels.data[o + 1] as f64, pixels.data[o + 2] as f64);
        let darkness = self.target - (r * 0.2126 + g * 0.7152 + b * 0.0722);
        let max = r.max(g).max(b);
        let chroma = max - r.min(g).min(b);
        if chroma <= 5.0f64.max(darkness * 0.3) || (neutral && chroma <= max * 0.5) { darkness } else { 0.0 }
    }

    fn remove_isolated(&mut self, pixels: &mut Pixels, cx: i64, cy: i64, neutral: bool) -> bool {
        self.protect = false; self.halo = 0;
        if self.repeated_peak < 5.0 { return false; }
        let (gr, side) = (self.glyph_radius, self.glyph_side);
        let sr = self.search_radius;
        let (mut peak, mut peak_x, mut peak_y) = (0.0f64, 0i64, 0i64);
        for dy in -sr..=sr {
            for dx in -sr..=sr {
                if dx * dx + dy * dy > sr * sr { continue; }
                let value = self.darkness_at(pixels, cx, cy, dx, dy, neutral);
                if value > peak { peak = value; peak_x = dx; peak_y = dy; }
            }
        }
        if peak < 5.0 { return false; }
        let uncommon = peak > 18.0f64.max(self.repeated_peak * 1.4);
        let threshold = 3.0f64.max(peak * 0.12);
        let start = ((peak_y + gr) * side + peak_x + gr) as usize;
        self.seen.fill(0); self.seen[start] = 2; self.queue[0] = start;
        let (mut head, mut tail) = (0usize, 1usize);
        let (mut mass, mut sum_x, mut sum_y, mut sum_xx, mut sum_yy, mut sum_xy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (peak_x, peak_x, peak_y, peak_y);
        let mut touches_edge = false;
        while head < tail {
            let index = self.queue[head]; head += 1;
            let dx = index as i64 % side - gr;
            let dy = index as i64 / side - gr;
            let weight = 0.0f64.max(self.darkness_at(pixels, cx, cy, dx, dy, neutral) - threshold);
            let (fx, fy) = (dx as f64, dy as f64);
            mass += weight; sum_x += fx * weight; sum_y += fy * weight;
            sum_xx += fx * fx * weight; sum_yy += fy * fy * weight; sum_xy += fx * fy * weight;
            min_x = min_x.min(dx); max_x = max_x.max(dx); min_y = min_y.min(dy); max_y = max_y.max(dy);
            if dx.abs() >= gr - 1 || dy.abs() >= gr - 1 { touches_edge = true; continue; }
            for oy in -1i64..=1 {
                for ox in -1i64..=1 {
                    let next = index as i64 + oy * side + ox;
                    if next < 0 || next as usize >= self.seen.len() { continue; }
                    let next = next as usize;
                    if self.seen[next] != 0 { continue; }
                    self.seen[next] = 1;
                    if self.darkness_at(pixels, cx, cy, dx + ox, dy + oy, neutral) > threshold {
                        self.seen[next] = 2; self.queue[tail] = next; tail += 1;
                    }
                }
            }
        }
        if mass == 0.0 || tail < 2 { self.isolated_rejected += 1; return false; }
        if touches_edge {
            self.protect = true; self.halo = 1; self.protected_components += 1; self.isolated_rejected += 1; return false;
        }
        let mx = sum_x / mass;
        let my = sum_y / mass;
        let vx = sum_xx / mass - mx * mx;
        let vy = sum_yy / mass - my * my;
        let covariance = sum_xy / mass - mx * my;
        let separation = js_hypot(vx - vy, 2.0 * covariance);
        let narrow = 0.1f64.max((vx + vy - separation) / 2.0);
        let wide = (vx + vy + separation) / 2.0;
        let extent = (max_x - min_x).max(max_y - min_y) as f64;
        let mut occupies_center = false;
        'center: for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                if self.seen[((dy + gr) * side + dx + gr) as usize] == 2 { occupies_center = true; break 'center; }
            }
        }
        let off_center = !occupies_center || js_hypot(mx, my) > 1.0f64.max(self.px * 0.035);
        if wide / narrow > 3.2 || extent > gr as f64 * 1.35 || off_center {
            self.protect = off_center || extent > self.px * 0.12;
            self.halo = if extent > self.px * 0.12 { 1 } else { 0 };
            if self.protect { self.protected_components += 1; }
            self.isolated_rejected += 1; return false;
        }
        if uncommon {
            let ink_threshold = (self.repeated_peak * 1.2).max(peak * 0.6);
            for i in 0..tail {
                let index = self.queue[i];
                let dx = index as i64 % side - gr;
                let dy = index as i64 / side - gr;
                if self.darkness_at(pixels, cx, cy, dx, dy, neutral) < ink_threshold { self.seen[index] = 1; }
            }
            self.protect = true; self.protected_components += 1; self.isolated_rejected += 1; return false;
        }
        let halo = 1.5f64.max(self.px * 0.025);
        let rx = (max_x - min_x) as f64 / 2.0 + halo;
        let ry = (max_y - min_y) as f64 / 2.0 + halo;
        let center_x = (min_x + max_x) as f64 / 2.0;
        let center_y = (min_y + max_y) as f64 / 2.0;
        let y_end = (center_y + ry).ceil() as i64;
        let x_end = (center_x + rx).ceil() as i64;
        let mut dy = (center_y - ry).floor() as i64;
        while dy <= y_end {
            let mut dx = (center_x - rx).floor() as i64;
            while dx <= x_end {
                let ex = (dx as f64 - center_x) / rx;
                let ey = (dy as f64 - center_y) / ry;
                if ex * ex + ey * ey <= 1.0 {
                    let darkness = self.darkness_at(pixels, cx, cy, dx, dy, neutral);
                    let belongs = dx.abs() <= gr && dy.abs() <= gr && self.seen[((dy + gr) * side + dx + gr) as usize] == 2;
                    if darkness > 0.0 && darkness <= peak + 1.0 && (belongs || darkness <= threshold) { self.set_paper(pixels, cx + dx, cy + dy); }
                }
                dx += 1;
            }
            dy += 1;
        }
        self.isolated_removed += 1;
        true
    }

    fn isolated_column(&mut self, pixels: &mut Pixels, column: usize) {
        let (gr, side) = (self.glyph_radius, self.glyph_side);
        for row in 0..ROWS {
            let c = self.centres[column * 51 + row];
            if self.remove_isolated(pixels, c.x, c.y, c.neutral) { continue; }
            for g in 0..self.glyph_offsets.len() {
                let (dx, dy, glyph_index) = self.glyph_offsets[g];
                if self.protect {
                    let mut belongs = false;
                    'halo: for oy in -self.halo..=self.halo {
                        for ox in -self.halo..=self.halo {
                            if (dx + ox).abs() > gr || (dy + oy).abs() > gr { continue; }
                            if self.seen[((dy + oy + gr) * side + dx + ox + gr) as usize] == 2 { belongs = true; break 'halo; }
                        }
                    }
                    if belongs { continue; }
                }
                let lift = self.darkness[glyph_index];
                if lift < 4 { continue; }
                let (x, y) = (c.x + dx, c.y + dy);
                let m = pixels.metrics(x, y);
                if m.saturation > 0.42 || m.luminance < 70.0 { continue; }
                let mut stroke = false;
                for (sx, sy) in self.stroke_offsets {
                    let before = pixels.metrics(x - sx, y - sy);
                    let after = pixels.metrics(x + sx, y + sy);
                    if before.luminance < 245.0 && after.luminance < 245.0
                        && before.luminance <= m.luminance + 28.0 && after.luminance <= m.luminance + 28.0 { stroke = true; break; }
                }
                if stroke { continue; }
                // Reads and writes outside the buffer are ignored by a typed array.
                let Some(o) = pixels.at(x, y) else { continue; };
                for channel in 0..3 {
                    pixels.data[o + channel] = clamp_byte(self.target.min(pixels.data[o + channel] as f64 + lift as f64));
                }
            }
        }
    }
}

// ---- WASM interface ------------------------------------------------------------------------------

/// Exported for differential tests; the scanner calls `is_yellow` from Rust only.
#[no_mangle]
pub extern "C" fn scan_is_yellow(r: u32, g: u32, b: u32) -> i32 { if r < 256 && g < 256 && b < 256 && is_yellow(r as f64, g as f64, b as f64) { 1 } else { 0 } }

/// Context handle owned by the host adapter. `data` stays owned (and freed) by the host.
pub struct Handle { stencil: Stencil, data: *mut u8, len: usize }

/// `observations` is `count` records of five f64: column, row, x, y, neutral flag (0/1).
#[no_mangle]
pub unsafe extern "C" fn scan_stencil_new(data: *mut u8, len: usize, width: usize, height: usize, px_per_cm: f64, target_paper: f64,
    observations: *const f64, count: usize, geometric_count: usize) -> *mut Handle {
    if data.is_null() || width == 0 || height == 0 || width > 16384 || height > 16384 || len != width * height * 4
        || count > 1_000_000 || (observations.is_null() && count > 0) || geometric_count > count { return std::ptr::null_mut(); }
    let records = if count == 0 { &[][..] } else { std::slice::from_raw_parts(observations, count * 5) };
    let observed = records.chunks_exact(5).map(|r| Observation { column: r[0], row: r[1], x: r[2], y: r[3], neutral: r[4] != 0.0 }).collect();
    match Stencil::new(width, height, px_per_cm, target_paper, observed, geometric_count) {
        Ok(stencil) => Box::into_raw(Box::new(Handle { stencil, data, len })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// 1 = more work remains, 0 = finished, -1 = invalid.
#[no_mangle]
pub unsafe extern "C" fn scan_stencil_step(handle: *mut Handle) -> i32 {
    let Some(handle) = handle.as_mut() else { return -1; };
    let data = std::slice::from_raw_parts_mut(handle.data, handle.len);
    match handle.stencil.step(data) { Ok(true) => 1, Ok(false) => 0, Err(_) => -1 }
}

/// Writes seven f64: observed-dots flag, observations, photographed, confirmed, removed, rejected, protected.
#[no_mangle]
pub unsafe extern "C" fn scan_stencil_result(handle: *mut Handle, out: *mut f64) -> i32 {
    let Some(handle) = handle.as_ref() else { return -1; };
    if out.is_null() || !handle.stencil.finished() { return -1; }
    let s = handle.stencil.stats();
    let values = [if s.observed_dots { 1.0 } else { 0.0 }, s.observations as f64, s.photographed as f64, s.confirmed as f64,
        s.isolated_removed as f64, s.isolated_rejected as f64, s.protected_components as f64];
    std::ptr::copy_nonoverlapping(values.as_ptr(), out, 7);
    0
}

#[no_mangle]
pub unsafe extern "C" fn scan_stencil_drop(handle: *mut Handle) {
    if !handle.is_null() { drop(Box::from_raw(handle)); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(stencil: &mut Stencil, data: &mut [u8]) { while stencil.step(data).unwrap() {} }

    #[test] fn rounding_matches_javascript() {
        assert_eq!(js_round(2.5), 3.0); assert_eq!(js_round(-2.5), -2.0); assert_eq!(js_round(-0.4), 0.0);
        assert_eq!(js_round(0.49999999999999994), 0.0); assert_eq!(js_round(1.5), 2.0);
        assert_eq!(clamp_byte(2.5), 2); assert_eq!(clamp_byte(3.5), 4); assert_eq!(clamp_byte(-3.0), 0); assert_eq!(clamp_byte(300.0), 255);
    }
    #[test] fn hypot_is_exact_for_pythagorean_triples_and_scaled() {
        assert_eq!(js_hypot(3.0, 4.0), 5.0); assert_eq!(js_hypot(0.0, 0.0), 0.0);
        assert_eq!(js_hypot(-5.0, 0.0), 5.0); assert!(js_hypot(f64::NAN, 1.0).is_nan());
        assert_eq!(js_hypot(f64::NAN, f64::INFINITY), f64::INFINITY);
    }
    #[test] fn yellow_predicate() {
        assert!(is_yellow(230.0, 200.0, 90.0)); assert!(!is_yellow(120.0, 120.0, 120.0));
        assert!(!is_yellow(40.0, 40.0, 10.0)); assert!(!is_yellow(240.0, 100.0, 90.0)); assert!(!is_yellow(30.0, 200.0, 220.0));
    }
    #[test] fn rejects_bad_parameters() {
        assert!(Stencil::new(0, 10, 85.0, 245.0, vec![], 0).is_err());
        assert!(Stencil::new(10, 10, f64::NAN, 245.0, vec![], 0).is_err());
        assert!(Stencil::new(10, 10, 85.0, f64::INFINITY, vec![], 0).is_err());
        assert!(Stencil::new(10, 10, 85.0, 245.0, vec![], 1).is_err());
        let mut stencil = Stencil::new(10, 10, 85.0, 245.0, vec![], 0).unwrap();
        assert!(stencil.step(&mut vec![0u8; 12]).is_err());
    }
    #[test] fn frame_rail_is_cleared_and_footer_ink_is_kept() {
        let (w, h, px) = (400usize, 560usize, 20.0f64);
        let mut data = vec![240u8; w * h * 4];
        // Yellow rail along x = 1.5 cm, and a dark handwriting pixel in the footer band.
        let rail = (1.5 * px) as usize;
        for y in 40..500 { let o = (y * w + rail) * 4; data[o] = 230; data[o + 1] = 190; data[o + 2] = 40; }
        let (wx, wy) = ((2.0 * px) as usize, (27.43 * px).round() as usize);
        let o = (wy * w + wx) * 4; data[o] = 20; data[o + 1] = 20; data[o + 2] = 40;
        let mut stencil = Stencil::new(w, h, px, 240.0, vec![], 0).unwrap();
        run(&mut stencil, &mut data);
        assert!(stencil.finished());
        assert_eq!(&data[(100 * w + rail) * 4..(100 * w + rail) * 4 + 3], &[240, 240, 240]);
        assert_eq!(&data[o..o + 3], &[20, 20, 40]);
        assert!(!stencil.stats().observed_dots);
    }
    #[test] fn tiny_and_odd_images_never_panic() {
        for (w, h, px) in [(1usize, 1usize, 1.0f64), (3, 700, 50.0), (700, 3, 90.0), (17, 19, 0.5), (64, 64, 85.0)] {
            let mut data: Vec<u8> = (0..w * h * 4).map(|i| ((i * 37 + i / 7) % 256) as u8).collect();
            let mut stencil = Stencil::new(w, h, px, 250.0, vec![], 0).unwrap();
            run(&mut stencil, &mut data);
            assert!(stencil.finished());
        }
    }
    #[test] fn observations_outside_range_or_lattice_are_ignored() {
        let observed: Vec<Observation> = (0..700).map(|i| Observation { column: (i % 35) as f64, row: (i / 35) as f64, x: -50.0, y: 9.0, neutral: false }).collect();
        let mut stencil = Stencil::new(200, 300, 20.0, 240.0, observed, 700).unwrap();
        let mut data = vec![200u8; 200 * 300 * 4];
        run(&mut stencil, &mut data);
        let stats = stencil.stats();
        assert_eq!((stats.observations, stats.photographed, stats.confirmed), (0, 0, 0));
    }
    #[test] fn banded_steps_match_a_single_pass_and_are_deterministic() {
        let (w, h, px) = (300usize, 420usize, 14.0f64);
        let base: Vec<u8> = (0..w * h * 4).map(|i| if i % 4 == 3 { 255 } else { (150 + (i * 13) % 90) as u8 }).collect();
        let (mut a, mut b) = (base.clone(), base);
        let mut sa = Stencil::new(w, h, px, 245.0, vec![], 0).unwrap();
        let mut sb = Stencil::new(w, h, px, 245.0, vec![], 0).unwrap();
        run(&mut sa, &mut a);
        let mut steps = 0; while sb.step(&mut b).unwrap() { steps += 1; }
        assert!(steps > 30); assert_eq!(a, b);
        assert!(!sb.step(&mut b).unwrap());
    }
}
