//! Inventory, weapon wheel, pickups, shooting and practice targets.

use std::sync::Arc;

use forja_anim::{AnimController, Clip, ControllerDef, Skeleton};
use forja_core::{App, Plugin, Stage, Time};
use forja_gameplay::{Health, Inventory, ItemCategory, ItemRegistry, WeaponCategory, WeaponRegistry, WeaponState, WeaponWheel};
use forja_math::{Iso, Mat4, Quat, Vec3, Vec4};
use forja_physics::{BodyHandle, ColliderDesc, CollisionGroups, PhysicsWorld, RigidBodyDesc, Shape, layers};
use forja_render::{PointLight, debug};
use forja_world::SpawnKind;

use super::camera::CameraRig;
use super::player::Player;
use super::vehicles::Driving;
use super::world::GameWorld;
use super::{Hud, Intent, Paths, RenderItem, RenderScene};

pub struct CombatPlugin;

pub struct Pickup {
    pub item: String,
    pub ammo: Option<(String, u32)>,
    pub position: Vec3,
    pub model: Option<usize>,
    pub model_fit: Mat4,
    pub taken: bool,
}

pub struct Target {
    pub body: BodyHandle,
    pub health: Health,
    pub model: usize,
    pub skeleton: Skeleton,
    pub anim: Option<AnimController>,
    pub scale: f32,
    pub facing: f32,
}

pub struct Combat {
    pub items: ItemRegistry,
    pub weapons: WeaponRegistry,
    pub inventory: Inventory,
    pub wheel: WeaponWheel,
    pub equipped: Option<WeaponState>,
    pub pickups: Vec<Pickup>,
    pub targets: Vec<Target>,
    tracers: Vec<(Vec3, Vec3, f32)>,
    flashes: Vec<(Vec3, f32)>,
    weapon_models: Vec<(String, usize, Mat4)>,
    pub shots_fired: u32,
    pub hits: u32,
}

impl Combat {
    pub fn reserve(&self) -> u32 {
        self.equipped.as_ref().and_then(|w| w.def.ammo.as_ref()).map(|a| self.inventory.count(a)).unwrap_or(0)
    }

    pub fn equip(&mut self, weapon_id: &str) {
        if self.equipped.as_ref().is_some_and(|w| w.def.id == weapon_id) {
            return;
        }
        if let Some(def) = self.weapons.get(weapon_id) {
            self.equipped = Some(WeaponState::new(def.clone()));
        }
    }
}

/// Uniform scale and centering so a model fits `size` meters along its longest axis.
fn fit(scene: &forja_assets::SceneData, size: f32) -> Mat4 {
    let b = scene.bounds();
    let s = size / (b.max - b.min).max_element().max(1e-3);
    Mat4::from_scale(Vec3::splat(s)) * Mat4::from_translation(-b.center())
}

fn load_target(app: &mut App, position: Vec3, facing: f32, variant: usize) -> Option<Target> {
    let paths = app.world.res::<Paths>().clone();
    // Realistic people when the MakeHuman content is present.
    if super::humans::HumanLibrary::available(&paths) {
        let specs = super::humans::npc_specs();
        let spec = specs[variant % specs.len()].clone();
        let human = app.world.scope::<super::humans::HumanLibrary, _>(|w, lib| lib.spawn(&paths, w.res_mut::<RenderScene>(), &spec));
        if let Some(h) = human {
            let physics = app.world.res_mut::<PhysicsWorld>();
            let body = physics.add_body(RigidBodyDesc::kinematic().position(position + Vec3::Y * 0.9));
            physics.add_collider(ColliderDesc::new(Shape::capsule(0.5, 0.35)).groups(CollisionGroups::new(layers::CHARACTER, u32::MAX)), Some(body));
            return Some(Target { body, health: Health { hp: 120.0, max_hp: 120.0, ..Default::default() }, model: h.model, skeleton: h.skeleton, anim: h.anim, scale: 1.0, facing });
        }
    }
    let rs = app.world.res_mut::<RenderScene>();
    let model = rs.load(&paths.asset("quaternius_robot_expressive/RobotExpressive.glb"))?;
    let scene = Arc::clone(&rs.models[model]);
    let skeleton = Skeleton::from_scene(&scene, 0)?;
    let clips = Clip::bind_all(&scene, &skeleton, None);
    let anim = std::fs::read_to_string(paths.data.join("anim/robot_humanoid.toml"))
        .map_err(|e| e.to_string())
        .and_then(|t| ControllerDef::from_toml(&t))
        .and_then(|def| AnimController::new(&def, &skeleton, clips))
        .map_err(|e| log::warn!("target animation: {e}"))
        .ok();
    let scale = 1.8 / scene.bounds().max.y.max(0.1);
    let physics = app.world.res_mut::<PhysicsWorld>();
    let body = physics.add_body(RigidBodyDesc::kinematic().position(position + Vec3::Y * 0.9));
    physics.add_collider(ColliderDesc::new(Shape::capsule(0.5, 0.4)).groups(CollisionGroups::new(layers::CHARACTER, u32::MAX)), Some(body));
    Some(Target { body, health: Health { hp: 120.0, max_hp: 120.0, ..Default::default() }, model, skeleton, anim, scale, facing })
}

impl Plugin for CombatPlugin {
    fn name(&self) -> &'static str {
        "combat"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["player", "forja_physics"]
    }

    fn build(&self, app: &mut App) {
        let paths = app.world.res::<Paths>().clone();
        let items = ItemRegistry::load(&paths.data.join("items")).unwrap_or_else(|e| {
            log::warn!("items: {e}");
            ItemRegistry::default()
        });
        let weapons = WeaponRegistry::load(&paths.data.join("weapons")).unwrap_or_else(|e| {
            log::warn!("weapons: {e}");
            WeaponRegistry::default()
        });
        let mut inventory = Inventory::new(24, 40.0);
        inventory.add(&items, "fists", 1);
        inventory.add(&items, "medkit", 2);
        inventory.add(&items, "snack", 3);
        // Pickups at weapon spawns, or in front of the player.
        let desc = &app.world.res::<GameWorld>().desc;
        let player = desc.spawn(SpawnKind::Player).unwrap_or(Iso::from_pos(Vec3::Y));
        let mut spots: Vec<Vec3> = desc.spawns_of(SpawnKind::Weapon).map(|s| s.iso.pos).collect();
        let fwd = player.rot * Vec3::Z;
        let side = player.rot * Vec3::X;
        // Fan out in front of the player, the first (pistol) straight ahead.
        for k in [0.0f32, -1.0, 1.0, -2.0, 2.0] {
            if spots.len() >= 5 {
                break;
            }
            spots.push(player.pos + fwd * 4.0 + side * k * 1.6);
        }
        let loot: [(&str, Option<(&str, u32)>); 5] = [
            ("pistol", Some(("ammo_9mm", 48))),
            ("blaster_smg", Some(("ammo_9mm", 120))),
            ("blaster_shotgun", Some(("ammo_12g", 24))),
            ("carbine", Some(("ammo_556", 90))),
            ("rifle", Some(("ammo_762", 15))),
        ];
        let mut pickups = Vec::new();
        let mut weapon_models = Vec::new();
        for (i, (item, ammo)) in loot.iter().enumerate() {
            let model_path = items.get(item).and_then(|d| d.model.clone());
            let (model, model_fit) = match model_path {
                Some(p) => {
                    let rs = app.world.res_mut::<RenderScene>();
                    match rs.load(&paths.asset(&p)) {
                        Some(m) => {
                            let size = if *item == "pistol" { 0.32 } else { 0.85 };
                            (Some(m), fit(rs.model(m), size))
                        }
                        None => (None, Mat4::IDENTITY),
                    }
                }
                None => (None, Mat4::IDENTITY),
            };
            pickups.push(Pickup {
                item: item.to_string(),
                ammo: ammo.map(|(a, n)| (a.to_string(), n)),
                position: spots[i % spots.len()] + Vec3::Y * 0.1,
                model,
                model_fit,
                taken: false,
            });
        }
        // In-hand models come from the weapon definitions.
        let ids: Vec<(String, String)> = items.iter().filter_map(|d| d.weapon.clone()).filter_map(|w| weapons.get(&w).and_then(|d| d.model.clone()).map(|m| (w, m))).collect();
        for (wid, model_path) in ids {
            let rs = app.world.res_mut::<RenderScene>();
            if let Some(m) = rs.load(&paths.asset(&model_path)) {
                let size = if weapons.get(&wid).is_some_and(|d| d.category == WeaponCategory::Pistol || d.category == WeaponCategory::Smg) { 0.3 } else { 1.0 };
                let f = fit(rs.model(m), size);
                weapon_models.push((wid, m, f));
            }
        }
        let mut targets = Vec::new();
        for k in 0..3 {
            let p = player.pos + fwd * (14.0 + k as f32 * 3.0) + side * (k as f32 - 1.0) * 2.5;
            if let Some(t) = load_target(app, p - Vec3::Y * 0.5, (-fwd).x.atan2((-fwd).z), k) {
                targets.push(t);
            }
        }
        app.insert_resource(Combat {
            items,
            weapons,
            inventory,
            wheel: WeaponWheel::default(),
            equipped: None,
            pickups,
            targets,
            tracers: Vec::new(),
            flashes: Vec::new(),
            weapon_models,
            shots_fired: 0,
            hits: 0,
        });
        app.add_system(Stage::Update, "combat_inventory", inventory_system);
        app.add_system(Stage::FixedUpdate, "combat_fire", fire_system);
        app.add_system_at(Stage::PostUpdate, "combat_render", 30, render_system);
    }
}

fn inventory_system(w: &mut forja_core::World) {
    let player_pos = w.res::<Player>().position();
    let driving = w.res::<Driving>().0.is_some();
    // Consume this system's one-shot actions.
    let (select, holster, use_item, drop_item, equip_item, reload) = {
        let i = w.res_mut::<Intent>();
        (i.select.take(), Intent::take(&mut i.holster), i.use_item.take(), i.drop_item.take(), i.equip_item.take(), Intent::take(&mut i.reload))
    };
    let mut messages = Vec::new();
    let mut heal = 0.0;
    let mut armor = 0.0;
    {
        let c = w.res_mut::<Combat>();
        // Walk over pickups.
        for k in 0..c.pickups.len() {
            if c.pickups[k].taken || driving || (c.pickups[k].position - player_pos).length() > 1.6 {
                continue;
            }
            let (item, ammo) = (c.pickups[k].item.clone(), c.pickups[k].ammo.clone());
            if c.inventory.add(&c.items, &item, 1) == 0 {
                if let Some((a, n)) = ammo {
                    let left = c.inventory.add(&c.items, &a, n);
                    messages.push(format!("+{} {}", n - left, c.items.get(&a).map(|d| d.name.as_str()).unwrap_or(&a)));
                }
                messages.push(format!("Picked up {}", c.items.get(&item).map(|d| d.name.as_str()).unwrap_or(&item)));
                c.pickups[k].taken = true;
            } else {
                messages.push("Inventory full".into());
            }
        }
        let (items, weapons) = (c.items.clone(), c.weapons.clone());
        c.wheel.refresh(&c.inventory, &items, &weapons);
        if let Some(cat) = select {
            match c.wheel.select_category(cat).map(str::to_string) {
                Some(id) => c.equip(&id),
                None => messages.push(format!("No {cat:?} weapon")),
            }
        }
        if holster {
            c.wheel.holster();
            c.equipped = None;
        }
        if let Some(item) = equip_item
            && let Some(wid) = items.get(&item).and_then(|d| d.weapon.clone())
        {
            c.equip(&wid);
        }
        // Dropped the equipped weapon from the inventory: unequip.
        if let Some(eq) = &c.equipped
            && !c.inventory.slots.iter().flatten().any(|s| items.get(&s.item).and_then(|d| d.weapon.as_deref()) == Some(eq.def.id.as_str()))
        {
            c.equipped = None;
        }
        if let Some(item) = use_item
            && let Some(def) = items.get(&item)
            && matches!(def.category, ItemCategory::Consumable | ItemCategory::Armor)
            && c.inventory.remove(&item, 1) == 1
        {
            if def.category == ItemCategory::Armor {
                armor = def.value;
            } else {
                heal = def.value;
            }
            messages.push(format!("Used {}", def.name));
        }
        if let Some(item) = drop_item
            && c.inventory.remove(&item, 1) == 1
        {
            c.pickups.push(Pickup { item: item.clone(), ammo: None, position: player_pos - Vec3::Y * 0.8, model: None, model_fit: Mat4::IDENTITY, taken: false });
            messages.push(format!("Dropped {}", items.get(&item).map(|d| d.name.as_str()).unwrap_or(&item)));
        }
        if reload {
            let reserve = c.reserve();
            if let Some(wpn) = &mut c.equipped {
                wpn.start_reload(reserve);
            }
        }
    }
    let p = w.res_mut::<Player>();
    p.health.heal(heal);
    p.health.armor = (p.health.armor + armor).min(100.0);
    let hud = w.res_mut::<Hud>();
    for m in messages {
        hud.message(m);
    }
}

fn fire_system(w: &mut forja_core::World) {
    let dt = w.res::<Time>().fixed_dt;
    let intent = w.res::<Intent>().clone();
    let driving = w.res::<Driving>().0.is_some();
    let dead = w.res::<Player>().health.is_dead();
    let cam = w.res::<RenderScene>().camera;
    let shooter = w.res::<Player>().controller.body;
    let muzzle = (w.res::<Player>().hand_world * Mat4::from_translation(Vec3::new(0.0, -0.9, 0.5))).transform_point3(Vec3::ZERO);
    let mut hud_hit = false;
    let mut kick = None;
    w.scope::<Combat, _>(|w, c| {
        let reserve = c.reserve();
        let ammo_item = c.equipped.as_ref().and_then(|e| e.def.ammo.clone());
        if let Some(wpn) = &mut c.equipped {
            let taken = wpn.tick(dt, reserve);
            if taken > 0
                && let Some(a) = &ammo_item
            {
                c.inventory.remove(a, taken);
            }
            if wpn.in_mag == 0 && intent.fire && !wpn.is_reloading() {
                wpn.start_reload(reserve);
            }
            let trigger = intent.fire && !driving && !dead;
            let physics = w.res_mut::<PhysicsWorld>();
            // Aim from the camera through the crosshair; start the ray past
            // the player so we never hit ourselves.
            let origin = cam.position + cam.forward() * 2.5;
            let (hits, shots) = wpn.trigger(physics, trigger, intent.aim, origin, cam.forward(), Some(shooter));
            if shots > 0 {
                c.shots_fired += shots;
                c.flashes.push((muzzle, 0.05));
                kick = Some(wpn.recoil_kick());
                let end = hits.first().map(|h| h.point).unwrap_or(origin + cam.forward() * wpn.def.range.min(200.0));
                c.tracers.push((muzzle, end, 0.06));
            }
            for h in hits {
                if let Some(t) = c.targets.iter_mut().find(|t| t.body == h.body) {
                    t.health.damage(h.damage);
                    hud_hit = true;
                    c.hits += 1;
                }
                c.tracers.push((h.point, h.point + h.normal * 0.15, 0.25));
            }
        }
        for t in &mut c.tracers {
            t.2 -= dt;
        }
        c.tracers.retain(|t| t.2 > 0.0);
        for f in &mut c.flashes {
            f.1 -= dt;
        }
        c.flashes.retain(|f| f.1 > 0.0);
    });
    if hud_hit {
        w.res_mut::<Hud>().hit_marker = 1.0;
    }
    if let Some(q) = kick {
        let rig = w.res_mut::<CameraRig>();
        let (yaw, pitch, _) = q.to_euler(forja_math::EulerRot::YXZ);
        rig.recoil.0 += yaw;
        rig.recoil.1 = (rig.recoil.1 + pitch).min(0.35);
    }
}

fn render_system(w: &mut forja_core::World) {
    let dt = w.res::<Time>().delta;
    let t = w.res::<Time>().elapsed as f32;
    let hand = w.res::<Player>().hand_world;
    let human = w.res::<Player>().avatar.human.is_some();
    let cam_fwd = w.res::<RenderScene>().camera.forward();
    let driving = w.res::<Driving>().0.is_some();
    let mut items = Vec::new();
    let mut lights = Vec::new();
    let mut lines = Vec::new();
    w.scope::<Combat, _>(|w, c| {
        for p in &c.pickups {
            if p.taken {
                continue;
            }
            let spin = Mat4::from_rotation_translation(Quat::from_rotation_y(t * 1.5), p.position + Vec3::Y * (0.6 + (t * 2.0).sin() * 0.08));
            if let Some(m) = p.model {
                items.push(RenderItem::new(m, spin * p.model_fit));
            }
            lights.push(PointLight { position: p.position + Vec3::Y * 0.8, radius: 3.0, color: Vec3::new(0.3, 0.8, 1.0), intensity: 4.0 });
        }
        // Weapon in hand.
        if !driving
            && let Some(eq) = &c.equipped
            && let Some((_, m, f)) = c.weapon_models.iter().find(|(id, _, _)| *id == eq.def.id)
        {
            let placed = if human {
                // Barrel (+X of the scanned guns) along the aim, grip down.
                let aim = cam_fwd.normalize();
                let side = aim.cross(Vec3::Y).normalize_or(Vec3::X);
                let up = side.cross(aim);
                let basis = Mat4::from_cols(aim.extend(0.0), up.extend(0.0), (-side).extend(0.0), Vec4::W);
                let wrist = hand.w_axis.truncate();
                Mat4::from_translation(wrist + aim * 0.06 - up * 0.02) * basis * *f
            } else {
                let grip = hand * Mat4::from_translation(Vec3::new(0.0, -0.85, 0.25)) * Mat4::from_rotation_y(-std::f32::consts::FRAC_PI_2);
                // Undo the character scale so the gun keeps its real size.
                let (_, r, p) = grip.to_scale_rotation_translation();
                Mat4::from_rotation_translation(r, p) * *f
            };
            items.push(RenderItem::new(*m, placed));
        }
        for (a, b, _) in &c.tracers {
            debug::line(&mut lines, *a, *b, Vec4::new(1.0, 0.85, 0.4, 1.0));
        }
        for (p, _) in &c.flashes {
            lights.push(PointLight { position: *p, radius: 8.0, color: Vec3::new(1.0, 0.7, 0.3), intensity: 120.0 });
        }
        // Practice targets.
        let physics = w.res::<PhysicsWorld>();
        for tg in &mut c.targets {
            let Some(b) = physics.body(tg.body) else { continue };
            let feet = b.position() - Vec3::Y * 0.9;
            let mut mats = Vec::new();
            if let Some(anim) = &mut tg.anim {
                anim.set("dead", tg.health.is_dead() as u32 as f32);
                anim.set("wave", if tg.health.is_dead() { 0.0 } else { 1.0 });
                anim.update(dt);
                tg.skeleton.skinning_matrices(anim.pose(), &mut mats);
            }
            let transform = Mat4::from_rotation_translation(Quat::from_rotation_y(tg.facing), feet) * Mat4::from_scale(Vec3::splat(tg.scale));
            let mut item = RenderItem::new(tg.model, transform);
            if !mats.is_empty() {
                item.joints = Some(Arc::new(mats));
            }
            let hp = tg.health.hp / tg.health.max_hp;
            item.tint = Vec4::new(1.0, 0.4 + 0.6 * hp, 0.4 + 0.6 * hp, 1.0);
            items.push(item);
        }
    });
    let rs = w.res_mut::<RenderScene>();
    rs.dynamic.extend(items);
    rs.lights.extend(lights);
    rs.lines.extend(lines);
    // HUD.
    let (weapon, ammo, reloading) = {
        let c = w.res::<Combat>();
        let reserve = c.reserve();
        match &c.equipped {
            Some(e) => (Some(e.def.name.clone()), e.def.ammo.as_ref().map(|_| (e.in_mag, reserve)), e.is_reloading().then(|| e.reload_progress())),
            None => (None, None, None),
        }
    };
    let aiming = w.res::<Intent>().aim;
    let health = w.res::<Player>().health;
    let hud = w.res_mut::<Hud>();
    hud.weapon = weapon;
    hud.ammo = ammo;
    hud.reloading = reloading;
    hud.aiming = aiming;
    hud.health = health.hp;
    hud.armor = health.armor;
}

/// Category keys 1-6.
pub const WHEEL_KEYS: [(&str, WeaponCategory); 6] = [
    ("weapon_melee", WeaponCategory::Melee),
    ("weapon_pistol", WeaponCategory::Pistol),
    ("weapon_smg", WeaponCategory::Smg),
    ("weapon_shotgun", WeaponCategory::Shotgun),
    ("weapon_rifle", WeaponCategory::Rifle),
    ("weapon_sniper", WeaponCategory::Sniper),
];
