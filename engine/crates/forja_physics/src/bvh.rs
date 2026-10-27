//! Bounding volume hierarchies.
//!
//! * [`DynamicTree`]: incremental AABB tree with fat leaves and AVL-style
//!   rotations (same design as Box2D's `b2DynamicTree`). Used by the broadphase.
//! * [`StaticBvh`]: flat, top-down built tree for immutable triangle soups.

use forja_math::{Aabb, Ray, Vec3};

const NULL: u32 = u32::MAX;

#[derive(Clone, Debug)]
struct Node<T> {
    aabb: Aabb,
    parent: u32,
    child1: u32,
    child2: u32,
    /// Leaf = 0, free = -1.
    height: i32,
    data: Option<T>,
}

impl<T> Node<T> {
    fn is_leaf(&self) -> bool {
        self.child1 == NULL
    }
}

pub struct DynamicTree<T> {
    nodes: Vec<Node<T>>,
    root: u32,
    free: Vec<u32>,
}

impl<T: Copy> Default for DynamicTree<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy> DynamicTree<T> {
    pub fn new() -> Self {
        Self { nodes: Vec::new(), root: NULL, free: Vec::new() }
    }

    fn alloc(&mut self) -> u32 {
        let node = Node { aabb: Aabb::EMPTY, parent: NULL, child1: NULL, child2: NULL, height: 0, data: None };
        if let Some(i) = self.free.pop() {
            self.nodes[i as usize] = node;
            i
        } else {
            self.nodes.push(node);
            self.nodes.len() as u32 - 1
        }
    }

    fn release(&mut self, i: u32) {
        let n = &mut self.nodes[i as usize];
        n.height = -1;
        n.data = None;
        self.free.push(i);
    }

    pub fn insert(&mut self, aabb: Aabb, data: T) -> u32 {
        let leaf = self.alloc();
        let n = &mut self.nodes[leaf as usize];
        n.aabb = aabb;
        n.data = Some(data);
        self.insert_leaf(leaf);
        leaf
    }

    pub fn remove(&mut self, proxy: u32) {
        self.remove_leaf(proxy);
        self.release(proxy);
    }

    /// Re-inserts the proxy if `aabb` escaped its fat bounds. The new fat AABB
    /// is grown by `margin` and stretched along `displacement`. Returns true
    /// when the tree changed.
    pub fn move_proxy(&mut self, proxy: u32, aabb: Aabb, margin: f32, displacement: Vec3) -> bool {
        if self.nodes[proxy as usize].aabb.contains(&aabb) {
            return false;
        }
        self.remove_leaf(proxy);
        let mut fat = aabb.expanded(margin);
        let d = displacement * 2.0;
        fat.min += d.min(Vec3::ZERO);
        fat.max += d.max(Vec3::ZERO);
        self.nodes[proxy as usize].aabb = fat;
        self.insert_leaf(proxy);
        true
    }

    pub fn fat_aabb(&self, proxy: u32) -> Aabb {
        self.nodes[proxy as usize].aabb
    }

    pub fn data(&self, proxy: u32) -> T {
        self.nodes[proxy as usize].data.expect("leaf")
    }

    pub fn height(&self) -> i32 {
        if self.root == NULL { 0 } else { self.nodes[self.root as usize].height }
    }

    pub fn query(&self, aabb: &Aabb, mut f: impl FnMut(u32, T)) {
        if self.root == NULL {
            return;
        }
        // An AVL-balanced tree of 2^32 leaves is < 64 levels deep; the DFS
        // stack never exceeds height + 1 entries.
        let mut stack = [0u32; 256];
        stack[0] = self.root;
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let i = stack[sp];
            let n = &self.nodes[i as usize];
            if !n.aabb.intersects(aabb) {
                continue;
            }
            if n.is_leaf() {
                f(i, n.data.expect("leaf"));
            } else {
                stack[sp] = n.child1;
                stack[sp + 1] = n.child2;
                sp += 2;
            }
        }
    }

    /// Visits leaves whose AABB the ray enters before `max_t`. The callback
    /// returns the new `max_t` (return the hit distance to clip, or the input
    /// value to keep going).
    pub fn raycast(&self, ray: &Ray, mut max_t: f32, mut f: impl FnMut(T, f32) -> f32) {
        if self.root == NULL {
            return;
        }
        let mut stack = [0u32; 256];
        stack[0] = self.root;
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            if n.aabb.ray_hit(ray, max_t).is_none() {
                continue;
            }
            if n.is_leaf() {
                max_t = f(n.data.expect("leaf"), max_t);
            } else {
                stack[sp] = n.child1;
                stack[sp + 1] = n.child2;
                sp += 2;
            }
        }
    }

    fn insert_leaf(&mut self, leaf: u32) {
        if self.root == NULL {
            self.root = leaf;
            self.nodes[leaf as usize].parent = NULL;
            return;
        }
        let leaf_aabb = self.nodes[leaf as usize].aabb;
        let mut index = self.root;
        while !self.nodes[index as usize].is_leaf() {
            let n = &self.nodes[index as usize];
            let (c1, c2) = (n.child1, n.child2);
            let area = n.aabb.half_area();
            let combined_area = n.aabb.union(&leaf_aabb).half_area();
            let cost = 2.0 * combined_area;
            let inheritance = 2.0 * (combined_area - area);
            let child_cost = |c: u32| {
                let cn = &self.nodes[c as usize];
                let u = cn.aabb.union(&leaf_aabb).half_area();
                if cn.is_leaf() { u + inheritance } else { u - cn.aabb.half_area() + inheritance }
            };
            let cost1 = child_cost(c1);
            let cost2 = child_cost(c2);
            if cost < cost1 && cost < cost2 {
                break;
            }
            index = if cost1 < cost2 { c1 } else { c2 };
        }
        let sibling = index;
        let old_parent = self.nodes[sibling as usize].parent;
        let new_parent = self.alloc();
        {
            let sib_aabb = self.nodes[sibling as usize].aabb;
            let sib_h = self.nodes[sibling as usize].height;
            let p = &mut self.nodes[new_parent as usize];
            p.parent = old_parent;
            p.aabb = leaf_aabb.union(&sib_aabb);
            p.height = sib_h + 1;
            p.child1 = sibling;
            p.child2 = leaf;
        }
        if old_parent != NULL {
            let op = &mut self.nodes[old_parent as usize];
            if op.child1 == sibling {
                op.child1 = new_parent;
            } else {
                op.child2 = new_parent;
            }
        } else {
            self.root = new_parent;
        }
        self.nodes[sibling as usize].parent = new_parent;
        self.nodes[leaf as usize].parent = new_parent;
        self.refit_from(new_parent);
    }

    fn refit_from(&mut self, mut index: u32) {
        while index != NULL {
            index = self.balance(index);
            let (c1, c2) = (self.nodes[index as usize].child1, self.nodes[index as usize].child2);
            let h = 1 + self.nodes[c1 as usize].height.max(self.nodes[c2 as usize].height);
            let aabb = self.nodes[c1 as usize].aabb.union(&self.nodes[c2 as usize].aabb);
            let n = &mut self.nodes[index as usize];
            n.height = h;
            n.aabb = aabb;
            index = n.parent;
        }
    }

    fn remove_leaf(&mut self, leaf: u32) {
        if leaf == self.root {
            self.root = NULL;
            return;
        }
        let parent = self.nodes[leaf as usize].parent;
        let grand = self.nodes[parent as usize].parent;
        let sibling = {
            let p = &self.nodes[parent as usize];
            if p.child1 == leaf { p.child2 } else { p.child1 }
        };
        if grand != NULL {
            let g = &mut self.nodes[grand as usize];
            if g.child1 == parent {
                g.child1 = sibling;
            } else {
                g.child2 = sibling;
            }
            self.nodes[sibling as usize].parent = grand;
            self.release(parent);
            self.refit_from(grand);
        } else {
            self.root = sibling;
            self.nodes[sibling as usize].parent = NULL;
            self.release(parent);
        }
    }

    /// Single left or right rotation if `a` is imbalanced. Returns the new root
    /// of the subtree.
    fn balance(&mut self, ia: u32) -> u32 {
        let a = &self.nodes[ia as usize];
        if a.is_leaf() || a.height < 2 {
            return ia;
        }
        let ib = a.child1;
        let ic = a.child2;
        let balance = self.nodes[ic as usize].height - self.nodes[ib as usize].height;
        if balance > 1 {
            self.rotate_up(ia, ic, ib, false)
        } else if balance < -1 {
            self.rotate_up(ia, ib, ic, true)
        } else {
            ia
        }
    }

    /// Promote child `ix` of `ia` (whose other child is `iother`).
    /// `x_is_child1` says which slot of `ia` held `ix`.
    fn rotate_up(&mut self, ia: u32, ix: u32, iother: u32, x_is_child1: bool) -> u32 {
        let i_f = self.nodes[ix as usize].child1;
        let i_g = self.nodes[ix as usize].child2;
        let a_parent = self.nodes[ia as usize].parent;
        self.nodes[ix as usize].child1 = ia;
        self.nodes[ix as usize].parent = a_parent;
        self.nodes[ia as usize].parent = ix;
        if a_parent != NULL {
            let p = &mut self.nodes[a_parent as usize];
            if p.child1 == ia {
                p.child1 = ix;
            } else {
                p.child2 = ix;
            }
        } else {
            self.root = ix;
        }
        let (keep, give) = if self.nodes[i_f as usize].height > self.nodes[i_g as usize].height {
            (i_f, i_g)
        } else {
            (i_g, i_f)
        };
        self.nodes[ix as usize].child2 = keep;
        if x_is_child1 {
            self.nodes[ia as usize].child1 = give;
        } else {
            self.nodes[ia as usize].child2 = give;
        }
        self.nodes[give as usize].parent = ia;
        let a_aabb = self.nodes[iother as usize].aabb.union(&self.nodes[give as usize].aabb);
        let a_h = 1 + self.nodes[iother as usize].height.max(self.nodes[give as usize].height);
        self.nodes[ia as usize].aabb = a_aabb;
        self.nodes[ia as usize].height = a_h;
        self.nodes[ix as usize].aabb = a_aabb.union(&self.nodes[keep as usize].aabb);
        self.nodes[ix as usize].height = 1 + a_h.max(self.nodes[keep as usize].height);
        ix
    }

    /// Debug check of parent links, heights and bounds.
    pub fn validate(&self) {
        if self.root == NULL {
            return;
        }
        assert_eq!(self.nodes[self.root as usize].parent, NULL);
        let mut stack = vec![self.root];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            if n.is_leaf() {
                assert_eq!(n.height, 0);
                continue;
            }
            let (c1, c2) = (&self.nodes[n.child1 as usize], &self.nodes[n.child2 as usize]);
            assert_eq!(c1.parent, i);
            assert_eq!(c2.parent, i);
            assert_eq!(n.height, 1 + c1.height.max(c2.height));
            assert!(n.aabb.contains(&c1.aabb) && n.aabb.contains(&c2.aabb));
            stack.push(n.child1);
            stack.push(n.child2);
        }
    }
}

/// Immutable BVH over primitive AABBs. Leaves hold up to [`StaticBvh::LEAF`] primitives.
#[derive(Clone, Debug, Default)]
pub struct StaticBvh {
    nodes: Vec<FlatNode>,
    /// Primitive indices referenced by leaves.
    pub prims: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
struct FlatNode {
    aabb: Aabb,
    /// Leaf: first primitive; inner: index of the right child (left is next).
    start: u32,
    /// Leaf: primitive count; inner: 0.
    count: u32,
}

impl StaticBvh {
    pub const LEAF: usize = 4;

    /// Top-down build with binned SAH splits (Wald 2007, "On fast
    /// construction of SAH-based bounding volume hierarchies"): 12 bins on
    /// the widest centroid axis, a split only where it is cheaper than a
    /// leaf. Falls back to a median split for degenerate centroid bounds.
    pub fn build(aabbs: &[Aabb]) -> Self {
        let mut bvh = StaticBvh { nodes: Vec::with_capacity(aabbs.len() / 2 + 1), prims: (0..aabbs.len() as u32).collect() };
        if !aabbs.is_empty() {
            let centers: Vec<Vec3> = aabbs.iter().map(|a| a.center()).collect();
            bvh.build_node(aabbs, &centers, 0, aabbs.len(), 0);
        }
        bvh.nodes.shrink_to_fit();
        bvh
    }

    fn build_node(&mut self, aabbs: &[Aabb], centers: &[Vec3], start: usize, end: usize, depth: u32) -> usize {
        let mut bounds = Aabb::EMPTY;
        let mut cbounds = Aabb::EMPTY;
        for &p in &self.prims[start..end] {
            bounds = bounds.union(&aabbs[p as usize]);
            cbounds.grow_point(centers[p as usize]);
        }
        let idx = self.nodes.len();
        self.nodes.push(FlatNode { aabb: bounds, start: start as u32, count: (end - start) as u32 });
        let n = end - start;
        if n <= Self::LEAF {
            return idx;
        }
        let ext = cbounds.max - cbounds.min;
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        const BINS: usize = 12;
        let lo = cbounds.min[axis];
        let width = ext[axis];
        let mut mid = (start + end) / 2;
        let mut split_done = false;
        // Deep branches fall back to median splits so traversal stacks stay
        // bounded (depth <= 40 + log2(n)).
        if width > 1e-6 && depth < 40 {
            let scale = BINS as f32 / width;
            let bin_of = |c: f32| (((c - lo) * scale) as usize).min(BINS - 1);
            let mut counts = [0usize; BINS];
            let mut boxes = [Aabb::EMPTY; BINS];
            for &p in &self.prims[start..end] {
                let b = bin_of(centers[p as usize][axis]);
                counts[b] += 1;
                boxes[b] = boxes[b].union(&aabbs[p as usize]);
            }
            // Sweep: cost of splitting after bin i.
            let mut right_area = [0.0f32; BINS];
            let mut right_count = [0usize; BINS];
            let (mut acc, mut cnt) = (Aabb::EMPTY, 0usize);
            for i in (1..BINS).rev() {
                acc = acc.union(&boxes[i]);
                cnt += counts[i];
                right_area[i] = if cnt > 0 { acc.half_area() } else { 0.0 };
                right_count[i] = cnt;
            }
            let (mut acc, mut cnt) = (Aabb::EMPTY, 0usize);
            let mut best = (f32::MAX, 0usize);
            for i in 0..BINS - 1 {
                acc = acc.union(&boxes[i]);
                cnt += counts[i];
                if cnt == 0 || right_count[i + 1] == 0 {
                    continue;
                }
                let cost = acc.half_area() * cnt as f32 + right_area[i + 1] * right_count[i + 1] as f32;
                if cost < best.0 {
                    best = (cost, i);
                }
            }
            // Traversal cost ~ one primitive test; keep small leaves.
            let leaf_cost = bounds.half_area() * n as f32;
            if best.0 < f32::MAX && (best.0 + bounds.half_area() < leaf_cost || n > 2 * Self::LEAF) {
                let split = best.1;
                let (mut i, mut j) = (start, end);
                while i < j {
                    if bin_of(centers[self.prims[i] as usize][axis]) <= split {
                        i += 1;
                    } else {
                        j -= 1;
                        self.prims.swap(i, j);
                    }
                }
                if i > start && i < end {
                    mid = i;
                    split_done = true;
                }
            } else if best.0 < f32::MAX && n <= 2 * Self::LEAF {
                return idx; // a leaf is cheaper
            }
        }
        if !split_done {
            self.prims[start..end].select_nth_unstable_by(mid - start, |a, b| centers[*a as usize][axis].total_cmp(&centers[*b as usize][axis]));
        }
        self.build_node(aabbs, centers, start, mid, depth + 1);
        let right = self.build_node(aabbs, centers, mid, end, depth + 1);
        self.nodes[idx].start = right as u32;
        self.nodes[idx].count = 0;
        idx
    }

    /// Heap bytes of the nodes and primitive indices.
    pub fn memory_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<FlatNode>() + self.prims.capacity() * 4
    }

    pub fn bounds(&self) -> Aabb {
        self.nodes.first().map(|n| n.aabb).unwrap_or(Aabb::EMPTY)
    }

    pub fn query(&self, aabb: &Aabb, mut f: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = [0u32; 128];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let i = stack[sp] as usize;
            let n = self.nodes[i];
            if !n.aabb.intersects(aabb) {
                continue;
            }
            if n.count > 0 {
                for &p in &self.prims[n.start as usize..(n.start + n.count) as usize] {
                    f(p);
                }
            } else {
                stack[sp] = (i + 1) as u32;
                stack[sp + 1] = n.start;
                sp += 2;
            }
        }
    }

    /// Closest-hit traversal; `f(prim, max_t)` returns the new `max_t`.
    pub fn raycast(&self, ray: &Ray, mut max_t: f32, mut f: impl FnMut(u32, f32) -> f32) -> f32 {
        if self.nodes.is_empty() {
            return max_t;
        }
        let mut stack = [0u32; 128];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let i = stack[sp] as usize;
            let n = self.nodes[i];
            if n.aabb.ray_hit(ray, max_t).is_none() {
                continue;
            }
            if n.count > 0 {
                for &p in &self.prims[n.start as usize..(n.start + n.count) as usize] {
                    max_t = f(p, max_t);
                }
            } else {
                stack[sp] = (i + 1) as u32;
                stack[sp + 1] = n.start;
                sp += 2;
            }
        }
        max_t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forja_math::vec3;

    fn lcg(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (*seed >> 8) as f32 / (1u32 << 24) as f32
    }

    #[test]
    fn dynamic_tree_matches_brute_force() {
        let mut tree = DynamicTree::new();
        let mut seed = 7;
        let mut boxes = Vec::new();
        for i in 0..300u32 {
            let c = vec3(lcg(&mut seed), lcg(&mut seed), lcg(&mut seed)) * 100.0;
            let a = Aabb::from_center_half(c, Vec3::splat(1.0 + lcg(&mut seed)));
            let p = tree.insert(a, i);
            boxes.push((p, a));
        }
        tree.validate();
        // Move and remove some.
        for (i, (p, a)) in boxes.iter_mut().enumerate() {
            if i % 3 == 0 {
                let d = vec3(5.0, 0.0, -3.0);
                *a = Aabb::new(a.min + d, a.max + d);
                tree.move_proxy(*p, *a, 0.1, d);
            }
        }
        let removed: Vec<_> = boxes.iter().enumerate().filter(|(i, _)| i % 7 == 0).map(|(_, b)| b.0).collect();
        for p in &removed {
            tree.remove(*p);
        }
        tree.validate();
        assert!(tree.height() < 20, "tree is unbalanced: {}", tree.height());
        let q = Aabb::from_center_half(Vec3::splat(50.0), Vec3::splat(20.0));
        let mut found = Vec::new();
        tree.query(&q, |_, d| found.push(d));
        found.sort();
        let mut expect: Vec<u32> = boxes
            .iter()
            .enumerate()
            .filter(|(i, (_, a))| i % 7 != 0 && a.intersects(&q))
            .map(|(i, _)| i as u32)
            .collect();
        expect.sort();
        // Fat AABBs may report extra candidates, never miss one.
        for e in &expect {
            assert!(found.contains(e));
        }
    }

    #[test]
    fn static_bvh_query() {
        let aabbs: Vec<Aabb> =
            (0..100).map(|i| Aabb::from_center_half(vec3(i as f32 * 2.0, 0.0, 0.0), Vec3::splat(0.5))).collect();
        let bvh = StaticBvh::build(&aabbs);
        let mut hits = Vec::new();
        bvh.query(&Aabb::new(vec3(9.0, -1.0, -1.0), vec3(13.0, 1.0, 1.0)), |p| hits.push(p));
        // Leaves report candidates; exact tests are the caller's job.
        assert!(hits.contains(&5) && hits.contains(&6));
        assert!(hits.iter().all(|h| (3..=8).contains(h)), "{hits:?}");
    }
}
