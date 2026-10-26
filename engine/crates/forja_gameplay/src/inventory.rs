//! Slot inventory with stacking and a weight limit, plus a GTA-style weapon
//! wheel that groups owned weapons by category.

use serde::{Deserialize, Serialize};

use crate::items::{ItemCategory, ItemRegistry};
use crate::weapons::{WeaponCategory, WeaponRegistry};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStack {
    pub item: String,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inventory {
    pub slots: Vec<Option<ItemStack>>,
    pub max_weight: f32,
}

impl Inventory {
    pub fn new(slots: usize, max_weight: f32) -> Self {
        Self { slots: vec![None; slots], max_weight }
    }

    pub fn weight(&self, reg: &ItemRegistry) -> f32 {
        self.slots.iter().flatten().map(|s| reg.get(&s.item).map_or(0.0, |d| d.weight) * s.count as f32).sum()
    }

    pub fn count(&self, item: &str) -> u32 {
        self.slots.iter().flatten().filter(|s| s.item == item).map(|s| s.count).sum()
    }

    pub fn contains(&self, item: &str) -> bool {
        self.count(item) > 0
    }

    /// Adds up to `count` units; returns how many did not fit (slots or weight).
    pub fn add(&mut self, reg: &ItemRegistry, item: &str, count: u32) -> u32 {
        let Some(def) = reg.get(item) else { return count };
        let free_weight = (self.max_weight - self.weight(reg)).max(0.0);
        let by_weight = if def.weight > 0.0 { (free_weight / def.weight + 1e-4).floor() as u32 } else { u32::MAX };
        let mut left = count.min(by_weight);
        let rejected = count - left;
        let max = def.max_stack.max(1);
        for s in self.slots.iter_mut().flatten() {
            if left == 0 {
                break;
            }
            if s.item == item && s.count < max {
                let n = left.min(max - s.count);
                s.count += n;
                left -= n;
            }
        }
        for slot in self.slots.iter_mut() {
            if left == 0 {
                break;
            }
            if slot.is_none() {
                let n = left.min(max);
                *slot = Some(ItemStack { item: item.to_string(), count: n });
                left -= n;
            }
        }
        left + rejected
    }

    /// Removes up to `count`; returns how many were removed.
    pub fn remove(&mut self, item: &str, count: u32) -> u32 {
        let mut left = count;
        for slot in self.slots.iter_mut().rev() {
            if left == 0 {
                break;
            }
            if let Some(s) = slot
                && s.item == item
            {
                let n = left.min(s.count);
                s.count -= n;
                left -= n;
                if s.count == 0 {
                    *slot = None;
                }
            }
        }
        count - left
    }

    pub fn swap(&mut self, a: usize, b: usize) {
        if a < self.slots.len() && b < self.slots.len() {
            self.slots.swap(a, b);
        }
    }

    /// Moves `n` units from slot `from` to an empty slot. Returns the new slot.
    pub fn split(&mut self, from: usize, n: u32) -> Option<usize> {
        let empty = self.slots.iter().position(Option::is_none)?;
        let s = self.slots.get_mut(from)?.as_mut()?;
        if n == 0 || n >= s.count {
            return None;
        }
        s.count -= n;
        let item = s.item.clone();
        self.slots[empty] = Some(ItemStack { item, count: n });
        Some(empty)
    }

    /// Merges stacks of the same item and moves items to the front.
    pub fn compact(&mut self, reg: &ItemRegistry) {
        let items: Vec<ItemStack> = self.slots.iter_mut().filter_map(Option::take).collect();
        for s in items {
            self.add(reg, &s.item, s.count);
        }
    }

    pub fn items_of(&self, reg: &ItemRegistry, cat: ItemCategory) -> Vec<&ItemStack> {
        self.slots.iter().flatten().filter(|s| reg.get(&s.item).is_some_and(|d| d.category == cat)).collect()
    }
}

/// Weapons grouped by category with one selection per category.
#[derive(Clone, Debug, Default)]
pub struct WeaponWheel {
    /// `(category, weapon ids owned, selected index)`.
    pub slots: Vec<(WeaponCategory, Vec<String>, usize)>,
    pub active: Option<(usize, usize)>,
}

impl WeaponWheel {
    /// Rebuilds the wheel from the weapon items in an inventory.
    pub fn refresh(&mut self, inv: &Inventory, items: &ItemRegistry, weapons: &WeaponRegistry) {
        let active_id = self.active_weapon().map(str::to_string);
        let mut slots: Vec<(WeaponCategory, Vec<String>, usize)> = WeaponCategory::WHEEL.iter().map(|c| (*c, Vec::new(), 0)).collect();
        for s in inv.items_of(items, ItemCategory::Weapon) {
            let Some(wid) = items.get(&s.item).and_then(|d| d.weapon.clone()) else { continue };
            let Some(w) = weapons.get(&wid) else { continue };
            if let Some(slot) = slots.iter_mut().find(|(c, _, _)| *c == w.category)
                && !slot.1.contains(&wid)
            {
                slot.1.push(wid);
            }
        }
        self.slots = slots;
        self.active = active_id.and_then(|id| {
            self.slots.iter().enumerate().find_map(|(ci, (_, list, _))| list.iter().position(|w| *w == id).map(|wi| (ci, wi)))
        });
        if let Some((c, w)) = self.active {
            self.slots[c].2 = w;
        }
    }

    /// Selects a category; pressing it again cycles inside it.
    pub fn select_category(&mut self, cat: WeaponCategory) -> Option<&str> {
        let ci = self.slots.iter().position(|(c, _, _)| *c == cat)?;
        let n = self.slots[ci].1.len();
        if n == 0 {
            return None;
        }
        if self.active.is_some_and(|(c, _)| c == ci) {
            self.slots[ci].2 = (self.slots[ci].2 + 1) % n;
        }
        self.active = Some((ci, self.slots[ci].2));
        self.active_weapon()
    }

    pub fn holster(&mut self) {
        self.active = None;
    }

    pub fn active_weapon(&self) -> Option<&str> {
        let (c, w) = self.active?;
        self.slots.get(c)?.1.get(w).map(String::as_str)
    }
}
