//! 2D sections for sweeps and lofts, and 3D paths (polylines, curves,
//! Bezier splines, arcs).

use forja_math::{Vec2, Vec3};

use super::poly;
use super::spec::{PathSpec, Section};

pub const SECTION_SHAPES: &[&str] = &["ellipse", "circle", "rect", "square", "superellipse", "polygon", "lens", "teardrop", "i_beam"];

pub fn check_section_shape(s: &str) -> Result<(), String> {
    let k = s.trim().to_ascii_lowercase();
    if SECTION_SHAPES.contains(&k.as_str()) {
        Ok(())
    } else {
        Err(format!("unknown section shape `{s}`{}; expected one of {}", super::parse::did_you_mean(&k, SECTION_SHAPES), SECTION_SHAPES.join(", ")))
    }
}

/// Points of a section (counter-clockwise in (side, up) when closed) and
/// whether it is closed. `res` = points for curved shapes.
pub fn section_points(s: &Section, res: usize) -> (Vec<Vec2>, bool) {
    let res = res.max(4);
    let (w, h) = (s.size[0] * 0.5, s.size[1] * 0.5);
    let mut pts: Vec<Vec2> = if let Some(p) = &s.points {
        p.iter().map(|q| Vec2::new(q[0], q[1])).collect()
    } else {
        let shape = s.shape.as_deref().unwrap_or("ellipse").to_ascii_lowercase();
        match shape.as_str() {
            "rect" | "square" => vec![Vec2::new(-w, -h), Vec2::new(w, -h), Vec2::new(w, h), Vec2::new(-w, h)],
            "polygon" => {
                let n = s.sides.max(3) as usize;
                (0..n).map(|k| Vec2::from_angle(k as f32 / n as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2) * Vec2::new(w, h)).collect()
            }
            "superellipse" => {
                let e = s.exponent.max(0.2);
                (0..res)
                    .map(|k| {
                        let a = k as f32 / res as f32 * std::f32::consts::TAU;
                        let (c, sn) = (a.cos(), a.sin());
                        Vec2::new(w * c.signum() * c.abs().powf(2.0 / e), h * sn.signum() * sn.abs().powf(2.0 / e))
                    })
                    .collect()
            }
            "lens" => (0..res)
                .map(|k| {
                    let a = k as f32 / res as f32 * std::f32::consts::TAU;
                    Vec2::new(w * a.cos(), h * a.sin() * a.sin().abs().powf(0.6))
                })
                .collect(),
            "teardrop" => (0..res)
                .map(|k| {
                    let a = k as f32 / res as f32 * std::f32::consts::TAU;
                    let r = 0.5 + 0.5 * (a * 0.5).sin().abs();
                    Vec2::new(w * a.cos() * r, h * a.sin())
                })
                .collect(),
            "i_beam" => {
                let (fl, wb) = (h * 0.25, w * 0.2);
                vec![
                    Vec2::new(-w, -h),
                    Vec2::new(w, -h),
                    Vec2::new(w, -h + fl),
                    Vec2::new(wb, -h + fl),
                    Vec2::new(wb, h - fl),
                    Vec2::new(w, h - fl),
                    Vec2::new(w, h),
                    Vec2::new(-w, h),
                    Vec2::new(-w, h - fl),
                    Vec2::new(-wb, h - fl),
                    Vec2::new(-wb, -h + fl),
                    Vec2::new(-w, -h + fl),
                ]
            }
            _ => (0..res).map(|k| Vec2::from_angle(k as f32 / res as f32 * std::f32::consts::TAU) * Vec2::new(w, h)).collect(),
        }
    };
    if s.rotate != 0.0 {
        let r = Vec2::from_angle(s.rotate.to_radians());
        pts = pts.into_iter().map(|q| r.rotate(q)).collect();
    }
    let closed = !s.open;
    if closed && poly::area(&pts) < 0.0 {
        pts.reverse();
    }
    (pts, closed)
}

/// Resamples a closed (or open) polyline to `n` points evenly by arc
/// length, starting at its first point.
pub fn resample(pts: &[Vec2], n: usize, closed: bool) -> Vec<Vec2> {
    let m = pts.len();
    if m < 2 || n < 2 {
        return pts.to_vec();
    }
    let segs = if closed { m } else { m - 1 };
    let mut acc = vec![0.0f32; segs + 1];
    for i in 0..segs {
        acc[i + 1] = acc[i] + (pts[(i + 1) % m] - pts[i]).length();
    }
    let total = acc[segs].max(1e-9);
    let count = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(n);
    let mut j = 0;
    for k in 0..n {
        let d = total * k as f32 / count as f32;
        while j + 1 < segs && acc[j + 1] < d {
            j += 1;
        }
        let t = ((d - acc[j]) / (acc[j + 1] - acc[j]).max(1e-9)).clamp(0.0, 1.0);
        out.push(pts[j].lerp(pts[(j + 1) % m], t));
    }
    out
}

/// Centripetal-free uniform Catmull-Rom point.
pub fn catmull(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    (p1 * 2.0 + (p2 - p0) * t + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2 + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * t3) * 0.5
}

/// Smooth curve through points (Catmull-Rom), `per` samples per span.
pub fn curve_through(pts: &[Vec3], per: usize, closed: bool) -> Vec<Vec3> {
    let m = pts.len();
    if m < 3 {
        return pts.to_vec();
    }
    let get = |i: isize| -> Vec3 {
        if closed {
            pts[i.rem_euclid(m as isize) as usize]
        } else if i < 0 {
            pts[0] * 2.0 - pts[1]
        } else if i as usize >= m {
            pts[m - 1] * 2.0 - pts[m - 2]
        } else {
            pts[i as usize]
        }
    };
    let spans = if closed { m } else { m - 1 };
    let mut out = Vec::with_capacity(spans * per + 1);
    for s in 0..spans as isize {
        for k in 0..per {
            out.push(catmull(get(s - 1), get(s), get(s + 1), get(s + 2), k as f32 / per as f32));
        }
    }
    if !closed {
        out.push(pts[m - 1]);
    }
    out
}

/// Samples a path spec. Returns points and whether the path is closed.
pub fn path_points(p: &PathSpec, detail: f32) -> Result<(Vec<Vec3>, bool), String> {
    let v3 = |a: &[f32; 3]| Vec3::from_array(*a);
    if let Some(pts) = &p.points {
        if pts.len() < 2 {
            return Err("path.points needs at least 2 points".into());
        }
        return Ok((pts.iter().map(v3).collect(), p.closed));
    }
    if let Some(pts) = &p.curve {
        if pts.len() < 2 {
            return Err("path.curve needs at least 2 points".into());
        }
        let per = p.samples.map(|s| s as usize).unwrap_or((12.0 * detail) as usize).max(2);
        return Ok((curve_through(&pts.iter().map(v3).collect::<Vec<_>>(), per, p.closed), p.closed));
    }
    if let Some(pts) = &p.bezier {
        if pts.len() < 4 || (pts.len() - 1) % 3 != 0 {
            return Err(format!("path.bezier needs 3k + 1 control points, got {}", pts.len()));
        }
        let per = p.samples.map(|s| s as usize).unwrap_or((16.0 * detail) as usize).max(2);
        let c: Vec<Vec3> = pts.iter().map(v3).collect();
        let mut out = Vec::new();
        for s in (0..c.len() - 1).step_by(3) {
            for k in 0..per {
                let t = k as f32 / per as f32;
                let u = 1.0 - t;
                out.push(c[s] * (u * u * u) + c[s + 1] * (3.0 * u * u * t) + c[s + 2] * (3.0 * u * t * t) + c[s + 3] * (t * t * t));
            }
        }
        out.push(*c.last().unwrap());
        return Ok((out, p.closed));
    }
    if let (Some(c), Some(r)) = (p.center, p.radius) {
        let n = p.normal.map(|n| Vec3::from_array(n).normalize_or(Vec3::Y)).unwrap_or(Vec3::Y);
        let a = p.axis.map(Vec3::from_array).unwrap_or(if n.y.abs() > 0.9 { Vec3::X } else { Vec3::Y.cross(n).normalize_or(Vec3::X) });
        let a = (a - n * a.dot(n)).normalize_or(n.any_orthonormal_vector());
        let mut b = n.cross(a);
        // Vertical arcs rise from their first axis (angles go "up").
        if b.y < -0.1 {
            b = -b;
        }
        let [a0, a1] = p.angles.unwrap_or([0.0, 360.0]);
        let full = (a1 - a0).abs() >= 359.99;
        let seg = p.samples.map(|s| s as usize).unwrap_or(((a1 - a0).abs() / 6.0 * detail).ceil() as usize).max(3);
        let count = if full { seg } else { seg + 1 };
        let pts = (0..count)
            .map(|k| {
                let t = (a0 + (a1 - a0) * k as f32 / seg as f32).to_radians();
                Vec3::from_array(c) + (a * t.cos() + b * t.sin()) * r
            })
            .collect();
        return Ok((pts, full || p.closed));
    }
    Err("a path needs `points`, `curve`, `bezier`, or `center` + `radius` (arc)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_and_resampling() {
        let s = Section { points: None, shape: Some("superellipse".into()), size: [2.0, 1.0], exponent: 4.0, sides: 0, rotate: 0.0, open: false };
        let (p, closed) = section_points(&s, 32);
        assert!(closed && p.len() == 32 && poly::area(&p) > 0.0);
        let r = resample(&p, 50, true);
        assert_eq!(r.len(), 50);
        let arc = PathSpec { center: Some([0.0, 0.0, 0.0]), radius: Some(5.0), angles: Some([0.0, 180.0]), normal: Some([0.0, 0.0, 1.0]), ..Default::default() };
        let (pts, closed) = path_points(&arc, 1.0).unwrap();
        assert!(!closed);
        assert!((pts[0] - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4);
        assert!((pts.last().unwrap().x + 5.0).abs() < 1e-3);
        assert!(pts.iter().all(|p| p.y >= -1e-4), "vertical arc goes up");
    }
}
