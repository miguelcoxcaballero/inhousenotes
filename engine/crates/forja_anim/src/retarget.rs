//! Retargeting between humanoid skeletons with different proportions, rest
//! poses and bone names.
//!
//! Each mapped bone gets the *direction* the source bone points at every
//! frame (swing), applied on top of the target's own rest orientation, so a
//! T-pose source drives an A-pose target correctly. Bones that are better
//! matched by full orientation (hips, head, hands, feet) copy the source's
//! world-space rotation delta. The root translation is scaled by leg length.
//! The result is baked into an ordinary [`Clip`] for the target skeleton.

use forja_assets::Interpolation;
use forja_math::{Mat4, Quat, Vec3};

use crate::{Clip, Curve, Skeleton, Track};

#[derive(Clone, Debug)]
pub enum BoneRule {
    /// Point the target bone (towards `target_child`) where the source bone
    /// points (towards `source_child`).
    Direction { source: String, source_child: String, target: String, target_child: String },
    /// Copy the world rotation delta from rest.
    Orientation { source: String, target: String },
}

#[derive(Clone, Debug)]
pub struct RetargetMap {
    pub rules: Vec<BoneRule>,
    /// `(source hips, target hips)` for root motion.
    pub root: (String, String),
    /// Target bones that keep their own rest orientation and only take the
    /// source's change from rest. Clavicles: mocap rigs (CMU) model them
    /// sloping up 15-20 degrees, game rigs slope down; copying the absolute
    /// direction shrugs the shoulders by ~30 degrees.
    pub rest_relative: Vec<String>,
}

impl RetargetMap {
    /// Quaternius Robot Expressive -> MakeHuman default skeleton.
    pub fn robot_to_makehuman() -> Self {
        let d = |s: &str, sc: &str, t: &str, tc: &str| BoneRule::Direction { source: s.into(), source_child: sc.into(), target: t.into(), target_child: tc.into() };
        let o = |s: &str, t: &str| BoneRule::Orientation { source: s.into(), target: t.into() };
        let mut rules = vec![
            o("Hips", "root"),
            d("Abdomen", "Torso", "spine05", "spine03"),
            d("Torso", "Neck", "spine03", "neck01"),
            d("Neck", "Head", "neck01", "head"),
            o("Head", "head"),
        ];
        for side in ["L", "R"] {
            rules.extend([
                d(&format!("Shoulder.{side}"), &format!("UpperArm.{side}"), &format!("clavicle.{side}"), &format!("upperarm01.{side}")),
                d(&format!("UpperArm.{side}"), &format!("LowerArm.{side}"), &format!("upperarm01.{side}"), &format!("lowerarm01.{side}")),
                d(&format!("LowerArm.{side}"), &format!("Palm2.{side}"), &format!("lowerarm01.{side}"), &format!("wrist.{side}")),
                o(&format!("Palm2.{side}"), &format!("wrist.{side}")),
                d(&format!("UpperLeg.{side}"), &format!("LowerLeg.{side}"), &format!("upperleg01.{side}"), &format!("lowerleg01.{side}")),
                d(&format!("LowerLeg.{side}"), &format!("Foot.{side}"), &format!("lowerleg01.{side}"), &format!("foot.{side}")),
                o(&format!("Foot.{side}"), &format!("foot.{side}")),
            ]);
        }
        Self { rules, root: ("Hips".into(), "root".into()), rest_relative: vec!["clavicle.L".into(), "clavicle.R".into()] }
    }
}

/// Humanoid body parts recognised from bone names of common rigs
/// (Mixamo, Renderpeople, MakeHuman, Quaternius, UE/Unity humanoids).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Hips,
    Spine,
    Chest,
    Neck,
    Head,
    Clavicle(Side),
    UpperArm(Side),
    LowerArm(Side),
    Hand(Side),
    UpperLeg(Side),
    LowerLeg(Side),
    Foot(Side),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    L,
    R,
}

fn side_of(n: &str) -> Option<Side> {
    let l = n.to_ascii_lowercase();
    // A lone "l"/"r" word, as in 3ds Max Biped ("Bip01 L Thigh").
    let words: Vec<&str> = l.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    if words.len() > 2 && words.contains(&"l") {
        return Some(Side::L);
    }
    if words.len() > 2 && words.contains(&"r") {
        return Some(Side::R);
    }
    let ends = |s: &str| l.ends_with(s);
    if l.contains("left") || ends(".l") || ends("_l") || ends(" l") || l.starts_with("l_") || l.contains("_l_") || l.contains(".l.") || ends("l") && l.len() > 3 && (l.contains("arm") || l.contains("leg") || l.contains("hand") || l.contains("foot")) && !l.contains("right") && l.chars().rev().nth(1).is_some_and(|c| !c.is_alphabetic()) {
        return Some(Side::L);
    }
    if l.contains("right") || ends(".r") || ends("_r") || ends(" r") || l.starts_with("r_") || l.contains("_r_") || l.contains(".r.") {
        return Some(Side::R);
    }
    None
}

fn clean(n: &str) -> String {
    let l = n.to_ascii_lowercase();
    let l = l.rsplit(':').next().unwrap_or(&l).to_string();
    l.replace("left", "").replace("right", "")
}

/// Finds humanoid roles in a skeleton (first match in hierarchy order).
pub fn humanoid_roles(skel: &Skeleton) -> std::collections::HashMap<Role, usize> {
    use std::collections::HashMap;
    let mut roles: HashMap<Role, usize> = HashMap::new();
    let has = |n: &str, keys: &[&str]| keys.iter().any(|k| n.contains(k));
    for &j in &skel.order {
        let raw = &skel.joints[j].name;
        let n = clean(raw);
        if has(&n, &["finger", "thumb", "index", "middle", "ring", "pinky", "toe", "twist", "end", "pole", "ik", "eye", "jaw", "breast", "tongue"]) {
            continue;
        }
        let side = side_of(raw);
        let role = match side {
            Some(s) => {
                if has(&n, &["clavicle", "collar"]) || (has(&n, &["shoulder"]) && !roles.contains_key(&Role::Clavicle(s))) && !has(&n, &["upperarm"]) {
                    Some(Role::Clavicle(s))
                } else if has(&n, &["forearm", "lowerarm", "elbow"]) {
                    Some(Role::LowerArm(s))
                } else if has(&n, &["upperarm", "uparm"]) || (n.contains("arm") && !has(&n, &["fore", "lower"])) {
                    Some(Role::UpperArm(s))
                } else if has(&n, &["hand", "wrist", "palm"]) {
                    Some(Role::Hand(s))
                } else if has(&n, &["upleg", "thigh", "upperleg"]) {
                    Some(Role::UpperLeg(s))
                } else if has(&n, &["calf", "shin", "lowerleg", "knee"]) || (n.contains("leg") && !has(&n, &["up", "upper"])) {
                    Some(Role::LowerLeg(s))
                } else if has(&n, &["foot", "ankle"]) {
                    Some(Role::Foot(s))
                } else {
                    None
                }
            }
            None => {
                if has(&n, &["hips", "pelvis"]) || (n == "root" && !roles.contains_key(&Role::Hips)) {
                    Some(Role::Hips)
                } else if has(&n, &["neck"]) {
                    Some(Role::Neck)
                } else if has(&n, &["head"]) && !has(&n, &["top"]) {
                    Some(Role::Head)
                } else if has(&n, &["chest", "torso", "spine2", "spine02", "upperspine"]) {
                    Some(Role::Chest)
                } else if has(&n, &["spine", "abdomen", "belly"]) {
                    Some(Role::Spine)
                } else {
                    None
                }
            }
        };
        if let Some(r) = role {
            roles.entry(r).or_insert(j);
        }
    }
    // Rigs without a "chest" name (CMU: Spine, Spine1): the spine joint the
    // neck hangs from is the chest.
    if let (None, Some(&neck)) = (roles.get(&Role::Chest), roles.get(&Role::Neck)) {
        if let Some(p) = skel.joints[neck].parent {
            let n = clean(&skel.joints[p].name);
            if Some(&p) != roles.get(&Role::Spine) && has(&n, &["spine", "back"]) {
                roles.insert(Role::Chest, p);
            }
        }
    }
    roles
}

impl RetargetMap {
    /// Builds the rules from humanoid roles found in both skeletons.
    pub fn auto(source: &Skeleton, target: &Skeleton) -> Self {
        let (rs, rt) = (humanoid_roles(source), humanoid_roles(target));
        let name = |sk: &Skeleton, j: usize| sk.joints[j].name.clone();
        let mut rules = Vec::new();
        let dir = |a: Role, b: Role, rules: &mut Vec<BoneRule>| {
            if let (Some(&sa), Some(&sb), Some(&ta), Some(&tb)) = (rs.get(&a), rs.get(&b), rt.get(&a), rt.get(&b)) {
                rules.push(BoneRule::Direction { source: name(source, sa), source_child: name(source, sb), target: name(target, ta), target_child: name(target, tb) });
            }
        };
        let ori = |a: Role, rules: &mut Vec<BoneRule>| {
            if let (Some(&sa), Some(&ta)) = (rs.get(&a), rt.get(&a)) {
                rules.push(BoneRule::Orientation { source: name(source, sa), target: name(target, ta) });
            }
        };
        ori(Role::Hips, &mut rules);
        dir(Role::Spine, Role::Chest, &mut rules);
        dir(Role::Chest, Role::Neck, &mut rules);
        dir(Role::Neck, Role::Head, &mut rules);
        ori(Role::Head, &mut rules);
        for s in [Side::L, Side::R] {
            dir(Role::Clavicle(s), Role::UpperArm(s), &mut rules);
            dir(Role::UpperArm(s), Role::LowerArm(s), &mut rules);
            dir(Role::LowerArm(s), Role::Hand(s), &mut rules);
            ori(Role::Hand(s), &mut rules);
            dir(Role::UpperLeg(s), Role::LowerLeg(s), &mut rules);
            dir(Role::LowerLeg(s), Role::Foot(s), &mut rules);
            ori(Role::Foot(s), &mut rules);
        }
        let root = (rs.get(&Role::Hips).map(|j| name(source, *j)).unwrap_or_default(), rt.get(&Role::Hips).map(|j| name(target, *j)).unwrap_or_default());
        let rest_relative = [Side::L, Side::R].iter().filter_map(|s| rt.get(&Role::Clavicle(*s))).map(|j| name(target, *j)).collect();
        Self { rules, root, rest_relative }
    }
}

fn rot(m: &Mat4) -> Quat {
    Quat::from_mat4(m).normalize()
}

fn pos(m: &Mat4) -> Vec3 {
    m.w_axis.truncate()
}

/// Bakes `clip` (bound to `source`) onto `target` at `fps`.
pub fn retarget(clip: &Clip, source: &Skeleton, target: &Skeleton, map: &RetargetMap, fps: f32) -> Clip {
    let frames = ((clip.duration * fps).ceil() as usize).max(1) + 1;
    let dt = clip.duration / (frames - 1) as f32;
    let resolve = |name: &str, skel: &Skeleton| skel.find(name);
    // Resolve rules once.
    enum R {
        Dir(usize, usize, usize, usize),
        Ori(usize, usize),
    }
    let rules: Vec<R> = map
        .rules
        .iter()
        .filter_map(|r| match r {
            BoneRule::Direction { source: s, source_child: sc, target: t, target_child: tc } => {
                Some(R::Dir(resolve(s, source)?, resolve(sc, source)?, resolve(t, target)?, resolve(tc, target)?))
            }
            BoneRule::Orientation { source: s, target: t } => Some(R::Ori(resolve(s, source)?, resolve(t, target)?)),
        })
        .collect();
    let mut src_rest = Vec::new();
    source.model_matrices(&source.rest_pose(), &mut src_rest);
    let mut tgt_rest = Vec::new();
    target.model_matrices(&target.rest_pose(), &mut tgt_rest);
    let root = (resolve(&map.root.0, source), resolve(&map.root.1, target));
    // Leg-length ratio from hip heights above the lowest joint.
    let floor = |m: &[Mat4]| m.iter().map(|x| pos(x).y).fold(f32::MAX, f32::min);
    let ratio = match root {
        (Some(s), Some(t)) => ((pos(&tgt_rest[t]).y - floor(&tgt_rest)) / (pos(&src_rest[s]).y - floor(&src_rest)).max(1e-3)).abs(),
        _ => 1.0,
    };
    let touched: Vec<usize> = rules.iter().map(|r| match r {
        R::Dir(_, _, t, _) | R::Ori(_, t) => *t,
    }).collect();
    // Target world rotations that make each mapped bone point where the
    // source bone points, solved parents first.
    let solve_directions = |src_m: &[Mat4]| -> Vec<Quat> {
        let mut world_rot: Vec<Quat> = vec![Quat::IDENTITY; target.len()];
        for &i in &target.order {
            let parent = target.joints[i].parent.map(|p| world_rot[p]).unwrap_or_else(|| rot(&target.root_parent));
            let mut w = parent * target.joints[i].rest.r;
            for r in &rules {
                match *r {
                    R::Dir(s, sc, tt, tc) if tt == i => {
                        let desired = (pos(&src_m[sc]) - pos(&src_m[s])).normalize_or_zero();
                        let rest_world = rot(&tgt_rest[i]);
                        let rest_dir_local = rest_world.inverse() * (pos(&tgt_rest[tc]) - pos(&tgt_rest[i]));
                        let current = (w * rest_dir_local).normalize_or_zero();
                        if current != Vec3::ZERO && desired != Vec3::ZERO {
                            w = Quat::from_rotation_arc(current, desired) * w;
                        }
                    }
                    R::Ori(s, tt) if tt == i => {
                        let delta = rot(&src_m[s]) * rot(&src_rest[s]).inverse();
                        w = delta * rot(&tgt_rest[i]);
                    }
                    _ => {}
                }
            }
            world_rot[i] = w.normalize();
        }
        world_rot
    };
    // 1) Pose the target like the source's rest pose (T vs A pose), using
    //    bone directions only. 2) Every frame, apply each source bone's full
    //    world rotation change since its rest (swing and twist) on top of
    //    that aligned pose. Twist then follows the performer instead of
    //    being left arbitrary, which is what kept forearms and hands
    //    wrung out.
    let mut aligned = solve_directions(&src_rest);
    // Rest-relative bones keep the target's own rest pose; their children
    // stay aligned to the source (direction rules are absolute).
    for name in &map.rest_relative {
        if let Some(i) = resolve(name, target) {
            aligned[i] = rot(&tgt_rest[i]);
        }
    }
    let source_of: Vec<Option<usize>> = (0..target.len())
        .map(|i| {
            rules.iter().find_map(|r| match *r {
                R::Dir(s, _, t, _) | R::Ori(s, t) if t == i => Some(s),
                _ => None,
            })
        })
        .collect();
    let mut rotations: Vec<Vec<Quat>> = vec![Vec::with_capacity(frames); target.len()];
    let mut root_t: Vec<Vec3> = Vec::with_capacity(frames);
    let mut src_pose = source.rest_pose();
    let mut src_m = Vec::new();
    // Floors: the source's lowest joint over the whole take, the target's
    // lowest joint at rest.
    let src_floor = (0..frames)
        .map(|f| {
            src_pose.local.clone_from(&source.rest_pose().local);
            clip.sample(f as f32 * dt, false, &mut src_pose);
            source.model_matrices(&src_pose, &mut src_m);
            floor(&src_m)
        })
        .fold(f32::MAX, f32::min);
    let tgt_floor = floor(&tgt_rest);
    for f in 0..frames {
        let t = f as f32 * dt;
        src_pose.local.clone_from(&source.rest_pose().local);
        clip.sample(t, false, &mut src_pose);
        source.model_matrices(&src_pose, &mut src_m);
        let mut world_rot: Vec<Quat> = vec![Quat::IDENTITY; target.len()];
        let mut local_rot: Vec<Quat> = target.joints.iter().map(|j| j.rest.r).collect();
        for &i in &target.order {
            let parent = target.joints[i].parent.map(|p| world_rot[p]).unwrap_or_else(|| rot(&target.root_parent));
            let w = match source_of[i] {
                Some(s) => (rot(&src_m[s]) * rot(&src_rest[s]).inverse()) * aligned[i],
                // Unmapped bones (fingers, twist helpers) follow their parent.
                None => parent * target.joints[i].rest.r,
            };
            world_rot[i] = w.normalize();
            local_rot[i] = (parent.inverse() * world_rot[i]).normalize();
        }
        for (k, q) in local_rot.into_iter().enumerate() {
            rotations[k].push(q);
        }
        if let (Some(s), Some(t)) = root {
            let mut delta = (pos(&src_m[s]) - pos(&src_rest[s])) * ratio;
            // Height: the source hips above the source's floor, scaled,
            // above the target's floor, so the feet stay on the ground
            // whatever either rig's rest pose was.
            delta.y = tgt_floor + (pos(&src_m[s]).y - src_floor) * ratio - pos(&tgt_rest[t]).y;
            // Back into the target root's parent space.
            let parent_inv = target.joints[t].parent.map(|_| Quat::IDENTITY).unwrap_or_else(|| rot(&target.root_parent).inverse());
            root_t.push(target.joints[t].rest.t + parent_inv * delta);
        }
    }
    let times: Vec<f32> = (0..frames).map(|f| f as f32 * dt).collect();
    let mut tracks = Vec::new();
    for (k, qs) in rotations.into_iter().enumerate() {
        // Bones that never leave their rest rotation need no track.
        if !touched.contains(&k) && qs.iter().all(|q| q.abs_diff_eq(target.joints[k].rest.r, 1e-4)) {
            continue;
        }
        let mut tr = Track { joint: k, ..Default::default() };
        tr.rotation = Some(Curve { times: times.clone(), values: qs, interpolation: Interpolation::Linear });
        if root.1 == Some(k) && !root_t.is_empty() {
            tr.translation = Some(Curve { times: times.clone(), values: root_t.clone(), interpolation: Interpolation::Linear });
        }
        tracks.push(tr);
    }
    let coverage = tracks.len() as f32 / target.len().max(1) as f32;
    Clip { name: clip.name.clone(), duration: clip.duration, tracks, coverage, root_joint: root.1 }
}

/// Retargets every clip in `clips`.
pub fn retarget_all(clips: &[Clip], source: &Skeleton, target: &Skeleton, map: &RetargetMap, fps: f32) -> Vec<Clip> {
    clips.iter().map(|c| retarget(c, source, target, map, fps)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_and_roles_from_common_names() {
        assert_eq!(side_of("mixamorig:LeftForeArm"), Some(Side::L));
        assert_eq!(side_of("UpperArm.R"), Some(Side::R));
        assert_eq!(side_of("lowerarm01.L"), Some(Side::L));
        assert_eq!(side_of("Spine"), None);
        assert_eq!(side_of("Bip01 L Thigh"), Some(Side::L));
        assert_eq!(side_of("Bip01 R Forearm"), Some(Side::R));
    }
}
