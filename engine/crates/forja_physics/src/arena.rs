//! Generational arena used for bodies and colliders.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Handle {
    pub index: u32,
    pub generation: u32,
}

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self { slots: Vec::new(), free: Vec::new(), len: 0 }
    }
}

impl<T> Arena<T> {
    pub fn insert(&mut self, value: T) -> Handle {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            Handle { index, generation: slot.generation }
        } else {
            self.slots.push(Slot { generation: 0, value: Some(value) });
            Handle { index: self.slots.len() as u32 - 1, generation: 0 }
        }
    }

    pub fn remove(&mut self, h: Handle) -> Option<T> {
        let slot = self.slots.get_mut(h.index as usize)?;
        if slot.generation != h.generation {
            return None;
        }
        let v = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(h.index);
        self.len -= 1;
        Some(v)
    }

    #[inline]
    pub fn get(&self, h: Handle) -> Option<&T> {
        self.slots.get(h.index as usize).filter(|s| s.generation == h.generation).and_then(|s| s.value.as_ref())
    }

    #[inline]
    pub fn get_mut(&mut self, h: Handle) -> Option<&mut T> {
        self.slots.get_mut(h.index as usize).filter(|s| s.generation == h.generation).and_then(|s| s.value.as_mut())
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Upper bound for dense per-slot side tables.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.value.as_ref().map(|v| (Handle { index: i as u32, generation: s.generation }, v))
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, s)| {
            let generation = s.generation;
            s.value.as_mut().map(|v| (Handle { index: i as u32, generation }, v))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_handles_are_rejected() {
        let mut a = Arena::default();
        let h = a.insert(1);
        assert_eq!(a.remove(h), Some(1));
        let h2 = a.insert(2);
        assert_eq!(h2.index, h.index);
        assert!(a.get(h).is_none());
        assert_eq!(a.get(h2), Some(&2));
    }
}
