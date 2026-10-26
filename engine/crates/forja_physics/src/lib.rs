//! # forja_physics
//!
//! Modular rigid body physics for open-world games.
//!
//! Pipeline per step: broadphase (dynamic AABB tree) → narrowphase
//! (SAT/clipping and closest points, speculative contacts) → soft-step
//! solver with substeps → sleeping islands. Gameplay controllers
//! ([`vehicle`], [`character`]) sit on top and only use the public API, so
//! they work with any world built from [`ColliderDesc`]s.
//!
//! ```
//! use forja_physics::*;
//! use forja_math::{Vec3, vec3};
//!
//! let mut world = PhysicsWorld::default();
//! world.add_collider(ColliderDesc::new(Shape::cuboid(vec3(50.0, 0.5, 50.0))).translation(vec3(0.0, -0.5, 0.0)), None);
//! let (ball, _) = world.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 5.0, 0.0)), ColliderDesc::new(Shape::sphere(0.5)));
//! for _ in 0..120 { world.step(1.0 / 60.0); }
//! assert!((world.body(ball).unwrap().position().y - 0.5).abs() < 0.05);
//! ```

pub mod arena;
pub mod body;
pub mod broadphase;
pub mod bvh;
pub mod character;
pub mod narrowphase;
pub mod shape;
mod solver;
pub mod vehicle;
pub mod world;

pub use body::{
    BodyHandle, BodyType, Collider, ColliderDesc, ColliderHandle, CollisionGroups, RigidBody, RigidBodyDesc, layers,
};
pub use broadphase::{Broadphase, TreeBroadphase};
pub use character::{CharacterConfig, CharacterController, CharacterState};
pub use narrowphase::{ContactPoint, Manifold};
pub use shape::{ConvexHull, HeightField, MassProperties, Shape, TriMesh};
pub use vehicle::{Vehicle, VehicleConfig, VehicleControls, WheelConfig};
pub use world::{CollisionEvent, PhysicsConfig, PhysicsWorld, QueryFilter, RayHit, StepStats};

use forja_core::{App, Plugin, Stage, World};

/// Vehicles registered with the plugin are updated before every step.
#[derive(Default)]
pub struct Vehicles(pub Vec<Vehicle>);

/// Registers a [`PhysicsWorld`] resource stepped in [`Stage::FixedUpdate`].
pub struct PhysicsPlugin {
    pub config: PhysicsConfig,
}

impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self { config: PhysicsConfig::default() }
    }
}

impl Plugin for PhysicsPlugin {
    fn name(&self) -> &'static str {
        "forja_physics"
    }

    fn build(&self, app: &mut App) {
        app.insert_resource(PhysicsWorld::new(self.config.clone()))
            .insert_resource(Vehicles::default())
            .add_system_at(Stage::FixedUpdate, "physics_step", 100, physics_step);
    }
}

fn physics_step(w: &mut World) {
    let dt = w.res::<forja_core::Time>().fixed_dt;
    w.scope::<Vehicles, _>(|w, vehicles| {
        let physics = w.res_mut::<PhysicsWorld>();
        for v in &mut vehicles.0 {
            v.update(physics, dt);
        }
        physics.step(dt);
    });
}
