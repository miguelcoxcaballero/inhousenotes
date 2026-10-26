//! MakeHuman humans (CC0 bodies, clothes, hair) animated with clips
//! retargeted from the Quaternius robot: the fallback when no Rocketbox
//! avatar is downloaded (see `people.rs`). Every character shares one
//! skeleton layout, so clips are retargeted once and reused.

use std::sync::Arc;

use forja_anim::retarget::{RetargetMap, retarget_all};
use forja_anim::{AnimController, Clip, ControllerDef, Skeleton};
use forja_assets::makehuman::{HumanSpec, build_human};
use forja_math::{Mat4, Vec3};

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
    HumanSpec::presets().into_iter().skip(1).collect()
}

/// Bends the right arm so the forearm points along `aim` (model space),
/// blended by `weight`. Runs on model matrices after animation sampling.
pub fn aim_arm(avatar: &HumanAvatar, model: &mut [Mat4], aim: Vec3, weight: f32) {
    if let Some(arm) = avatar.right_arm {
        super::people::aim_arm(&avatar.skeleton, arm, model, aim, weight);
    }
}
