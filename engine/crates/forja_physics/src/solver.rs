//! Contact solver: soft-step sequential impulses with substepping, warm
//! starting, relaxation and restitution (after Box2D v3 "Solver2D" soft step,
//! extended to 3D).

use forja_math::{Mat3, Quat, Vec3, orthonormal_basis};

use crate::body::{BodyHandle, BodyType, ColliderHandle};
use crate::narrowphase::Manifold;
use crate::world::{PhysicsWorld, StepStats};

#[derive(Clone, Debug)]
pub(crate) struct CachedManifold {
    pub m: Manifold,
    /// Per point: normal, tangent1, tangent2 impulse.
    pub impulses: [[f32; 3]; 4],
}

#[derive(Clone, Debug)]
pub(crate) struct PairCache {
    pub manifolds: Vec<CachedManifold>,
    pub friction: f32,
    pub restitution: f32,
}

impl PairCache {
    pub fn new(manifolds: &[Manifold], old: Option<&PairCache>, friction: f32, restitution: f32) -> Self {
        let manifolds = manifolds
            .iter()
            .map(|m| {
                let mut impulses = [[0.0; 3]; 4];
                if let Some(om) = old.and_then(|o| o.manifolds.iter().find(|om| om.m.key == m.key && om.m.normal.dot(m.normal) > 0.95)) {
                    for (i, p) in m.points().iter().enumerate() {
                        if let Some(j) = om.m.points().iter().position(|q| q.id == p.id) {
                            impulses[i] = om.impulses[j];
                        }
                    }
                }
                CachedManifold { m: *m, impulses }
            })
            .collect();
        Self { manifolds, friction, restitution }
    }

    pub fn touching(&self) -> bool {
        self.manifolds.iter().any(|m| m.m.min_separation() <= 0.005)
    }

    pub fn total_impulse(&self) -> f32 {
        self.manifolds.iter().map(|m| m.impulses[..m.m.len()].iter().map(|i| i[0]).sum::<f32>()).sum()
    }
}

#[derive(Clone, Copy)]
struct SBody {
    v: Vec3,
    w: Vec3,
    inv_m: f32,
    inv_i: Mat3,
    dp: Vec3,
    dq: Quat,
    com0: Vec3,
    rot0: Quat,
    /// Constant acceleration from gravity and applied forces.
    accel: Vec3,
    ang_accel: Vec3,
    lin_damp: f32,
    ang_damp: f32,
    dynamic: bool,
}

impl SBody {
    const STATIC: SBody = SBody {
        v: Vec3::ZERO,
        w: Vec3::ZERO,
        inv_m: 0.0,
        inv_i: Mat3::ZERO,
        dp: Vec3::ZERO,
        dq: Quat::IDENTITY,
        com0: Vec3::ZERO,
        rot0: Quat::IDENTITY,
        accel: Vec3::ZERO,
        ang_accel: Vec3::ZERO,
        lin_damp: 0.0,
        ang_damp: 0.0,
        dynamic: false,
    };
}

#[derive(Clone, Copy, Default)]
struct CPoint {
    ra: Vec3,
    rb: Vec3,
    adjusted_sep: f32,
    normal_mass: f32,
    tangent_mass: [f32; 2],
    normal_impulse: f32,
    tangent_impulse: [f32; 2],
    max_normal_impulse: f32,
    relative_velocity: f32,
}

struct Constraint {
    a: usize,
    b: usize,
    normal: Vec3,
    tangents: [Vec3; 2],
    friction: f32,
    restitution: f32,
    points: [CPoint; 4],
    count: usize,
    key: (ColliderHandle, ColliderHandle),
    manifold: usize,
}

#[derive(Clone, Copy)]
struct Softness {
    bias_rate: f32,
    mass_scale: f32,
    impulse_scale: f32,
}

fn make_soft(hertz: f32, zeta: f32, h: f32) -> Softness {
    if hertz == 0.0 {
        return Softness { bias_rate: 0.0, mass_scale: 1.0, impulse_scale: 0.0 };
    }
    let omega = 2.0 * std::f32::consts::PI * hertz;
    let a1 = 2.0 * zeta + h * omega;
    let a2 = h * omega * a1;
    let a3 = 1.0 / (1.0 + a2);
    Softness { bias_rate: omega / a1, mass_scale: a2 * a3, impulse_scale: a3 }
}

#[inline]
fn eff_mass(a: &SBody, b: &SBody, ra: Vec3, rb: Vec3, dir: Vec3) -> f32 {
    let rna = ra.cross(dir);
    let rnb = rb.cross(dir);
    let k = a.inv_m + b.inv_m + rna.dot(a.inv_i * rna) + rnb.dot(b.inv_i * rnb);
    if k > 0.0 { 1.0 / k } else { 0.0 }
}

#[inline]
fn apply(bodies: &mut [SBody], a: usize, b: usize, ra: Vec3, rb: Vec3, p: Vec3) {
    // Index 0 is the shared static body; never write to it.
    if a != 0 {
        let ba = &mut bodies[a];
        ba.v -= p * ba.inv_m;
        ba.w -= ba.inv_i * ra.cross(p);
    }
    if b != 0 {
        let bb = &mut bodies[b];
        bb.v += p * bb.inv_m;
        bb.w += bb.inv_i * rb.cross(p);
    }
}

#[inline]
fn rel_vel(bodies: &[SBody], a: usize, b: usize, ra: Vec3, rb: Vec3) -> Vec3 {
    let (ba, bb) = (&bodies[a], &bodies[b]);
    (bb.v + bb.w.cross(rb)) - (ba.v + ba.w.cross(ra))
}

pub(crate) fn solve(world: &mut PhysicsWorld, dt: f32) -> StepStats {
    let cfg = world.config.clone();
    let substeps = cfg.substeps.max(1);
    let h = dt / substeps as f32;
    let inv_h = 1.0 / h;
    let soft = make_soft(cfg.contact_hertz.min(0.25 / h), cfg.contact_damping_ratio, h);

    // Solver bodies. Slot 0 is the shared static body.
    let mut sbodies = vec![SBody::STATIC];
    let mut handles: Vec<BodyHandle> = vec![world.ground()];
    let mut map = vec![0usize; world.bodies.capacity()];
    for (hnd, b) in world.bodies.iter() {
        if b.body_type == BodyType::Static || b.sleeping {
            continue;
        }
        let dynamic = b.body_type == BodyType::Dynamic;
        map[hnd.index as usize] = sbodies.len();
        handles.push(BodyHandle(hnd));
        let accel = if dynamic { cfg.gravity * b.gravity_scale + b.force * b.inv_mass } else { Vec3::ZERO };
        let inv_i = if dynamic { b.inv_inertia_world() } else { Mat3::ZERO };
        sbodies.push(SBody {
            v: b.linvel,
            w: b.angvel,
            inv_m: if dynamic { b.inv_mass } else { 0.0 },
            inv_i,
            dp: Vec3::ZERO,
            dq: Quat::IDENTITY,
            com0: b.com(),
            rot0: b.iso.rot,
            accel,
            ang_accel: inv_i * b.torque,
            lin_damp: 1.0 / (1.0 + h * b.linear_damping),
            ang_damp: 1.0 / (1.0 + h * b.angular_damping),
            dynamic,
        });
    }

    // Constraints.
    let mut constraints: Vec<Constraint> = Vec::new();
    let mut contact_points = 0;
    for (key, pair) in world.pairs.iter() {
        let (Some(ca), Some(cb)) = (world.colliders.get(key.0.0), world.colliders.get(key.1.0)) else { continue };
        let (ia, ib) = (map[ca.body.0.index as usize], map[cb.body.0.index as usize]);
        if !sbodies[ia].dynamic && !sbodies[ib].dynamic {
            continue;
        }
        for (mi, cm) in pair.manifolds.iter().enumerate() {
            let n = cm.m.normal;
            let (t1, t2) = orthonormal_basis(n);
            let mut c = Constraint {
                a: ia,
                b: ib,
                normal: n,
                tangents: [t1, t2],
                friction: pair.friction,
                restitution: pair.restitution,
                points: [CPoint::default(); 4],
                count: cm.m.len(),
                key: *key,
                manifold: mi,
            };
            let (ba, bb) = (sbodies[ia], sbodies[ib]);
            for (k, p) in cm.m.points().iter().enumerate() {
                let ra = if ia == 0 { p.point } else { p.point - ba.com0 };
                let rb = if ib == 0 { p.point } else { p.point - bb.com0 };
                let cp = &mut c.points[k];
                cp.ra = ra;
                cp.rb = rb;
                cp.adjusted_sep = p.separation - (rb - ra).dot(n);
                cp.normal_mass = eff_mass(&ba, &bb, ra, rb, n);
                cp.tangent_mass = [eff_mass(&ba, &bb, ra, rb, t1), eff_mass(&ba, &bb, ra, rb, t2)];
                cp.normal_impulse = cm.impulses[k][0];
                cp.tangent_impulse = [cm.impulses[k][1], cm.impulses[k][2]];
                cp.relative_velocity = n.dot(rel_vel(&sbodies, ia, ib, ra, rb));
            }
            contact_points += c.count;
            constraints.push(c);
        }
    }
    // Static anchors are world points: slot 0 never moves, so its identity
    // dp/dq keep the separation update exact.

    let max_rot = 0.25 * std::f32::consts::PI;
    for _ in 0..substeps {
        // Integrate velocities.
        for b in sbodies.iter_mut().skip(1) {
            if !b.dynamic {
                continue;
            }
            b.v = (b.v + b.accel * h) * b.lin_damp;
            b.w = (b.w + b.ang_accel * h) * b.ang_damp;
            let speed = b.v.length();
            if speed > cfg.max_linear_speed {
                b.v *= cfg.max_linear_speed / speed;
            }
            let wl = b.w.length() * h;
            if wl > max_rot {
                b.w *= max_rot / wl;
            }
        }
        // Warm start.
        for c in &constraints {
            for p in &c.points[..c.count] {
                let imp = c.normal * p.normal_impulse + c.tangents[0] * p.tangent_impulse[0] + c.tangents[1] * p.tangent_impulse[1];
                apply(&mut sbodies, c.a, c.b, p.ra, p.rb, imp);
            }
        }
        solve_contacts(&mut constraints, &mut sbodies, soft, inv_h, cfg.max_push_velocity, true);
        // Integrate positions.
        for b in sbodies.iter_mut().skip(1) {
            b.dp += b.v * h;
            let w = b.w;
            let dq = b.dq + Quat::from_xyzw(w.x, w.y, w.z, 0.0) * b.dq * (0.5 * h);
            b.dq = dq.normalize();
        }
        for _ in 0..cfg.relax_iterations.max(1) {
            solve_contacts(&mut constraints, &mut sbodies, soft, inv_h, cfg.max_push_velocity, false);
        }
    }
    // Restitution.
    for c in &mut constraints {
        if c.restitution == 0.0 {
            continue;
        }
        let n = c.normal;
        for k in 0..c.count {
            let p = c.points[k];
            if p.relative_velocity > -cfg.restitution_threshold || p.max_normal_impulse == 0.0 {
                continue;
            }
            let vn = n.dot(rel_vel(&sbodies, c.a, c.b, p.ra, p.rb));
            let imp = -p.normal_mass * (vn + c.restitution * p.relative_velocity);
            let new = (p.normal_impulse + imp).max(0.0);
            let delta = new - p.normal_impulse;
            c.points[k].normal_impulse = new;
            apply(&mut sbodies, c.a, c.b, p.ra, p.rb, n * delta);
        }
    }
    // Store impulses for warm starting.
    for c in &constraints {
        if let Some(pair) = world.pairs.get_mut(&c.key) {
            let cm = &mut pair.manifolds[c.manifold];
            for k in 0..c.count {
                let p = &c.points[k];
                cm.impulses[k] = [p.normal_impulse, p.tangent_impulse[0], p.tangent_impulse[1]];
            }
        }
    }
    // Write back and update sleep timers.
    let mut awake = 0;
    for (i, sb) in sbodies.iter().enumerate().skip(1) {
        let b = world.bodies.get_mut(handles[i].0).expect("body exists");
        let rot = (sb.dq * sb.rot0).normalize();
        let com = sb.com0 + sb.dp;
        b.iso.rot = rot;
        b.iso.pos = com - rot * b.local_com;
        b.linvel = sb.v;
        b.angvel = sb.w;
        b.force = Vec3::ZERO;
        b.torque = Vec3::ZERO;
        let still = sb.v.length_squared() < cfg.sleep_linear_threshold.powi(2)
            && sb.w.length_squared() < cfg.sleep_angular_threshold.powi(2);
        b.sleep_time = if b.can_sleep && still && sb.dynamic { b.sleep_time + dt } else { 0.0 };
        awake += 1;
    }
    // Islands: union-find over dynamic bodies linked by contacts.
    let mut parent: Vec<usize> = (0..sbodies.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut kinematic_touch = vec![false; sbodies.len()];
    for c in &constraints {
        let (da, db) = (sbodies[c.a].dynamic, sbodies[c.b].dynamic);
        if da && db {
            let (ra, rb) = (find(&mut parent, c.a), find(&mut parent, c.b));
            parent[ra] = rb;
        } else if c.a != 0 && !da && sbodies[c.a].v.length_squared() > 0.0 {
            kinematic_touch[c.b] = true;
        } else if c.b != 0 && !db && sbodies[c.b].v.length_squared() > 0.0 {
            kinematic_touch[c.a] = true;
        }
    }
    let mut island_min = vec![f32::MAX; sbodies.len()];
    for i in 1..sbodies.len() {
        if !sbodies[i].dynamic {
            continue;
        }
        let r = find(&mut parent, i);
        let t = if kinematic_touch[i] { 0.0 } else { world.bodies.get(handles[i].0).expect("body").sleep_time };
        island_min[r] = island_min[r].min(t);
    }
    let mut to_sync = Vec::with_capacity(awake);
    for i in 1..sbodies.len() {
        to_sync.push(handles[i]);
        if !sbodies[i].dynamic {
            continue;
        }
        let r = find(&mut parent, i);
        if island_min[r] >= cfg.time_to_sleep {
            let b = world.bodies.get_mut(handles[i].0).expect("body");
            b.sleeping = true;
            b.linvel = Vec3::ZERO;
            b.angvel = Vec3::ZERO;
        }
    }
    world.body_colliders_sync_all(&to_sync);
    StepStats {
        awake_bodies: awake,
        pairs: world.pairs.len(),
        manifolds: constraints.len(),
        contact_points,
        step_ms: 0.0,
    }
}

fn solve_contacts(
    constraints: &mut [Constraint],
    bodies: &mut [SBody],
    soft: Softness,
    inv_h: f32,
    max_push: f32,
    use_bias: bool,
) {
    for c in constraints.iter_mut() {
        let n = c.normal;
        let (a, b) = (c.a, c.b);
        for k in 0..c.count {
            let p = c.points[k];
            let (ba, bb) = (&bodies[a], &bodies[b]);
            // Current separation from the accumulated position deltas.
            let pra = ba.dq * p.ra;
            let prb = bb.dq * p.rb;
            let d = (bb.dp - ba.dp) + (prb - pra);
            let s = n.dot(d) + p.adjusted_sep;
            let (mut bias, mut mass_scale, mut impulse_scale) = (0.0, 1.0, 0.0);
            if s > 0.0 {
                bias = s * inv_h; // speculative
            } else if use_bias {
                bias = (soft.bias_rate * s).max(-max_push);
                mass_scale = soft.mass_scale;
                impulse_scale = soft.impulse_scale;
            }
            let vn = n.dot(rel_vel(bodies, a, b, p.ra, p.rb));
            let imp = -p.normal_mass * mass_scale * (vn + bias) - impulse_scale * p.normal_impulse;
            let new = (p.normal_impulse + imp).max(0.0);
            let delta = new - p.normal_impulse;
            let cp = &mut c.points[k];
            cp.normal_impulse = new;
            cp.max_normal_impulse = cp.max_normal_impulse.max(delta);
            apply(bodies, a, b, p.ra, p.rb, n * delta);
        }
        for k in 0..c.count {
            let p = c.points[k];
            let dv = rel_vel(bodies, a, b, p.ra, p.rb);
            let max_f = c.friction * p.normal_impulse;
            let mut ti = p.tangent_impulse;
            let old = ti;
            for j in 0..2 {
                ti[j] += -p.tangent_mass[j] * dv.dot(c.tangents[j]);
            }
            let len = (ti[0] * ti[0] + ti[1] * ti[1]).sqrt();
            if len > max_f && len > 0.0 {
                let s = max_f / len;
                ti[0] *= s;
                ti[1] *= s;
            }
            c.points[k].tangent_impulse = ti;
            let imp = c.tangents[0] * (ti[0] - old[0]) + c.tangents[1] * (ti[1] - old[1]);
            apply(bodies, a, b, p.ra, p.rb, imp);
        }
    }
}
