//! Data-driven animation state machine.
//!
//! ```toml
//! name = "humanoid"
//! initial = "locomotion"
//!
//! [[state]]
//! name = "locomotion"
//! blend = { param = "speed", points = [[0.0, "Idle"], [1.8, "Walking"], [5.5, "Running"]] }
//!
//! [[state]]
//! name = "jump"
//! clip = "Jump"
//! looping = false
//!
//! [[state]]
//! name = "move"
//! blend = { param = "gait", points = [[1.0, "Walking"], [2.0, "Running"]] }
//! # Playback speed multiplier read from a parameter (stride matching:
//! # movement speed / the clip's natural speed). 1 when unset.
//! rate = "stride"
//!
//! [[transition]]
//! from = "*"
//! to = "jump"
//! when = [{ param = "jump", gt = 0.5 }]
//! fade = 0.1
//!
//! [[transition]]
//! from = "jump"
//! to = "locomotion"
//! on_finish = true
//! fade = 0.2
//!
//! [[layer]]
//! name = "aim"
//! clip = "holding-right-shoot"
//! mask_root = "torso"
//! weight_param = "aiming"
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use forja_math::Vec3;
use serde::Deserialize;

use crate::{Clip, Pose, Skeleton};

#[derive(Clone, Debug, Deserialize)]
pub struct ControllerDef {
    pub name: String,
    pub initial: String,
    #[serde(default)]
    pub root_motion: bool,
    #[serde(rename = "state")]
    pub states: Vec<StateDef>,
    #[serde(rename = "transition", default)]
    pub transitions: Vec<TransitionDef>,
    #[serde(rename = "layer", default)]
    pub layers: Vec<LayerDef>,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub struct StateDef {
    pub name: String,
    #[serde(default)]
    pub clip: Option<String>,
    #[serde(default)]
    pub blend: Option<BlendDef>,
    #[serde(default = "one")]
    pub speed: f32,
    #[serde(default = "yes")]
    pub looping: bool,
    /// Parameter that scales playback speed on top of `speed` (1 when unset).
    #[serde(default)]
    pub rate: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BlendDef {
    pub param: String,
    /// `(parameter value, clip name)` sorted by value.
    pub points: Vec<(f32, String)>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CondDef {
    pub param: String,
    #[serde(default)]
    pub gt: Option<f32>,
    #[serde(default)]
    pub lt: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TransitionDef {
    /// State name or `"*"` for any state.
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub when: Vec<CondDef>,
    /// Fire when a non-looping source state reaches its end.
    #[serde(default)]
    pub on_finish: bool,
    #[serde(default)]
    pub fade: f32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LayerDef {
    pub name: String,
    pub clip: String,
    /// Bone whose subtree the layer drives.
    pub mask_root: String,
    pub weight_param: String,
    #[serde(default = "yes")]
    pub looping: bool,
}

impl ControllerDef {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

enum Source {
    Clip(usize),
    Blend { param: String, points: Vec<(f32, usize)> },
}

struct State {
    name: String,
    source: Source,
    speed: f32,
    looping: bool,
    rate: Option<String>,
}

struct Layer {
    clip: usize,
    mask: Vec<f32>,
    param: String,
    weight: f32,
    time: f32,
    looping: bool,
}

struct Playing {
    state: usize,
    /// Seconds for clip states, normalised phase (0..1) for blend spaces.
    time: f32,
}

pub struct AnimController {
    states: Vec<State>,
    transitions: Vec<(Option<usize>, usize, Vec<CondDef>, bool, f32)>,
    layers: Vec<Layer>,
    /// Shared by every controller built from the same set (crowds).
    clips: Arc<[Clip]>,
    params: HashMap<String, f32>,
    current: Playing,
    previous: Option<(Playing, f32, f32)>,
    pose: Pose,
    scratch: Pose,
    rest: Pose,
    root_motion: bool,
    root_joint: Option<usize>,
}

impl AnimController {
    /// Resolves clip names; fails with the list of missing clips so a pack
    /// that does not fit the definition is reported, not silently broken.
    /// `clips` may be an `Arc<[Clip]>` shared by many controllers.
    pub fn new(def: &ControllerDef, skeleton: &Skeleton, clips: impl Into<Arc<[Clip]>>) -> Result<Self, String> {
        let clips: Arc<[Clip]> = clips.into();
        let find_clip = |name: &str| clips.iter().position(|c| c.name == name).ok_or_else(|| format!("missing clip {name:?}"));
        let mut errors = Vec::new();
        let mut states = Vec::new();
        for s in &def.states {
            let source = if let Some(b) = &s.blend {
                let mut points = Vec::new();
                for (v, c) in &b.points {
                    match find_clip(c) {
                        Ok(i) => points.push((*v, i)),
                        Err(e) => errors.push(e),
                    }
                }
                points.sort_by(|a, b| a.0.total_cmp(&b.0));
                Source::Blend { param: b.param.clone(), points }
            } else if let Some(c) = &s.clip {
                match find_clip(c) {
                    Ok(i) => Source::Clip(i),
                    Err(e) => {
                        errors.push(e);
                        Source::Clip(0)
                    }
                }
            } else {
                errors.push(format!("state {} needs `clip` or `blend`", s.name));
                Source::Clip(0)
            };
            states.push(State { name: s.name.clone(), source, speed: s.speed, looping: s.looping, rate: s.rate.clone() });
        }
        let state_index = |n: &str| states.iter().position(|s| s.name == n);
        let mut transitions = Vec::new();
        for t in &def.transitions {
            let from = if t.from == "*" { None } else { state_index(&t.from) };
            match (t.from == "*" || from.is_some(), state_index(&t.to)) {
                (true, Some(to)) => transitions.push((from, to, t.when.clone(), t.on_finish, t.fade)),
                _ => errors.push(format!("transition {} -> {} names an unknown state", t.from, t.to)),
            }
        }
        let mut layers = Vec::new();
        for l in &def.layers {
            match (find_clip(&l.clip), skeleton.find(&l.mask_root)) {
                (Ok(clip), Some(_)) => layers.push(Layer {
                    clip,
                    mask: skeleton.subtree_mask(&l.mask_root),
                    param: l.weight_param.clone(),
                    weight: 0.0,
                    time: 0.0,
                    looping: l.looping,
                }),
                (Err(e), _) => errors.push(e),
                (_, None) => errors.push(format!("layer {} mask bone {:?} not in skeleton", l.name, l.mask_root)),
            }
        }
        let Some(initial) = state_index(&def.initial) else {
            errors.push(format!("initial state {:?} not found", def.initial));
            return Err(errors.join("; "));
        };
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        let rest = skeleton.rest_pose();
        let root_joint = clips.iter().find_map(|c| c.root_joint);
        Ok(Self {
            states,
            transitions,
            layers,
            clips,
            params: HashMap::new(),
            current: Playing { state: initial, time: 0.0 },
            previous: None,
            pose: rest.clone(),
            scratch: rest.clone(),
            rest,
            root_motion: def.root_motion,
            root_joint,
        })
    }

    pub fn set(&mut self, param: &str, value: f32) {
        self.params.insert(param.to_string(), value);
    }

    pub fn get(&self, param: &str) -> f32 {
        self.params.get(param).copied().unwrap_or(0.0)
    }

    pub fn state(&self) -> &str {
        &self.states[self.current.state].name
    }

    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }

    fn finished(&self, p: &Playing) -> bool {
        let s = &self.states[p.state];
        match s.source {
            Source::Clip(c) => !s.looping && p.time >= self.clips[c].duration,
            Source::Blend { .. } => !s.looping && p.time >= 1.0,
        }
    }

    /// Blend-space neighbours and weight for the current parameter value.
    fn blend_pair(&self, param: &str, points: &[(f32, usize)]) -> (usize, usize, f32) {
        let v = self.get(param);
        if points.len() == 1 || v <= points[0].0 {
            return (points[0].1, points[0].1, 0.0);
        }
        for w in points.windows(2) {
            if v <= w[1].0 {
                let t = (v - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
                return (w[0].1, w[1].1, t);
            }
        }
        let last = points[points.len() - 1].1;
        (last, last, 0.0)
    }

    fn advance(&self, p: &mut Playing, dt: f32) -> Vec3 {
        let s = &self.states[p.state];
        let dt = dt * s.rate.as_ref().map_or(1.0, |r| self.params.get(r).copied().unwrap_or(1.0)).max(0.0);
        let t0 = p.time;
        match &s.source {
            Source::Clip(c) => {
                p.time += dt * s.speed;
                if self.root_motion { self.clips[*c].root_motion(t0, p.time) } else { Vec3::ZERO }
            }
            Source::Blend { param, points } => {
                let (a, b, w) = self.blend_pair(param, points);
                let (da, db) = (self.clips[a].duration, self.clips[b].duration);
                let dur = da + (db - da) * w;
                p.time += dt * s.speed / dur.max(1e-3);
                if !s.looping {
                    p.time = p.time.min(1.0);
                }
                if self.root_motion {
                    self.clips[a].root_motion(t0 * da, p.time * da).lerp(self.clips[b].root_motion(t0 * db, p.time * db), w)
                } else {
                    Vec3::ZERO
                }
            }
        }
    }

    fn sample(&self, p: &Playing, out: &mut Pose, scratch: &mut Pose) {
        let s = &self.states[p.state];
        out.local.clone_from(&self.rest.local);
        match &s.source {
            Source::Clip(c) => self.clips[*c].sample(p.time, s.looping, out),
            Source::Blend { param, points } => {
                let (a, b, w) = self.blend_pair(param, points);
                let (ca, cb) = (&self.clips[a], &self.clips[b]);
                let phase = if s.looping { p.time.rem_euclid(1.0) } else { p.time.min(1.0) };
                ca.sample(phase * ca.duration, s.looping, out);
                if a != b && w > 0.0 {
                    scratch.local.clone_from(&self.rest.local);
                    cb.sample(phase * cb.duration, s.looping, scratch);
                    out.blend(scratch, w);
                }
            }
        }
    }

    /// Advances time, applies transitions and rebuilds the pose. Returns the
    /// root motion displacement in model space (zero when disabled).
    pub fn update(&mut self, dt: f32) -> Vec3 {
        let cur = self.current.state;
        let finished = self.finished(&self.current);
        let fire = self.transitions.iter().find(|(from, to, when, on_finish, _)| {
            *to != cur
                && from.is_none_or(|f| f == cur)
                && (!*on_finish || finished)
                && when.iter().all(|c| {
                    let v = self.get(&c.param);
                    c.gt.is_none_or(|g| v > g) && c.lt.is_none_or(|l| v < l)
                })
                && (*on_finish || !when.is_empty())
        });
        if let Some(&(_, to, _, _, fade)) = fire {
            let old = std::mem::replace(&mut self.current, Playing { state: to, time: 0.0 });
            self.previous = if fade > 0.0 { Some((old, 0.0, fade)) } else { None };
        }
        let mut cur = std::mem::replace(&mut self.current, Playing { state: 0, time: 0.0 });
        let mut motion = self.advance(&mut cur, dt);
        self.current = cur;
        let mut pose = std::mem::replace(&mut self.pose, Pose { local: Vec::new() });
        let mut scratch = std::mem::replace(&mut self.scratch, Pose { local: Vec::new() });
        self.sample(&self.current, &mut pose, &mut scratch);
        if let Some((mut prev, elapsed, dur)) = self.previous.take() {
            let prev_motion = self.advance(&mut prev, dt);
            let elapsed = elapsed + dt;
            let k = (elapsed / dur).clamp(0.0, 1.0);
            let w = k * k * (3.0 - 2.0 * k); // smoothstep
            if k < 1.0 {
                let mut prev_pose = self.rest.clone();
                self.sample(&prev, &mut prev_pose, &mut scratch);
                prev_pose.blend(&pose, w);
                pose = prev_pose;
                motion = prev_motion.lerp(motion, w);
                self.previous = Some((prev, elapsed, dur));
            }
        }
        for i in 0..self.layers.len() {
            let target = self.get(&self.layers[i].param).clamp(0.0, 1.0);
            let l = &mut self.layers[i];
            l.weight += (target - l.weight) * (dt * 12.0).min(1.0);
            l.time += dt;
            if l.weight > 1e-3 {
                let (clip, time, looping, weight) = (l.clip, l.time, l.looping, l.weight);
                scratch.local.clone_from(&pose.local);
                self.clips[clip].sample(time, looping, &mut scratch);
                pose.blend_masked(&scratch, weight, &self.layers[i].mask);
            }
        }
        if self.root_motion
            && let Some(r) = self.root_joint
        {
            let rest = self.rest.local[r];
            pose.lock_root(r, &rest);
        }
        self.pose = pose;
        self.scratch = scratch;
        motion
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_rig::arm;

    const DEF: &str = r#"
        name = "arm"
        initial = "rest"
        root_motion = true

        [[state]]
        name = "rest"
        blend = { param = "speed", points = [[0.0, "bend"], [1.0, "slide"]] }
        rate = "stride"

        [[state]]
        name = "once"
        clip = "bend"
        looping = false

        [[transition]]
        from = "*"
        to = "once"
        when = [{ param = "trigger", gt = 0.5 }]
        fade = 0.2

        [[transition]]
        from = "once"
        to = "rest"
        on_finish = true
        fade = 0.1

        [[layer]]
        name = "elbow_only"
        clip = "bend"
        mask_root = "elbow"
        weight_param = "aim"
    "#;

    fn controller() -> AnimController {
        let scene = arm();
        let skel = Skeleton::from_scene(&scene, 0).unwrap();
        let clips = Clip::bind_all(&scene, &skel, None);
        AnimController::new(&ControllerDef::from_toml(DEF).unwrap(), &skel, clips).unwrap()
    }

    #[test]
    fn transitions_fade_and_finish() {
        let mut c = controller();
        c.update(0.016);
        assert_eq!(c.state(), "rest");
        c.set("trigger", 1.0);
        c.update(0.016);
        assert_eq!(c.state(), "once");
        c.set("trigger", 0.0);
        for _ in 0..80 {
            c.update(0.016);
        }
        assert_eq!(c.state(), "rest", "on_finish transition did not fire");
    }

    #[test]
    fn blend_space_moves_root_with_speed() {
        let mut c = controller();
        c.set("speed", 0.0);
        let still: Vec3 = (0..30).map(|_| c.update(1.0 / 30.0)).sum();
        assert!(still.length() < 1e-4);
        c.set("speed", 1.0);
        let moved: Vec3 = (0..30).map(|_| c.update(1.0 / 30.0)).sum();
        assert!((moved.x - 2.0).abs() < 0.1, "root motion over one loop: {moved:?}");
        // The root bone itself stays locked in place.
        assert!(c.pose().local[0].t.x.abs() < 1e-5);
    }

    #[test]
    fn rate_parameter_scales_playback() {
        let travel = |rate: Option<f32>| {
            let mut c = controller();
            c.set("speed", 1.0);
            if let Some(r) = rate {
                c.set("stride", r);
            }
            (0..15).map(|_| c.update(1.0 / 30.0)).sum::<Vec3>().x
        };
        let (normal, unset, double) = (travel(Some(1.0)), travel(None), travel(Some(2.0)));
        assert!((normal - unset).abs() < 1e-5, "unset rate must mean 1");
        assert!((double - 2.0 * normal).abs() < 0.05, "{double} vs {normal}");
    }

    #[test]
    fn controllers_share_one_clip_set() {
        let scene = arm();
        let skel = Skeleton::from_scene(&scene, 0).unwrap();
        let clips: Arc<[Clip]> = Clip::bind_all(&scene, &skel, None).into();
        let def = ControllerDef::from_toml(DEF).unwrap();
        let a = AnimController::new(&def, &skel, clips.clone()).unwrap();
        let b = AnimController::new(&def, &skel, clips.clone()).unwrap();
        assert!(std::ptr::eq(a.clips().as_ptr(), b.clips().as_ptr()));
    }

    #[test]
    fn layer_drives_only_its_subtree() {
        let mut c = controller();
        c.set("speed", 1.0);
        c.set("aim", 1.0);
        // 20 frames = 0.67 s into the looping layer clip (not a loop boundary).
        for _ in 0..20 {
            c.update(1.0 / 30.0);
        }
        assert!(c.pose().local[1].r.to_axis_angle().1 > 0.1, "layer did not rotate the elbow");
    }

    #[test]
    fn missing_clips_are_reported() {
        let scene = arm();
        let skel = Skeleton::from_scene(&scene, 0).unwrap();
        let def = ControllerDef::from_toml(&DEF.replace("\"slide\"", "\"nope\"")).unwrap();
        let err = AnimController::new(&def, &skel, Clip::bind_all(&scene, &skel, None)).err().unwrap();
        assert!(err.contains("nope"));
    }
}
