# Physics research notes for Forja

These notes record the upgrade of `forja_physics` (broadphase, narrowphase,
soft-step solver, vehicles, bike, character) against the default procedural
city: what was measured, which published techniques were applied, and what
they bought. Each technique is weighed against what the engine already had.

**Status**: `done` means it is now in `forja_physics`, `existing` means it was
there before this pass, `evaluated` means it was considered and not worth it
at Forja's scale (with the reason), and `next` is a recommendation.

**Budget.** The target machine is a Samsung Galaxy Book4 Pro (Intel Core
Ultra 5/7 "Meteor Lake", power-limited, P+E cores). We assume it runs at about
60% of the 4-core desktop container the numbers below come from, so the
"< 2 ms of physics per frame" goal means < 1.2 ms here. Everything is single
threaded: the whole step is ~0.07 ms in the game scene, far below the point
where splitting it into tasks pays for itself on hybrid cores.

## How the city reaches physics

`SpanishCity::build` (the default map, 146 237 render placements) emits:

| Collider | Count | Notes |
|---|---|---|
| Triangle mesh | 1 | 265 795 triangles, 643 481 vertices, 1179 x 43 x 413 m: facades, kerbs, stair streets, ramps, all in one mesh |
| Cuboid | 1 | the ground slab under the whole map |
| Capsule | 629 | tree trunks |

The mesh's BVH answers every query (pairs, rays, sweeps) in O(log n), so one
big mesh is fine for the broadphase: every moving body pairs with it once and
the narrowphase walks only the triangles near the body. Building the mesh BVH
takes ~0.1 s; the city build itself takes ~95 s and ~3.6 GB because of the
render content, not physics. Vertices and indices take 10.4 MB.

## Measuring

`cargo run --release -p forja_world --example physbench -- [micro|city] [--game] [--cache FILE]`

* `city` builds the default city (or loads the colliders from `--cache`, which
  the first run writes) and runs 900 steps at 60 Hz around the player spawn,
  timing every stage. `--game` is the sandbox's load: 8 cars, the bike, the
  player, 10 crates and 12 kinematic AI cars (the traffic plugin gives
  colliders to the AI cars within 45 m). The default is a stress load: 24
  character controllers, 60 crates and 24 AI cars.
* `micro` runs the stability scenes: stacking and jitter, tunnelling at
  50 m/s, debris on kerbs and stairs, the character on slopes and stairs, the
  bike parked and ridden, and a car on a 12° alley.

Before = the engine at `b64f4ab` with only the stage timers added. Times are
in ms per 60 Hz step on the shared 4-core container (other jobs were running,
so maxima are noise; read the mean, median and p95).

### Game scene (sandbox load)

| Stage (ms) | Before mean | After mean | Before p95 | After p95 |
|---|---|---|---|---|
| **Total physics per step** | **0.325** | **0.164** | 0.378 | 0.186 |
| `PhysicsWorld::step` | 0.203 | 0.073 | 0.282 | 0.111 |
| broadphase + pairs | 0.012 | 0.011 | 0.015 | 0.012 |
| narrowphase | 0.122 | 0.041 | 0.155 | 0.068 |
| solver | 0.062 | 0.014 | 0.099 | 0.023 |
| write-back + islands | 0.008 | 0.006 | 0.009 | 0.017 |
| vehicles (rays, tyres) | 0.055 | 0.077 | 0.063 | 0.070 |
| character | 0.066 | 0.014 | 0.122 | 0.021 |
| contact points (mean) | 46 | 12 | | |

At one fixed step per frame that is ~0.16 ms on the desktop and ~0.3 ms on the
laptop estimate, well inside the 2 ms budget. Vehicles cost a little more
because each wheel now integrates its spin with four implicit sub-steps.

### Stress scene

| Stage (ms) | Before mean | After mean | Before p95 | After p95 |
|---|---|---|---|---|
| **Total physics per step** | **4.060** | **1.201** | 8.253 | 2.344 |
| `PhysicsWorld::step` | 0.798 | 0.470 | 1.046 | 0.620 |
| narrowphase | 0.422 | 0.244 | 0.523 | 0.326 |
| solver | 0.280 | 0.123 | 0.328 | 0.145 |
| vehicles | 0.103 | 0.105 | 0.120 | 0.123 |
| 24 characters | 3.152 | 0.623 | 6.959 | 0.825 |
| contact points (mean / max) | 200 / 428 | 86 / 226 | | |

### Memory

| | Before | After |
|---|---|---|
| City mesh BVH | 17.2 MB (nodes allocated for 2N, 4x what was used) | 6.1 MB (binned SAH, exact size) |
| Mesh vertices + indices | 10.4 MB | 10.4 MB |
| BVH build | 66-74 ms | ~110 ms (SAH binning) |

### Stability

| Check | Before | After |
|---|---|---|
| Stack of 10 boxes, 4 s, no sleeping: max speed at rest | 0.002-0.034 m/s, different every run (hash-map solve order) | 0.002 m/s, identical every run |
| Pyramid of 55 boxes | top 9 mm low, stable | same |
| Sphere / box / capsule at 50 m/s with 20 rad/s spin into a 5 cm box wall | stopped | stopped |
| ... into a single-sided mesh wall (a facade) | **all three tunnelled** | all stopped |
| Sedan at 50 m/s into a mesh wall | stopped | stopped |
| 60 small boxes and spheres dropped on kerb + stairs | max sink 0.6 mm | max sink 0.9 mm |
| Character up 17/30 cm stairs: largest per-frame height jump (smoothed) | 0.273 m (pops) | 0.071 m raw, 0.055 m smoothed |
| Character on stairs: vertical velocity jerk | 28.6 m/s | 3.8 m/s |
| Character on 30° / 40° slopes, up and down | grounded 100% | grounded 100%, walking speed kept along the slope |
| Character update cost (micro scenes) | 0.012-0.026 ms | 0.002-0.004 ms |
| Bike parked 10 s | held upright by a hidden torque, never sleeps | rests on its kickstand at 15°, sleeps |
| Parked bike nudged (25 N s) / shoved (120 N s) | unmoved (torque) | stays on the stand / falls over |
| Bike ridden, 20 s slalom | **fell (90°)** | max lean 46°, never falls, 43 m/s |
| Bike stopped and left | upright by the torque | drops its stand, rests on it |
| Sedan parked on a 12° alley with the handbrake | creeps 0.27 m in 4 s | 0.00 m, falls asleep |
| Sedan full throttle up the 12° alley, 3 s | 9.8 m/s | 7.1 m/s (traction-limited launch, wheel inertia) |
| Parked cars | never sleep (suspension forces wake them every step) | sleep |

## Techniques

| # | Technique | Reference | Impact | Cost | Status |
|---|---|---|---|---|---|
| 1 | Soft step (TGS-soft) with sub-steps and relax iterations | Catto, "Solver2D" (2024) and Box2D v3; Macklin et al., "Small Steps in Physics Simulation" (SCA 2019) | Stiff, stable stacks at 60 Hz with 4 sub-steps | the solver | existing |
| 2 | Deterministic solve order (sorted pairs) | Box2D v3 determinism notes | Identical results every run; jitter no longer depends on hash seeds | sort of ~100 keys | done |
| 3 | Speculative contacts with a speed-scaled margin | Catto, "Speculative contacts" (GDC 2013 / Box2D v3) | No tunnelling of fast linear motion | existing | existing, **fixed** |
| 4 | Continuous collision (time of impact vs static) for fast bodies | Box2D v3 `b2SolveContinuous`; Mirtich, "Impulse-based dynamic simulation of rigid body systems" (1996), conservative advancement | Spinning bodies no longer sweep a corner through walls and facades | only bodies moving > half their thickness per step | done |
| 5 | Triangle-soup contact reduction and internal-edge removal | Gregorius, "Robust Contact Creation for Physics Simulations" (GDC 2015); Jolt `ContactConstraintManager` / `InternalEdgeRemovingCollector` | Merges per-triangle manifolds into <= 4 points per normal, drops seam edge contacts: 55-75% fewer contact points, no snagging on mesh seams | per mesh pair | done |
| 6 | Allocation-free narrowphase, analytic capsule-triangle | Ericson, *Real-Time Collision Detection* (2005) §5.1 | Removes 5-6 heap allocations per box/triangle test and the 26-evaluation line search per capsule-triangle | none | done |
| 7 | Warm starting with stable feature keys | Catto, "Iterative Dynamics with Temporal Coherence" (GDC 2005) | Merged mesh manifolds keep their impulses (quantised normal as key) | none | done (keys), existing (warm start) |
| 8 | Binned SAH static BVH for the city mesh | Wald, "On fast construction of SAH-based bounding volume hierarchies" (2007) | Tighter tree for ray and pair queries; 17.2 → 6.1 MB with exact allocation | +40 ms at load | done |
| 9 | Quantised / compressed BVH nodes | Mahovsky & Wyvill, "Memory-conserving bounding volume hierarchies" (2006); Ylitie et al., "Efficient incoherent ray traversal on GPUs through compressed wide BVHs" (2017) | Would save ~3 MB of 6 | decode cost per node | evaluated: not worth at 6 MB |
| 10 | Streamed, cell-based static colliders | Box2D v3 static tree split; open-world streaming practice | Bounded memory on bigger maps | split at build | next (for maps > ~1 M triangles) |
| 11 | Graph colouring + SIMD constraint batches | Catto, "Solver2D" / Box2D v3 graph colouring; "SIMD Matters" (2024) | 2-4x solver throughput on thousands of contacts | wide restructuring | evaluated: the game has 12-90 contact points per step (solver 0.014 ms) |
| 12 | Parallel islands / narrowphase (rayon) | Box2D v3 task system; Jolt job system | Scales thousands of bodies | task overhead; uneven on hybrid P/E cores | evaluated: whole step 0.07 ms, tasks would cost more than they save |
| 13 | XPBD rigid bodies | Müller et al., "Detailed Rigid Body Simulation with Extended Position Based Dynamics" (SCA 2020) | Best for joints, ragdolls, large mass ratios | | evaluated: TGS-soft is the same sub-stepping family and handles contacts equally; next for ragdolls and joints |
| 14 | Sleeping islands | Catto (Box2D) | | | existing; vehicles now sleep (non-waking forces) |
| 15 | Simulation focus (activation radius) | open-world practice (GTA-style ambient population) | Bodies beyond 160 m of the player are frozen; far parked cars and debris cost nothing | one distance test per awake body | done |
| 16 | Fixed timestep with render interpolation | Fiedler, "Fix Your Timestep!" (2004) | No judder at 75-240 Hz or uneven frame rates | one slerp per rendered body | done |
| 17 | Pacejka Magic Formula, combined slip on normalised slip | Pacejka & Bakker, "The Magic Formula Tyre Model" (1992); Pacejka, *Tire and Vehicle Dynamics* (3rd ed., 2012) ch. 4; Beckman, "The Physics of Racing" part 25 | Real slip ratio and slip angle, braking or wheelspin trade off against cornering grip | ~0.009 ms per car | done |
| 18 | Brush tyre model | Pacejka 2012 ch. 3 | | | evaluated: MF chosen because its coefficients map onto the existing `grip` / `slide_grip` / `peak_slip_deg` handling data |
| 19 | Wheel spin dynamics with implicit sub-steps; static friction at walking pace | Bernard & Clover, "Tire Modeling for Low-Speed and High-Speed Calculations" (SAE 950311, 1995) | Stable stiff tyres at 60 Hz, no creeping or ringing at rest, parked cars hold on hills | 4 scalar sub-steps per wheel | done |
| 20 | Traction control / ABS on combined slip | standard vehicle-dynamics practice | Arcade-friendly launches and stops, no power oversteer | none | done (`traction_control`, 0 disables) |
| 21 | Motorcycle lean: balance steer, countersteer, upright constraint | Sharp, "The stability and control of motorcycles" (1971); Limebeer & Sharp, "Bicycles, motorcycles and models" (IEEE CSM 2006); Meijaard et al., Whipple bicycle benchmark (Proc. R. Soc. A 2007) | Leans into turns, countersteers, never lies down while ridden | none | done |
| 22 | Toroidal tyre contact and kickstand | motorcycle tyre geometry (crown radius) | A leaning bike pivots about its tyres; parked it rests on the stand, shoved it falls | one extra sphere collider when parked | done |
| 23 | Swept collide-and-slide character | Fauerby, "Improved Collision detection and Response" (2003); PhysX CCT / Jolt `CharacterVirtual` | Step-up, snap-down, slope limit, creases, moving platforms; 5x cheaper | 3-6 sweeps per update | done |
| 24 | Step smoothing of the rendered character | Source engine / Unreal step smoothing | Glides up stairs and kerbs instead of popping | none | done |

### 2-4. Solver, determinism and fast bodies

The soft-step solver (Box2D v3's TGS-soft, extended to 3D, 4 sub-steps and a
relax pass) was already sound and is kept. Two bugs undermined it:

* **Broadphase tunnelling.** After the solve, colliders were re-synced to the
  broadphase with a zero displacement, so a fast body's fat AABB never
  reached ahead and the pair with a thin wall only appeared after the body
  had passed it. The fat boxes are now stretched along the predicted motion
  of the next step. This alone fixed spheres and boxes through facades.
* **Order-dependent results.** Pairs were solved in `HashMap` order, which
  changes every run. Pairs are now sorted, so the same input gives the same
  output (a regression test checks it).

Speculative contacts stop fast translation but not fast rotation: a spinning
capsule's far end can cross a thin wall within one step. Fast bodies (moving
more than half their thickness per step) are now swept from their start to
their end pose against static geometry by conservative advancement and
clamped at the time of impact, as Box2D v3 does. Pairs already touching at
the start are ignored so cars can scrape along walls.

### 5-8. Narrowphase and the city mesh

The per-pair narrowphase allocated vectors for every box and triangle
(vertices, normals, planes, face loops) and searched capsule-triangle
distances with a 26-evaluation golden-section line search. It now uses inline
buffers and an analytic segment-triangle test.

Every triangle touching a body produced its own manifold of up to four
points, so a crate on a tessellated pavement could carry a dozen manifolds,
and spheres rolling over triangle seams hit the shared edges ("ghost
collisions"). Manifolds from one mesh are now merged by normal and reduced to
four points (deepest, farthest, largest area, farthest outside), and edge or
vertex contacts that duplicate a neighbouring face contact are dropped. The
merged manifold's key is its quantised normal, so warm starting survives
triangles entering and leaving it.

The mesh BVH was a median split that reserved nodes for 2N primitives (4x the
nodes it built). It is now a binned SAH build (12 bins) with exact
allocation: 6.1 MB instead of 17.2 MB.

### 15-16. Doing less, and smoothly

* Vehicle suspension pushed every step and woke the chassis, so parked cars
  and the bike never slept and were simulated against the city mesh forever.
  Controllers now add forces without waking (`RigidBody::add_force_at_point`)
  and a sleeping vehicle with idle controls is skipped.
* `PhysicsConfig::active_radius` with `PhysicsWorld::set_focus` freezes dynamic
  bodies far from the player (the sandbox uses 160 m around the player or the
  car they drive). AI traffic already only gets kinematic proxies within 45 m.
* Physics runs at a fixed 60 Hz in `Stage::FixedUpdate`. Bodies now keep
  their pose from the start of the step and `RigidBody::interpolated_iso(alpha)`
  blends to the current one with `Time::alpha`. Cars, wheels, crates, the
  player (`CharacterController::interpolated_feet`) and the camera are drawn
  from the interpolated poses.

### 17-20. Cars

The tyre model was an arcade grip curve with a friction circle and wheels
that spun at road speed. Each wheel now has spin state, and the force comes
from the Magic Formula on the normalised combined slip
`s = |(κ / κ_peak, tan α / tan α_peak)|`, `F = μ F_z sin(C atan(B s))`, split
along the slip direction, so braking or wheelspin cost cornering grip. The
coefficients follow from the handling data: `C` puts the sliding plateau at
`slide_grip`, `B` puts the peak at `peak_slip_deg` and `peak_slip_ratio`.

Stiff tyres at 60 Hz need care. Each wheel integrates
`I dω/dt = T_drive - T_brake - F_x r` with four implicit sub-steps, and the
road force applied to the chassis is the one consistent with the spin change.
Below walking pace the slip curves are replaced by a static-friction
constraint on the contact (using the chassis' effective mass at the contact
point and cancelling the wheel's share of gravity), so parked cars neither
creep nor ring. The gearbox shifts on road speed (as an automatic's output
shaft), the engine revs with wheelspin, and traction control and ABS act on
the combined slip. (The steering, the assists and the wheel-spin
integrator were reworked later: see "Handling pass" below.)

### 21-22. The motorbike

The bike was held upright by a stiff PD torque that worked parked but failed
under load: its rays lost the ground at high lean, its pitch damping also
damped the yaw of the turn (so the heading fell behind the velocity and it
spun out), and parked bikes stood upright by magic. The new model:

* **Ridden**: the steering input sets a target lean (full lock = 45°, eased
  in up to ~12 m/s). The front wheel steers to the balance angle of the
  current lean, `δ = atan(L g tan φ / v²)`, plus a countersteer on the lean
  error that scales with `1/v²` like a real bike's. A velocity-level upright
  constraint drives the roll rate towards the lean target with a bounded
  rider strength, rolling about the tyre contact line rather than the centre
  of mass. Below 2.5 m/s the rider puts a foot down and holds it upright.
  Cornering forces act at the road, so they balance gravity at the right lean.
* **Tyre geometry**: the wheels cast straight down and roll on the tyre's
  crown (radius 9 cm), so a leaning bike keeps its contact patch and pivots
  about the line under its wheels.
* **Parked** (`Vehicle::rider == false`, set by the sandbox for every bike the
  player is not riding): the bike drops a kickstand (a massless sphere
  collider on its left side) and rests on it at ~15°; a shove knocks it over
  and a bike let go while rolling falls over like a real one. The sandbox now
  spawns it at its ride height already leaning on the stand: dropped from
  0.8 m (as before) it bounced off its suspension and fell on its side, which
  `apps/sandbox/tests/physics_game.rs` caught.

### 23-24. The character

The controller moved by small steps and pushed itself out of whatever it
overlapped, querying contacts four times per step; on stairs it popped up to
27 cm in a frame. It is now a swept collide-and-slide controller:
`PhysicsWorld::cast_shape` sweeps the capsule by conservative advancement,
the remaining motion slides along the contact plane (and along the crease of
two planes), and walking motion follows walkable ground at the walking speed.
Kerbs and stair noses are climbed by rolling over their edges: a contact on
the capsule's rounded bottom whose top face is walkable counts as support
(a short probe beyond the contact finds that face). Higher steps use
"up, forward, down". While grounded the capsule is swept down by the snap
distance, so it stays on stairs and downhill streets. Steep slopes are walls
while walking and slides in the air, and standing on a kinematic or dynamic
body carries the character with it. The vertical pops of steps and snaps go
into a decaying visual offset, so the avatar and camera glide.

## Public API changes

All callers in the workspace were updated; existing call sites compile as
before.

* New: `PhysicsWorld::cast_shape`, `ShapeHit`, `set_focus` / `focus`,
  `interpolated_iso`; `PhysicsConfig::active_radius`; `StepStats` stage times;
  `RigidBody::prev_iso`, `interpolated_iso`, `add_force_at_point`, `add_torque`;
  `StaticBvh::memory_bytes`.
* `CharacterController`: `prev_position`, `visual_offset`, `smoothed_feet`,
  `interpolated_feet`, `interpolated_position`, `carry_to`; `CharacterState`
  gains `ground_body`, `ground_velocity`, `on_steep`, `stepped`;
  `CharacterConfig::step_smoothing`.
* `Vehicle`: `rider`, `lean_target`, `lean()`, `wheel_iso_at`;
  `VehicleConfig::{peak_slip_ratio, wheel_inertia, traction_control}`;
  `BikeConfig::{countersteer, max_roll_rate, feet_down_speed, kickstand,
  kickstand_lean_deg, tyre_crown}`; `WheelState::slip_ratio`. New handling
  fields default to the sedan's values, so the TOML files load unchanged.

## Handling pass: cars a keyboard can drive

Report: "cars lose control far too easily" and "cars stop dead on small road
bumps". Both were measured with a keyboard-like driver (binary throttle and
brake, steering keys fully down or released) in
`crates/forja_gameplay/tests/handling.rs`; `cargo test -p forja_gameplay
--test handling -- --ignored --nocapture` prints the whole table for every
preset at 30/60/100/140 km/h.

### What was wrong

| Symptom (before) | Cause |
|---|---|
| Left-right flick or a lane change at 100 km/h spins every car (slip 55-70 deg, 180 deg in the sedan and SUV at 140) | A key gave `max_steer / (1 + 0.025 v)`: 18 deg of lock at 100 km/h, where the grip limit needs about 1 deg plus the tyre's 7 deg of slip. The fronts were thrown far past their peak in a quarter of a second and the tail followed. |
| Skidpad 1.3-1.55 g, yet 100-0 km/h in 34-68 m | `grip` 1.25-1.45 (racing slicks) with brakes too weak to use it (the SUV had 0.68 g of brake force) and an even front/rear split. |
| Handbrake at 60 km/h and above: 150-180 deg, no recovery | Locked rear tyres have no cornering force at all; nothing limited the slide. |
| Cars stop dead, flip or turn round on a flat road mesh, at 5 cm steps and at kerbs | Not the wheels. Speculative contacts used a margin of `speed * dt` (0.46 m at 100 km/h) and the SAT axis of greatest separation as the contact normal. For a chassis 25 cm above the road, that axis towards the *next* triangle of the mesh (or a kerb it would clear) is horizontal, so the solver stopped the car at a seam it never touches. |
| Front wheels at -12 % slip while rolling free below 25 km/h | The wheel-spin integrator used the tangent of the slip curve, which is flat past the peak: after any wheelspin or lock a light wheel jumped from one side of the curve to the other every sub-step (a limit cycle) and lost its cornering grip. |
| 0-100 km/h slower than the torque allows | No clutch slip: the engine lugged at idle (30-50 % torque) off the line. |

### What changed

* **Swept speculative contacts** (`narrowphase::Sweep`, `collide_swept`):
  for polytope pairs (boxes, hulls, mesh and heightfield triangles) the
  narrowphase gets the pair's relative displacement over the step. If any
  SAT axis separates the shapes both now and after the step (by more than
  the resting margin plus the reach of their rotation), they cannot meet
  during it and no contact is made. Real impacts are untouched: the axis
  along the approach closes, so the contact stays (the tunnelling tests
  pass unchanged).
* **Speed-sensitive steering**: full input asks for the lock the turn needs
  at the limit of grip, `atan(L μ g / v²)`, plus the slip angle that puts the
  front tyres at their peak (`steer_slip * peak_slip_deg`), never more. The
  input winds on at `steer_rate` (slower at speed, 1.8x faster back to
  centre) and passes through the inverse of the slip curve, so half a stick
  is half the cornering force: analog input stays proportional.
* **Caster / countersteer assist** (`countersteer_assist`): the front wheels
  follow the direction the front axle travels in, so the input commands
  front slip rather than a wheel angle. Hands off, the wheels point down the
  road; in a slide they countersteer on their own, and a key still held into
  a slide stops feeding it once the rear is past 1.4x its peak slip.
* **Balance**: `rear_grip` (wider rear tyres), tyre **load sensitivity**
  (`load_sensitivity`: grip lost per doubling of load) and the front share
  of the roll stiffness (`anti_roll_front`) make every preset push its nose
  before the tail lets go. Tyre forces now act lower (`force_height`
  0.05-0.5 instead of 0.55), so weight transfer, dive and squat are closer
  to the real thing.
* **Brakes**: `brake_bias` (65 % front by default) and brake forces that can
  use the tyres. **ABS** (`abs`) limits each wheel's torque to what its tyre
  holds at its current load and backs off past the peak slip; **traction
  control** does the same for the drive torque and leaves room in the
  friction circle for the cornering the tyre is doing.
* **Stability control** (`stability_control`): yaw-rate target from the
  bicycle model, `r = v δ / (L (1 + v²/v_ch²))` bounded by `μ g / v`. Too
  much yaw, or a rear axle past its peak slip: the outer front wheel is
  braked and the throttle cut. Too little: the inner rear is braked lightly.
  The corrections are brake torques through the tyre model, so they are
  bounded by grip. With the handbrake pulled it allows a slide of ~22 deg.
* **Handbrake**: `handbrake_grip` is now the share of free-rolling cornering
  grip the locked wheels keep (0.45), so it starts a slide instead of a spin.
* **Differential** (`diff_lock`): a clutch torque from the faster to the
  slower driven wheel, bounded by a preload plus a share of the drive torque.
* **Wheel spin**: brake, drive and tyre torque are solved in one implicit
  step whose slope is never below the secant to the free-rolling speed
  (no limit cycle, wheels lock and release cleanly).
* **Wheel contact**: car wheels cast five rays across the tyre's
  circumference, each shortened by the chord there, so the wheel climbs a
  step with its leading edge like a disc. The contact normal is the road's
  slope under the axle, never a kerb face or the edge being climbed.
  Dampers blow off at 2.5x the static load and a bump stop takes the end of
  the travel.
* **Launch**: the clutch slips up to 60 % of the torque peak's rpm in first
  and reverse.
* **Bike**: the lean target is capped at what the tyres hold
  (`atan(0.85 μ)`); it shares the wheel, ABS and traction control changes.

### Before and after

Default car (`sports` handling on the CarConcept model), keyboard driver.
"slip" is the largest body slip angle; "rec" the time to run straight again
with the hands off.

| Manoeuvre | Before | After |
|---|---|---|
| Left-right flick, 100 km/h | SPUN, slip 63 deg, rec 2.9 s | slip 2.2 deg, rec 0.2 s |
| Double lane change, 100 km/h | SPUN, 6.5 m off the path | slip 2.4 deg, 0.73 m |
| Double lane change, 140 km/h | SPUN | slip 5.1 deg, 1.42 m |
| Steering taps, 140 km/h | slide, slip 36 deg | slip 0.4 deg |
| Key held 3 s, 140 km/h | slip 12.8 deg, rec 1.0 s | slip 8.1 deg, rec 0.5 s |
| Brake while turning, 100 km/h | slip 12.7 deg | slip 9.9 deg, rec 0.3 s |
| Lift-off mid-corner, 100 km/h | slip 4.3 deg | slip 6.6 deg, rec 0.2 s |
| Handbrake tap, 60 km/h | SPUN 114 deg | slip 13 deg, turned 43 deg, rec 0.4 s |
| Handbrake tap, 100 km/h | SPUN 180 deg, never recovers | slip 24 deg, turned 32 deg, rec 0.8 s |
| Flat mesh road / 5 cm step / 10 cm kerb, 100 km/h | flipped or turned round | 99-100 % of the speed kept, heading unchanged |

| Preset | 0-100 km/h | 100-0 km/h | Skidpad |
|---|---|---|---|
| concept (default car) | 6.3 -> 5.8 s | 36.2 -> 32.4 m | 1.55 -> 0.98 g |
| sports | 5.5 -> 5.2 s | 34.3 -> 32.4 m | 1.52 -> 0.95 g |
| sedan | 7.0 -> 7.2 s | 38.0 -> 37.9 m | 1.29 -> 0.85 g |
| suv | >8 -> 7.8 s | 56.3 -> 41.2 m | 1.15 -> 0.80 g |
| truck | >8 s | 67.8 -> 46.2 m | 0.99 -> 0.71 g |

The regression tests in `tests/handling.rs` cover, for every car preset: the
70 km/h double lane change, lane change and flick at 100 and 140 km/h, the
full-throttle launch, steering taps at 130 km/h, a held key at every speed,
trail braking, lift-off, the handbrake turn (turns the car at 40 km/h,
recoverable slide at 70 and 100), 2 and 5 cm steps and a 10 cm chamfered
kerb at 100 km/h (straight and at 25 deg), mesh seams at 180 km/h, the
performance figures, analog proportionality, and the bike at 100-180 km/h.

## Remaining issues and next steps

* Wheels are ray fans in the rolling plane: a car can still clip a kerb with
  the side of a tyre. Next: cylinder casts per wheel (`cast_shape`).
* The swept speculative test covers polytopes; spheres and capsules against
  meshes keep the plain margin (they are small and slow here).
* The bike's low-speed handling (feet down below 2.5 m/s) is simplified, and
  bike wheels are rays too.
* Joints and ragdolls do not exist yet; XPBD (Müller et al. 2020) is the
  recommended route for them.
* The benchmark's cars drive blindly with a sine steer for 15 s; some end up
  against facades or kerbs or leave the edge of the map (the "tilt after run"
  line), which is the scenario, not the solver.
* City pedestrians are animated by `forja_ai` and use rays, not physics; the
  stress scene's "pedestrians" are character controllers.
* If a future map has thousands of awake bodies, revisit graph colouring and
  parallel islands (items 11-12) with large task granularity, sized for
  hybrid P/E cores.
