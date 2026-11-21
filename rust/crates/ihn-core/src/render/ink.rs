//! Pen ink as a filled outline whose width follows the pen pressure (W7).
//!
//! The centreline is the same centripetal Catmull–Rom spline as before (`pen::segment`). Each spline segment is
//! sampled, every sample gets a radius from the smoothed pressure, and the samples become one closed "ribbon"
//! (left edge forward, round end cap, right edge back, round start cap) filled with the nonzero rule. Every part
//! of a ribbon winds the same way, so where a stroke crosses itself the overlap adds up and never leaves a hole.
//! At very sharp turns the ribbon is split into two that share the turning sample, so the outer side gets a round
//! join instead of a spike or a notch.
//!
//! Strokes whose pressure does not vary (mouse, touch, old data with a constant value) keep exactly the chosen
//! width, so they look like the round-capped strokes drawn before.

use super::cmd::Cmds;
use crate::jsmath;
use std::f64::consts::PI;

type P = (f64, f64);

/// Width multiplier at pressure 0 (lightest touch).
/// 6.2.1: a very slight variation, as the owner asked: ×0.85 at the lightest touch, about ×1 at normal writing
/// pressure (p ≈ 0.35), ×1.15 at full pressure.
pub const MIN_FACTOR: f64 = 0.85;
/// Added to `MIN_FACTOR` at full pressure: the multiplier is `MIN_FACTOR + SPAN · p^GAMMA`.
pub const SPAN: f64 = 0.3;
/// < 1 lifts the light end of the range a little.
pub const GAMMA: f64 = 0.6;
/// Weight of each new pressure sample in the running average (lower = smoother width changes; 6.2.1: 0.12, so the
/// width changes gradually instead of in steps).
pub const PRESSURE_SMOOTHING: f64 = 0.12;
/// A stroke counts as pressure-sensitive when its pressure varies by more than this.
pub const PRESSURE_RANGE: f32 = 0.04;
/// Centreline sample spacing in page px (smaller = smoother outline, more path points).
pub const STEP: f64 = 1.5;
/// Outline samples closer than this (page px) to the line between their kept neighbours, in position and in
/// radius, are dropped (fewer vertices to fill, no visible change).
pub const SIMPLIFY: f64 = 0.15;
/// Turns sharper than about 100° split the ribbon: the cosine of the angle between the incoming and outgoing
/// direction below this value (cos 100° ≈ −0.17).
pub const SPLIT_COS: f64 = -0.17;

#[cfg(test)]
thread_local! {
    /// Tests that compare with call traces recorded from 5.18.0 draw pens the 5.18.0 way, so those traces still
    /// check everything else bit for bit; the outline has its own tests.
    pub static LEGACY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether pens are drawn as pressure outlines (always, outside the 5.18.0 trace tests).
pub fn enabled() -> bool {
    #[cfg(test)]
    if LEGACY.with(|l| l.get()) {
        return false;
    }
    true
}

/// Whether `p` carries real pen pressure (it changes along the stroke).
pub fn has_pressure(p: Option<&[f32]>) -> bool {
    let Some(p) = p else { return false };
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for &v in p.iter().filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    hi - lo > PRESSURE_RANGE
}

/// The largest width multiplier (full pressure).
pub fn max_factor() -> f64 { MIN_FACTOR + SPAN }

/// Width multiplier for a (smoothed) pressure in 0..1.
pub fn factor(p: f64) -> f64 { MIN_FACTOR + SPAN * jsmath::pow(p.clamp(0.0, 1.0), GAMMA) }

/// Radius of every point: `width / 2`, scaled by the smoothed pressure when the stroke has real pressure.
/// The smoothing is causal (each value depends only on earlier samples), so a live stroke and the same stroke
/// once committed get the same radii.
pub fn radii(width: f64, n: usize, p: Option<&[f32]>) -> Vec<f64> {
    let half = width.max(0.0) * 0.5;
    if !has_pressure(p) {
        return vec![half; n];
    }
    let p = p.unwrap_or(&[]);
    let mut out = Vec::with_capacity(n);
    let mut avg = f64::NAN;
    for i in 0..n {
        let v = p.get(i).copied().filter(|v| v.is_finite()).map(f64::from).unwrap_or(if avg.is_nan() { 0.5 } else { avg });
        avg = if avg.is_nan() { v } else { avg + PRESSURE_SMOOTHING * (v - avg) };
        out.push(half * factor(avg));
    }
    out
}

fn pt(xy: &[f64], i: usize) -> P { (xy[2 * i], xy[2 * i + 1]) }

/// Control points of the spline segment p1→p2 (the same maths as `pen::segment`); `None` for a straight line.
fn controls(p0: P, p1: P, p2: P, p3: P) -> Option<(P, P)> {
    let (ax, ay) = (p2.0 - p1.0, p2.1 - p1.1);
    let length2 = ax * ax + ay * ay;
    if length2 < 4.0 {
        return None;
    }
    let (bx, by, cx, cy) = (p1.0 - p0.0, p1.1 - p0.1, p3.0 - p2.0, p3.1 - p2.1);
    let d1 = (bx * bx + by * by).sqrt().sqrt();
    let d2 = length2.sqrt().sqrt();
    let d3 = (cx * cx + cy * cy).sqrt().sqrt();
    let (mut c1, mut c2) = (p1, p2);
    if d1 >= 1e-3 {
        let (a, b) = (d1 * d1, d2 * d2);
        let (k, n) = (2.0 * a + 3.0 * d1 * d2 + b, 3.0 * d1 * (d1 + d2));
        c1 = ((a * p2.0 - b * p0.0 + k * p1.0) / n, (a * p2.1 - b * p0.1 + k * p1.1) / n);
    }
    if d3 >= 1e-3 {
        let (a, b) = (d3 * d3, d2 * d2);
        let (k, n) = (2.0 * a + 3.0 * d3 * d2 + b, 3.0 * d3 * (d3 + d2));
        c2 = ((a * p1.0 - b * p3.0 + k * p2.0) / n, (a * p1.1 - b * p3.1 + k * p2.1) / n);
    }
    Some((c1, c2))
}

/// A point of the sampled centreline with its radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// Samples of the spline segments `from..to` (segment i joins points i and i + 1), including both ends.
pub fn samples(xy: &[f64], radii: &[f64], from: usize, to: usize) -> Vec<Sample> {
    let n = xy.len() / 2;
    let mut out = Vec::new();
    if n == 0 || from >= to || to >= n {
        return out;
    }
    let end = n - 1;
    let first = pt(xy, from);
    out.push(Sample { x: first.0, y: first.1, r: radii[from] });
    for i in from..to {
        let (p0, p1, p2, p3) = (pt(xy, i.saturating_sub(1)), pt(xy, i), pt(xy, i + 1), pt(xy, (i + 2).min(end)));
        let (r1, r2) = (radii[i], radii[i + 1]);
        let chord = ((p2.0 - p1.0) * (p2.0 - p1.0) + (p2.1 - p1.1) * (p2.1 - p1.1)).sqrt();
        let steps = ((chord / STEP).ceil() as usize).clamp(1, 32);
        let curve = controls(p0, p1, p2, p3);
        for k in 1..=steps {
            let t = k as f64 / steps as f64;
            let (x, y) = match curve {
                None => (p1.0 + (p2.0 - p1.0) * t, p1.1 + (p2.1 - p1.1) * t),
                Some((c1, c2)) => {
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    (a * p1.0 + b * c1.0 + c * c2.0 + d * p2.0, a * p1.1 + b * c1.1 + c * c2.1 + d * p2.1)
                }
            };
            out.push(Sample { x, y, r: r1 + (r2 - r1) * t });
        }
    }
    out
}

/// Ramer–Douglas–Peucker over (x, y, r): keeps the samples that move the outline by more than `SIMPLIFY`.
fn simplify(s: &[Sample]) -> Vec<Sample> {
    if s.len() < 3 {
        return s.to_vec();
    }
    let mut keep = vec![false; s.len()];
    keep[0] = true;
    keep[s.len() - 1] = true;
    let mut stack = vec![(0usize, s.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (p, q) = (s[a], s[b]);
        let (dx, dy, dr) = (q.x - p.x, q.y - p.y, q.r - p.r);
        let len2 = dx * dx + dy * dy;
        let (mut worst, mut at) = (0.0, a);
        for (i, m) in s.iter().enumerate().take(b).skip(a + 1) {
            // Parameter of the projection on the chord; distance in position plus the radius error there.
            let t = if len2 > 1e-12 { (((m.x - p.x) * dx + (m.y - p.y) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (ex, ey) = (p.x + dx * t - m.x, p.y + dy * t - m.y);
            let e = (ex * ex + ey * ey).sqrt() + (p.r + dr * t - m.r).abs();
            if e > worst {
                (worst, at) = (e, i);
            }
        }
        if worst > SIMPLIFY {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    s.iter().zip(keep).filter_map(|(v, k)| k.then_some(*v)).collect()
}

/// Drops samples that sit on the previous one (they have no direction).
fn distinct(samples: &[Sample]) -> Vec<Sample> {
    let mut out: Vec<Sample> = Vec::with_capacity(samples.len());
    for &s in samples {
        if !(s.x.is_finite() && s.y.is_finite() && s.r.is_finite()) {
            continue;
        }
        match out.last_mut() {
            Some(last) if (s.x - last.x).abs() < 1e-6 && (s.y - last.y).abs() < 1e-6 => last.r = last.r.max(s.r),
            _ => out.push(s),
        }
    }
    out
}

fn unit(dx: f64, dy: f64) -> P {
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-12 { (1.0, 0.0) } else { (dx / len, dy / len) }
}

/// One closed ribbon through `s` (≥ 2 distinct samples). With y pointing down, `arc` runs clockwise on screen.
fn ribbon(c: &mut Cmds, s: &[Sample]) {
    let n = s.len();
    // Unit tangent at each sample: the chord through its neighbours.
    let tangent = |i: usize| {
        let (a, b) = (s[i.saturating_sub(1)], s[(i + 1).min(n - 1)]);
        unit(b.x - a.x, b.y - a.y)
    };
    // Normal on the "left" side: the tangent turned a quarter anticlockwise on screen.
    let normal = |t: P| (t.1, -t.0);
    let mut right = Vec::with_capacity(n);
    for (i, p) in s.iter().enumerate() {
        let m = normal(tangent(i));
        let (lx, ly) = (p.x + m.0 * p.r, p.y + m.1 * p.r);
        if i == 0 {
            c.move_to(lx, ly);
        } else {
            c.line_to(lx, ly);
        }
        right.push((p.x - m.0 * p.r, p.y - m.1 * p.r));
    }
    // End cap: from the left edge round the front to the right edge.
    let (last, m) = (s[n - 1], normal(tangent(n - 1)));
    let a = jsmath::atan2(m.1, m.0);
    c.arc(last.x, last.y, last.r, a, a + PI);
    for &(x, y) in right.iter().rev() {
        c.line_to(x, y);
    }
    // Start cap: from the right edge round the back to the left edge.
    let (first, m) = (s[0], normal(tangent(0)));
    let a = jsmath::atan2(m.1, m.0);
    c.arc(first.x, first.y, first.r, a + PI, a + 2.0 * PI);
    c.close_path();
}

/// A filled circle (a stroke reduced to one place).
fn disc(c: &mut Cmds, s: Sample) {
    c.move_to(s.x + s.r, s.y);
    c.arc(s.x, s.y, s.r, 0.0, 2.0 * PI);
    c.close_path();
}

/// Appends the outline of `samples` to the current path: one ribbon per run between sharp turns.
pub fn outline(c: &mut Cmds, samples: &[Sample]) {
    let s = simplify(&distinct(samples));
    match s.len() {
        0 => return,
        1 => return disc(c, s[0]),
        _ => {}
    }
    let mut start = 0;
    for i in 1..s.len() - 1 {
        let (a, b) = (unit(s[i].x - s[i - 1].x, s[i].y - s[i - 1].y), unit(s[i + 1].x - s[i].x, s[i + 1].y - s[i].y));
        if a.0 * b.0 + a.1 * b.1 < SPLIT_COS {
            ribbon(c, &s[start..=i]);
            start = i;
        }
    }
    ribbon(c, &s[start..]);
}

/// The whole stroke's outline (≥ 2 points).
pub fn stroke(c: &mut Cmds, xy: &[f64], width: f64, p: Option<&[f32]>) {
    let n = xy.len() / 2;
    if n < 2 {
        return;
    }
    let r = radii(width, n, p);
    outline(c, &samples(xy, &r, 0, n - 1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_pressure_keeps_the_chosen_width() {
        assert!(!has_pressure(None));
        assert!(!has_pressure(Some(&[0.5, 0.5, 0.5])));
        assert!(!has_pressure(Some(&[1.0, 1.0])));
        assert_eq!(radii(3.0, 3, Some(&[1.0, 1.0, 1.0])), vec![1.5; 3]);
        assert_eq!(radii(3.0, 2, None), vec![1.5; 2]);
    }

    #[test]
    fn pressure_scales_and_smooths_the_width() {
        let p = [0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let r = radii(2.0, p.len(), Some(&p));
        assert!((r[0] - MIN_FACTOR).abs() < 1e-9, "lightest touch");
        // The running average climbs towards full pressure without jumping there at once.
        assert!(r.windows(2).all(|w| w[1] > w[0]));
        // Running average after k samples of full pressure: 1 − (1 − PRESSURE_SMOOTHING)^k.
        for k in [1usize, 7] {
            let avg = 1.0 - (0..k).fold(1.0, |acc, _| acc * (1.0 - PRESSURE_SMOOTHING));
            assert!((r[k] - factor(avg)).abs() < 1e-9, "{k}: {} vs {}", r[k], factor(avg));
        }
        assert!(r[7] < factor(1.0));
        // A live prefix gets the same radii as the whole stroke.
        assert_eq!(radii(2.0, 4, Some(&p[..4])), r[..4]);
        assert!((factor(0.0) - MIN_FACTOR).abs() < 1e-12 && (factor(1.0) - (MIN_FACTOR + SPAN)).abs() < 1e-12);
    }

    #[test]
    fn samples_follow_the_spline_and_interpolate_radii() {
        let xy = [0.0, 0.0, 10.0, 0.0, 20.0, 0.0];
        let s = samples(&xy, &[1.0, 2.0, 3.0], 0, 2);
        assert_eq!(s.first(), Some(&Sample { x: 0.0, y: 0.0, r: 1.0 }));
        assert_eq!(s.last(), Some(&Sample { x: 20.0, y: 0.0, r: 3.0 }));
        assert!(s.iter().all(|p| p.y.abs() < 1e-9));
        assert!(s.windows(2).all(|w| w[1].x > w[0].x && (w[1].x - w[0].x) <= 2.0 * STEP));
        // A live part (segment 1 only) starts where the previous part ended.
        let tail = samples(&xy, &[1.0, 2.0, 3.0], 1, 2);
        assert_eq!(tail[0], Sample { x: 10.0, y: 0.0, r: 2.0 });
        assert_eq!(tail.last(), s.last());
    }

    use super::super::cmd::op;

    /// Op codes of path-building commands (`move`, `line`, `arc`, `close`), decoded with their argument counts.
    fn codes(bytes: &[u8]) -> Vec<u8> {
        let (mut i, mut out) = (0, Vec::new());
        while i < bytes.len() {
            let code = bytes[i];
            i += 1 + 8 * match code {
                op::MOVE_TO | op::LINE_TO => 2,
                op::ARC => 5,
                op::CLOSE_PATH => 0,
                _ => panic!("unexpected op {code}"),
            };
            out.push(code);
        }
        out
    }
    fn count(bytes: &[u8], code: u8) -> usize { codes(bytes).iter().filter(|&&c| c == code).count() }

    #[test]
    fn a_straight_stroke_is_one_closed_ribbon() {
        let mut c = Cmds::new();
        stroke(&mut c, &[0.0, 0.0, 10.0, 0.0], 2.0, None);
        let b = c.take();
        assert_eq!((count(&b, op::MOVE_TO), count(&b, op::ARC), count(&b, op::CLOSE_PATH)), (1, 2, 1));
    }

    #[test]
    fn a_hairpin_splits_into_two_ribbons() {
        let r = [1.0; 3];
        let mut c = Cmds::new();
        outline(&mut c, &samples(&[0.0, 0.0, 10.0, 0.0, 0.0, 0.5], &r, 0, 2));
        let mut straight = Cmds::new();
        outline(&mut straight, &samples(&[0.0, 0.0, 10.0, 0.0, 20.0, 0.0], &r, 0, 2));
        // Each ribbon has two caps.
        assert_eq!(count(&c.take(), op::ARC), 4);
        assert_eq!(count(&straight.take(), op::ARC), 2);
    }

    #[test]
    fn simplify_drops_samples_that_do_not_move_the_outline() {
        // A straight line of constant width keeps its two ends; a bend and a width change are kept.
        let line: Vec<Sample> = (0..50).map(|i| Sample { x: i as f64, y: 0.0, r: 1.0 }).collect();
        assert_eq!(simplify(&line).len(), 2);
        let mut bent = line.clone();
        bent[25].y = 1.0;
        let kept = simplify(&bent);
        assert!(kept.len() <= 5 && kept.iter().any(|s| s.y == 1.0), "{kept:?}");
        let mut wider = line;
        wider[10].r = 1.5;
        assert!(simplify(&wider).iter().any(|s| s.r == 1.5));
    }

    #[test]
    fn repeated_points_make_a_dot() {
        let mut c = Cmds::new();
        stroke(&mut c, &[5.0, 5.0, 5.0, 5.0], 2.0, None);
        assert_eq!(count(&c.take(), op::ARC), 1);
    }
}
