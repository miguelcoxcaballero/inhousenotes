//! Broadphase interface. Swap the implementation with
//! [`crate::PhysicsWorld::with_broadphase`].

use forja_math::{Aabb, Ray, Vec3};

use crate::body::ColliderHandle;
use crate::bvh::DynamicTree;

pub trait Broadphase: Send {
    fn insert(&mut self, collider: ColliderHandle, aabb: Aabb) -> u32;
    fn remove(&mut self, proxy: u32);
    /// Called when a collider moved. `displacement` predicts the next move.
    fn update(&mut self, proxy: u32, aabb: Aabb, displacement: Vec3);
    fn fat_aabb(&self, proxy: u32) -> Aabb;
    fn query(&self, aabb: &Aabb, f: &mut dyn FnMut(ColliderHandle));
    /// `f` returns the new maximum distance.
    fn raycast(&self, ray: &Ray, max_t: f32, f: &mut dyn FnMut(ColliderHandle, f32) -> f32);
}

/// Default broadphase: one dynamic AABB tree with fat leaves.
pub struct TreeBroadphase {
    tree: DynamicTree<ColliderHandle>,
    pub margin: f32,
}

impl Default for TreeBroadphase {
    fn default() -> Self {
        Self { tree: DynamicTree::new(), margin: 0.1 }
    }
}

impl Broadphase for TreeBroadphase {
    fn insert(&mut self, collider: ColliderHandle, aabb: Aabb) -> u32 {
        self.tree.insert(aabb.expanded(self.margin), collider)
    }

    fn remove(&mut self, proxy: u32) {
        self.tree.remove(proxy);
    }

    fn update(&mut self, proxy: u32, aabb: Aabb, displacement: Vec3) {
        self.tree.move_proxy(proxy, aabb, self.margin, displacement);
    }

    fn fat_aabb(&self, proxy: u32) -> Aabb {
        self.tree.fat_aabb(proxy)
    }

    fn query(&self, aabb: &Aabb, f: &mut dyn FnMut(ColliderHandle)) {
        self.tree.query(aabb, |_, c| f(c));
    }

    fn raycast(&self, ray: &Ray, max_t: f32, f: &mut dyn FnMut(ColliderHandle, f32) -> f32) {
        self.tree.raycast(ray, max_t, f);
    }
}
