//! Top-down debug view of the network and the agents, rasterised to an RGB
//! buffer (no dependencies; callers save it as PNG):
//!
//! - grey: street lanes; thin dark grey: turn connectors;
//! - stop lines in the colour of their light (green / amber / red), white
//!   for STOP, yellow for give way;
//! - zebras: white, green while the walk signal is on;
//! - vehicles: blue boxes (red when braking, yellow outline while
//!   changing lane), parked cars dark grey;
//! - pedestrians: orange dots (green crossing, magenta waiting at the
//!   kerb, cyan in an activity, red fleeing).

use forja_math::Vec2;

use crate::geom::left;
use crate::peds::PedState;
use crate::signals::{Aspect, PedSignal};
use crate::sim::TrafficSim;
use crate::spec::Control;

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    origin: Vec2,
    scale: f32,
}

impl Canvas {
    /// A canvas showing the world rectangle `lo..hi` at `px_per_m`.
    pub fn new(lo: Vec2, hi: Vec2, px_per_m: f32) -> Self {
        let width = (((hi.x - lo.x) * px_per_m) as u32).max(16);
        let height = (((hi.y - lo.y) * px_per_m) as u32).max(16);
        let mut rgb = vec![0u8; (width * height * 3) as usize];
        for px in rgb.chunks_mut(3) {
            px.copy_from_slice(&[24, 26, 30]);
        }
        Self { width, height, rgb, origin: lo, scale: px_per_m }
    }

    fn to_px(&self, p: Vec2) -> Vec2 {
        // +z up in the image.
        Vec2::new((p.x - self.origin.x) * self.scale, self.height as f32 - (p.y - self.origin.y) * self.scale)
    }

    fn put(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height {
            let i = ((y as u32 * self.width + x as u32) * 3) as usize;
            self.rgb[i..i + 3].copy_from_slice(&c);
        }
    }

    pub fn disc(&mut self, c: Vec2, r_m: f32, col: [u8; 3]) {
        let p = self.to_px(c);
        let r = (r_m * self.scale).max(1.0);
        let (x0, x1, y0, y1) = ((p.x - r) as i32, (p.x + r) as i32, (p.y - r) as i32, (p.y + r) as i32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(p) <= r {
                    self.put(x, y, col);
                }
            }
        }
    }

    pub fn line(&mut self, a: Vec2, b: Vec2, w_m: f32, col: [u8; 3]) {
        let pa = self.to_px(a);
        let pb = self.to_px(b);
        let w = (w_m * self.scale * 0.5).max(0.5);
        let (lo, hi) = (pa.min(pb) - Vec2::splat(w + 1.0), pa.max(pb) + Vec2::splat(w + 1.0));
        let ab = pb - pa;
        let l2 = ab.length_squared().max(1e-6);
        for y in lo.y as i32..=hi.y as i32 {
            for x in lo.x as i32..=hi.x as i32 {
                let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let t = ((q - pa).dot(ab) / l2).clamp(0.0, 1.0);
                if (pa + ab * t).distance(q) <= w {
                    self.put(x, y, col);
                }
            }
        }
    }

    /// Filled oriented box.
    pub fn obb(&mut self, c: Vec2, fwd: Vec2, half: Vec2, col: [u8; 3]) {
        let l = left(fwd);
        let corners = [c + fwd * half.y + l * half.x, c + fwd * half.y - l * half.x, c - fwd * half.y - l * half.x, c - fwd * half.y + l * half.x].map(|p| self.to_px(p));
        let lo = corners.iter().fold(Vec2::splat(f32::MAX), |a, b| a.min(*b));
        let hi = corners.iter().fold(Vec2::splat(f32::MIN), |a, b| a.max(*b));
        for y in lo.y as i32..=hi.y as i32 {
            for x in lo.x as i32..=hi.x as i32 {
                let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let inside = (0..4).all(|k| {
                    let a = corners[k];
                    let b = corners[(k + 1) % 4];
                    (b - a).perp_dot(q - a) <= 0.0
                }) || (0..4).all(|k| {
                    let a = corners[k];
                    let b = corners[(k + 1) % 4];
                    (b - a).perp_dot(q - a) >= 0.0
                });
                if inside {
                    self.put(x, y, col);
                }
            }
        }
    }
}

/// Renders `sim` inside `lo..hi`.
pub fn render(sim: &TrafficSim, lo: Vec2, hi: Vec2, px_per_m: f32) -> Canvas {
    let mut c = Canvas::new(lo, hi, px_per_m);
    let net = &sim.net;
    // Sidewalks (walk graph).
    for e in &net.ped.edges {
        let (a, b) = (net.ped.nodes[e.a as usize].pos, net.ped.nodes[e.b as usize].pos);
        if matches!(e.kind, crate::network::WalkKind::Sidewalk { .. } | crate::network::WalkKind::Corner) {
            c.line(a, b, (e.half * 2.0).max(1.0), [52, 50, 46]);
        }
    }
    for l in &net.lanes {
        let (col, w) = if l.is_connector() { ([70, 72, 78], 0.25) } else { ([120, 122, 128], 0.5) };
        for w2 in l.path.pts.windows(2) {
            c.line(w2[0], w2[1], w, col);
        }
        if !l.is_connector() {
            // Direction tick near the end.
            let (p, d) = l.path.sample(l.len() * 0.5);
            c.line(p, p - d * 1.5 + left(d) * 0.7, 0.25, [150, 150, 160]);
        }
    }
    for (ci, cw) in net.crosswalks.iter().enumerate() {
        let col = match net.crosswalk_signal(ci as u32, sim.t) {
            Some(PedSignal::Walk) => [60, 230, 90],
            Some(PedSignal::Flash) => [150, 230, 60],
            Some(PedSignal::DontWalk) => [200, 200, 200],
            None => [235, 235, 235],
        };
        let d = (cw.b - cw.a).normalize_or_zero();
        let mut t = 0.3;
        while t < cw.len() {
            c.line(cw.a + d * t, cw.a + d * (t + 0.5), cw.width * 0.8, col);
            t += 1.0;
        }
    }
    for (ai, ap) in net.approaches.iter().enumerate() {
        let col = match ap.control {
            Control::Signal => match net.approach_aspect(ai as u32, sim.t).0 {
                Aspect::Green => [40, 240, 80],
                Aspect::Amber => [255, 180, 0],
                Aspect::Red => [255, 40, 30],
            },
            Control::Stop => [255, 255, 255],
            Control::Yield => [250, 220, 40],
            Control::None => continue,
        };
        for &l in &ap.lanes {
            let lane = &net.lanes[l as usize];
            let (p, d) = lane.path.sample(lane.stop_s);
            let n = left(d) * lane.width * 0.5;
            c.line(p - n, p + n, 0.6, col);
        }
    }
    for p in &sim.parked {
        c.obb(p.pos, p.fwd, Vec2::new(p.width * 0.5, p.len * 0.5), [70, 70, 74]);
    }
    for v in &sim.vehicles {
        let col = if v.braking { [230, 60, 60] } else { [70, 140, 255] };
        c.obb(v.pos, v.fwd, v.half(), col);
        c.disc(v.pos + v.fwd * (v.len * 0.5 - 0.4), 0.35, [240, 240, 255]);
        if v.indicator != 0 {
            c.disc(v.pos - v.fwd * (v.len * 0.5 - 0.3) + left(v.fwd) * (-v.indicator as f32) * v.width * 0.5, 0.4, [255, 220, 0]);
        }
    }
    for e in &sim.externals {
        c.obb(e.pos, e.fwd, e.half, [255, 255, 255]);
    }
    for p in &sim.peds {
        let col = match p.state {
            PedState::Cross { .. } => [60, 255, 120],
            PedState::Wait { .. } => [255, 60, 255],
            PedState::Activity { .. } => [60, 220, 255],
            PedState::Flee { .. } => [255, 30, 30],
            PedState::Walk => [255, 150, 40],
        };
        c.disc(p.pos, 0.45, col);
    }
    c
}
