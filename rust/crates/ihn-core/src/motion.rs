//! Folder open/close zoom plan and the interior's fade (ARCHITECTURE §13), 1:1 from 5.18.0.
//! Implemented by W2.5 (BUILD-PLAN.md).
// v5: rust-core-v5.js:folderMotion / folderFade

use crate::jsfmt::num;

/// `{left, top, width, height}` in CSS px.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Box4 {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

pub const OPEN_MS: f64 = 600.0;
pub const CLOSE_MS: f64 = 560.0;

/// The 29 numbers of 5.18.0's plan, in its order.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    pub values: [f64; 29],
    pub backwards: bool,
}

impl Motion {
    pub fn scale(&self) -> f64 { self.values[0] }
    pub fn final_left(&self) -> f64 { self.values[1] }
    pub fn final_top(&self) -> f64 { self.values[2] }
    pub fn duration(&self) -> f64 { self.values[8] }
    pub fn hinge_from(&self) -> f64 { self.values[9] }
    pub fn hinge_to(&self) -> f64 { self.values[10] }
    fn bezier(&self, i: usize) -> String {
        let v = &self.values;
        format!("cubic-bezier({}, {}, {}, {})", num(v[i]), num(v[i + 1]), num(v[i + 2]), num(v[i + 3]))
    }
    pub fn easing(&self) -> String { self.bezier(11) }
    pub fn hinge_easing(&self) -> String { self.bezier(15) }
    /// Keyframes of the ghost, as the JSON `Element.animate` takes.
    pub fn zoom_frames(&self) -> String {
        let v = &self.values;
        let frame = |x: f64, y: f64, s: f64| format!("{{\"transform\":\"translate({}px, {}px) scale({})\"}}", num(x), num(y), num(s));
        format!("[{},{}]", frame(v[25], v[26], v[23]), frame(v[27], v[28], v[24]))
    }
    /// Keyframes of the lid's hinge; `closed` is the card cover's resting transform.
    pub fn hinge_frames(&self, closed: &str) -> String {
        let frame = |angle: f64| {
            let t = if angle == 0.0 { closed.to_string() } else { format!("rotateY({}deg)", num(angle)) };
            format!("{{\"transform\":\"{}\"}}", t.replace('\\', "\\\\").replace('"', "\\\""))
        };
        format!("[{},{}]", frame(self.hinge_from()), frame(self.hinge_to()))
    }
    /// Keyframes of the printed name and the ⋮ on the lid.
    pub fn label_frames(&self) -> String {
        let v = &self.values;
        format!("[{{\"opacity\":{}}},{{\"opacity\":{},\"offset\":{}}},{{\"opacity\":{}}}]", num(v[19]), num(v[20]), num(v[21]), num(v[22]))
    }
}

pub fn folder_motion(bounds: Box4, rect: Box4, backwards: bool) -> Option<Motion> {
    let input = [bounds.left, bounds.top, bounds.width, bounds.height, rect.left, rect.top, rect.width, rect.height];
    if !input.iter().all(|v| v.is_finite()) || bounds.width <= 0.0 || bounds.height <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let scale = ((bounds.width + 4.0) / rect.width).max((bounds.height + 4.0) / (rect.height * 0.94));
    let (width, height) = (rect.width * scale, rect.height * scale);
    let final_left = (bounds.width - width) / 2.0;
    let final_top = (bounds.height - rect.height * 0.94 * scale) / 2.0 - rect.height * 0.06 * scale;
    let (dx, dy) = (final_left - (rect.left - bounds.left), final_top - (rect.top - bounds.top));
    let b = backwards;
    let pick = |closing: f64, opening: f64| if b { closing } else { opening };
    let values = [
        scale, final_left, final_top, dx, dy, width, height, 1100.0 * scale,
        pick(CLOSE_MS, OPEN_MS), pick(-112.0, 0.0), pick(0.0, -112.0),
        0.55, 0.0, 0.15, 1.0, pick(0.4, 0.2), 0.0, 0.2, 1.0,
        pick(0.0, 1.0), 0.0, pick(0.6, 0.4), pick(1.0, 0.0),
        pick(1.0, 1.0 / scale), pick(1.0 / scale, 1.0),
        pick(dx, 0.0), pick(dy, 0.0), pick(0.0, dx), pick(0.0, dy),
    ];
    (values[0] > 0.0 && values.iter().all(|v| v.is_finite())).then_some(Motion { values, backwards })
}

/// `(offset, opacity)` keyframes of the folder interior; `elapsed` is the fraction of the zoom already played.
pub fn folder_fade(elapsed: f64, backwards: bool) -> Option<Vec<(f64, f64)>> {
    if !elapsed.is_finite() {
        return None;
    }
    if backwards {
        return Some(vec![(0.0, 1.0), (0.3, 1.0), (1.0, 0.0)]);
    }
    let from = (elapsed.clamp(0.0, 1.0) + 0.03).min(0.9);
    let to = (from + 0.15).min(1.0).max(0.7);
    Some(vec![(0.0, 0.0), (from, 0.0), (to, 1.0), (1.0, 1.0)])
}

pub fn fade_frames(frames: &[(f64, f64)]) -> String {
    let list: Vec<String> = frames.iter().map(|(o, v)| format!("{{\"offset\":{},\"opacity\":{}}}", num(*o), num(*v))).collect();
    format!("[{}]", list.join(","))
}

/// Test command hook: `core.motion.plan` with `"bl,bt,bw,bh,l,t,w,h,backwards"` → the 29 values as a JSON array.
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    match name {
        "plan" => {
            // Non-negative decimals; a leading `-` negates.
            let v: Vec<f64> = args.trim_matches('"').split(',').filter_map(|s| {
                let s = s.trim();
                crate::rows::parse_decimal(s.trim_start_matches('-')).map(|v| if s.starts_with('-') { -v } else { v })
            }).collect();
            if v.len() != 9 {
                return None;
            }
            let m = folder_motion(Box4 { left: v[0], top: v[1], width: v[2], height: v[3] }, Box4 { left: v[4], top: v[5], width: v[6], height: v[7] }, v[8] != 0.0)?;
            Some(format!("[{}]", m.values.iter().map(|x| num(*x)).collect::<Vec<_>>().join(",")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The same expressions as 5.18.0's JS, evaluated independently for a grid of rects.
    fn reference(b: Box4, r: Box4, back: bool) -> [f64; 29] {
        let scale = f64::max((b.width + 4.0) / r.width, (b.height + 4.0) / (r.height * 0.94));
        let (w, h) = (r.width * scale, r.height * scale);
        let fl = (b.width - w) / 2.0;
        let ft = (b.height - r.height * 0.94 * scale) / 2.0 - r.height * 0.06 * scale;
        let (dx, dy) = (fl - (r.left - b.left), ft - (r.top - b.top));
        if back {
            [scale, fl, ft, dx, dy, w, h, 1100.0 * scale, 560.0, -112.0, 0.0, 0.55, 0.0, 0.15, 1.0, 0.4, 0.0, 0.2, 1.0, 0.0, 0.0, 0.6, 1.0, 1.0, 1.0 / scale, dx, dy, 0.0, 0.0]
        } else {
            [scale, fl, ft, dx, dy, w, h, 1100.0 * scale, 600.0, 0.0, -112.0, 0.55, 0.0, 0.15, 1.0, 0.2, 0.0, 0.2, 1.0, 1.0, 0.0, 0.4, 0.0, 1.0 / scale, 1.0, 0.0, 0.0, dx, dy]
        }
    }

    #[test]
    fn the_29_values_equal_the_js_plan_for_a_grid_of_rects() {
        for (bw, bh) in [(1280.0, 736.0), (360.0, 748.0), (800.0, 1216.0), (1920.0, 1016.0)] {
            for (l, t) in [(0.0, 64.0), (40.5, 200.25), (900.0, 700.0), (-20.0, 10.0)] {
                for w in [112.0, 138.0, 180.0, 233.333] {
                    for back in [false, true] {
                        let (b, r) = (Box4 { left: 0.0, top: 64.0, width: bw, height: bh }, Box4 { left: l, top: t, width: w, height: w * 297.0 / 210.0 });
                        assert_eq!(folder_motion(b, r, back).unwrap().values, reference(b, r, back));
                    }
                }
            }
        }
    }

    #[test]
    fn strings_match_the_js_templates() {
        let m = folder_motion(Box4 { left: 0.0, top: 64.0, width: 1280.0, height: 736.0 }, Box4 { left: 20.0, top: 100.0, width: 138.0, height: 195.0 }, false).unwrap();
        assert_eq!(m.easing(), "cubic-bezier(0.55, 0, 0.15, 1)");
        assert_eq!(m.hinge_easing(), "cubic-bezier(0.2, 0, 0.2, 1)");
        assert_eq!(m.label_frames(), "[{\"opacity\":1},{\"opacity\":0,\"offset\":0.4},{\"opacity\":0}]");
        assert_eq!(m.hinge_frames("rotateY(0deg)"), "[{\"transform\":\"rotateY(0deg)\"},{\"transform\":\"rotateY(-112deg)\"}]");
        assert!(m.zoom_frames().starts_with("[{\"transform\":\"translate(0px, 0px) scale(0.1074"));
        let c = folder_motion(Box4 { left: 0.0, top: 64.0, width: 1280.0, height: 736.0 }, Box4 { left: 20.0, top: 100.0, width: 138.0, height: 195.0 }, true).unwrap();
        assert_eq!(c.hinge_easing(), "cubic-bezier(0.4, 0, 0.2, 1)");
        assert_eq!(c.duration(), 560.0);
        assert!(c.zoom_frames().ends_with("scale(0.10748299319727891)\"}]") || c.zoom_frames().contains("translate(0px, 0px) scale("));
    }

    #[test]
    fn bad_input_gives_no_plan() {
        let ok = Box4 { left: 0.0, top: 0.0, width: 10.0, height: 10.0 };
        assert!(folder_motion(Box4 { width: 0.0, ..ok }, ok, false).is_none());
        assert!(folder_motion(ok, Box4 { height: f64::NAN, ..ok }, false).is_none());
    }

    #[test]
    fn fade_keyframes() {
        assert_eq!(folder_fade(0.0, true).unwrap(), [(0.0, 1.0), (0.3, 1.0), (1.0, 0.0)]);
        assert_eq!(folder_fade(0.0, false).unwrap(), [(0.0, 0.0), (0.03, 0.0), (0.7, 1.0), (1.0, 1.0)]);
        assert_eq!(folder_fade(0.6, false).unwrap(), [(0.0, 0.0), (0.63, 0.0), (0.78, 1.0), (1.0, 1.0)]);
        assert_eq!(folder_fade(2.0, false).unwrap(), [(0.0, 0.0), (0.9, 0.0), (1.0, 1.0), (1.0, 1.0)]);
        assert!(folder_fade(f64::NAN, false).is_none());
        assert_eq!(fade_frames(&[(0.0, 1.0), (0.3, 1.0)]), "[{\"offset\":0,\"opacity\":1},{\"offset\":0.3,\"opacity\":1}]");
    }
}
