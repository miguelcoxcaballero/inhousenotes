//! The physics world: storage, step pipeline, sleeping and queries.

use std::collections::{HashMap, HashSet};

use forja_math::{Aabb, Iso, Mat3, Ray, Vec3};

use crate::arena::Arena;
use crate::body::{BodyHandle, BodyType, Collider, ColliderDesc, ColliderHandle, CollisionGroups, RigidBody, RigidBodyDesc};
use crate::broadphase::{Broadphase, TreeBroadphase};
use crate::narrowphase::{self, Manifold};
use crate::shape::{MassProperties, Shape};
use crate::solver::{self, PairCache};

#[derive(Clone, Debug)]
pub struct PhysicsConfig {
    pub gravity: Vec3,
    /// Solver substeps per step (soft-step TGS).
    pub substeps: u32,
    /// Extra relaxation iterations per substep.
    pub relax_iterations: u32,
    pub contact_hertz: f32,
    pub contact_damping_ratio: f32,
    /// Max speed at which overlap is resolved (m/s).
    pub max_push_velocity: f32,
    pub restitution_threshold: f32,
    /// Distance at which contacts become speculative.
    pub speculative_margin: f32,
    pub sleep_linear_threshold: f32,
    pub sleep_angular_threshold: f32,
    pub time_to_sleep: f32,
    pub max_linear_speed: f32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            substeps: 4,
            relax_iterations: 1,
            contact_hertz: 30.0,
            contact_damping_ratio: 10.0,
            max_push_velocity: 3.0,
            restitution_threshold: 1.0,
            speculative_margin: 0.04,
            sleep_linear_threshold: 0.05,
            sleep_angular_threshold: 0.05,
            time_to_sleep: 0.5,
            max_linear_speed: 400.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StepStats {
    pub awake_bodies: usize,
    pub pairs: usize,
    pub manifolds: usize,
    pub contact_points: usize,
    pub step_ms: f32,
}

/// Filter for scene queries.
#[derive(Clone, Copy, Debug)]
pub struct QueryFilter {
    pub groups: CollisionGroups,
    pub exclude_body: Option<BodyHandle>,
    pub include_sensors: bool,
}

impl Default for QueryFilter {
    fn default() -> Self {
        Self { groups: CollisionGroups::ALL, exclude_body: None, include_sensors: false }
    }
}

impl QueryFilter {
    pub fn exclude(body: BodyHandle) -> Self {
        Self { exclude_body: Some(body), ..Default::default() }
    }

    pub fn with_groups(mut self, groups: CollisionGroups) -> Self {
        self.groups = groups;
        self
    }

    fn accepts(&self, c: &Collider) -> bool {
        (self.include_sensors || !c.sensor) && self.groups.test(&c.groups) && self.exclude_body != Some(c.body)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub collider: ColliderHandle,
    pub body: BodyHandle,
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub feature: u32,
}

/// Sensor overlap or contact begin/end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionEvent {
    Started(ColliderHandle, ColliderHandle),
    Stopped(ColliderHandle, ColliderHandle),
}

pub struct PhysicsWorld {
    pub config: PhysicsConfig,
    pub(crate) bodies: Arena<RigidBody>,
    pub(crate) colliders: Arena<Collider>,
    broadphase: Box<dyn Broadphase>,
    pub(crate) pairs: HashMap<(ColliderHandle, ColliderHandle), PairCache>,
    sensors: HashSet<(ColliderHandle, ColliderHandle)>,
    ground: BodyHandle,
    pub events: Vec<CollisionEvent>,
    pub stats: StepStats,
    scratch_manifolds: Vec<Manifold>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new(PhysicsConfig::default())
    }
}

impl PhysicsWorld {
    pub fn new(config: PhysicsConfig) -> Self {
        Self::with_broadphase(config, Box::new(TreeBroadphase::default()))
    }

    pub fn with_broadphase(config: PhysicsConfig, broadphase: Box<dyn Broadphase>) -> Self {
        let mut bodies = Arena::default();
        let ground = BodyHandle(bodies.insert(RigidBodyDesc::fixed().build()));
        Self {
            config,
            bodies,
            colliders: Arena::default(),
            broadphase,
            pairs: HashMap::new(),
            sensors: HashSet::new(),
            ground,
            events: Vec::new(),
            stats: StepStats::default(),
            scratch_manifolds: Vec::new(),
        }
    }

    /// Static body that owns colliders added without a parent.
    pub fn ground(&self) -> BodyHandle {
        self.ground
    }

    pub fn add_body(&mut self, desc: RigidBodyDesc) -> BodyHandle {
        BodyHandle(self.bodies.insert(desc.build()))
    }

    pub fn add_collider(&mut self, desc: ColliderDesc, parent: Option<BodyHandle>) -> ColliderHandle {
        let body_h = parent.unwrap_or(self.ground);
        let body = self.bodies.get(body_h.0).expect("parent body exists");
        assert!(
            !(desc.shape.is_static_only() && body.body_type == BodyType::Dynamic),
            "triangle meshes and height fields can only be attached to static or kinematic bodies"
        );
        let world_iso = body.iso.mul(&desc.local);
        let aabb = desc.shape.aabb(&world_iso);
        let collider = Collider {
            shape: desc.shape,
            local: desc.local,
            density: desc.density,
            friction: desc.friction,
            restitution: desc.restitution,
            groups: desc.groups,
            sensor: desc.sensor,
            user_data: desc.user_data,
            body: body_h,
            proxy: 0,
            world_iso,
            aabb,
        };
        let h = ColliderHandle(self.colliders.insert(collider));
        let proxy = self.broadphase.insert(h, aabb);
        self.colliders.get_mut(h.0).expect("just inserted").proxy = proxy;
        let body = self.bodies.get_mut(body_h.0).expect("parent body exists");
        body.colliders.push(h);
        body.wake();
        self.update_mass(body_h);
        h
    }

    /// Convenience: dynamic body with a single collider.
    pub fn add_dynamic(&mut self, body: RigidBodyDesc, collider: ColliderDesc) -> (BodyHandle, ColliderHandle) {
        let b = self.add_body(body);
        let c = self.add_collider(collider, Some(b));
        (b, c)
    }

    pub fn remove_collider(&mut self, h: ColliderHandle) -> Option<Collider> {
        let c = self.colliders.remove(h.0)?;
        self.broadphase.remove(c.proxy);
        self.pairs.retain(|k, _| k.0 != h && k.1 != h);
        self.sensors.retain(|k| k.0 != h && k.1 != h);
        if let Some(b) = self.bodies.get_mut(c.body.0) {
            b.colliders.retain(|x| *x != h);
            b.wake();
        }
        self.update_mass(c.body);
        Some(c)
    }

    pub fn remove_body(&mut self, h: BodyHandle) -> Option<RigidBody> {
        assert_ne!(h, self.ground, "the ground body cannot be removed");
        let cols = self.bodies.get(h.0)?.colliders.clone();
        for c in cols {
            self.remove_collider(c);
        }
        self.bodies.remove(h.0)
    }

    pub fn body(&self, h: BodyHandle) -> Option<&RigidBody> {
        self.bodies.get(h.0)
    }

    pub fn body_mut(&mut self, h: BodyHandle) -> Option<&mut RigidBody> {
        self.bodies.get_mut(h.0)
    }

    pub fn collider(&self, h: ColliderHandle) -> Option<&Collider> {
        self.colliders.get(h.0)
    }

    pub fn bodies(&self) -> impl Iterator<Item = (BodyHandle, &RigidBody)> {
        self.bodies.iter().map(|(h, b)| (BodyHandle(h), b))
    }

    pub fn colliders(&self) -> impl Iterator<Item = (ColliderHandle, &Collider)> {
        self.colliders.iter().map(|(h, c)| (ColliderHandle(h), c))
    }

    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// Recomputes mass, center of mass and inertia from the body's colliders.
    pub fn update_mass(&mut self, h: BodyHandle) {
        let Some(body) = self.bodies.get(h.0) else { return };
        let parts: Vec<MassProperties> = body
            .colliders
            .iter()
            .filter_map(|c| self.colliders.get(c.0))
            .filter(|c| !c.sensor)
            .map(|c| c.shape.mass_properties(c.density).transformed(&c.local))
            .collect();
        let mut mp = MassProperties::combine(&parts);
        let body = self.bodies.get_mut(h.0).expect("checked");
        if let Some(com) = body.com_override {
            mp.inertia += shift_inertia(mp.mass, mp.com - com);
            mp.com = com;
        }
        body.local_com = mp.com;
        body.mass = mp.mass;
        if body.body_type == BodyType::Dynamic && mp.mass > 0.0 {
            body.inv_mass = 1.0 / mp.mass;
            body.inertia_local = mp.inertia;
            body.inv_inertia_local = mp.inertia.inverse();
        } else {
            body.inv_mass = 0.0;
            body.inertia_local = Mat3::ZERO;
            body.inv_inertia_local = Mat3::ZERO;
        }
    }

    /// Teleports a body, refreshing its colliders immediately.
    pub fn set_body_iso(&mut self, h: BodyHandle, iso: Iso) {
        if let Some(b) = self.bodies.get_mut(h.0) {
            b.set_iso(iso);
        }
        self.sync_body_colliders(h, Vec3::ZERO);
    }

    fn sync_body_colliders(&mut self, h: BodyHandle, displacement: Vec3) {
        let Some(body) = self.bodies.get(h.0) else { return };
        let iso = body.iso;
        for ch in body.colliders.clone() {
            if let Some(c) = self.colliders.get_mut(ch.0) {
                c.world_iso = iso.mul(&c.local);
                c.aabb = c.shape.aabb(&c.world_iso);
                self.broadphase.update(c.proxy, c.aabb, displacement);
            }
        }
    }

    /// Advances the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        let t0 = std::time::Instant::now();
        self.events.clear();
        // 1. Broadphase update for moving bodies.
        let moving: Vec<BodyHandle> = self
            .bodies
            .iter()
            .filter(|(_, b)| b.body_type != BodyType::Static && !b.sleeping)
            .map(|(h, _)| BodyHandle(h))
            .collect();
        for h in &moving {
            let disp = self.bodies.get(h.0).map(|b| b.linvel * dt).unwrap_or_default();
            self.sync_body_colliders(*h, disp);
        }
        // 2. Candidate pairs, waking sleeping bodies we touch.
        let mut queue: Vec<ColliderHandle> =
            moving.iter().flat_map(|h| self.bodies.get(h.0).map(|b| b.colliders.clone()).unwrap_or_default()).collect();
        let mut candidates: HashSet<(ColliderHandle, ColliderHandle)> = HashSet::new();
        let mut qi = 0;
        let mut found = Vec::new();
        while qi < queue.len() {
            let ch = queue[qi];
            qi += 1;
            let Some(c) = self.colliders.get(ch.0) else { continue };
            let fat = self.broadphase.fat_aabb(c.proxy);
            found.clear();
            self.broadphase.query(&fat, &mut |o| found.push(o));
            for &o in &found {
                if o == ch {
                    continue;
                }
                let oc = self.colliders.get(o.0).expect("tree is in sync");
                if oc.body == c.body || !oc.groups.test(&c.groups) {
                    continue;
                }
                let (ba, bb) = (self.bodies.get(c.body.0).expect("body"), self.bodies.get(oc.body.0).expect("body"));
                if ba.body_type != BodyType::Dynamic && bb.body_type != BodyType::Dynamic && !(c.sensor || oc.sensor) {
                    continue;
                }
                if bb.sleeping && bb.body_type == BodyType::Dynamic && c.aabb.intersects(&oc.aabb) && !c.sensor {
                    let ob = oc.body;
                    let cols = bb.colliders.clone();
                    self.bodies.get_mut(ob.0).expect("body").wake();
                    queue.extend(cols);
                }
                candidates.insert(if ch < o { (ch, o) } else { (o, ch) });
            }
        }
        // 3. Narrowphase.
        let base_margin = self.config.speculative_margin;
        let mut new_pairs: HashMap<(ColliderHandle, ColliderHandle), PairCache> = HashMap::with_capacity(candidates.len());
        let mut new_sensors = HashSet::new();
        for key in candidates {
            let (ca, cb) = (self.colliders.get(key.0.0).expect("c"), self.colliders.get(key.1.0).expect("c"));
            self.scratch_manifolds.clear();
            if ca.sensor || cb.sensor {
                narrowphase::collide(&ca.shape, &ca.world_iso, &cb.shape, &cb.world_iso, 0.0, &mut self.scratch_manifolds);
                if self.scratch_manifolds.iter().any(|m| m.min_separation() <= 0.0) {
                    new_sensors.insert(key);
                }
                continue;
            }
            // Speculative distance grows with the closing speed so fast bodies
            // cannot skip through thin geometry in one step.
            let speed = |c: &Collider| self.bodies.get(c.body.0).map_or(0.0, |b| b.linvel.length() + b.angvel.length() * c.shape.local_aabb().half_extents().max_element());
            let margin = base_margin + ((speed(ca) + speed(cb)) * dt).min(2.0);
            narrowphase::collide(&ca.shape, &ca.world_iso, &cb.shape, &cb.world_iso, margin, &mut self.scratch_manifolds);
            if self.scratch_manifolds.is_empty() {
                continue;
            }
            let old = self.pairs.get(&key);
            let cache = PairCache::new(
                &self.scratch_manifolds,
                old,
                mix_friction(ca.friction, cb.friction),
                ca.restitution.max(cb.restitution),
            );
            new_pairs.insert(key, cache);
        }
        for k in new_sensors.difference(&self.sensors) {
            self.events.push(CollisionEvent::Started(k.0, k.1));
        }
        for k in self.sensors.difference(&new_sensors) {
            self.events.push(CollisionEvent::Stopped(k.0, k.1));
        }
        self.sensors = new_sensors;
        for (k, v) in &new_pairs {
            let touching = v.touching();
            let was = self.pairs.get(k).is_some_and(|o| o.touching());
            if touching && !was {
                self.events.push(CollisionEvent::Started(k.0, k.1));
            }
        }
        for (k, v) in &self.pairs {
            if v.touching() && !new_pairs.get(k).is_some_and(|n| n.touching()) {
                self.events.push(CollisionEvent::Stopped(k.0, k.1));
            }
        }
        // Pairs between sleeping bodies are kept so they warm start on wake.
        for (k, v) in self.pairs.drain() {
            if !new_pairs.contains_key(&k) {
                let sleeping = |c: ColliderHandle| {
                    self.colliders.get(c.0).and_then(|c| self.bodies.get(c.body.0)).is_some_and(|b| b.sleeping || b.body_type == BodyType::Static)
                };
                if sleeping(k.0) && sleeping(k.1) {
                    new_pairs.insert(k, v);
                }
            }
        }
        self.pairs = new_pairs;
        // 4. Solve.
        let stats = solver::solve(self, dt);
        self.stats = StepStats { step_ms: t0.elapsed().as_secs_f32() * 1000.0, ..stats };
    }

    pub(crate) fn body_colliders_sync_all(&mut self, handles: &[BodyHandle]) {
        for h in handles {
            self.sync_body_colliders(*h, Vec3::ZERO);
        }
    }

    // -- queries ----------------------------------------------------------

    pub fn raycast(&self, ray: &Ray, max_t: f32, filter: QueryFilter) -> Option<RayHit> {
        let mut best: Option<RayHit> = None;
        self.broadphase.raycast(ray, max_t, &mut |ch, max_t| {
            let Some(c) = self.colliders.get(ch.0) else { return max_t };
            if !filter.accepts(c) {
                return max_t;
            }
            match c.shape.raycast(&c.world_iso, ray, max_t) {
                Some(hit) => {
                    best = Some(RayHit {
                        collider: ch,
                        body: c.body,
                        t: hit.t,
                        point: ray.at(hit.t),
                        normal: hit.normal,
                        feature: hit.feature,
                    });
                    hit.t
                }
                None => max_t,
            }
        });
        best
    }

    /// Colliders whose AABB overlaps `aabb`.
    pub fn query_aabb(&self, aabb: &Aabb, filter: QueryFilter, mut f: impl FnMut(ColliderHandle, &Collider)) {
        self.broadphase.query(aabb, &mut |ch| {
            if let Some(c) = self.colliders.get(ch.0)
                && filter.accepts(c)
                && c.aabb.intersects(aabb)
            {
                f(ch, c);
            }
        });
    }

    /// Contact manifolds between a free-floating shape and the world.
    pub fn shape_contacts(&self, shape: &Shape, iso: &Iso, margin: f32, filter: QueryFilter) -> Vec<(ColliderHandle, Manifold)> {
        let aabb = shape.aabb(iso).expanded(margin);
        let mut out = Vec::new();
        let mut tmp = Vec::new();
        self.query_aabb(&aabb, filter, |ch, c| {
            tmp.clear();
            narrowphase::collide(shape, iso, &c.shape, &c.world_iso, margin, &mut tmp);
            out.extend(tmp.iter().map(|m| (ch, *m)));
        });
        out
    }

    /// Total normal impulse of the last step between two colliders.
    pub fn contact_impulse(&self, a: ColliderHandle, b: ColliderHandle) -> f32 {
        let key = if a < b { (a, b) } else { (b, a) };
        self.pairs.get(&key).map(|p| p.total_impulse()).unwrap_or(0.0)
    }

    /// All touching pairs with their total normal impulse.
    pub fn contact_pairs(&self) -> impl Iterator<Item = (ColliderHandle, ColliderHandle, f32)> + '_ {
        self.pairs.iter().filter(|(_, p)| p.touching()).map(|(k, p)| (k.0, k.1, p.total_impulse()))
    }
}

fn mix_friction(a: f32, b: f32) -> f32 {
    (a * b).sqrt()
}

/// Inertia correction when moving the reference point by `d` away from the COM.
fn shift_inertia(m: f32, d: Vec3) -> Mat3 {
    let dd = d.dot(d);
    (Mat3::IDENTITY * dd - Mat3::from_cols(d * d.x, d * d.y, d * d.z)) * m
}
