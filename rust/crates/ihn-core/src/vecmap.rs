//! Sorted-vector map and set (W6.2z size work).
//!
//! Drop-in replacements for the `BTreeMap`/`BTreeSet` API subset the sync engine uses, for collections that hold a
//! handful of entries (one per device, peer, connection, request in flight or live stroke). Every `BTreeMap<K, V>`
//! instantiation links its own copy of the B-tree (search, insert, split, remove, rebalance, iterators, drop: ~5 KB
//! of wasm each); a sorted `Vec<(K, V)>` needs a binary search and `Vec::insert`/`remove`. The observable behaviour
//! is the same: entries are unique by key and always iterated in ascending key order, `insert` on an existing key
//! keeps the key and replaces the value, `collect`/`extend` behave like inserting in order (the last value of a
//! repeated key wins), and equality, ordering and hashing compare the sorted entries like the std types do.
use std::borrow::Borrow;
use std::fmt;
use std::ops::{Bound, Index, RangeBounds};

/// A map kept as a vector of `(key, value)` sorted by key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VecMap<K, V> {
    v: Vec<(K, V)>,
}

impl<K, V> Default for VecMap<K, V> {
    fn default() -> Self { VecMap { v: Vec::new() } }
}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for VecMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.debug_map().entries(self.v.iter().map(|(k, v)| (k, v))).finish() }
}

impl<K, V> VecMap<K, V> {
    pub const fn new() -> Self { VecMap { v: Vec::new() } }
    pub fn len(&self) -> usize { self.v.len() }
    pub fn is_empty(&self) -> bool { self.v.is_empty() }
    pub fn clear(&mut self) { self.v.clear() }
    pub fn iter(&self) -> Iter<'_, K, V> { Iter(self.v.iter()) }
    pub fn iter_mut(&mut self) -> IterMut<'_, K, V> { IterMut(self.v.iter_mut()) }
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &K> + ExactSizeIterator + Clone { self.v.iter().map(|e| &e.0) }
    pub fn values(&self) -> impl DoubleEndedIterator<Item = &V> + ExactSizeIterator + Clone { self.v.iter().map(|e| &e.1) }
    pub fn values_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut V> + ExactSizeIterator { self.v.iter_mut().map(|e| &mut e.1) }
    pub fn into_keys(self) -> impl DoubleEndedIterator<Item = K> + ExactSizeIterator { self.v.into_iter().map(|e| e.0) }
    pub fn into_values(self) -> impl DoubleEndedIterator<Item = V> + ExactSizeIterator { self.v.into_iter().map(|e| e.1) }
    pub fn first_key_value(&self) -> Option<(&K, &V)> { self.v.first().map(|(k, v)| (k, v)) }
    pub fn last_key_value(&self) -> Option<(&K, &V)> { self.v.last().map(|(k, v)| (k, v)) }
    pub fn pop_first(&mut self) -> Option<(K, V)> { (!self.v.is_empty()).then(|| self.v.remove(0)) }
    pub fn pop_last(&mut self) -> Option<(K, V)> { self.v.pop() }
    /// Keeps the entries for which `f` returns true, visiting them in ascending key order.
    pub fn retain(&mut self, mut f: impl FnMut(&K, &mut V) -> bool) { self.v.retain_mut(|(k, v)| f(k, v)) }
}

impl<K: Ord, V> VecMap<K, V> {
    fn find<Q: Ord + ?Sized>(&self, key: &Q) -> Result<usize, usize>
    where
        K: Borrow<Q>,
    {
        self.v.binary_search_by(|e| e.0.borrow().cmp(key))
    }
    pub fn get<Q: Ord + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.find(key).ok().map(|i| &self.v[i].1)
    }
    pub fn get_key_value<Q: Ord + ?Sized>(&self, key: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
    {
        self.find(key).ok().map(|i| (&self.v[i].0, &self.v[i].1))
    }
    pub fn get_mut<Q: Ord + ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        self.find(key).ok().map(|i| &mut self.v[i].1)
    }
    pub fn contains_key<Q: Ord + ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.find(key).is_ok()
    }
    /// Like `BTreeMap::insert`: an existing key is kept and its old value returned.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.find(&key) {
            Ok(i) => Some(std::mem::replace(&mut self.v[i].1, value)),
            Err(i) => {
                self.v.insert(i, (key, value));
                None
            }
        }
    }
    pub fn remove<Q: Ord + ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        self.remove_entry(key).map(|e| e.1)
    }
    pub fn remove_entry<Q: Ord + ?Sized>(&mut self, key: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
    {
        self.find(key).ok().map(|i| self.v.remove(i))
    }
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
        let at = self.find(&key);
        Entry { map: self, key, at }
    }
    /// Like `BTreeMap::range`: the entries whose key lies in `range`, ascending (double-ended). Unlike the B-tree it
    /// returns nothing instead of panicking for a range whose start is after its end.
    pub fn range<R: RangeBounds<K>>(&self, range: R) -> Iter<'_, K, V> {
        let lo = match range.start_bound() {
            Bound::Included(k) => self.v.partition_point(|e| e.0 < *k),
            Bound::Excluded(k) => self.v.partition_point(|e| e.0 <= *k),
            Bound::Unbounded => 0,
        };
        let hi = match range.end_bound() {
            Bound::Included(k) => self.v.partition_point(|e| e.0 <= *k),
            Bound::Excluded(k) => self.v.partition_point(|e| e.0 < *k),
            Bound::Unbounded => self.v.len(),
        };
        Iter(self.v[lo..hi.max(lo)].iter())
    }
}

/// `VecMap::entry`.
pub struct Entry<'a, K, V> {
    map: &'a mut VecMap<K, V>,
    key: K,
    at: Result<usize, usize>,
}

impl<'a, K, V> Entry<'a, K, V> {
    pub fn key(&self) -> &K { &self.key }
    pub fn or_insert_with(self, f: impl FnOnce() -> V) -> &'a mut V {
        let i = match self.at {
            Ok(i) => i,
            Err(i) => {
                self.map.v.insert(i, (self.key, f()));
                i
            }
        };
        &mut self.map.v[i].1
    }
    pub fn or_insert(self, value: V) -> &'a mut V { self.or_insert_with(|| value) }
    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
    pub fn and_modify(self, f: impl FnOnce(&mut V)) -> Self {
        if let Ok(i) = self.at {
            f(&mut self.map.v[i].1);
        }
        self
    }
}

impl<K: Ord + Borrow<Q>, Q: Ord + ?Sized, V> Index<&Q> for VecMap<K, V> {
    type Output = V;
    fn index(&self, key: &Q) -> &V { self.get(key).expect("no entry found for key") }
}

impl<K: Ord, V> Extend<(K, V)> for VecMap<K, V> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl<'a, K: Ord + Copy, V: Copy> Extend<(&'a K, &'a V)> for VecMap<K, V> {
    fn extend<I: IntoIterator<Item = (&'a K, &'a V)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(*k, *v);
        }
    }
}

impl<K: Ord, V> FromIterator<(K, V)> for VecMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut m = VecMap::new();
        m.extend(iter);
        m
    }
}

impl<K: Ord, V, const N: usize> From<[(K, V); N]> for VecMap<K, V> {
    fn from(a: [(K, V); N]) -> Self { a.into_iter().collect() }
}

/// `VecMap::iter`.
pub struct Iter<'a, K, V>(std::slice::Iter<'a, (K, V)>);

impl<K, V> Clone for Iter<'_, K, V> {
    fn clone(&self) -> Self { Iter(self.0.clone()) }
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> { self.0.next().map(|(k, v)| (k, v)) }
    fn size_hint(&self) -> (usize, Option<usize>) { self.0.size_hint() }
}

impl<K, V> DoubleEndedIterator for Iter<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> { self.0.next_back().map(|(k, v)| (k, v)) }
}

impl<K, V> ExactSizeIterator for Iter<'_, K, V> {}

/// `VecMap::iter_mut`.
pub struct IterMut<'a, K, V>(std::slice::IterMut<'a, (K, V)>);

impl<'a, K, V> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);
    fn next(&mut self) -> Option<Self::Item> { self.0.next().map(|(k, v)| (&*k, v)) }
    fn size_hint(&self) -> (usize, Option<usize>) { self.0.size_hint() }
}

impl<K, V> DoubleEndedIterator for IterMut<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> { self.0.next_back().map(|(k, v)| (&*k, v)) }
}

impl<K, V> ExactSizeIterator for IterMut<'_, K, V> {}

impl<'a, K, V> IntoIterator for &'a VecMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;
    fn into_iter(self) -> Iter<'a, K, V> { self.iter() }
}

impl<'a, K, V> IntoIterator for &'a mut VecMap<K, V> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;
    fn into_iter(self) -> IterMut<'a, K, V> { self.iter_mut() }
}

impl<K, V> IntoIterator for VecMap<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;
    fn into_iter(self) -> Self::IntoIter { self.v.into_iter() }
}

/// A set kept as a sorted vector (see [`VecMap`]).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VecSet<T> {
    v: Vec<T>,
}

impl<T> Default for VecSet<T> {
    fn default() -> Self { VecSet { v: Vec::new() } }
}

impl<T: fmt::Debug> fmt::Debug for VecSet<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.debug_set().entries(self.v.iter()).finish() }
}

impl<T> VecSet<T> {
    pub const fn new() -> Self { VecSet { v: Vec::new() } }
    pub fn len(&self) -> usize { self.v.len() }
    pub fn is_empty(&self) -> bool { self.v.is_empty() }
    pub fn clear(&mut self) { self.v.clear() }
    pub fn iter(&self) -> std::slice::Iter<'_, T> { self.v.iter() }
    pub fn first(&self) -> Option<&T> { self.v.first() }
    pub fn last(&self) -> Option<&T> { self.v.last() }
    pub fn pop_first(&mut self) -> Option<T> { (!self.v.is_empty()).then(|| self.v.remove(0)) }
    pub fn pop_last(&mut self) -> Option<T> { self.v.pop() }
    pub fn retain(&mut self, f: impl FnMut(&T) -> bool) { self.v.retain(f) }
}

impl<T: Ord> VecSet<T> {
    fn find<Q: Ord + ?Sized>(&self, value: &Q) -> Result<usize, usize>
    where
        T: Borrow<Q>,
    {
        self.v.binary_search_by(|e| e.borrow().cmp(value))
    }
    pub fn contains<Q: Ord + ?Sized>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
    {
        self.find(value).is_ok()
    }
    /// Like `BTreeSet::insert`: false (and the set unchanged) when an equal value is present.
    pub fn insert(&mut self, value: T) -> bool {
        match self.find(&value) {
            Ok(_) => false,
            Err(i) => {
                self.v.insert(i, value);
                true
            }
        }
    }
    pub fn remove<Q: Ord + ?Sized>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
    {
        self.take(value).is_some()
    }
    pub fn take<Q: Ord + ?Sized>(&mut self, value: &Q) -> Option<T>
    where
        T: Borrow<Q>,
    {
        self.find(value).ok().map(|i| self.v.remove(i))
    }
}

impl<T: Ord> Extend<T> for VecSet<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for x in iter {
            self.insert(x);
        }
    }
}

impl<'a, T: Ord + Copy> Extend<&'a T> for VecSet<T> {
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
        for x in iter {
            self.insert(*x);
        }
    }
}

impl<T: Ord> FromIterator<T> for VecSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut s = VecSet::new();
        s.extend(iter);
        s
    }
}

impl<'a, T> IntoIterator for &'a VecSet<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter { self.v.iter() }
}

impl<T> IntoIterator for VecSet<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter { self.v.into_iter() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// A deterministic sequence of inserts, overwrites, removals, entries and retains gives the same contents,
    /// order, return values and `Debug` text as the std B-tree types.
    #[test]
    fn behaves_like_btree() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let (mut a, mut b) = (BTreeMap::new(), VecMap::new());
        let (mut sa, mut sb) = (BTreeSet::new(), VecSet::new());
        for step in 0..20_000u64 {
            let k = next() % 64;
            match next() % 7 {
                0 | 1 => assert_eq!(a.insert(k, step), b.insert(k, step)),
                2 => assert_eq!(a.remove(&k), b.remove(&k)),
                3 => {
                    *a.entry(k).or_insert(step) += 1;
                    *b.entry(k).or_insert(step) += 1;
                }
                4 => {
                    let m = next() % 5;
                    a.retain(|key, v| (key + *v) % 5 != m);
                    b.retain(|key, v| (key + *v) % 5 != m);
                }
                5 => {
                    assert_eq!(a.get(&k), b.get(&k));
                    let hi = k + next() % 8;
                    assert!(a.range(k..=hi).eq(b.range(k..=hi)) && a.range(k..hi).rev().eq(b.range(k..hi).rev()));
                    assert!(a.range(..k).eq(b.range(..k)) && a.range(k..).eq(b.range(k..)));
                }
                _ => assert_eq!((sa.insert(k), sa.remove(&(k / 2))), (sb.insert(k), sb.remove(&(k / 2)))),
            }
            assert!(a.iter().eq(b.iter()) && sa.iter().eq(sb.iter()));
        }
        assert_eq!(format!("{a:?} {sa:?}"), format!("{b:?} {sb:?}"));
        let pairs = [(3u64, 'a'), (1, 'b'), (3, 'c'), (2, 'd')];
        let (ca, cb): (BTreeMap<_, _>, VecMap<_, _>) = (pairs.into_iter().collect(), pairs.into_iter().collect());
        assert!(ca.iter().eq(cb.iter()));
    }
}
