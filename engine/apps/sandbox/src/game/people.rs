//! Realistic people: Rocketbox avatars (Microsoft, MIT) driven by CMU
//! motion capture through an animation controller
//! (`data/anim/biped_humanoid.toml`), with procedural life on top: blinks,
//! saccades, head and eye look-at and breathing.
//!
//! Every avatar found in `assets/downloaded/rocketbox_*` is loaded,
//! retargeted and uploaded once. All the people of one type share its GPU
//! model and its clip set, so a person costs one controller and one joint
//! palette.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use forja_anim::retarget::{RetargetMap, Role, Side, humanoid_roles, retarget};
use forja_anim::{AnimController, Clip, ControllerDef, Skeleton, mocap};
use forja_assets::{SceneData, TextureData};
use forja_math::{Mat4, Quat, Vec3, smoothing};

use super::{Paths, RenderItem, RenderScene};

/// Walk cycles a person may use (crowd variety); the first is the default.
pub const WALKS: [&str; 3] = ["Walking", "WalkAlt", "WalkSlow"];

/// One avatar model with its skeleton, clips and measured gait.
pub struct AvatarType {
    pub name: String,
    pub model: usize,
    pub skeleton: Skeleton,
    pub clips: Arc<[Clip]>,
    /// Natural ground speed of each locomotion clip (m/s).
    pub speeds: HashMap<String, f32>,
    pub rig: Rig,
}

impl AvatarType {
    pub fn gait(&self, walk: &str) -> Gait {
        let walk_speed = self.speeds.get(walk).or(self.speeds.get(WALKS[0])).copied().unwrap_or(1.2);
        let run = self.speeds.get("Running").copied().unwrap_or(3.0).max(walk_speed + 0.5);
        Gait { walk: walk_speed, run }
    }
}

/// Avatar files and loaded types.
#[derive(Default)]
pub struct PeopleLibrary {
    /// By file and texture size limit.
    loaded: HashMap<(PathBuf, u32), Option<Arc<AvatarType>>>,
    def: Option<Option<ControllerDef>>,
}

impl PeopleLibrary {
    /// Avatar files in the downloads: `rocketbox_*/Export/*.fbx` (the
    /// `_facial` variants are skipped), sorted by name.
    pub fn catalogue(paths: &Paths) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(dirs) = std::fs::read_dir(&paths.downloaded) else { return out };
        for d in dirs.flatten() {
            let name = d.file_name().to_string_lossy().to_string();
            if !name.starts_with("rocketbox_") {
                continue;
            }
            let Ok(files) = std::fs::read_dir(d.path().join("Export")) else { continue };
            for f in files.flatten() {
                let p = f.path();
                let stem = p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("fbx")) && !stem.ends_with("_facial") {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }

    pub fn available(paths: &Paths) -> bool {
        paths.asset("cmu_mocap/walk_natural.bvh").exists() && !Self::catalogue(paths).is_empty()
    }

    fn controller(&mut self, paths: &Paths) -> Option<&ControllerDef> {
        self.def
            .get_or_insert_with(|| {
                std::fs::read_to_string(paths.data.join("anim/biped_humanoid.toml"))
                    .map_err(|e| e.to_string())
                    .and_then(|t| ControllerDef::from_toml(&t))
                    .map_err(|e| log::warn!("biped_humanoid.toml: {e}"))
                    .ok()
            })
            .as_ref()
    }

    /// Loads the avatars at `files` (once each, in parallel) and uploads
    /// them. Textures are limited to `max_texture` pixels (crowds do not
    /// need 2K skin up close; the player does).
    pub fn load(&mut self, paths: &Paths, rs: &mut RenderScene, files: &[PathBuf], max_texture: u32) -> Vec<Arc<AvatarType>> {
        let todo: Vec<&PathBuf> = files.iter().filter(|f| !self.loaded.contains_key(&((*f).clone(), max_texture))).collect();
        if todo.is_empty() {
            return files.iter().filter_map(|f| self.loaded.get(&(f.clone(), max_texture)).cloned().flatten()).collect();
        }
        let mocap_dir = paths.asset("cmu_mocap");
        let death = death_clip(paths);
        // A few at a time: each avatar holds ~200 MB of 2K textures until
        // they are shrunk.
        let mut built: Vec<(PathBuf, Option<Prepared>)> = Vec::new();
        for chunk in todo.chunks(3) {
            built.extend(std::thread::scope(|s| {
                let jobs: Vec<_> = chunk.iter().map(|f| s.spawn(|| ((*f).clone(), prepare(f, &mocap_dir, death.as_ref(), max_texture)))).collect();
                jobs.into_iter().filter_map(|j| j.join().ok()).collect::<Vec<_>>()
            }));
        }
        for (path, prepared) in built {
            let kind = prepared.map(|p| {
                let model = rs.add_model(Arc::new(p.scene));
                Arc::new(AvatarType { name: p.name, model, skeleton: p.skeleton, clips: p.clips, speeds: p.speeds, rig: p.rig })
            });
            self.loaded.insert((path, max_texture), kind);
        }
        files.iter().filter_map(|f| self.loaded.get(&(f.clone(), max_texture)).cloned().flatten()).collect()
    }

    /// A new person of type `kind`. `walk` picks the walk cycle (see
    /// [`WALKS`]); `seed` varies blinks, breathing and timing.
    pub fn spawn(&mut self, paths: &Paths, kind: &Arc<AvatarType>, walk: &str, seed: u64) -> Option<Person> {
        let def = self.controller(paths)?;
        let walk = if kind.speeds.contains_key(walk) { walk } else { WALKS[0] };
        let def = with_walk(def, walk);
        let anim = AnimController::new(&def, &kind.skeleton, kind.clips.clone()).map_err(|e| log::warn!("{}: {e}", kind.name)).ok()?;
        Some(Person { kind: kind.clone(), anim, gait: kind.gait(walk), life: Life::new(seed), model: Vec::new() })
    }
}

/// The controller definition with its walk cycle replaced by `walk`.
fn with_walk(def: &ControllerDef, walk: &str) -> ControllerDef {
    let mut def = def.clone();
    for s in &mut def.states {
        if let Some(b) = &mut s.blend {
            for p in &mut b.points {
                if p.1 == WALKS[0] {
                    p.1 = walk.to_string();
                }
            }
        }
    }
    def
}

struct Prepared {
    name: String,
    scene: SceneData,
    skeleton: Skeleton,
    clips: Arc<[Clip]>,
    speeds: HashMap<String, f32>,
    rig: Rig,
}

/// The Quaternius robot's death fall with its skeleton (motion capture
/// has no fall), to retarget onto every avatar.
fn death_clip(paths: &Paths) -> Option<(Skeleton, Clip)> {
    let robot = forja_assets::load_gltf(&paths.asset("quaternius_robot_expressive/RobotExpressive.glb")).ok()?;
    let skel = Skeleton::from_scene(&robot, 0)?;
    let clip = Clip::bind_all(&robot, &skel, None).into_iter().find(|c| c.name == "Death")?;
    Some((skel, clip))
}

fn prepare(path: &Path, mocap_dir: &Path, death: Option<&(Skeleton, Clip)>, max_texture: u32) -> Option<Prepared> {
    let t = std::time::Instant::now();
    let mut scene = forja_assets::load_model(path).map_err(|e| log::warn!("{}: {e}", path.display())).ok()?;
    forja_assets::character::make_photoreal(&mut scene);
    shrink_textures(&mut scene, max_texture);
    let skeleton = Skeleton::from_scene(&scene, 0)?;
    let mut clips = mocap::load(mocap_dir, &mocap::cmu_set(), &skeleton, Vec3::Z, 30.0);
    if !clips.iter().any(|c| c.name == WALKS[0]) {
        log::warn!("{}: no walk clip in {}", path.display(), mocap_dir.display());
        return None;
    }
    // A fall for when people are shot; standing still if the robot is missing.
    let fall = match death {
        Some((src, clip)) => retarget(clip, src, &skeleton, &RetargetMap::auto(src, &skeleton), 30.0),
        None => clips.iter().find(|c| c.name == "Idle").cloned().unwrap_or_else(|| clips[0].clone()),
    };
    clips.push(Clip { name: "Death".into(), ..fall });
    let feet = feet(&skeleton);
    let speeds: HashMap<String, f32> = clips.iter().filter(|c| WALKS.contains(&c.name.as_str()) || c.name == "Running").map(|c| (c.name.clone(), stride_speed(c, &skeleton, feet))).collect();
    let rig = Rig::new(&skeleton);
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    log::info!("person {name}: {} clips, walk {:.2} m/s, run {:.2} m/s, in {:.2?}", clips.len(), speeds.get(WALKS[0]).unwrap_or(&0.0), speeds.get("Running").unwrap_or(&0.0), t.elapsed());
    Some(Prepared { name, scene, skeleton, clips: clips.into(), speeds, rig })
}

fn feet(skel: &Skeleton) -> [Option<usize>; 2] {
    let roles = humanoid_roles(skel);
    [roles.get(&Role::Foot(Side::L)).copied(), roles.get(&Role::Foot(Side::R)).copied()]
}

/// Ground speed a looping in-place cycle was captured at: the speed at
/// which the supporting foot slides backwards (median over the frames
/// where one foot is planted). Playing the clip at `speed / stride_speed`
/// keeps that foot still on the ground.
pub fn stride_speed(clip: &Clip, skel: &Skeleton, feet: [Option<usize>; 2]) -> f32 {
    let [Some(l), Some(r)] = feet else { return 0.0 };
    let n = 120;
    let dt = clip.duration / n as f32;
    let mut m = Vec::new();
    let mut pose = skel.rest_pose();
    let mut pos = Vec::with_capacity(n + 1);
    for k in 0..=n {
        pose.local.clone_from(&skel.rest_pose().local);
        clip.sample(dt * k as f32, true, &mut pose);
        skel.model_matrices(&pose, &mut m);
        pos.push([m[l].w_axis.truncate(), m[r].w_axis.truncate()]);
    }
    let floor = pos.iter().flat_map(|p| [p[0].y, p[1].y]).fold(f32::MAX, f32::min);
    let mut speeds: Vec<f32> = Vec::new();
    for k in 0..n {
        for f in 0..2 {
            let (a, b) = (pos[k][f], pos[k + 1][f]);
            // Planted: the lower foot, within 3 cm of the floor, in both frames.
            let other = 1 - f;
            if a.y <= pos[k][other].y && b.y <= pos[k + 1][other].y && a.y < floor + 0.03 && b.y < floor + 0.03 {
                speeds.push(Vec3::new(b.x - a.x, 0.0, b.z - a.z).length() / dt);
            }
        }
    }
    if speeds.is_empty() {
        return 0.0;
    }
    speeds.sort_by(f32::total_cmp);
    speeds[speeds.len() / 2]
}

/// Halves textures larger than `max` until they fit (box filter). Data
/// maps (normals, specular) go to half that unless `max` is full size.
fn shrink_textures(scene: &mut SceneData, max: u32) {
    for t in &mut scene.textures {
        let max = if t.srgb || max >= 2048 { max } else { max / 2 };
        while t.width > max || t.height > max {
            *t = half(t);
        }
    }
}

fn half(t: &TextureData) -> TextureData {
    let (w, h) = ((t.width / 2).max(1), (t.height / 2).max(1));
    let (sw, sh) = (t.width as usize, t.height as usize);
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..4 {
                let px = |dx: usize, dy: usize| t.rgba8[(((y * 2 + dy).min(sh - 1)) * sw + (x * 2 + dx).min(sw - 1)) * 4 + c] as u32;
                out[(y * w as usize + x) * 4 + c] = ((px(0, 0) + px(1, 0) + px(0, 1) + px(1, 1) + 2) / 4) as u8;
            }
        }
    }
    TextureData { name: t.name.clone(), width: w, height: h, rgba8: out.into(), srgb: t.srgb }
}

/// How fast a body moves, mapped onto the controller's gait parameters
/// (see `data/anim/biped_humanoid.toml`).
#[derive(Clone, Copy, Debug)]
pub struct Gait {
    /// Natural speeds of the walk and run cycles (m/s).
    pub walk: f32,
    pub run: f32,
}

impl Gait {
    pub fn drive(&self, anim: &mut AnimController, speed: f32) {
        let g = ((speed - self.walk) / (self.run - self.walk)).clamp(0.0, 1.0);
        let natural = self.walk + (self.run - self.walk) * g;
        anim.set("moving", if speed > 0.15 { 1.0 } else { 0.0 });
        anim.set("gait", 1.0 + g);
        anim.set("stride", (speed / natural).clamp(0.5, 1.8));
    }
}

/// Joints that procedural life moves, found by role and name on any
/// humanoid rig (Biped, MakeHuman, Mixamo-like).
pub struct Rig {
    chest: Option<Chain>,
    neck: Option<Chain>,
    head: Option<Chain>,
    clavicles: [Option<Chain>; 2],
    eyes: [Option<Chain>; 2],
    /// `[side][top, bottom]` eyelid joints.
    lids: [[Option<Chain>; 2]; 2],
    /// Face forward and left in the head joint's frame.
    head_fwd: Vec3,
    head_left: Vec3,
    eye_fwd: [Vec3; 2],
    pub right_arm: Option<(usize, usize)>,
    pub right_hand: Option<usize>,
}

/// A joint and every joint below it (rotated together).
struct Chain {
    joint: usize,
    subtree: Vec<usize>,
}

impl Rig {
    pub fn new(skel: &Skeleton) -> Self {
        let roles = humanoid_roles(skel);
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); skel.len()];
        for (i, j) in skel.joints.iter().enumerate() {
            if let Some(p) = j.parent {
                children[p].push(i);
            }
        }
        let chain = |joint: usize| {
            let mut subtree = vec![joint];
            let mut k = 0;
            while k < subtree.len() {
                subtree.extend(children[subtree[k]].iter().copied());
                k += 1;
            }
            Chain { joint, subtree }
        };
        let role = |r: Role| roles.get(&r).copied().map(chain);
        let named = |side: char, part: &str| {
            skel.joints.iter().position(|j| {
                let n = j.name.to_ascii_lowercase().replace([' ', '_', '.', '-'], "");
                let sided = n.contains(&format!("{side}eye")) || n.ends_with(side) && n.contains("eye");
                sided && n.contains(part) && !n.contains("brow") && !n.contains("lash")
            })
        };
        let eye = |side: char| {
            skel.joints.iter().position(|j| {
                let n = j.name.to_ascii_lowercase().replace([' ', '_', '.', '-'], "");
                (n.ends_with(&format!("{side}eye")) || n == format!("eye{side}")) && !n.contains("blink") && !n.contains("lid")
            })
        };
        let mut rest = Vec::new();
        skel.model_matrices(&skel.rest_pose(), &mut rest);
        let local = |j: Option<usize>, v: Vec3| j.map(|j| rest[j].inverse().transform_vector3(v).normalize_or_zero()).unwrap_or(v);
        let head = roles.get(&Role::Head).copied();
        let eyes = [eye('l'), eye('r')];
        Self {
            chest: role(Role::Chest).or_else(|| role(Role::Spine)),
            neck: role(Role::Neck),
            head: head.map(chain),
            clavicles: [role(Role::Clavicle(Side::L)), role(Role::Clavicle(Side::R))],
            eyes: eyes.map(|e| e.map(chain)),
            lids: ['l', 'r'].map(|s| [named(s, "blinktop").or(named(s, "uplid")).map(chain), named(s, "blinkbottom").or(named(s, "lowlid")).map(chain)]),
            head_fwd: local(head, Vec3::Z),
            head_left: local(head, Vec3::X),
            eye_fwd: eyes.map(|e| local(e, Vec3::Z)),
            right_arm: roles.get(&Role::UpperArm(Side::R)).copied().zip(roles.get(&Role::Hand(Side::R)).copied()),
            right_hand: roles.get(&Role::Hand(Side::R)).copied(),
        }
    }
}

/// Rotates the joints of `subtree` rigidly by `q` about `pivot` (model space).
fn rotate(model: &mut [Mat4], subtree: &[usize], pivot: Vec3, q: Quat) {
    if q.w.abs() > 0.999_999 {
        return;
    }
    let r = Mat4::from_translation(pivot) * Mat4::from_quat(q) * Mat4::from_translation(-pivot);
    for &j in subtree {
        model[j] = r * model[j];
    }
}

fn pos(m: &Mat4) -> Vec3 {
    m.w_axis.truncate()
}

/// Direction with a yaw (about +Y, from +Z) and a pitch (up positive).
fn dir(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos())
}

fn yaw_pitch(d: Vec3) -> (f32, f32) {
    (d.x.atan2(d.z), d.y.clamp(-1.0, 1.0).asin())
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Small deterministic generator (SplitMix64) for per-person variety.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn f(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }

    pub fn index(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

/// Procedural layers that make a body read as alive.
pub struct Life {
    rng: Rng,
    /// Seconds to the next blink, and time into the current one.
    blink_in: f32,
    blink: Option<f32>,
    blink_len: f32,
    double_blink: bool,
    breath: f32,
    breath_rate: f32,
    /// 0 rested .. 1 out of breath (after running).
    exertion: f32,
    /// Smoothed gaze direction (body frame yaw, pitch) and its weight.
    gaze: (f32, f32),
    gaze_w: f32,
    /// Eye direction in the head frame, smoothed, plus a saccade offset.
    eye_local: Option<Vec3>,
    saccade: (f32, f32),
    saccade_in: f32,
    /// How much of the life to run: 1 full, 0 body only (far away).
    pub detail: f32,
    /// Holds the eyelids at this closure (review renders).
    pub lid_override: Option<f32>,
}

impl Life {
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        Self {
            blink_in: rng.range(0.3, 4.0),
            blink: None,
            blink_len: 0.15,
            double_blink: false,
            breath: rng.range(0.0, std::f32::consts::TAU),
            breath_rate: rng.range(0.22, 0.3),
            exertion: 0.0,
            gaze: (0.0, -0.05),
            gaze_w: 0.0,
            eye_local: None,
            saccade: (0.0, 0.0),
            saccade_in: rng.range(0.2, 1.5),
            detail: 1.0,
            lid_override: None,
            rng,
        }
    }

    /// Eyelid closure 0..1 for the current blink: a fast close (~50 ms),
    /// a short hold and a slower opening.
    pub fn blink_amount(&self) -> f32 {
        let Some(t) = self.blink else { return 0.0 };
        let (close, hold) = (self.blink_len * 0.33, self.blink_len * 0.14);
        let s = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        if t < close {
            s(t / close)
        } else if t < close + hold {
            1.0
        } else {
            1.0 - s((t - close - hold) / (self.blink_len - close - hold))
        }
    }

    fn start_blink(&mut self) {
        self.blink = Some(0.0);
        self.blink_len = self.rng.range(0.13, 0.18);
    }

    fn step_timers(&mut self, dt: f32, speed: f32) {
        // Blinks every 2-6 s, sometimes doubled.
        match self.blink {
            Some(t) if t + dt >= self.blink_len => {
                self.blink = None;
                if self.double_blink {
                    self.double_blink = false;
                    self.blink_in = 0.12;
                } else {
                    self.blink_in = self.rng.range(2.0, 6.0);
                    self.double_blink = self.rng.chance(0.12);
                }
            }
            Some(t) => self.blink = Some(t + dt),
            None => {
                self.blink_in -= dt;
                if self.blink_in <= 0.0 {
                    self.start_blink();
                }
            }
        }
        // Breathing quickens and deepens after running, then settles.
        let target = ((speed - 2.2) / 2.0).clamp(0.0, 1.0);
        let k = if target > self.exertion { 0.25 } else { 0.06 };
        self.exertion += (target - self.exertion) * (dt * k).min(1.0);
        self.breath += dt * std::f32::consts::TAU * self.breath_rate * (1.0 + 1.4 * self.exertion);
        // Saccades: the eyes jump to a new spot every 0.3-2.5 s.
        self.saccade_in -= dt;
        if self.saccade_in <= 0.0 {
            self.saccade_in = self.rng.range(0.3, 2.5);
            self.saccade = (self.rng.range(-0.06, 0.06), self.rng.range(-0.04, 0.03));
        }
    }

    /// Applies breathing, look-at, eye aim and eyelids to the animated
    /// model matrices. `target` is a point of interest in model space.
    pub fn apply(&mut self, rig: &Rig, model: &mut [Mat4], target: Option<Vec3>, dt: f32, speed: f32) {
        self.step_timers(dt, speed);
        if self.detail <= 0.0 {
            return;
        }
        // Breathing: the chest rises and the shoulders lift a little.
        let b = self.breath.sin() * (1.0 + 0.8 * self.exertion);
        if let Some(c) = &rig.chest {
            let p = pos(&model[c.joint]);
            rotate(model, &c.subtree, p, Quat::from_rotation_x(-0.008 * b));
        }
        for (k, c) in rig.clavicles.iter().enumerate() {
            if let Some(c) = c {
                let s = if k == 0 { 1.0 } else { -1.0 };
                let p = pos(&model[c.joint]);
                rotate(model, &c.subtree, p, Quat::from_rotation_z(s * 0.012 * b));
            }
        }
        let Some(head) = &rig.head else { return };
        // Head look-at: the neck and head share the turn; targets behind
        // the body are ignored rather than wrung round.
        let eye_mid = match &rig.eyes {
            [Some(l), Some(r)] => (pos(&model[l.joint]) + pos(&model[r.joint])) * 0.5,
            _ => pos(&model[head.joint]) + Vec3::Y * 0.09,
        };
        let fwd = model[head.joint].transform_vector3(rig.head_fwd).normalize_or(Vec3::Z);
        let (want, want_w) = match target {
            Some(t) => {
                let (y, p) = yaw_pitch((t - eye_mid).normalize_or(fwd));
                if y.abs() > 1.75 || (t - eye_mid).length() < 0.2 { (self.gaze, 0.0) } else { ((y.clamp(-1.2, 1.2), p.clamp(-0.6, 0.45)), 1.0) }
            }
            None => (self.gaze, 0.0),
        };
        // A large gaze shift often comes with a blink.
        if want_w > 0.0 && self.gaze_w > 0.5 && (wrap(want.0 - self.gaze.0).abs() > 0.5) && self.blink.is_none() && self.rng.chance(0.5) {
            self.start_blink();
        }
        let k = smoothing(4.5, dt);
        self.gaze.0 += wrap(want.0 - self.gaze.0) * k;
        self.gaze.1 += (want.1 - self.gaze.1) * k;
        self.gaze_w += (want_w - self.gaze_w) * smoothing(2.5, dt);
        let w = self.gaze_w * self.detail;
        if w > 1e-3 {
            let (fy, fp) = yaw_pitch(fwd);
            let (dy, dp) = (wrap(self.gaze.0 - fy) * w, (self.gaze.1 - fp) * w);
            // Yaw about the vertical, then pitch about the face's own
            // left axis: turning never rolls the head over.
            let turn = |model: &mut [Mat4], c: &Chain, k: f32| {
                let f = model[head.joint].transform_vector3(rig.head_fwd).normalize_or(Vec3::Z);
                let (cy, cp) = yaw_pitch(f);
                let (ty, tp) = (fy + dy * k, fp + dp * k);
                let yaw = Quat::from_rotation_y(wrap(ty - cy));
                let side = Vec3::Y.cross(yaw * f).normalize_or(Vec3::X);
                let q = Quat::from_axis_angle(side, -(tp - cp)) * yaw;
                let p = pos(&model[c.joint]);
                rotate(model, &c.subtree, p, q);
            };
            if let Some(n) = &rig.neck {
                turn(model, n, 0.4);
            }
            turn(model, head, 1.0);
        }
        // Eyes: converge on the target, limited to what eyes can do, with
        // saccades; stored in the head frame so they ride with the head.
        let head_m = model[head.joint];
        let (_, head_rot, _) = head_m.to_scale_rotation_translation();
        let face = head_m.transform_vector3(rig.head_fwd).normalize_or(Vec3::Z);
        let look = match target {
            Some(t) if want_w > 0.0 => (t - eye_mid).normalize_or(face),
            _ => face,
        };
        let (ly, lp) = yaw_pitch(look);
        let (sy, sp) = self.saccade;
        let look = dir(ly + sy, lp + sp);
        // At most ~28 degrees from the face direction.
        let angle = face.angle_between(look);
        let look = if angle > 0.49 { Quat::IDENTITY.slerp(Quat::from_rotation_arc(face, look), 0.49 / angle) * face } else { look };
        let want_local = head_rot.inverse() * look;
        let el = self.eye_local.get_or_insert(want_local);
        *el = el.lerp(want_local, smoothing(28.0, dt)).normalize_or(want_local);
        let eye_dir = head_rot * *el;
        for (side, e) in rig.eyes.iter().enumerate() {
            if let Some(e) = e {
                let now = model[e.joint].transform_vector3(rig.eye_fwd[side]).normalize_or(face);
                rotate(model, &e.subtree, pos(&model[e.joint]), Quat::from_rotation_arc(now, eye_dir));
            }
        }
        // Eyelids rotate about the eyeball: blinks, and the upper lid
        // follows the gaze down.
        let blink = self.lid_override.unwrap_or_else(|| self.blink_amount());
        let left = head_m.transform_vector3(rig.head_left).normalize_or(Vec3::X);
        let up = face.cross(left).normalize_or(Vec3::Y);
        let gaze_pitch = eye_dir.dot(up).clamp(-1.0, 1.0).asin();
        let top = blink * 0.72 - gaze_pitch * 0.55;
        let bottom = -blink * 0.17 - gaze_pitch * 0.25;
        for side in 0..2 {
            let Some(e) = &rig.eyes[side] else { continue };
            let pivot = pos(&model[e.joint]);
            for (lid, angle) in rig.lids[side].iter().zip([top, bottom]) {
                if let Some(l) = lid {
                    rotate(model, &l.subtree, pivot, Quat::from_axis_angle(left, angle));
                }
            }
        }
    }
}

/// A person: an avatar type, its animation controller and its life.
pub struct Person {
    pub kind: Arc<AvatarType>,
    pub anim: AnimController,
    pub gait: Gait,
    pub life: Life,
    model: Vec<Mat4>,
}

impl Person {
    /// Steps the animation for a body moving at `speed` m/s placed at
    /// `transform`; `look` is a world-space point of interest.
    pub fn update(&mut self, dt: f32, speed: f32, transform: &Mat4, look: Option<Vec3>) {
        self.gait.drive(&mut self.anim, speed);
        self.anim.update(dt);
        self.kind.skeleton.model_matrices(self.anim.pose(), &mut self.model);
        let target = look.map(|p| transform.inverse().transform_point3(p));
        self.life.apply(&self.kind.rig, &mut self.model, target, dt, speed);
    }

    /// Model-space joint matrices of the current pose (after `update`).
    pub fn pose(&mut self) -> &mut [Mat4] {
        &mut self.model
    }

    pub fn render_item(&self, transform: Mat4) -> RenderItem {
        let joints: Vec<Mat4> = self.model.iter().zip(&self.kind.skeleton.inverse_bind).map(|(m, ib)| *m * *ib).collect();
        let mut item = RenderItem::new(self.kind.model, transform);
        item.joints = Some(Arc::new(joints));
        item
    }

    /// Eye height above the feet in the current pose.
    pub fn eye_height(&self) -> f32 {
        self.kind.rig.head.as_ref().and_then(|h| self.model.get(h.joint)).map(|m| m.w_axis.y + 0.09).unwrap_or(1.65)
    }
}

/// Bends the right arm so the hand points along `aim` (model space),
/// blended by `weight`: the whole arm rotates about the shoulder.
pub fn aim_arm(skel: &Skeleton, arm: (usize, usize), model: &mut [Mat4], aim: Vec3, weight: f32) {
    let (upper, wrist) = arm;
    if weight <= 0.0 {
        return;
    }
    let shoulder = pos(&model[upper]);
    let to_wrist = (pos(&model[wrist]) - shoulder).normalize_or_zero();
    let target = aim.normalize_or_zero();
    if to_wrist == Vec3::ZERO || target == Vec3::ZERO {
        return;
    }
    let q = Quat::IDENTITY.slerp(Quat::from_rotation_arc(to_wrist, target), weight.min(1.0));
    let below: Vec<usize> = (0..model.len()).filter(|&j| is_descendant(skel, j, upper)).collect();
    rotate(model, &below, shoulder, q);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_closes_fast_and_opens() {
        let mut life = Life::new(1);
        life.start_blink();
        let mut seen = Vec::new();
        for _ in 0..12 {
            life.step_timers(1.0 / 60.0, 0.0);
            seen.push(life.blink_amount());
        }
        assert!(seen.iter().any(|b| *b > 0.99), "{seen:?}");
        assert!(seen[1] > 0.4, "closing should take ~50 ms: {seen:?}");
        assert_eq!(*seen.last().unwrap(), 0.0, "{seen:?}");
    }

    #[test]
    fn blinks_every_two_to_six_seconds() {
        let mut life = Life::new(7);
        let (mut starts, mut prev) = (0, false);
        for _ in 0..(60 * 120) {
            life.step_timers(1.0 / 60.0, 0.0);
            let on = life.blink.is_some();
            if on && !prev {
                starts += 1;
            }
            prev = on;
        }
        // 120 s at one blink per ~4 s (plus a few doubles).
        assert!((20..=45).contains(&starts), "{starts} blinks in two minutes");
    }

    #[test]
    fn gait_matches_playback_to_speed() {
        let gait = Gait { walk: 1.2, run: 3.2 };
        let scene = forja_anim_test_scene();
        let skel = Skeleton::from_scene(&scene, 0).unwrap();
        let def = ControllerDef::from_toml(include_str!("../../../../data/anim/biped_humanoid.toml")).unwrap();
        let clips: Vec<Clip> = ["Idle", "Talk", "Wait", "Walking", "Running", "Jump", "Punch", "Death"]
            .iter()
            .map(|n| Clip { name: n.to_string(), duration: 1.0, tracks: Vec::new(), coverage: 1.0, root_joint: None })
            .collect();
        let mut anim = AnimController::new(&def, &skel, clips).unwrap();
        gait.drive(&mut anim, 1.2);
        assert_eq!((anim.get("gait"), anim.get("stride")), (1.0, 1.0));
        gait.drive(&mut anim, 0.9);
        assert!((anim.get("stride") - 0.75).abs() < 1e-5);
        gait.drive(&mut anim, 2.2);
        assert!((anim.get("gait") - 1.5).abs() < 1e-5 && (anim.get("stride") - 1.0).abs() < 1e-5);
        gait.drive(&mut anim, 4.0);
        assert!((anim.get("stride") - 1.25).abs() < 1e-5);
        anim.update(0.5);
        assert_eq!(anim.state(), "move");
        gait.drive(&mut anim, 0.0);
        anim.update(0.1);
        assert_eq!(anim.state(), "idle");
    }

    fn forja_anim_test_scene() -> SceneData {
        use forja_assets::{NodeData, SkinData, Trs};
        let node = |name: &str, parent: Option<usize>, t: Vec3| NodeData { name: name.into(), parent, local: Trs { t, ..Default::default() }, ..Default::default() };
        SceneData {
            nodes: vec![node("hips", None, Vec3::Y), node("Bip01 Spine1", Some(0), Vec3::Y * 0.7)],
            roots: vec![0],
            skins: vec![SkinData { name: "skin".into(), joints: vec![0, 1], inverse_bind: vec![Mat4::IDENTITY; 2] }],
            ..Default::default()
        }
    }

    #[test]
    fn rotation_about_a_pivot_is_rigid() {
        let mut m = vec![Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)), Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0))];
        rotate(&mut m, &[0, 1], Vec3::Y, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!((pos(&m[0]) - Vec3::Y).length() < 1e-5);
        assert!((pos(&m[1]) - Vec3::new(-1.0, 1.0, 0.0)).length() < 1e-5);
    }
}
