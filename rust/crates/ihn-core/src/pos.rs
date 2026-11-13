//! Fractional position keys for page order (ARCHITECTURE §4.1). A key is a base-256 fraction in (0, 1) without a
//! trailing zero byte; keys compare bytewise. Page order = live pages sorted by `(pos, id)`.

/// Longest key a decoder accepts (a key grows by one byte per ~250 appends or per 8 inserts at one spot).
pub const MAX_POS_LEN: usize = 1024;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Pos(pub Vec<u8>);

impl Pos {
    /// Valid stored key: non-empty, no trailing zero, within the length limit.
    pub fn is_valid(&self) -> bool { self.0.last().is_some_and(|&b| b != 0) && self.0.len() <= MAX_POS_LEN }

    /// Key of the `rank`-th page (0-based) of an imported document: deterministic, ascending, with room around it.
    pub fn rank(rank: u32) -> Pos {
        let n = rank.saturating_add(1);
        Pos(vec![(n >> 24) as u8, (n >> 16) as u8, (n >> 8) as u8, n as u8, 0x80])
    }

    /// A key strictly between `lo` and `hi` (`None` = the open end). Requires `lo < hi` when both are given;
    /// for `lo >= hi` it returns a key just above `lo` (callers re-key the pages that must follow, see `Tx`).
    pub fn between(lo: Option<&Pos>, hi: Option<&Pos>) -> Pos {
        let lo = lo.map_or(&[][..], |p| &p.0);
        let Some(hi) = hi.map(|p| &p.0[..]).filter(|hi| lo < *hi) else {
            return Pos(after(lo, 0));
        };
        if lo.is_empty() {
            return Pos(before(hi));
        }
        // First digit where they differ (lo padded with zeros; hi > lo, so hi has a digit there).
        let i = (0..hi.len()).find(|&i| lo.get(i).copied().unwrap_or(0) != hi[i]).unwrap_or(hi.len() - 1);
        let (a, b) = (lo.get(i).copied().unwrap_or(0), hi[i]);
        if b - a > 1 {
            let mut out = hi[..i].to_vec();
            out.push(a + (b - a) / 2);
            return Pos(out);
        }
        // Adjacent digits: keep lo's digit and go just above the rest of lo.
        Pos(after(lo, i + 1))
    }
}

/// Smallest-growth key above `lo`, free to differ from digit `from` on.
fn after(lo: &[u8], from: usize) -> Vec<u8> {
    let mut out = lo[..from.min(lo.len())].to_vec();
    out.resize(from, 0);
    for &d in lo.get(from..).unwrap_or(&[]) {
        if d < 255 {
            out.push(d + 1);
            return out;
        }
        out.push(255);
    }
    // Appending past [255, …, 255] opens a new digit at 1 (255 more appends per byte); otherwise take the middle.
    out.push(if from == 0 && !lo.is_empty() { 1 } else { 128 });
    out
}

/// Smallest-growth key below `hi` (and above zero).
fn before(hi: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &d in hi {
        if d > 1 {
            out.push(d - 1);
            return out;
        }
        out.push(0);
        if d == 1 {
            break;
        }
    }
    out.push(128);
    out
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.pos.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Rng;

    #[test]
    fn between_is_strictly_inside_and_valid() {
        let p = |b: &[u8]| Pos(b.to_vec());
        let check = |lo: Option<&Pos>, hi: Option<&Pos>| {
            let m = Pos::between(lo, hi);
            assert!(m.is_valid(), "{lo:?} {hi:?} -> {m:?}");
            assert!(lo.is_none_or(|l| l < &m) && hi.is_none_or(|h| &m < h), "{lo:?} {hi:?} -> {m:?}");
            m
        };
        assert_eq!(check(None, None), p(&[128]));
        assert_eq!(check(Some(&p(&[128])), None), p(&[129]));
        assert_eq!(check(Some(&p(&[255])), None), p(&[255, 1]));
        assert_eq!(check(Some(&p(&[255, 1])), None), p(&[255, 2]));
        assert_eq!(check(None, Some(&p(&[128]))), p(&[127]));
        assert_eq!(check(None, Some(&p(&[1]))), p(&[0, 128]));
        assert_eq!(check(None, Some(&p(&[0, 128]))), p(&[0, 127]));
        assert_eq!(check(None, Some(&p(&[0, 0, 1]))), p(&[0, 0, 0, 128]));
        assert_eq!(check(Some(&p(&[5])), Some(&p(&[9]))), p(&[7]));
        assert_eq!(check(Some(&p(&[5])), Some(&p(&[6]))), p(&[5, 128]));
        assert_eq!(check(Some(&p(&[5])), Some(&p(&[5, 3]))), p(&[5, 1]));
        assert_eq!(check(Some(&p(&[5])), Some(&p(&[5, 1]))), p(&[5, 0, 128]));
        assert_eq!(check(Some(&p(&[5, 255])), Some(&p(&[6]))), p(&[5, 255, 128]));
        check(Some(&p(&[5, 255, 255])), Some(&p(&[6])));
        check(Some(&p(&[5, 0, 7])), Some(&p(&[5, 1])));
        // lo >= hi: just above lo.
        assert!(Pos::between(Some(&p(&[7])), Some(&p(&[7]))) > p(&[7]));
        assert!(Pos::rank(0) < Pos::rank(1) && Pos::rank(255) < Pos::rank(256) && Pos::rank(0).is_valid());
        check(None, Some(&Pos::rank(0)));
        check(Some(&Pos::rank(3)), Some(&Pos::rank(4)));
    }

    #[test]
    fn random_inserts_keep_order_and_keys_short() {
        let mut rng = Rng::new(11);
        let mut keys: Vec<Pos> = Vec::new();
        for _ in 0..3000 {
            let at = match rng.below(4) {
                0 => keys.len(),
                1 => 0,
                _ => rng.below(keys.len() as u64 + 1) as usize,
            };
            let key = Pos::between(at.checked_sub(1).map(|i| &keys[i]), keys.get(at));
            assert!(key.is_valid());
            keys.insert(at, key);
        }
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        // 2,000 appended pages (import, pull-to-add) stay tiny.
        let mut last: Option<Pos> = None;
        for _ in 0..2000 {
            let next = Pos::between(last.as_ref(), None);
            assert!(last.as_ref().is_none_or(|l| l < &next) && next.0.len() <= 9);
            last = Some(next);
        }
    }
}
