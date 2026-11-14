//! Small stable sorts for cold paths (W6.2s size work).
//!
//! Every `slice::sort*` call site instantiates its own copy of the std driftsort/ipnsort (5–8 KB of wasm each; 35
//! copies were 190 KB of `ihn.wasm`). These helpers keep one non-generic merge sort of indices and a few hundred
//! bytes per call site. They are stable, so they return exactly what `sort`/`sort_by`/`sort_by_key` return, and
//! what `sort_unstable*` return whenever equal keys are indistinguishable or absent (all call sites). The hot paths of the
//! document index (`doc.rs`) keep the std sorts.
use std::cmp::Ordering;

type Gt<'a> = &'a mut dyn FnMut(usize, usize) -> bool;

/// Stable insertion sort of indices by `gt(a, b)` = "element a sorts after element b".
fn insertion(a: &mut [usize], gt: Gt) {
    for i in 1..a.len() {
        let x = a[i];
        let mut j = i;
        while j > 0 && gt(a[j - 1], x) {
            a[j] = a[j - 1];
            j -= 1;
        }
        a[j] = x;
    }
}

/// Stable merge sort of the identity permutation `0..n`: insertion-sorted runs of 8, then bottom-up merges.
fn order(n: usize, gt: Gt) -> Vec<usize> {
    const RUN: usize = 8;
    let mut a: Vec<usize> = (0..n).collect();
    for run in a.chunks_mut(RUN) {
        insertion(run, gt);
    }
    let mut b = vec![0; n];
    let mut w = RUN;
    while w < n {
        for s in (0..n).step_by(2 * w) {
            let (m, e) = ((s + w).min(n), (s + 2 * w).min(n));
            if m == e || !gt(a[m - 1], a[m]) {
                b[s..e].copy_from_slice(&a[s..e]);
                continue;
            }
            let (mut i, mut j) = (s, m);
            for k in s..e {
                if j >= e || (i < m && !gt(a[i], a[j])) {
                    b[k] = a[i];
                    i += 1;
                } else {
                    b[k] = a[j];
                    j += 1;
                }
            }
        }
        std::mem::swap(&mut a, &mut b);
        w *= 2;
    }
    a
}

/// `v.sort_by(cmp)`. Up to 16 elements it does not allocate.
pub fn by<T>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> Ordering) {
    let n = v.len();
    if n < 2 {
        return;
    }
    let mut small = [0usize; 16];
    let mut big;
    let p: &mut [usize] = {
        let gt = &mut |a: usize, b: usize| cmp(&v[a], &v[b]) == Ordering::Greater;
        if n <= small.len() {
            for (i, x) in small[..n].iter_mut().enumerate() {
                *x = i;
            }
            insertion(&mut small[..n], gt);
            &mut small[..n]
        } else {
            big = order(n, gt);
            &mut big
        }
    };
    // Apply the permutation in place: new[i] = old[p[i]], one cycle at a time.
    for i in 0..n {
        let mut j = i;
        while p[j] != usize::MAX {
            let k = std::mem::replace(&mut p[j], usize::MAX);
            if k == i {
                break;
            }
            v.swap(j, k);
            j = k;
        }
    }
}

/// `v.sort_by_key(key)`.
pub fn by_key<T, K: Ord>(v: &mut [T], mut key: impl FnMut(&T) -> K) { by(v, |a, b| key(a).cmp(&key(b))) }

/// `v.sort()`.
pub fn ord<T: Ord>(v: &mut [T]) { by(v, T::cmp) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_std_stable_sort() {
        let mut seed = 0x2545_f491_u64;
        for n in [0usize, 1, 2, 3, 7, 8, 9, 15, 16, 17, 31, 64, 100, 257, 1000] {
            for range in [3u64, 1000] {
                let v: Vec<(u64, usize)> = (0..n)
                    .map(|i| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        (seed % range, i)
                    })
                    .collect();
                let mut a = v.clone();
                let mut b = v.clone();
                a.sort_by_key(|x| x.0);
                by_key(&mut b, |x| x.0);
                assert_eq!(a, b, "n={n} range={range}");
                let mut c = v.clone();
                c.reverse();
                let mut d = c.clone();
                c.sort();
                ord(&mut d);
                assert_eq!(c, d);
                let mut s: Vec<(u64, usize)> = a.clone();
                by(&mut s, |x, y| y.cmp(x));
                a.sort_by(|x, y| y.cmp(x));
                assert_eq!(a, s);
            }
        }
    }
}
