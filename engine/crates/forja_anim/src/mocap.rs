//! Motion-capture animation sets: BVH takes retargeted onto any humanoid
//! and cut into game clips (looping locomotion in place, idles, one-shots).

use std::path::Path;

use forja_math::Vec3;

use crate::retarget::{RetargetMap, retarget};
use crate::{Clip, Skeleton, cycle};

#[derive(Clone, Debug)]
pub enum Cut {
    /// Seamless cycle with a length in `[min, max]` s after `skip` s.
    Loop { min: f32, max: f32, skip: f32 },
    /// A fixed time range, looped as is (idles, gestures).
    Slice { from: f32, to: f32 },
    /// The whole take, played once.
    Whole,
    /// The stillest `len`-second window of a long take (least root
    /// travel), looped: idles and waiting cut from performances that also
    /// walk around.
    Still { len: f32 },
}

#[derive(Clone, Debug)]
pub struct Take {
    /// BVH file stem in the mocap folder.
    pub file: &'static str,
    /// Clip name used by animation controllers.
    pub name: &'static str,
    pub cut: Cut,
    /// Remove net travel (the character controller moves the body).
    pub in_place: bool,
}

/// The CMU takes fetched as `cmu_mocap` (see `data/assets.toml`).
pub fn cmu_set() -> Vec<Take> {
    vec![
        // 82_08 "stand still": relaxed arms, small weight shifts, feet planted.
        Take { file: "idle_natural", name: "Idle", cut: Cut::Slice { from: 0.5, to: 8.5 }, in_place: true },
        // 35_01: CMU's natural walk (upright, relaxed arms, ~25 cm hand
        // swing). 07_01 swung the hands ~58 cm and read as odd.
        Take { file: "walk_natural", name: "Walking", cut: Cut::Loop { min: 0.9, max: 1.4, skip: 0.5 }, in_place: true },
        Take { file: "walk_slow", name: "WalkSlow", cut: Cut::Loop { min: 1.1, max: 1.8, skip: 0.5 }, in_place: true },
        Take { file: "walk_alt", name: "WalkAlt", cut: Cut::Loop { min: 0.9, max: 1.4, skip: 0.5 }, in_place: true },
        Take { file: "run", name: "Running", cut: Cut::Loop { min: 0.55, max: 0.85, skip: 0.25 }, in_place: true },
        Take { file: "jump", name: "Jump", cut: Cut::Whole, in_place: true },
        Take { file: "punch", name: "Punch", cut: Cut::Slice { from: 1.0, to: 3.0 }, in_place: true },
        Take { file: "talk", name: "Talk", cut: Cut::Still { len: 8.0 }, in_place: true },
        // Impatient waiting: weight shifts and hands on hips (111_28).
        Take { file: "idle_stand", name: "Wait", cut: Cut::Still { len: 8.0 }, in_place: true },
        Take { file: "shoot", name: "Shoot", cut: Cut::Slice { from: 1.0, to: 4.0 }, in_place: true },
    ]
}

/// Loads, retargets and cuts every take found in `dir`. Missing files are
/// skipped. `forward` is the target character's facing in model space.
pub fn load(dir: &Path, takes: &[Take], target: &Skeleton, forward: Vec3, fps: f32) -> Vec<Clip> {
    let mut out = Vec::new();
    let roles = crate::retarget::humanoid_roles(target);
    let feet: Vec<usize> = [crate::retarget::Role::Foot(crate::retarget::Side::L), crate::retarget::Role::Foot(crate::retarget::Side::R)].iter().filter_map(|r| roles.get(r).copied()).collect();
    for take in takes {
        let path = dir.join(format!("{}.bvh", take.file));
        let Ok(scene) = forja_assets::bvh::load_bvh(&path, forja_assets::bvh::CMU_SCALE) else { continue };
        let src = Skeleton::from_scene_any(&scene);
        let Some(raw) = Clip::bind_all(&scene, &src, None).into_iter().next() else { continue };
        let map = RetargetMap::auto(&src, target);
        let mut clip = retarget(&raw, &src, target, &map, fps);
        cycle::face_forward(&mut clip, target, forward);
        clip = match take.cut {
            Cut::Loop { min, max, skip } => cycle::extract_loop(&clip, min, max, skip),
            Cut::Slice { from, to } => cycle::slice(&clip, from, to),
            Cut::Whole => clip,
            Cut::Still { len } => {
                let from = cycle::stillest_window(&clip, target, len);
                cycle::slice(&clip, from, from + len)
            }
        };
        if take.in_place {
            cycle::in_place(&mut clip, target);
        }
        cycle::ground(&mut clip, target, &feet);
        if let Cut::Slice { .. } | Cut::Still { .. } = take.cut {
            // Close the loop of sliced holds too.
            clip = cycle::extract_loop(&clip, clip.duration * 0.85, clip.duration, 0.0);
        }
        clip.name = take.name.to_string();
        out.push(clip);
    }
    out
}
