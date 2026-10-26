//! # forja_anim
//!
//! Skeletal animation that works with any rig:
//!
//! * [`Skeleton`] is built from a glTF skin.
//! * [`Clip`] is resolved against a skeleton **by bone name**, so clips from
//!   one pack drive any skeleton that shares the names (retargeting between
//!   identical rigs; add a [`BoneMap`] for renamed rigs).
//! * [`Pose`] blending supports per-bone masks (upper body aims a weapon
//!   while the legs keep walking).
//! * [`AnimController`] is a state machine with crossfades and 1D blend
//!   spaces, defined in TOML so designers can change it without code.

pub mod controller;

use std::collections::HashMap;

use forja_assets::{ChannelValues, ClipData, Interpolation, SceneData, Trs, nlerp};
use forja_math::{Mat4, Quat, Vec3};

pub use controller::{AnimController, ControllerDef};

#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub rest: Trs,
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
    pub inverse_bind: Vec<Mat4>,
    /// Joints sorted so parents precede children.
    pub order: Vec<usize>,
    /// Transform of the skeleton's parent node in scene space.
    pub root_parent: Mat4,
    by_name: HashMap<String, usize>,
}

impl Skeleton {
    pub fn from_scene(scene: &SceneData, skin: usize) -> Option<Self> {
        let s = scene.skins.get(skin)?;
        let node_to_joint: HashMap<usize, usize> = s.joints.iter().enumerate().map(|(j, n)| (*n, j)).collect();
        let joints: Vec<Joint> = s
            .joints
            .iter()
            .map(|n| {
                let mut p = scene.nodes[*n].parent;
                while let Some(pi) = p {
                    if node_to_joint.contains_key(&pi) {
                        break;
                    }
                    p = scene.nodes[pi].parent;
                }
                Joint { name: scene.nodes[*n].name.clone(), parent: p.map(|pi| node_to_joint[&pi]), rest: scene.nodes[*n].local }
            })
            .collect();
        // Scene transform above the top-most joint (armature node, unit scale).
        let world = scene.world_matrices();
        let top = s.joints.iter().find(|n| scene.nodes[**n].parent.is_none_or(|p| !node_to_joint.contains_key(&p)));
        let root_parent = top.and_then(|n| scene.nodes[*n].parent).map(|p| world[p]).unwrap_or(Mat4::IDENTITY);
        let mut order = Vec::with_capacity(joints.len());
        let mut placed = vec![false; joints.len()];
        while order.len() < joints.len() {
            let before = order.len();
            for (i, j) in joints.iter().enumerate() {
                if !placed[i] && j.parent.is_none_or(|p| placed[p]) {
                    placed[i] = true;
                    order.push(i);
                }
            }
            if order.len() == before {
                return None; // cycle: malformed file
            }
        }
        let by_name = joints.iter().enumerate().map(|(i, j)| (j.name.clone(), i)).collect();
        Some(Self { joints, inverse_bind: s.inverse_bind.clone(), order, root_parent, by_name })
    }

    /// Skeleton over the whole node hierarchy, for rigid (unskinned) node
    /// animation such as Kenney's blocky characters or vehicle doors. Joint
    /// `i` is node `i`, so [`Skeleton::model_matrices`] gives node transforms.
    pub fn from_nodes(scene: &SceneData) -> Self {
        let joints: Vec<Joint> =
            scene.nodes.iter().map(|n| Joint { name: n.name.clone(), parent: n.parent, rest: n.local }).collect();
        let mut order = Vec::with_capacity(joints.len());
        let mut stack: Vec<usize> = scene.roots.iter().rev().copied().collect();
        while let Some(i) = stack.pop() {
            order.push(i);
            stack.extend(scene.nodes[i].children.iter().rev());
        }
        let by_name = joints.iter().enumerate().map(|(i, j)| (j.name.clone(), i)).collect();
        Self { inverse_bind: vec![Mat4::IDENTITY; joints.len()], joints, order, root_parent: Mat4::IDENTITY, by_name }
    }

    /// Skinned skeleton when the file has a skin, node hierarchy otherwise.
    pub fn from_scene_any(scene: &SceneData) -> Self {
        Self::from_scene(scene, 0).unwrap_or_else(|| Self::from_nodes(scene))
    }

    pub fn len(&self) -> usize {
        self.joints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    pub fn rest_pose(&self) -> Pose {
        Pose { local: self.joints.iter().map(|j| j.rest).collect() }
    }

    /// Model-space joint matrices for a pose.
    pub fn model_matrices(&self, pose: &Pose, out: &mut Vec<Mat4>) {
        out.clear();
        out.resize(self.joints.len(), Mat4::IDENTITY);
        for &i in &self.order {
            let local = pose.local[i].to_mat4();
            out[i] = match self.joints[i].parent {
                Some(p) => out[p] * local,
                None => self.root_parent * local,
            };
        }
    }

    /// Final skinning matrices (`model * inverse_bind`) for the GPU.
    pub fn skinning_matrices(&self, pose: &Pose, out: &mut Vec<Mat4>) {
        self.model_matrices(pose, out);
        for (m, ib) in out.iter_mut().zip(&self.inverse_bind) {
            *m *= *ib;
        }
    }

    /// Joints in the subtree rooted at `name` (inclusive), for blend masks.
    pub fn subtree_mask(&self, name: &str) -> Vec<f32> {
        let mut mask = vec![0.0; self.joints.len()];
        let Some(root) = self.find(name) else { return mask };
        for &i in &self.order {
            if i == root || self.joints[i].parent.is_some_and(|p| mask[p] > 0.0) {
                mask[i] = 1.0;
            }
        }
        mask
    }
}

/// Optional renames from a source rig to a target rig.
#[derive(Clone, Debug, Default)]
pub struct BoneMap(pub HashMap<String, String>);

#[derive(Clone, Debug)]
pub struct Curve<T> {
    pub times: Vec<f32>,
    pub values: Vec<T>,
    pub interpolation: Interpolation,
}

pub trait Lerp: Copy {
    fn lerp_to(self, o: Self, t: f32) -> Self;
    fn hermite(p0: Self, m0: Self, p1: Self, m1: Self, t: f32, dt: f32) -> Self;
}

impl Lerp for Vec3 {
    fn lerp_to(self, o: Self, t: f32) -> Self {
        self.lerp(o, t)
    }
    fn hermite(p0: Self, m0: Self, p1: Self, m1: Self, t: f32, dt: f32) -> Self {
        let (t2, t3) = (t * t, t * t * t);
        p0 * (2.0 * t3 - 3.0 * t2 + 1.0) + m0 * (dt * (t3 - 2.0 * t2 + t)) + p1 * (-2.0 * t3 + 3.0 * t2) + m1 * (dt * (t3 - t2))
    }
}

impl Lerp for Quat {
    fn lerp_to(self, o: Self, t: f32) -> Self {
        self.slerp(o, t)
    }
    fn hermite(p0: Self, m0: Self, p1: Self, m1: Self, t: f32, dt: f32) -> Self {
        let v = |q: Quat| forja_math::Vec4::from(q);
        let (t2, t3) = (t * t, t * t * t);
        let r = v(p0) * (2.0 * t3 - 3.0 * t2 + 1.0) + v(m0) * (dt * (t3 - 2.0 * t2 + t)) + v(p1) * (-2.0 * t3 + 3.0 * t2) + v(m1) * (dt * (t3 - t2));
        Quat::from_vec4(r).normalize()
    }
}

impl<T: Lerp> Curve<T> {
    fn sample(&self, t: f32) -> T {
        let n = self.times.len();
        let cubic = self.interpolation == Interpolation::CubicSpline;
        let value = |k: usize| if cubic { self.values[k * 3 + 1] } else { self.values[k] };
        if n == 1 || t <= self.times[0] {
            return value(0);
        }
        if t >= self.times[n - 1] {
            return value(n - 1);
        }
        let k = self.times.partition_point(|x| *x <= t) - 1;
        let (t0, t1) = (self.times[k], self.times[k + 1]);
        let dt = t1 - t0;
        let u = if dt > 0.0 { (t - t0) / dt } else { 0.0 };
        match self.interpolation {
            Interpolation::Step => value(k),
            Interpolation::Linear => value(k).lerp_to(value(k + 1), u),
            Interpolation::CubicSpline => {
                T::hermite(self.values[k * 3 + 1], self.values[k * 3 + 2], self.values[(k + 1) * 3 + 1], self.values[(k + 1) * 3], u, dt)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Track {
    pub joint: usize,
    pub translation: Option<Curve<Vec3>>,
    pub rotation: Option<Curve<Quat>>,
    pub scale: Option<Curve<Vec3>>,
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub tracks: Vec<Track>,
    /// Fraction of the target skeleton driven by this clip.
    pub coverage: f32,
    /// Root bone whose horizontal motion is extracted (if root motion is on).
    pub root_joint: Option<usize>,
}

impl Clip {
    /// Resolves `data` (from `source` scene) against `skeleton` by bone name.
    pub fn bind(data: &ClipData, source: &SceneData, skeleton: &Skeleton, map: Option<&BoneMap>) -> Clip {
        let mut tracks: HashMap<usize, Track> = HashMap::new();
        for ch in &data.channels {
            let src_name = &source.nodes[ch.node].name;
            let name = map.and_then(|m| m.0.get(src_name)).unwrap_or(src_name);
            let Some(j) = skeleton.find(name) else { continue };
            let t = tracks.entry(j).or_insert_with(|| Track { joint: j, ..Default::default() });
            match &ch.values {
                ChannelValues::Translation(v) => {
                    t.translation = Some(Curve { times: ch.times.clone(), values: v.clone(), interpolation: ch.interpolation })
                }
                ChannelValues::Rotation(v) => {
                    t.rotation = Some(Curve { times: ch.times.clone(), values: v.clone(), interpolation: ch.interpolation })
                }
                ChannelValues::Scale(v) => {
                    t.scale = Some(Curve { times: ch.times.clone(), values: v.clone(), interpolation: ch.interpolation })
                }
            }
        }
        let coverage = tracks.len() as f32 / skeleton.len().max(1) as f32;
        let root_joint = skeleton.order.iter().copied().find(|j| tracks.get(j).is_some_and(|t| t.translation.is_some()));
        let mut tracks: Vec<Track> = tracks.into_values().collect();
        tracks.sort_by_key(|t| t.joint);
        Clip { name: data.name.clone(), duration: data.duration.max(1e-3), tracks, coverage, root_joint }
    }

    /// Binds every clip of `source` to `skeleton`, dropping clips that do not fit.
    pub fn bind_all(source: &SceneData, skeleton: &Skeleton, map: Option<&BoneMap>) -> Vec<Clip> {
        source.clips.iter().map(|c| Clip::bind(c, source, skeleton, map)).filter(|c| !c.tracks.is_empty()).collect()
    }

    /// Writes the sampled local transforms into `pose` (untouched joints keep
    /// their current value, normally the rest pose).
    pub fn sample(&self, time: f32, looping: bool, pose: &mut Pose) {
        let t = if looping { time.rem_euclid(self.duration) } else { time.clamp(0.0, self.duration) };
        for tr in &self.tracks {
            let p = &mut pose.local[tr.joint];
            if let Some(c) = &tr.translation {
                p.t = c.sample(t);
            }
            if let Some(c) = &tr.rotation {
                p.r = c.sample(t);
            }
            if let Some(c) = &tr.scale {
                p.s = c.sample(t);
            }
        }
    }

    /// Horizontal root displacement between two clip times (handles one loop wrap).
    pub fn root_motion(&self, t0: f32, t1: f32) -> Vec3 {
        let Some(root) = self.root_joint else { return Vec3::ZERO };
        let Some(curve) = self.tracks.iter().find(|t| t.joint == root).and_then(|t| t.translation.as_ref()) else {
            return Vec3::ZERO;
        };
        let at = |t: f32| curve.sample(t.clamp(0.0, self.duration));
        let (a, b) = (t0.rem_euclid(self.duration), t1.rem_euclid(self.duration));
        let d = if b >= a { at(b) - at(a) } else { (at(self.duration) - at(a)) + (at(b) - at(0.0)) };
        Vec3::new(d.x, 0.0, d.z)
    }
}

/// Local joint transforms.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub local: Vec<Trs>,
}

impl Pose {
    /// `self = lerp(self, other, w)`.
    pub fn blend(&mut self, other: &Pose, w: f32) {
        if w <= 0.0 {
            return;
        }
        for (a, b) in self.local.iter_mut().zip(&other.local) {
            *a = a.lerp(b, w.min(1.0));
        }
    }

    /// Blend with a per-joint weight mask (e.g. upper body only).
    pub fn blend_masked(&mut self, other: &Pose, w: f32, mask: &[f32]) {
        for ((a, b), m) in self.local.iter_mut().zip(&other.local).zip(mask) {
            let k = (w * m).clamp(0.0, 1.0);
            if k > 0.0 {
                *a = a.lerp(b, k);
            }
        }
    }

    /// Removes the horizontal translation of `joint` (root motion is then
    /// applied to the character capsule instead).
    pub fn lock_root(&mut self, joint: usize, rest: &Trs) {
        let p = &mut self.local[joint];
        p.t = Vec3::new(rest.t.x, p.t.y, rest.t.z);
    }
}

/// Reference CPU skinning (tests, tools, ray-cast hit boxes).
pub fn skin_positions(positions: &[Vec3], joints: &[[u16; 4]], weights: &[[f32; 4]], matrices: &[Mat4]) -> Vec<Vec3> {
    positions
        .iter()
        .zip(joints.iter().zip(weights))
        .map(|(p, (j, w))| {
            let mut out = Vec3::ZERO;
            for k in 0..4 {
                if w[k] > 0.0 {
                    out += matrices[j[k] as usize].transform_point3(*p) * w[k];
                }
            }
            out
        })
        .collect()
}

/// Rotation blend helper re-exported for gameplay (aim offsets).
pub fn blend_rotation(a: Quat, b: Quat, t: f32) -> Quat {
    nlerp(a, b, t)
}

#[cfg(test)]
pub(crate) mod test_rig {
    use super::*;
    use forja_assets::{ChannelData, NodeData, SkinData};

    /// Two-bone arm: root at origin, child 1 m up. Clip "bend" rotates the
    /// child 90 degrees about Z over 1 s; clip "slide" moves the root +X.
    pub fn arm() -> SceneData {
        let nodes = vec![
            NodeData { name: "root".into(), children: vec![1], ..Default::default() },
            NodeData { name: "elbow".into(), parent: Some(0), local: Trs { t: Vec3::Y, ..Default::default() }, ..Default::default() },
        ];
        let bend = ClipData {
            name: "bend".into(),
            duration: 1.0,
            channels: vec![ChannelData {
                node: 1,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: ChannelValues::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)]),
            }],
        };
        let slide = ClipData {
            name: "slide".into(),
            duration: 1.0,
            channels: vec![ChannelData {
                node: 0,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: ChannelValues::Translation(vec![Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)]),
            }],
        };
        SceneData {
            nodes,
            roots: vec![0],
            skins: vec![SkinData { name: "arm".into(), joints: vec![0, 1], inverse_bind: vec![Mat4::IDENTITY, Mat4::from_translation(-Vec3::Y)] }],
            clips: vec![bend, slide],
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_and_skin() {
        let scene = test_rig::arm();
        let skel = Skeleton::from_scene(&scene, 0).unwrap();
        let clip = Clip::bind(&scene.clips[0], &scene, &skel, None);
        assert_eq!(clip.coverage, 0.5);
        let mut pose = skel.rest_pose();
        clip.sample(1.0, false, &mut pose);
        let mut m = Vec::new();
        skel.skinning_matrices(&pose, &mut m);
        // A vertex 1 m above the elbow (y = 2 in bind space) rotates to (-1, 1, 0).
        let v = skin_positions(&[Vec3::new(0.0, 2.0, 0.0)], &[[1, 0, 0, 0]], &[[1.0, 0.0, 0.0, 0.0]], &m);
        assert!((v[0] - Vec3::new(-1.0, 1.0, 0.0)).length() < 1e-4, "{:?}", v[0]);
        // Rest pose skinning is identity.
        skel.skinning_matrices(&skel.rest_pose(), &mut m);
        let v = skin_positions(&[Vec3::new(0.3, 2.0, 0.0)], &[[1, 0, 0, 0]], &[[1.0, 0.0, 0.0, 0.0]], &m);
        assert!((v[0] - Vec3::new(0.3, 2.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn blend_halfway() {
        let scene = test_rig::arm();
        let skel = Skeleton::from_scene(&scene, 0).unwrap();
        let clip = Clip::bind(&scene.clips[0], &scene, &skel, None);
        let mut a = skel.rest_pose();
        let mut b = skel.rest_pose();
        clip.sample(1.0, false, &mut b);
        a.blend(&b, 0.5);
        let angle = a.local[1].r.to_axis_angle().1;
        assert!((angle - std::f32::consts::FRAC_PI_4).abs() < 1e-3);
        let mut c = skel.rest_pose();
        c.blend_masked(&b, 1.0, &skel.subtree_mask("root"));
        assert!(c.local[1].r.abs_diff_eq(b.local[1].r, 1e-5));
        let mut d = skel.rest_pose();
        d.blend_masked(&b, 1.0, &[0.0, 0.0]);
        assert_eq!(d, skel.rest_pose());
    }

    #[test]
    fn root_motion_wraps() {
        let scene = test_rig::arm();
        let skel = Skeleton::from_scene(&scene, 0).unwrap();
        let clip = Clip::bind(&scene.clips[1], &scene, &skel, None);
        assert!((clip.root_motion(0.25, 0.75).x - 1.0).abs() < 1e-4);
        // 0.75 -> 1.25 wraps: 0.5 m before the loop point + 0.5 m after.
        assert!((clip.root_motion(0.75, 1.25).x - 1.0).abs() < 1e-4);
    }

    #[test]
    fn retarget_by_name_map() {
        let scene = test_rig::arm();
        let mut other = test_rig::arm();
        other.nodes[1].name = "forearm".into();
        let skel = Skeleton::from_scene(&other, 0).unwrap();
        assert_eq!(Clip::bind(&scene.clips[0], &scene, &skel, None).tracks.len(), 0);
        let map = BoneMap([("elbow".to_string(), "forearm".to_string())].into());
        assert_eq!(Clip::bind(&scene.clips[0], &scene, &skel, Some(&map)).tracks.len(), 1);
    }
}
