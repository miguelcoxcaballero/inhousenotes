//! Seeded PRNG for the in-repo property tests (ARCHITECTURE §2.4: no external crates). Deterministic across
//! platforms; a failing test prints its seed. Public so that `ihn-sync` and `ihn-app` tests can use it; the linker
//! drops it from the wasm module.

/// xorshift64* generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self { Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1) }
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 { self.next() % n }
    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 { (self.next() >> 11) as f64 / (1u64 << 53) as f64 }
    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 { lo + (hi - lo) * self.unit() }
    pub fn chance(&mut self, p: f64) -> bool { self.unit() < p }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T { &items[self.below(items.len() as u64) as usize] }
    /// Fisher–Yates.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i as u64 + 1) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(42), Rng::new(42));
        let mut items: Vec<u32> = (0..50).collect();
        for _ in 0..1000 {
            assert_eq!(a.next(), b.next());
            assert!(a.below(7) < 7);
            b.below(7);
            let u = a.unit();
            b.unit();
            assert!((0.0..1.0).contains(&u));
        }
        a.shuffle(&mut items);
        items.sort_unstable();
        assert_eq!(items, (0..50).collect::<Vec<_>>());
        assert_ne!(Rng::new(1).next(), Rng::new(2).next());
    }
}
