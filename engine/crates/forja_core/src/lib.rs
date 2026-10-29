//! Forja's app shell.
//!
//! An [`App`] owns a [`World`] (ECS entities plus typed resources) and a list of
//! systems grouped in [`Stage`]s. Every engine feature is a [`Plugin`] that adds
//! resources and systems; plugins never call each other directly, they share
//! resources and [`Events`]. That keeps each module swappable: replacing the
//! physics or the renderer means registering a different plugin.

use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub use forja_math as math;
pub use hecs;

/// Typed singleton storage.
#[derive(Default)]
pub struct Resources {
    map: HashMap<TypeId, Box<dyn Any + Send>>,
}

impl Resources {
    pub fn insert<T: Send + 'static>(&mut self, value: T) -> Option<T> {
        self.map.insert(TypeId::of::<T>(), Box::new(value)).map(|old| *old.downcast::<T>().expect("type id matches"))
    }

    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map.remove(&TypeId::of::<T>()).map(|b| *b.downcast::<T>().expect("type id matches"))
    }

    pub fn contains<T: 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>()).and_then(|b| b.downcast_ref())
    }

    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map.get_mut(&TypeId::of::<T>()).and_then(|b| b.downcast_mut())
    }

    pub fn get_or_insert_with<T: Send + 'static>(&mut self, f: impl FnOnce() -> T) -> &mut T {
        self.map.entry(TypeId::of::<T>()).or_insert_with(|| Box::new(f())).downcast_mut().expect("type id matches")
    }
}

/// Entities plus resources. Systems receive `&mut World`.
#[derive(Default)]
pub struct World {
    pub entities: hecs::World,
    pub resources: Resources,
}

impl World {
    pub fn res<T: 'static>(&self) -> &T {
        self.resources.get().unwrap_or_else(|| panic!("missing resource {}", type_name::<T>()))
    }

    pub fn res_mut<T: 'static>(&mut self) -> &mut T {
        self.resources.get_mut().unwrap_or_else(|| panic!("missing resource {}", type_name::<T>()))
    }

    /// Temporarily removes a resource so it can be used alongside `&mut World`.
    pub fn scope<T: Send + 'static, R>(&mut self, f: impl FnOnce(&mut World, &mut T) -> R) -> R {
        let mut value =
            self.resources.remove::<T>().unwrap_or_else(|| panic!("missing resource {}", type_name::<T>()));
        let out = f(self, &mut value);
        self.resources.insert(value);
        out
    }

    pub fn send<E: Send + 'static>(&mut self, event: E) {
        self.resources.get_or_insert_with(Events::<E>::default).send(event);
    }
}

/// Double-buffered event queue. Events live for two frames so every system
/// sees them once regardless of ordering.
pub struct Events<E> {
    current: Vec<E>,
    previous: Vec<E>,
}

impl<E> Default for Events<E> {
    fn default() -> Self {
        Self { current: Vec::new(), previous: Vec::new() }
    }
}

impl<E> Events<E> {
    pub fn send(&mut self, e: E) {
        self.current.push(e);
    }

    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.previous.iter().chain(self.current.iter())
    }

    pub fn drain(&mut self) -> impl Iterator<Item = E> + '_ {
        self.previous.drain(..).chain(self.current.drain(..))
    }

    pub fn update(&mut self) {
        self.previous.clear();
        std::mem::swap(&mut self.previous, &mut self.current);
    }
}

/// Order in which systems run each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// Once, before the first frame.
    Startup,
    /// Input, streaming, asset hot reload.
    PreUpdate,
    /// Zero or more times per frame at `Time::fixed_dt` (physics, vehicles).
    FixedUpdate,
    /// Gameplay at the variable frame rate.
    Update,
    /// Camera, transform sync, animation.
    PostUpdate,
    /// Building and submitting GPU work.
    Render,
}

const FRAME_STAGES: [Stage; 4] = [Stage::PreUpdate, Stage::Update, Stage::PostUpdate, Stage::Render];

pub type System = Box<dyn FnMut(&mut World) + Send>;

struct NamedSystem {
    name: &'static str,
    order: i32,
    run: System,
}

/// Frame and fixed-step timing.
#[derive(Clone, Debug)]
pub struct Time {
    pub delta: f32,
    pub elapsed: f64,
    pub fixed_dt: f32,
    pub frame: u64,
    /// Fraction of a fixed step left over, for render interpolation.
    pub alpha: f32,
    pub time_scale: f32,
    accumulator: f32,
    max_steps: u32,
    /// CPU budget of the fixed steps of one frame (`None`: unbounded, the
    /// deterministic default). See [`Time::set_fixed_budget`].
    fixed_budget: Option<Duration>,
}

impl Time {
    /// Bounds the CPU time the fixed steps of one frame may take. A slow
    /// frame asks for more fixed steps (6 at 10 FPS), each costing as much
    /// as before, which makes the next frame slower still: the "spiral of
    /// death" that pins a game at a low frame rate. With a budget, once it
    /// is spent the frame's remaining steps are dropped (the game runs
    /// slower than real time instead of at a lower frame rate). At least
    /// one step runs whenever one is due. Real-time loops set it; tests
    /// and replays keep the default (every step runs).
    pub fn set_fixed_budget(&mut self, budget: Option<Duration>) {
        self.fixed_budget = budget;
    }
}

impl Default for Time {
    fn default() -> Self {
        Self {
            delta: 0.0,
            elapsed: 0.0,
            fixed_dt: 1.0 / 60.0,
            frame: 0,
            alpha: 0.0,
            time_scale: 1.0,
            accumulator: 0.0,
            max_steps: 8,
            fixed_budget: None,
        }
    }
}

/// CPU time of every system in the last frame (ms), summed over the fixed
/// steps of that frame. Filled by [`App`] for the frame profiler.
#[derive(Clone, Debug, Default)]
pub struct SystemTimings {
    /// `(system name, ms)` in first-run order.
    pub last: Vec<(&'static str, f32)>,
    /// Fixed steps run in the last frame.
    pub fixed_steps: u32,
}

impl SystemTimings {
    fn begin_frame(&mut self) {
        for e in &mut self.last {
            e.1 = 0.0;
        }
        self.fixed_steps = 0;
    }

    fn add(&mut self, name: &'static str, ms: f32) {
        match self.last.iter_mut().find(|e| e.0 == name) {
            Some(e) => e.1 += ms,
            None => self.last.push((name, ms)),
        }
    }

    /// Total of all systems (ms).
    pub fn total(&self) -> f32 {
        self.last.iter().map(|e| e.1).sum()
    }
}

/// A unit of engine functionality.
pub trait Plugin {
    fn name(&self) -> &'static str {
        type_name::<Self>()
    }

    /// Plugins this one needs registered first, by [`Plugin::name`].
    fn dependencies(&self) -> &[&'static str] {
        &[]
    }

    fn build(&self, app: &mut App);
}

pub struct App {
    pub world: World,
    systems: HashMap<Stage, Vec<NamedSystem>>,
    plugins: Vec<&'static str>,
    started: bool,
    last_frame: Option<Instant>,
    frame_event_updaters: Vec<fn(&mut World)>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let mut world = World::default();
        world.resources.insert(Time::default());
        world.resources.insert(SystemTimings::default());
        Self {
            world,
            systems: HashMap::new(),
            plugins: Vec::new(),
            started: false,
            last_frame: None,
            frame_event_updaters: Vec::new(),
        }
    }

    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        let name = plugin.name();
        assert!(!self.has_plugin(name), "plugin {name} registered twice");
        for dep in plugin.dependencies() {
            assert!(self.has_plugin(dep), "plugin {name} needs {dep}; register it first");
        }
        log::debug!("plugin {name}");
        plugin.build(self);
        self.plugins.push(name);
        self
    }

    pub fn has_plugin(&self, name: &str) -> bool {
        self.plugins.iter().any(|p| *p == name)
    }

    pub fn plugins(&self) -> &[&'static str] {
        &self.plugins
    }

    pub fn insert_resource<T: Send + 'static>(&mut self, value: T) -> &mut Self {
        self.world.resources.insert(value);
        self
    }

    pub fn add_system(&mut self, stage: Stage, name: &'static str, f: impl FnMut(&mut World) + Send + 'static) -> &mut Self {
        self.add_system_at(stage, name, 0, f)
    }

    /// Adds a system with an explicit order inside its stage (lower runs
    /// first; equal orders keep registration order). Engine conventions:
    /// gameplay 0, `physics_step` 100, sync after physics 200.
    pub fn add_system_at(&mut self, stage: Stage, name: &'static str, order: i32, f: impl FnMut(&mut World) + Send + 'static) -> &mut Self {
        let list = self.systems.entry(stage).or_default();
        list.push(NamedSystem { name, order, run: Box::new(f) });
        list.sort_by_key(|s| s.order);
        self
    }

    /// Registers `Events<E>` so it is rotated once per frame.
    pub fn add_event<E: Send + 'static>(&mut self) -> &mut Self {
        self.world.resources.get_or_insert_with(Events::<E>::default);
        self.frame_event_updaters.push(|w| {
            if let Some(ev) = w.resources.get_mut::<Events<E>>() {
                ev.update();
            }
        });
        self
    }

    pub fn system_names(&self, stage: Stage) -> Vec<&'static str> {
        self.systems.get(&stage).map(|v| v.iter().map(|s| s.name).collect()).unwrap_or_default()
    }

    fn run_stage(&mut self, stage: Stage) {
        if let Some(list) = self.systems.get_mut(&stage) {
            for s in list.iter_mut() {
                let t = Instant::now();
                (s.run)(&mut self.world);
                let ms = t.elapsed().as_secs_f32() * 1000.0;
                if let Some(timings) = self.world.resources.get_mut::<SystemTimings>() {
                    timings.add(s.name, ms);
                }
            }
        }
    }

    /// Runs one frame using wall-clock time.
    pub fn update(&mut self) {
        let now = Instant::now();
        let dt = self.last_frame.map(|t| now - t).unwrap_or(Duration::from_secs_f32(1.0 / 60.0));
        self.last_frame = Some(now);
        self.update_with_dt(dt.as_secs_f32());
    }

    /// Runs one frame with an explicit delta (deterministic tests, replays).
    pub fn update_with_dt(&mut self, dt: f32) {
        if !self.started {
            self.started = true;
            self.run_stage(Stage::Startup);
        }
        for u in &self.frame_event_updaters {
            u(&mut self.world);
        }
        self.world.res_mut::<SystemTimings>().begin_frame();
        let steps = {
            let t = self.world.res_mut::<Time>();
            t.delta = dt.min(0.25) * t.time_scale;
            t.elapsed += t.delta as f64;
            t.frame += 1;
            t.accumulator += t.delta;
            let mut n = 0;
            while t.accumulator >= t.fixed_dt && n < t.max_steps {
                t.accumulator -= t.fixed_dt;
                n += 1;
            }
            if n == t.max_steps {
                // Spiral-of-death guard: drop the backlog.
                t.accumulator = t.accumulator.min(t.fixed_dt);
            }
            t.alpha = t.accumulator / t.fixed_dt;
            n
        };
        self.run_stage(FRAME_STAGES[0]);
        let budget = self.world.res::<Time>().fixed_budget;
        let t0 = Instant::now();
        let mut done = 0;
        while done < steps {
            if done > 0 && budget.is_some_and(|b| t0.elapsed() >= b) {
                // Out of budget: the rest of the backlog is dropped.
                break;
            }
            self.run_stage(Stage::FixedUpdate);
            done += 1;
        }
        self.world.res_mut::<SystemTimings>().fixed_steps = done;
        for stage in &FRAME_STAGES[1..] {
            self.run_stage(*stage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter(u32);
    struct CounterPlugin;
    impl Plugin for CounterPlugin {
        fn name(&self) -> &'static str {
            "counter"
        }
        fn build(&self, app: &mut App) {
            app.insert_resource(Counter(0)).add_system(Stage::FixedUpdate, "count", |w| w.res_mut::<Counter>().0 += 1);
        }
    }

    struct NeedsCounter;
    impl Plugin for NeedsCounter {
        fn dependencies(&self) -> &[&'static str] {
            &["counter"]
        }
        fn build(&self, _: &mut App) {}
    }

    #[test]
    fn fixed_steps_follow_time() {
        let mut app = App::new();
        app.add_plugin(CounterPlugin);
        for _ in 0..60 {
            app.update_with_dt(1.0 / 30.0);
        }
        assert_eq!(app.world.res::<Counter>().0, 120);
    }

    #[test]
    fn fixed_budget_drops_the_backlog() {
        let mut app = App::new();
        app.add_plugin(CounterPlugin);
        // Each step costs ~3 ms; a 10 FPS frame asks for 6 steps.
        app.add_system(Stage::FixedUpdate, "slow", |_| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_millis(3) {
                std::hint::spin_loop();
            }
        });
        app.update_with_dt(0.105);
        assert_eq!(app.world.res::<Counter>().0, 6);
        app.world.res_mut::<Time>().set_fixed_budget(Some(Duration::from_millis(5)));
        app.update_with_dt(0.105);
        let ran = app.world.res::<Counter>().0 - 6;
        assert!((1..=2).contains(&ran), "{ran} steps ran");
        assert_eq!(app.world.res::<SystemTimings>().fixed_steps, ran);
        // The dropped steps do not come back next frame.
        app.world.res_mut::<Time>().set_fixed_budget(None);
        let before = app.world.res::<Counter>().0;
        app.update_with_dt(1.0 / 60.0);
        assert!(app.world.res::<Counter>().0 - before <= 1);
    }

    #[test]
    #[should_panic(expected = "needs counter")]
    fn dependencies_are_checked() {
        App::new().add_plugin(NeedsCounter);
    }

    #[test]
    fn systems_run_in_order() {
        let mut app = App::new();
        app.insert_resource(Vec::<&'static str>::new());
        app.add_system_at(Stage::Update, "late", 100, |w| w.res_mut::<Vec<&'static str>>().push("late"));
        app.add_system(Stage::Update, "early", |w| w.res_mut::<Vec<&'static str>>().push("early"));
        app.update_with_dt(0.016);
        assert_eq!(app.world.res::<Vec<&'static str>>(), &vec!["early", "late"]);
        assert_eq!(app.system_names(Stage::Update), vec!["early", "late"]);
    }

    #[test]
    fn events_live_two_frames() {
        let mut app = App::new();
        app.add_event::<u32>();
        app.world.send(7u32);
        app.update_with_dt(0.016);
        assert_eq!(app.world.res::<Events<u32>>().iter().count(), 1);
        app.update_with_dt(0.016);
        assert_eq!(app.world.res::<Events<u32>>().iter().count(), 0);
    }
}
