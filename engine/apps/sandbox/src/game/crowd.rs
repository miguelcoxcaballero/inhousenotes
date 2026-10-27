//! Pedestrians: a varied crowd of Rocketbox people on the sidewalks, in
//! the plazas and down the alleys of whatever city is loaded.
//!
//! Walkers follow the iso-lines of the walkway's distance field (see
//! `walkable.rs`), each in its own lane, keeping to their side, at the
//! natural speed of their own walk cycle so the feet never slide. Some
//! chat in pairs (taking turns to talk), some wait or look in a shop
//! window, some cross the street after looking both ways. People glance
//! at whoever passes close, the player included.

use std::sync::Arc;

use forja_core::{App, Plugin, Stage, Time};
use forja_math::{Mat4, Quat, Ray, Vec2, Vec3, smoothing};
use forja_physics::{CollisionGroups, PhysicsWorld, QueryFilter, layers};

use super::people::{AvatarType, PeopleLibrary, Person, Rng, WALKS};
use super::player::Player;
use super::walkable::{Ground, Walkable};
use super::world::GameWorld;
use super::{Paths, RenderScene};

pub struct CrowdPlugin;

/// People within this distance of the player are simulated (m).
const RADIUS: f32 = 60.0;
/// Walkway area per person (m²): a lively Spanish barrio street.
const AREA_PER_PERSON: f32 = 38.0;
const MAX_PEOPLE: usize = 56;
/// Weight of the conversation gestures while someone is speaking.
const TALK: f32 = 0.45;
/// Life detail (blinks, eyes, breathing) only where it can be seen (m).
const DETAIL_DISTANCE: f32 = 22.0;

pub struct Crowd {
    pub walk: Option<Walkable>,
    types: Vec<Arc<AvatarType>>,
    pub peds: Vec<Ped>,
    rng: Rng,
    next_id: u64,
    filled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Walk,
    /// Standing for `left` seconds; with a partner when chatting.
    Stand { left: f32, partner: Option<u64>, talking: bool },
    /// At the kerb looking both ways (`wait`), then across along `dir`.
    Cross { dir: Vec2, left: f32, wait: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    Ahead,
    Player,
    Ped(u64),
    Point(Vec3),
}

pub struct Ped {
    pub id: u64,
    person: Person,
    pub pos: Vec2,
    y: f32,
    heading: Vec2,
    yaw: f32,
    speed: f32,
    cruise: f32,
    /// Lane: fraction of the walkway half width kept from the nearest edge.
    lane: f32,
    mode: Mode,
    /// Seconds to the next decision (stop, cross).
    decide_in: f32,
    look: Look,
    look_for: f32,
}

impl Ped {
    fn eye(&self) -> Vec3 {
        Vec3::new(self.pos.x, self.y, self.pos.y) + Vec3::Y * self.person.eye_height()
    }

    fn transform(&self) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_y(self.yaw), Vec3::new(self.pos.x, self.y, self.pos.y))
    }

    fn velocity(&self) -> Vec2 {
        if self.mode == Mode::Walk || matches!(self.mode, Mode::Cross { wait, .. } if wait <= 0.0) { self.heading * self.speed } else { Vec2::ZERO }
    }
}

impl Plugin for CrowdPlugin {
    fn name(&self) -> &'static str {
        "crowd"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["world", "player"]
    }

    fn build(&self, app: &mut App) {
        let paths = app.world.res::<Paths>().clone();
        let mut crowd = Crowd { walk: None, types: Vec::new(), peds: Vec::new(), rng: Rng::new(0x5eed_c0de), next_id: 1, filled: false };
        if PeopleLibrary::available(&paths) {
            let t = std::time::Instant::now();
            let desc = &app.world.res::<GameWorld>().desc;
            crowd.walk = Walkable::from_colliders(&desc.colliders, Vec2::ZERO, desc.extent, 0.5).filter(|w| w.area() > 1500.0);
            log::info!("walkway: {:.0} m² in {:.2?}", crowd.walk.as_ref().map_or(0.0, |w| w.area()), t.elapsed());
        }
        if crowd.walk.is_some() {
            // Everyone but the player's own avatar.
            let own = app.world.res::<Player>().avatar.person.as_ref().map(|p| p.kind.name.clone());
            let files: Vec<_> = PeopleLibrary::catalogue(&paths).into_iter().filter(|f| own.as_ref().is_none_or(|o| !f.ends_with(format!("{o}.fbx")))).collect();
            let t = std::time::Instant::now();
            crowd.types = app.world.scope::<PeopleLibrary, _>(|w, lib| lib.load(&paths, w.res_mut::<RenderScene>(), &files, 1024));
            log::info!("crowd: {} avatar types in {:.2?}", crowd.types.len(), t.elapsed());
        }
        app.insert_resource(crowd);
        app.add_system(Stage::Update, "crowd_update", crowd_update);
        app.add_system_at(Stage::PostUpdate, "crowd_render", 25, crowd_render);
    }
}

fn crowd_update(w: &mut forja_core::World) {
    let dt = w.res::<Time>().delta.min(0.1);
    if w.res::<Crowd>().walk.is_none() || w.res::<Crowd>().types.is_empty() || dt <= 0.0 {
        return;
    }
    let paths = w.res::<Paths>().clone();
    let (player_pos, player_eye) = {
        let p = w.res::<Player>();
        let feet = p.controller.feet();
        (Vec2::new(feet.x, feet.z), feet + Vec3::Y * 1.62)
    };
    let cam = w.res::<RenderScene>().camera;
    w.scope::<Crowd, _>(|w, crowd| {
        crowd.populate(w, &paths, player_pos, cam.position, cam.forward());
        crowd.step(w.res::<PhysicsWorld>(), dt, player_pos, player_eye);
    });
}

fn crowd_render(w: &mut forja_core::World) {
    let items: Vec<_> = w.res::<Crowd>().peds.iter().map(|p| p.person.render_item(p.transform())).collect();
    w.res_mut::<RenderScene>().dynamic.extend(items);
}

impl Crowd {
    /// Eyes of the nearest person within `r` of `from` and in front
    /// (`fwd`), for the player to glance at.
    pub fn glance_target(&self, from: Vec2, fwd: Vec2, r: f32) -> Option<Vec3> {
        self.peds
            .iter()
            .filter(|p| {
                let rel = p.pos - from;
                rel.length() < r && rel.normalize_or_zero().dot(fwd) > 0.2
            })
            .min_by(|a, b| a.pos.distance(from).total_cmp(&b.pos.distance(from)))
            .map(|p| p.eye())
    }

    /// Keeps the crowd around the player: drops people who fell far behind
    /// and brings new ones in out of sight.
    fn populate(&mut self, w: &mut forja_core::World, paths: &Paths, center: Vec2, cam: Vec3, cam_fwd: Vec3) {
        self.peds.retain(|p| p.pos.distance(center) < RADIUS + 15.0);
        // Partners of removed people stop waiting for them.
        let ids: Vec<u64> = self.peds.iter().map(|p| p.id).collect();
        for p in &mut self.peds {
            if let Mode::Stand { partner: Some(o), .. } = p.mode
                && !ids.contains(&o)
            {
                p.mode = Mode::Stand { left: 0.0, partner: None, talking: false };
            }
        }
        let want = ((self.walk.as_ref().expect("walkway").area_near(center, RADIUS) / AREA_PER_PERSON) as usize).min(MAX_PEOPLE);
        let first = !self.filled;
        let cam2 = Vec2::new(cam.x, cam.z);
        let fwd2 = Vec2::new(cam_fwd.x, cam_fwd.z).normalize_or_zero();
        let mut budget = if first { want * 3 } else { 2 };
        while self.peds.len() < want && budget > 0 {
            budget -= 1;
            let (rmin, rmax) = if first { (2.0, RADIUS) } else { (RADIUS * 0.6, RADIUS) };
            // Later arrivals appear out of view.
            let unseen = |p: Vec2| first || (p - cam2).dot(fwd2) < 0.0 || p.distance(cam2) > 45.0;
            let Some(p) = self.walk.as_ref().expect("walkway").random_spot(&mut self.rng, center, rmin, rmax, unseen) else { break };
            if self.peds.iter().any(|o| o.pos.distance(p) < 1.5) {
                continue;
            }
            let group = self.rng.f();
            if group < 0.18 {
                self.spawn_pair(w, paths, p);
            } else {
                let stand = group < 0.3;
                self.spawn_one(w, paths, p, stand);
            }
        }
        if first {
            log::info!("crowd: {} people around the player (want {want})", self.peds.len());
            if let Ok(path) = std::env::var("FORJA_WALKWAY_MAP") {
                let marks: Vec<Vec2> = self.peds.iter().map(|p| p.pos).chain(std::iter::once(center)).collect();
                let _ = self.walk.as_ref().expect("walkway").debug_image(&marks).save(&path);
            }
        }
        self.filled = true;
    }

    /// The avatar type least used near `p` (variety: no twins side by side).
    fn pick_type(&mut self, p: Vec2) -> Arc<AvatarType> {
        let mut best: Option<(f32, usize)> = None;
        let start = self.rng.index(self.types.len());
        for k in 0..self.types.len() {
            let i = (start + k) % self.types.len();
            let name = &self.types[i].name;
            let crowding: f32 = self.peds.iter().filter(|o| &o.person.kind.name == name).map(|o| 1.0 / (1.0 + o.pos.distance(p) / 10.0)).sum();
            if best.is_none_or(|b| crowding < b.0) {
                best = Some((crowding, i));
            }
        }
        self.types[best.map_or(0, |b| b.1)].clone()
    }

    fn new_ped(&mut self, w: &mut forja_core::World, paths: &Paths, p: Vec2, heading: Vec2, walking: bool) -> Option<Ped> {
        let kind = self.pick_type(p);
        // Mostly the natural walk; some people stroll or have their own gait.
        let walk = match self.rng.f() {
            x if x < 0.55 => WALKS[0],
            x if x < 0.8 => WALKS[1],
            _ => WALKS[2],
        };
        let seed = self.rng.next_u64();
        let mut person = w.scope::<PeopleLibrary, _>(|_, lib| lib.spawn(paths, &kind, walk, seed))?;
        let walkable = self.walk.as_ref()?;
        let s = walkable.sample(p);
        let cruise = person.gait.walk * self.rng.range(0.94, 1.06);
        // Already under way, with the cycles out of step with each other.
        if walking {
            person.gait.drive(&mut person.anim, cruise);
            person.anim.update(0.0);
        }
        person.anim.update(self.rng.range(0.0, 2.0));
        let id = self.next_id;
        self.next_id += 1;
        Some(Ped {
            id,
            person,
            pos: p,
            y: s.height,
            heading,
            yaw: heading.x.atan2(heading.y),
            speed: if walking { cruise } else { 0.0 },
            cruise,
            lane: self.rng.range(0.3, 0.85),
            mode: Mode::Walk,
            decide_in: self.rng.range(4.0, 30.0),
            look: Look::Ahead,
            look_for: self.rng.range(0.5, 3.0),
        })
    }

    /// Keep-right direction along the walkway at `p`.
    fn along(&self, p: Vec2) -> Vec2 {
        let s = self.walk.as_ref().expect("walkway").sample(p);
        Vec2::new(-s.inward.y, s.inward.x).normalize_or(Vec2::Y)
    }

    fn spawn_one(&mut self, w: &mut forja_core::World, paths: &Paths, p: Vec2, stand: bool) {
        // Most people keep to their right; some do not.
        let along = self.along(p) * if self.rng.chance(0.75) { 1.0 } else { -1.0 };
        let Some(mut ped) = self.new_ped(w, paths, p, along, !stand) else { return };
        if stand {
            // Facing a shop window or the street.
            let s = self.walk.as_ref().expect("walkway").sample(p);
            let wall = self.walk.as_ref().expect("walkway").ground(p - s.inward * (s.sdf + 0.6)) == Ground::Obstacle;
            let face = if wall { -s.inward } else { s.inward.perp() * if self.rng.chance(0.5) { 1.0 } else { -1.0 } };
            ped.yaw = face.x.atan2(face.y);
            ped.heading = face;
            ped.mode = Mode::Stand { left: self.rng.range(10.0, 60.0), partner: None, talking: false };
            ped.person.anim.set("stand", if self.rng.chance(0.6) { 2.0 } else { 0.0 });
        }
        self.peds.push(ped);
    }

    /// Two people chatting, facing each other.
    fn spawn_pair(&mut self, w: &mut forja_core::World, paths: &Paths, p: Vec2) {
        let along = self.along(p);
        let gap = self.rng.range(0.45, 0.6);
        let (a, b) = (p - along * gap, p + along * gap);
        let walk = self.walk.as_ref().expect("walkway");
        if walk.sample(a).sdf < 0.7 || walk.sample(b).sdf < 0.7 {
            return;
        }
        let (Some(mut x), Some(mut y)) = (self.new_ped(w, paths, a, along, false), self.new_ped(w, paths, b, -along, false)) else { return };
        let left = self.rng.range(30.0, 120.0);
        x.yaw = along.x.atan2(along.y);
        y.yaw = (-along).x.atan2(-along.y);
        x.mode = Mode::Stand { left, partner: Some(y.id), talking: true };
        y.mode = Mode::Stand { left: left + self.rng.range(0.0, 8.0), partner: Some(x.id), talking: false };
        x.look = Look::Ped(y.id);
        y.look = Look::Ped(x.id);
        for (q, talking) in [(&mut x, true), (&mut y, false)] {
            q.decide_in = self.rng.range(3.0, 8.0);
            q.person.anim.set("talk", if talking { TALK } else { 0.0 });
        }
        self.peds.push(x);
        self.peds.push(y);
    }

    fn step(&mut self, physics: &PhysicsWorld, dt: f32, player: Vec2, player_eye: Vec3) {
        let walk = self.walk.take().expect("walkway");
        let snapshot: Vec<(u64, Vec2, Vec2, Vec3)> = self.peds.iter().map(|p| (p.id, p.pos, p.velocity(), p.eye())).collect();
        let eye_of = |id: u64| snapshot.iter().find(|s| s.0 == id).map(|s| s.3);
        let mut turn_taking: Vec<u64> = Vec::new();
        for i in 0..self.peds.len() {
            let rng = &mut self.rng;
            let ped = &mut self.peds[i];
            let s = walk.sample(ped.pos);
            ped.decide_in -= dt;
            ped.look_for -= dt;
            let mut want_speed = 0.0;
            match ped.mode {
                Mode::Walk => {
                    // Lane keeping along the iso-lines of the walkway.
                    let mut along = Vec2::new(-s.inward.y, s.inward.x);
                    if along == Vec2::ZERO {
                        along = ped.heading;
                    }
                    if along.dot(ped.heading) < 0.0 {
                        along = -along;
                    }
                    let lane = (ped.lane * s.half).clamp(0.55, (s.half - 0.2).max(0.55));
                    let err = lane - s.sdf;
                    let mut desire = along + s.inward * (err * 1.4).clamp(-1.2, 1.2);
                    // Make room for people ahead and the player.
                    let right = Vec2::new(-ped.heading.y, ped.heading.x);
                    let mut limit = ped.cruise;
                    let others = snapshot.iter().filter(|o| o.0 != ped.id).map(|o| (o.1, o.2)).chain(std::iter::once((player, Vec2::ZERO)));
                    for (op, ov) in others {
                        let rel = op - ped.pos;
                        let d = rel.length();
                        let ahead = rel.dot(ped.heading);
                        if d > 3.0 || ahead <= 0.0 {
                            continue;
                        }
                        // Someone dead ahead is passed on the left (both
                        // step to their right), as people do.
                        let side = if rel.dot(right) > 0.15 { 1.0 } else { -1.0 };
                        let closing = (ped.heading * ped.speed - ov).dot(rel / d.max(1e-3)).max(0.0);
                        desire -= right * side * (1.0 - d / 3.0) * (0.6 + closing * 0.5);
                        if d < 1.3 && rel.dot(right).abs() < 0.55 {
                            limit = limit.min(ov.dot(ped.heading).max(0.0) * 0.9);
                        }
                    }
                    let dir = desire.normalize_or(ped.heading);
                    let turn = ped.heading.angle_to(dir);
                    let max_turn = 2.4 * dt;
                    ped.heading = Vec2::from_angle(turn.clamp(-max_turn, max_turn)).rotate(ped.heading).normalize_or(dir);
                    want_speed = (ped.cruise * (1.0 - 0.3 * (turn.abs() / 1.2).min(1.0))).min(limit);
                    // Now and then: stop for a moment, or cross the street.
                    if ped.decide_in <= 0.0 {
                        ped.decide_in = rng.range(8.0, 35.0);
                        let outward = -s.inward;
                        let road_side = walk.crossing(ped.pos, outward, 24.0);
                        let clear = |t: f32| {
                            let from = Vec3::new(ped.pos.x, ped.y + 0.6, ped.pos.y);
                            let filter = QueryFilter::default().with_groups(CollisionGroups::new(u32::MAX, layers::VEHICLE | layers::DYNAMIC));
                            physics.raycast(&Ray::new(from, Vec3::new(outward.x, 0.0, outward.y)), t, filter).is_none()
                        };
                        match road_side {
                            Some(t) if rng.chance(0.35) && clear(t) => {
                                ped.mode = Mode::Cross { dir: outward, left: t + s.sdf, wait: rng.range(0.8, 2.0) };
                            }
                            _ if rng.chance(0.12) => {
                                let wall = walk.ground(ped.pos - s.inward * (s.sdf + 0.6)) == Ground::Obstacle;
                                let face = if wall { -s.inward } else { ped.heading };
                                ped.mode = Mode::Stand { left: rng.range(3.0, 10.0), partner: None, talking: false };
                                ped.heading = face;
                                ped.person.anim.set("stand", if rng.chance(0.5) { 2.0 } else { 0.0 });
                            }
                            _ => {}
                        }
                    }
                }
                Mode::Stand { left, partner, talking } => {
                    let left = left - dt;
                    if let Some(o) = partner {
                        // Turn-taking: the speaker hands over every few seconds.
                        if ped.decide_in <= 0.0 && talking {
                            ped.decide_in = rng.range(3.0, 9.0);
                            turn_taking.push(ped.id);
                            turn_taking.push(o);
                        }
                    }
                    ped.mode = Mode::Stand { left, partner, talking };
                    if left <= 0.0 {
                        ped.mode = Mode::Walk;
                        ped.person.anim.set("stand", 0.0);
                        ped.person.anim.set("talk", 0.0);
                        let along = Vec2::new(-s.inward.y, s.inward.x).normalize_or(ped.heading);
                        ped.heading = if rng.chance(0.5) { along } else { -along };
                        ped.decide_in = rng.range(10.0, 30.0);
                    }
                }
                Mode::Cross { dir, left, wait } => {
                    if wait > 0.0 {
                        // Look one way, then the other, then go.
                        let side = if wait > 0.5 { 1.0 } else { -1.0 };
                        let p = ped.pos + dir * 4.0 + dir.perp() * side * 12.0;
                        ped.look = Look::Point(Vec3::new(p.x, ped.y + 1.0, p.y));
                        ped.look_for = 0.3;
                        ped.heading = dir;
                        ped.mode = Mode::Cross { dir, left, wait: wait - dt };
                    } else {
                        want_speed = ped.cruise * 1.12;
                        ped.heading = dir;
                        let left = left - ped.speed * dt;
                        ped.mode = if left <= 0.0 || (walk.walkable(ped.pos) && s.sdf > 0.8 && left < 3.0) { Mode::Walk } else { Mode::Cross { dir, left, wait } };
                        if ped.mode == Mode::Walk {
                            let along = Vec2::new(-s.inward.y, s.inward.x).normalize_or(dir);
                            ped.heading = if rng.chance(0.5) { along } else { -along };
                        }
                    }
                }
            }
            // Speed eases in and out; the body turns towards the motion.
            ped.speed += (want_speed - ped.speed) * smoothing(if want_speed > ped.speed { 2.5 } else { 4.0 }, dt);
            if ped.speed < 0.02 && want_speed == 0.0 {
                ped.speed = 0.0;
            }
            ped.pos += ped.heading * ped.speed * dt;
            let target_yaw = match ped.mode {
                Mode::Stand { partner: Some(o), .. } => snapshot.iter().find(|x| x.0 == o).map(|x| (x.1 - ped.pos).x.atan2((x.1 - ped.pos).y)).unwrap_or(ped.yaw),
                _ => ped.heading.x.atan2(ped.heading.y),
            };
            ped.yaw += wrap(target_yaw - ped.yaw) * smoothing(if ped.speed > 0.2 { 7.0 } else { 3.0 }, dt);
            ped.y += (walk.sample(ped.pos).height - ped.y) * smoothing(20.0, dt);
            // Where to look: the partner, whoever is near and in front, the
            // player now and then, or just ahead.
            if ped.look_for <= 0.0 {
                ped.look_for = rng.range(1.2, 4.0);
                let fwd = Vec2::from_angle(std::f32::consts::FRAC_PI_2 - ped.yaw);
                let in_front = |p: Vec2, r: f32| {
                    let rel = p - ped.pos;
                    rel.length() < r && rel.normalize_or_zero().dot(fwd) > 0.3
                };
                let partner = match ped.mode {
                    Mode::Stand { partner, .. } => partner,
                    _ => None,
                };
                let near = snapshot.iter().filter(|o| o.0 != ped.id && Some(o.0) != partner && in_front(o.1, 4.5)).min_by(|a, b| a.1.distance(ped.pos).total_cmp(&b.1.distance(ped.pos)));
                ped.look = if in_front(player, 8.0) && rng.chance(0.45) {
                    Look::Player
                } else if let Some(o) = partner.filter(|_| rng.chance(0.8)) {
                    Look::Ped(o)
                } else if let Some(o) = near.filter(|_| rng.chance(0.35)) {
                    Look::Ped(o.0)
                } else {
                    Look::Ahead
                };
            }
            let look = match ped.look {
                Look::Ahead => None,
                Look::Player => Some(player_eye),
                Look::Ped(o) => eye_of(o),
                Look::Point(p) => Some(p),
            };
            let d = ped.pos.distance(player);
            ped.person.life.detail = if d < DETAIL_DISTANCE { 1.0 } else { 0.0 };
            let transform = ped.transform();
            let speed = ped.speed;
            ped.person.update(dt, speed, &transform, look);
        }
        // Bodies never overlap: walkers are pushed apart (people standing
        // still hold their ground), then the lane keeping takes over again.
        for i in 0..self.peds.len() {
            for j in i + 1..self.peds.len() {
                let rel = self.peds[j].pos - self.peds[i].pos;
                let d = rel.length();
                const BODY: f32 = 0.62;
                if d >= BODY {
                    continue;
                }
                let n = if d > 1e-4 { rel / d } else { Vec2::X };
                let (mi, mj) = (self.peds[i].speed > 0.05, self.peds[j].speed > 0.05);
                let (wi, wj) = match (mi, mj) {
                    (true, false) => (1.0, 0.0),
                    (false, true) => (0.0, 1.0),
                    _ => (0.5, 0.5),
                };
                let push = BODY - d;
                self.peds[i].pos -= n * push * wi;
                self.peds[j].pos += n * push * wj;
            }
        }
        // Conversation turns: the speaker listens, the listener talks.
        for id in turn_taking {
            if let Some(p) = self.peds.iter_mut().find(|p| p.id == id)
                && let Mode::Stand { left, partner, talking } = p.mode
            {
                p.mode = Mode::Stand { left, partner, talking: !talking };
                p.decide_in = self.rng.range(3.0, 9.0);
                p.person.anim.set("talk", if talking { 0.0 } else { TALK });
            }
        }
        self.walk = Some(walk);
    }
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
