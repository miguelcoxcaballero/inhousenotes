//! The radar's projection: ground metres to screen points and back.
//!
//! A view looks down on `centre`, drawn at `anchor`, with the world
//! direction `forward` pointing up the screen. `tilt` adds the perspective
//! of GTA V's radar in vehicles: the ground ahead recedes (shrinks towards
//! the top), the ground behind comes closer.

use egui::{Pos2, Rect, pos2};
use forja_math::Vec2;

/// World north on the ground plane (`-Z`).
pub const NORTH: Vec2 = Vec2::new(0.0, -1.0);

#[derive(Clone, Copy, Debug)]
pub struct RadarView {
    /// The radar's rectangle on screen.
    pub rect: Rect,
    /// Where `centre` is drawn.
    pub anchor: Pos2,
    /// Ground point (`x`, world `z`) under the anchor.
    pub centre: Vec2,
    /// Unit ground direction that points up the screen.
    pub forward: Vec2,
    /// Screen points per metre at the anchor.
    pub scale: f32,
    /// Perspective strength: 0 flat; the scale at a screen height `h`
    /// above the anchor is divided by `1 + tilt * h_flat / rect.height()`.
    pub tilt: f32,
}

impl RadarView {
    /// A flat view with north up, `centre` in the middle of `rect`.
    pub fn north_up(rect: Rect, centre: Vec2, scale: f32) -> Self {
        Self { rect, anchor: rect.center(), centre, forward: NORTH, scale, tilt: 0.0 }
    }

    /// Ground direction that points to the right of the screen.
    pub fn right(&self) -> Vec2 {
        Vec2::new(-self.forward.y, self.forward.x)
    }

    /// Perspective divisor at `v` metres ahead of the centre.
    fn w(&self, v: f32) -> f32 {
        (1.0 + self.tilt * v * self.scale / self.rect.height().max(1.0)).max(0.05)
    }

    /// Screen position of a ground point.
    pub fn project(&self, p: Vec2) -> Pos2 {
        let d = p - self.centre;
        let (u, v) = (d.dot(self.right()), d.dot(self.forward));
        let w = self.w(v);
        pos2(self.anchor.x + u * self.scale / w, self.anchor.y - v * self.scale / w)
    }

    /// Ground point under a screen position.
    pub fn unproject(&self, s: Pos2) -> Vec2 {
        let (dx, dy) = (s.x - self.anchor.x, self.anchor.y - s.y);
        let h = self.rect.height().max(1.0);
        // dy = vs / (1 + tilt * vs / h), solved for vs = v * scale; past
        // the horizon the point is clamped far ahead.
        let den = (1.0 - self.tilt * dy / h).max(0.02);
        let vs = dy / den;
        let v = vs / self.scale;
        let u = dx * self.w(v) / self.scale;
        self.centre + self.right() * u + self.forward * v
    }

    /// Screen direction (unit, y down) of a ground direction at the centre.
    pub fn screen_dir(&self, dir: Vec2) -> egui::Vec2 {
        let v = egui::vec2(dir.dot(self.right()), -dir.dot(self.forward));
        if v.length_sq() > 0.0 { v.normalized() } else { egui::vec2(0.0, -1.0) }
    }

    /// Ground bounding box `(min, max)` of what `rect` shows.
    pub fn ground_bounds(&self) -> (Vec2, Vec2) {
        let r = self.rect;
        [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].iter().map(|c| self.unproject(*c)).fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)))
    }

    /// Metres per screen point at the anchor.
    pub fn metres_per_point(&self) -> f32 {
        1.0 / self.scale
    }
}

/// `p` kept inside `rect`: a point outside slides to the border along the
/// line from `from` (inside the rectangle). Returns the point and whether
/// it was moved.
pub fn clamp_to_rect(rect: Rect, from: Pos2, p: Pos2) -> (Pos2, bool) {
    if rect.contains(p) {
        return (p, false);
    }
    let d = p - from;
    let mut t = 1.0f32;
    if d.x > 0.0 {
        t = t.min((rect.right() - from.x) / d.x);
    } else if d.x < 0.0 {
        t = t.min((rect.left() - from.x) / d.x);
    }
    if d.y > 0.0 {
        t = t.min((rect.bottom() - from.y) / d.y);
    } else if d.y < 0.0 {
        t = t.min((rect.top() - from.y) / d.y);
    }
    (from + d * t.max(0.0), true)
}

/// Where the ray from `from` along `dir` leaves `rect`.
pub fn border_point(rect: Rect, from: Pos2, dir: egui::Vec2) -> Pos2 {
    let far = (rect.width() + rect.height()) * 4.0;
    clamp_to_rect(rect, from, from + dir * far).0
}

/// Cuts the corners of a textured mesh so it fits the rounded rectangle
/// `rect` with corner `radius`. Triangles that do not reach a corner are
/// kept as they are: the straight sides are left to the painter's clip
/// rectangle.
pub fn round_corners(mesh: &mut egui::Mesh, rect: Rect, radius: f32) {
    if radius <= 0.5 {
        return;
    }
    // Per corner: its arc centre and the outward normals of the tangent
    // lines that approximate the arc.
    const STEPS: usize = 5;
    let corners = [(rect.left_top() + egui::vec2(radius, radius), std::f32::consts::PI), (rect.right_top() + egui::vec2(-radius, radius), 1.5 * std::f32::consts::PI), (rect.right_bottom() - egui::vec2(radius, radius), 0.0), (rect.left_bottom() + egui::vec2(radius, -radius), 0.5 * std::f32::consts::PI)];
    let indices = std::mem::take(&mut mesh.indices);
    let mut poly: Vec<egui::epaint::Vertex> = Vec::with_capacity(12);
    let mut next: Vec<egui::epaint::Vertex> = Vec::with_capacity(12);
    for t in indices.chunks_exact(3) {
        let v = [mesh.vertices[t[0] as usize], mesh.vertices[t[1] as usize], mesh.vertices[t[2] as usize]];
        let lo = v[0].pos.min(v[1].pos).min(v[2].pos);
        let hi = v[0].pos.max(v[1].pos).max(v[2].pos);
        // Corner squares this triangle's box touches.
        let mut touched = [false; 4];
        let mut any = false;
        for (k, (c, _)) in corners.iter().enumerate() {
            let (sx, sy) = (if k == 0 || k == 3 { lo.x < c.x } else { hi.x > c.x }, if k < 2 { lo.y < c.y } else { hi.y > c.y });
            touched[k] = sx && sy;
            any |= touched[k];
        }
        if !any {
            mesh.indices.extend_from_slice(t);
            continue;
        }
        poly.clear();
        poly.extend_from_slice(&v);
        for (k, (c, a0)) in corners.iter().enumerate() {
            if !touched[k] {
                continue;
            }
            for s in 0..=STEPS {
                let a = a0 + s as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
                let n = egui::vec2(a.cos(), a.sin());
                // Keep the side where (p - c) . n <= radius.
                let side = |p: Pos2| (p - *c).dot(n) - radius;
                next.clear();
                for i in 0..poly.len() {
                    let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
                    let (dp, dq) = (side(p.pos), side(q.pos));
                    if dp <= 0.0 {
                        next.push(p);
                    }
                    if (dp < 0.0 && dq > 0.0) || (dp > 0.0 && dq < 0.0) {
                        let f = dp / (dp - dq);
                        next.push(egui::epaint::Vertex { pos: p.pos + (q.pos - p.pos) * f, uv: p.uv + (q.uv - p.uv) * f, color: p.color });
                    }
                }
                std::mem::swap(&mut poly, &mut next);
                if poly.len() < 3 {
                    break;
                }
            }
            if poly.len() < 3 {
                break;
            }
        }
        if poly.len() >= 3 {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend_from_slice(&poly);
            for i in 1..poly.len() as u32 - 1 {
                mesh.indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(tilt: f32, forward: Vec2) -> RadarView {
        let rect = Rect::from_min_size(pos2(20.0, 600.0), egui::vec2(300.0, 200.0));
        RadarView { rect, anchor: pos2(170.0, 730.0), centre: Vec2::new(40.0, -25.0), forward, scale: 1.5, tilt }
    }

    fn near(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn the_centre_is_drawn_at_the_anchor() {
        for tilt in [0.0, 0.6] {
            let v = view(tilt, Vec2::new(0.6, 0.8));
            assert!(near(v.project(v.centre), v.anchor));
        }
    }

    #[test]
    fn north_up_puts_east_right_and_south_down() {
        let v = view(0.0, NORTH);
        // 10 m north (-Z): 15 points up.
        assert!(near(v.project(v.centre + Vec2::new(0.0, -10.0)), v.anchor + egui::vec2(0.0, -15.0)));
        // 10 m east (+X): 15 points right.
        assert!(near(v.project(v.centre + Vec2::new(10.0, 0.0)), v.anchor + egui::vec2(15.0, 0.0)));
        assert!(near(v.project(v.centre + Vec2::new(0.0, 10.0)), v.anchor + egui::vec2(0.0, 15.0)));
    }

    #[test]
    fn the_map_turns_with_the_camera() {
        // Looking east: what is ahead (east) is up, south is to the right.
        let v = view(0.0, Vec2::new(1.0, 0.0));
        assert!(near(v.project(v.centre + Vec2::new(20.0, 0.0)), v.anchor + egui::vec2(0.0, -30.0)));
        assert!(near(v.project(v.centre + Vec2::new(0.0, 20.0)), v.anchor + egui::vec2(30.0, 0.0)));
        // North then points to the left of the screen.
        let n = v.screen_dir(NORTH);
        assert!((n.x + 1.0).abs() < 1e-5 && n.y.abs() < 1e-5);
        // Looking south, north is down.
        let n = view(0.0, Vec2::new(0.0, 1.0)).screen_dir(NORTH);
        assert!(n.x.abs() < 1e-5 && (n.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn project_and_unproject_are_inverse() {
        for tilt in [0.0, 0.35, 0.7] {
            let v = view(tilt, Vec2::new(-0.28, 0.96));
            for p in [Vec2::new(0.0, 0.0), Vec2::new(90.0, -40.0), Vec2::new(-30.0, 55.0), Vec2::new(41.0, -24.0)] {
                let back = v.unproject(v.project(p));
                assert!(back.distance(p) < 1e-2, "tilt {tilt}: {p:?} -> {back:?}");
            }
            for s in [pos2(20.0, 600.0), pos2(320.0, 800.0), pos2(200.0, 640.0)] {
                assert!(near(v.project(v.unproject(s)), s), "tilt {tilt}: {s:?}");
            }
        }
    }

    #[test]
    fn tilt_shrinks_the_ground_ahead_and_shows_more_of_it() {
        let (flat, tilted) = (view(0.0, NORTH), view(0.6, NORTH));
        // The same 20 m across covers fewer points far ahead than near.
        let width = |v: &RadarView, ahead: f32| {
            let c = v.centre + NORTH * ahead;
            v.project(c + Vec2::new(10.0, 0.0)).x - v.project(c - Vec2::new(10.0, 0.0)).x
        };
        assert!((width(&flat, 60.0) - width(&flat, 0.0)).abs() < 1e-3);
        assert!(width(&tilted, 60.0) < width(&tilted, 0.0) * 0.8);
        assert!(width(&tilted, -30.0) > width(&tilted, 0.0));
        // Ahead is still up, and the top edge reaches farther.
        assert!(tilted.project(tilted.centre + NORTH * 60.0).y < tilted.anchor.y);
        let top = pos2(170.0, 600.0);
        let reach = |v: &RadarView| (v.unproject(top) - v.centre).dot(NORTH);
        assert!(reach(&tilted) > reach(&flat) * 1.3);
        // The ground box covers every corner.
        let (lo, hi) = tilted.ground_bounds();
        let far = tilted.unproject(pos2(20.0, 600.0));
        assert!(lo.x <= far.x && far.x <= hi.x && lo.y <= far.y && far.y <= hi.y);
    }

    #[test]
    fn far_blips_slide_to_the_border() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        let from = pos2(100.0, 70.0);
        assert_eq!(clamp_to_rect(rect, from, pos2(150.0, 20.0)), (pos2(150.0, 20.0), false));
        // Straight up: the top edge above the anchor.
        assert_eq!(clamp_to_rect(rect, from, pos2(100.0, -500.0)), (pos2(100.0, 0.0), true));
        // To the right and a little up: the right edge, on the line.
        let (p, moved) = clamp_to_rect(rect, from, pos2(1100.0, -30.0));
        assert!(moved && (p.x - 200.0).abs() < 1e-3 && (p.y - 60.0).abs() < 1e-3, "{p:?}");
        assert_eq!(border_point(rect, from, egui::vec2(0.0, 1.0)), pos2(100.0, 100.0));
    }

    #[test]
    fn rounded_corners_cut_the_mesh() {
        let rect = Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(100.0, 60.0));
        let mut mesh = egui::Mesh::default();
        // A quad larger than the rectangle.
        mesh.add_rect_with_uv(rect.expand(20.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        round_corners(&mut mesh, rect, 12.0);
        assert!(mesh.indices.len() >= 6 && mesh.indices.len() % 3 == 0);
        let inside = |p: Pos2| {
            // Outside a corner square, or within the corner's circle.
            [rect.left_top() + egui::vec2(12.0, 12.0), rect.right_top() + egui::vec2(-12.0, 12.0), rect.right_bottom() - egui::vec2(12.0, 12.0), rect.left_bottom() + egui::vec2(12.0, -12.0)].iter().enumerate().all(|(k, c)| {
                let in_square = (if k == 0 || k == 3 { p.x < c.x } else { p.x > c.x }) && (if k < 2 { p.y < c.y } else { p.y > c.y });
                !in_square || (p - *c).length() <= 12.0 * 1.03
            })
        };
        for i in &mesh.indices {
            let p = mesh.vertices[*i as usize].pos;
            assert!(inside(p), "{p:?} is outside the rounded corner");
        }
        // The centre of the rectangle is still covered.
        let c = rect.center();
        let covered = mesh.indices.chunks_exact(3).any(|t| {
            let [a, b, d] = [mesh.vertices[t[0] as usize].pos, mesh.vertices[t[1] as usize].pos, mesh.vertices[t[2] as usize].pos];
            let s = |p: Pos2, q: Pos2| (q.x - p.x) * (c.y - p.y) - (q.y - p.y) * (c.x - p.x);
            let (x, y, z) = (s(a, b), s(b, d), s(d, a));
            (x >= 0.0 && y >= 0.0 && z >= 0.0) || (x <= 0.0 && y <= 0.0 && z <= 0.0)
        });
        assert!(covered);
        // No rounding: untouched.
        let mut m2 = egui::Mesh::default();
        m2.add_rect_with_uv(rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        round_corners(&mut m2, rect, 0.0);
        assert_eq!(m2.indices.len(), 6);
    }
}
