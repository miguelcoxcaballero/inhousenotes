//! Realistic humans (MakeHuman CC0 bodies, clothes, hair) animated with
//! clips retargeted from the Quaternius robot. Every character shares one
//! skeleton layout, so clips are retargeted once and reused.

use std::sync::Arc;

use forja_anim::retarget::{RetargetMap, retarget_all};
use forja_anim::{AnimController, Clip, ControllerDef, Skeleton};
use forja_assets::makehuman::{HumanSpec, build_human};
use forja_math::{Mat4, Quat, Vec3};

use super::{Paths, RenderScene};

#[derive(Default)]
pub struct HumanLibrary {
    clips: Option<Arc<Vec<Clip>>>,
    def: Option<ControllerDef>,
}

pub struct HumanAvatar {
    pub model: usize,
    pub skeleton: Skeleton,
    pub anim: Option<AnimController>,
    pub right_arm: Option<(usize, usize)>,
}

impl HumanLibrary {
    pub fn available(paths: &Paths) -> bool {
        paths.asset("makehuman_base/base.obj").exists() && paths.asset("makehuman_assets").exists()
    }

    /// Builds a character, uploads it to the render scene and gives it an
    /// animation controller.
    pub fn spawn(&mut self, paths: &Paths, rs: &mut RenderScene, spec: &HumanSpec) -> Option<HumanAvatar> {
        let scene = build_human(&paths.asset("makehuman_base"), &paths.asset("makehuman_assets"), spec).map_err(|e| log::warn!("human: {e}")).ok()?;
        let skeleton = Skeleton::from_scene(&scene, 0)?;
        if self.clips.is_none() {
            let robot_path = paths.asset("quaternius_robot_expressive/RobotExpressive.glb");
            let clips = forja_assets::load_gltf(&robot_path).ok().and_then(|robot| {
                let src = Skeleton::from_scene(&robot, 0)?;
                let src_clips = Clip::bind_all(&robot, &src, None);
                Some(retarget_all(&src_clips, &src, &skeleton, &RetargetMap::auto(&src, &skeleton), 30.0))
            });
            self.clips = Some(Arc::new(clips.unwrap_or_default()));
            self.def = std::fs::read_to_string(paths.data.join("anim/mh_humanoid.toml")).ok().and_then(|t| ControllerDef::from_toml(&t).ok());
        }
        let clips = self.clips.as_ref().map(|c| (**c).clone()).unwrap_or_default();
        let anim = self.def.as_ref().and_then(|def| AnimController::new(def, &skeleton, clips).map_err(|e| log::warn!("human animation: {e}")).ok());
        let right_arm = skeleton.find("upperarm01.R").zip(skeleton.find("wrist.R"));
        let model = rs.add_model(Arc::new(scene));
        Some(HumanAvatar { model, skeleton, anim, right_arm })
    }
}

/// Outfits for non-player characters.
pub fn npc_specs() -> Vec<HumanSpec> {
    let base = HumanSpec::casual_male();
    vec![
        HumanSpec {
            name: "worker".into(),
            skin: "skins/young_african_male".into(),
            proxies: vec!["clothes/male_worksuit01".into(), "clothes/shoes06".into(), "hair/short04".into(), "eyebrows/eyebrow010".into(), "eyelashes/eyelashes01".into(), "eyes/high-poly".into()],
            eye_color: "brown".into(),
        },
        HumanSpec {
            name: "tourist".into(),
            skin: "skins/young_caucasian_female".into(),
            proxies: vec!["clothes/female_casualsuit01".into(), "clothes/shoes02".into(), "hair/ponytail01".into(), "eyebrows/eyebrow001".into(), "eyelashes/eyelashes01".into(), "eyes/high-poly".into()],
            eye_color: "blue".into(),
        },
        HumanSpec {
            name: "local".into(),
            skin: "skins/middleage_asian_female".into(),
            proxies: vec!["clothes/female_casualsuit01".into(), "clothes/shoes02".into(), "hair/bob02".into(), "eyebrows/eyebrow001".into(), "eyelashes/eyelashes01".into(), "eyes/high-poly".into()],
            eye_color: "brownlight".into(),
        },
        HumanSpec { name: "commuter".into(), proxies: { let mut p = base.proxies.clone(); p[0] = "clothes/male_casualsuit02".into(); p }, ..base },
    ]
}

/// Bends the right arm so the forearm points along `aim` (model space),
/// blended by `weight`. Runs on model matrices after animation sampling.
pub fn aim_arm(avatar: &HumanAvatar, model: &mut [Mat4], aim: Vec3, weight: f32) {
    let Some((upper, wrist)) = avatar.right_arm else { return };
    if weight <= 0.0 {
        return;
    }
    let pos = |m: &Mat4| m.w_axis.truncate();
    let shoulder = pos(&model[upper]);
    // Rotate the whole arm chain around the shoulder so the wrist lies on
    // the aim ray, then straighten the elbow along it.
    let to_wrist = (pos(&model[wrist]) - shoulder).normalize_or_zero();
    let target = aim.normalize_or_zero();
    if to_wrist == Vec3::ZERO || target == Vec3::ZERO {
        return;
    }
    let q = Quat::IDENTITY.slerp(Quat::from_rotation_arc(to_wrist, target), weight.min(1.0));
    let pivot = Mat4::from_translation(shoulder) * Mat4::from_quat(q) * Mat4::from_translation(-shoulder);
    // Every joint below the upper arm moves with it.
    let below: Vec<usize> = (0..model.len()).filter(|&j| is_descendant(&avatar.skeleton, j, upper)).collect();
    for j in below {
        model[j] = pivot * model[j];
    }
}

fn is_descendant(skel: &Skeleton, mut j: usize, ancestor: usize) -> bool {
    loop {
        if j == ancestor {
            return true;
        }
        match skel.joints[j].parent {
            Some(p) => j = p,
            None => return false,
        }
    }
}
