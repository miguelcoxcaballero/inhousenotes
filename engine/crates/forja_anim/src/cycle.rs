//! Turning raw motion capture into game-ready clips: face a heading, stay
//! in place, and loop without a pop.

use forja_math::{Quat, Vec3};

use crate::{Clip, Curve, Skeleton};

fn root_track(clip: &mut Clip) -> Option<&mut crate::Track> {
    let root = clip.root_joint?;
    clip.tracks.iter_mut().find(|t| t.joint == root)
}

/// Rotates the whole motion about Y so the root travels along `forward`
/// (the character's facing direction in its own model space).
pub fn face_forward(clip: &mut Clip, skel: &Skeleton, forward: Vec3) {
    let Some(root) = clip.root_joint else { return };
    let rest = skel.joints[root].rest;
    let parent = skel.parent_rest_rotation(root);
    let Some(tr) = root_track(clip) else { return };
    let Some(tc) = &tr.translation else { return };
    let (a, b) = (parent * tc.values[0], parent * tc.values[tc.values.len() - 1]);
    let travel = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
    let heading = if travel.length() >= 0.4 {
        travel.normalize()
    } else {
        // Stationary take: the way the hips face on average.
        match hips_heading(clip_ref_root_rot(tr), parent, skel, root) {
            Some(h) => h,
            None => return,
        }
    };
    let yaw = Quat::from_rotation_arc(heading, Vec3::new(forward.x, 0.0, forward.z).normalize());
    // Express the world-space yaw in the root's parent space.
    let q = parent.inverse() * yaw * parent;
    if let Some(rc) = &mut tr.rotation {
        for r in &mut rc.values {
            *r = (q * *r).normalize();
        }
    }
    if let Some(tc) = &mut tr.translation {
        for t in &mut tc.values {
            *t = rest.t + q * (*t - rest.t);
        }
    }
}

fn clip_ref_root_rot(tr: &crate::Track) -> Vec<Quat> {
    tr.rotation.as_ref().map(|c| c.values.clone()).unwrap_or_default()
}

/// Mean facing of the root over the take: the rest-pose facing (assumed
/// +Z in model space) rotated by each frame's change from rest.
fn hips_heading(rots: Vec<Quat>, parent: Quat, skel: &Skeleton, root: usize) -> Option<Vec3> {
    if rots.is_empty() {
        return None;
    }
    let rest = skel.joints[root].rest.r;
    let mut sum = Vec3::ZERO;
    for r in &rots {
        let delta = parent * (*r * rest.inverse()) * parent.inverse();
        let f = delta * Vec3::Z;
        sum += Vec3::new(f.x, 0.0, f.z);
    }
    (sum.length() > 1e-3).then(|| sum.normalize())
}

/// Removes the root's net horizontal travel (the capsule moves the
/// character instead), keeping the natural side-to-side sway and bob.
pub fn in_place(clip: &mut Clip, skel: &Skeleton) {
    let Some(root) = clip.root_joint else { return };
    let parent = skel.parent_rest_rotation(root);
    let Some(tr) = root_track(clip) else { return };
    let Some(tc) = &mut tr.translation else { return };
    let n = tc.values.len();
    if n < 2 {
        return;
    }
    let rest = parent * skel.joints[root].rest.t;
    let (first, last) = (parent * tc.values[0], parent * tc.values[n - 1]);
    // Remove the linear travel, then centre the remaining sway on the rest
    // position so the character stays over its own origin.
    let resid: Vec<Vec3> = tc.values.iter().enumerate().map(|(k, t)| parent * *t - first.lerp(last, k as f32 / (n - 1) as f32)).collect();
    let mean = resid.iter().copied().sum::<Vec3>() / n as f32;
    for (t, r) in tc.values.iter_mut().zip(resid) {
        let w = parent * *t;
        let local = Vec3::new(r.x - mean.x + rest.x, w.y, r.z - mean.z + rest.z);
        *t = parent.inverse() * local;
    }
}

/// Shifts the root vertically so the supporting foot sits where the feet
/// sit at rest: the 10th percentile of the lower foot's height over the
/// clip (stance frames dominate it, flight phases do not).
pub fn ground(clip: &mut Clip, skel: &Skeleton, feet: &[usize]) {
    let Some(root) = clip.root_joint else { return };
    if feet.is_empty() {
        return;
    }
    let mut m = Vec::new();
    skel.model_matrices(&skel.rest_pose(), &mut m);
    let rest = feet.iter().map(|f| m[*f].w_axis.y).fold(f32::MAX, f32::min);
    let n = 60;
    let mut lows: Vec<f32> = (0..n)
        .map(|k| {
            let mut pose = skel.rest_pose();
            clip.sample(clip.duration * k as f32 / n as f32, true, &mut pose);
            skel.model_matrices(&pose, &mut m);
            feet.iter().map(|f| m[*f].w_axis.y).fold(f32::MAX, f32::min)
        })
        .collect();
    lows.sort_by(f32::total_cmp);
    let delta = rest - lows[n / 10];
    let parent = skel.parent_rest_rotation(root);
    let Some(tr) = root_track(clip) else { return };
    if let Some(tc) = &mut tr.translation {
        let off = parent.inverse() * Vec3::new(0.0, delta, 0.0);
        for t in &mut tc.values {
            *t += off;
        }
    }
}

/// Average speed of the root over the clip (m/s), for matching animation
/// playback to movement speed.
pub fn root_speed(clip: &Clip, skel: &Skeleton) -> f32 {
    let Some(root) = clip.root_joint else { return 0.0 };
    let parent = skel.parent_rest_rotation(root);
    let Some(tc) = clip.tracks.iter().find(|t| t.joint == root).and_then(|t| t.translation.as_ref()) else { return 0.0 };
    let (a, b) = (parent * tc.values[0], parent * tc.values[tc.values.len() - 1]);
    Vec3::new(b.x - a.x, 0.0, b.z - a.z).length() / clip.duration.max(1e-3)
}

/// Cuts the most seamless loop with a length in `[min_len, max_len]`
/// seconds (and starting after `skip` seconds), then spreads the remaining
/// mismatch over the cycle so the last frame equals the first. The clip
/// must be baked (uniform keys, as produced by retargeting).
pub fn extract_loop(clip: &Clip, min_len: f32, max_len: f32, skip: f32) -> Clip {
    let Some(times) = clip.tracks.iter().find_map(|t| t.rotation.as_ref().map(|c| c.times.clone())) else { return clip.clone() };
    let n = times.len();
    if n < 4 {
        return clip.clone();
    }
    let dt = times[1] - times[0];
    let rots: Vec<&Vec<Quat>> = clip.tracks.iter().filter_map(|t| t.rotation.as_ref().map(|c| &c.values)).collect();
    let dist = |i: usize, j: usize| -> f32 {
        let mut d = 0.0;
        for r in &rots {
            d += 1.0 - r[i].dot(r[j]).abs();
            // Velocity match: compare the next frames too.
            if i + 1 < n && j + 1 < n {
                d += 0.5 * (1.0 - r[i + 1].dot(r[j + 1]).abs());
            }
        }
        d
    };
    let (lo, hi) = (((min_len / dt) as usize).max(2), ((max_len / dt) as usize).max(3));
    let start0 = ((skip / dt) as usize).min(n.saturating_sub(lo + 2));
    let mut best = (f32::MAX, 0usize, n - 1);
    for i in start0..n {
        for len in lo..=hi {
            let j = i + len;
            if j + 1 >= n {
                break;
            }
            let d = dist(i, j);
            if d < best.0 {
                best = (d, i, j);
            }
        }
    }
    let (_, i0, j0) = best;
    let len = j0 - i0;
    let new_times: Vec<f32> = (0..=len).map(|k| k as f32 * dt).collect();
    let mut out = clip.clone();
    out.duration = len as f32 * dt;
    for tr in &mut out.tracks {
        if let Some(c) = &mut tr.rotation {
            let seg: Vec<Quat> = c.values[i0..=j0].to_vec();
            // Residual rotation from last to first frame, faded in.
            let fix = (seg[0] * seg[len].inverse()).normalize();
            let vals = seg.iter().enumerate().map(|(k, q)| (Quat::IDENTITY.slerp(fix, k as f32 / len as f32) * *q).normalize()).collect();
            *c = Curve { times: new_times.clone(), values: vals, interpolation: c.interpolation };
        }
        if let Some(c) = &mut tr.translation {
            let seg: Vec<Vec3> = c.values[i0..=j0].to_vec();
            let fix = seg[0] - seg[len];
            let vals = seg.iter().enumerate().map(|(k, t)| *t + fix * (k as f32 / len as f32)).collect();
            *c = Curve { times: new_times.clone(), values: vals, interpolation: c.interpolation };
        }
        if let Some(c) = &mut tr.scale {
            let vals = c.values[i0..=j0].to_vec();
            *c = Curve { times: new_times.clone(), values: vals, interpolation: c.interpolation };
        }
    }
    out
}

/// Start time of the `len`-second window with the least horizontal root
/// travel.
pub fn stillest_window(clip: &Clip, skel: &Skeleton, len: f32) -> f32 {
    let Some(root) = clip.root_joint else { return 0.0 };
    let Some(tc) = clip.tracks.iter().find(|t| t.joint == root).and_then(|t| t.translation.as_ref()) else { return 0.0 };
    let parent = skel.parent_rest_rotation(root);
    let pts: Vec<Vec3> = tc.values.iter().map(|t| {
        let w = parent * *t;
        Vec3::new(w.x, 0.0, w.z)
    })
    .collect();
    let n = pts.len();
    if n < 3 || clip.duration <= len {
        return 0.0;
    }
    let dt = clip.duration / (n - 1) as f32;
    let w = ((len / dt) as usize).min(n - 1);
    // Prefix sums of step lengths.
    let mut acc = vec![0.0f32; n];
    for i in 1..n {
        acc[i] = acc[i - 1] + (pts[i] - pts[i - 1]).length();
    }
    let best = (0..n - w).min_by(|&a, &b| (acc[a + w] - acc[a]).total_cmp(&(acc[b + w] - acc[b]))).unwrap_or(0);
    best as f32 * dt
}

/// A short slice of a clip as a looping hold (e.g. an idle taken from a
/// longer performance).
pub fn slice(clip: &Clip, from: f32, to: f32) -> Clip {
    let mut out = clip.clone();
    let to = to.min(clip.duration);
    out.duration = (to - from).max(0.0);
    for tr in &mut out.tracks {
        let cut = |times: &[f32]| -> (usize, usize) {
            let a = times.partition_point(|t| *t < from);
            let b = times.partition_point(|t| *t <= to).max(a + 1);
            (a, b)
        };
        if let Some(c) = &mut tr.rotation {
            let (a, b) = cut(&c.times);
            c.values = c.values[a..b].to_vec();
            c.times = c.times[a..b].iter().map(|t| t - from).collect();
        }
        if let Some(c) = &mut tr.translation {
            let (a, b) = cut(&c.times);
            c.values = c.values[a..b].to_vec();
            c.times = c.times[a..b].iter().map(|t| t - from).collect();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Interpolation, Track};

    fn sine_clip() -> Clip {
        // One joint swinging with a 1 s period, sampled at 30 fps for 3.3 s.
        let times: Vec<f32> = (0..100).map(|i| i as f32 / 30.0).collect();
        let values = times.iter().map(|t| Quat::from_rotation_x((t * std::f32::consts::TAU).sin() * 0.5)).collect();
        Clip {
            name: "swing".into(),
            duration: times[99],
            tracks: vec![Track { joint: 0, rotation: Some(Curve { times, values, interpolation: Interpolation::Linear }), ..Default::default() }],
            coverage: 1.0,
            root_joint: None,
        }
    }

    #[test]
    fn loop_matches_the_period_and_closes() {
        let c = extract_loop(&sine_clip(), 0.7, 1.5, 0.0);
        assert!((c.duration - 1.0).abs() < 0.05, "loop {}", c.duration);
        let v = &c.tracks[0].rotation.as_ref().unwrap().values;
        assert!(v[0].dot(v[v.len() - 1]).abs() > 0.99999);
    }
}
