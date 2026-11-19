//! W6.3p: allocator accounting for memory investigations (`--features memstat`, never in a shipped build).
//!
//! Every allocation carries a 16-byte header with its size and the tag current when it was made: the index of the
//! module whose `event`/`frame`/`init` was running (`app.rs` sets it), or a finer tag set with [`with`]. The
//! `app.mem` test command reports live bytes by tag, the peak, the live bytes by tag at the peak and the largest
//! live blocks. Without the feature every function here is an empty inline.

#[cfg(feature = "memstat")]
mod imp {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::UnsafeCell;

    pub const TAGS: usize = 128;
    const H: usize = 16;
    const BIG: usize = 32 << 10;
    const BIGS: usize = 2048;

    pub struct Stats {
        pub tag: u8,
        pub live: [i64; TAGS],
        pub count: [i64; TAGS],
        pub total: i64,
        pub peak: i64,
        pub at_peak: [i64; TAGS],
        pub big: [(usize, u32, u8); BIGS],
        /// Linear-memory growths: (pages after, live total, size of the allocation, tag).
        pub grow: [(u32, i64, u32, u8); 256],
        pub pages: u32,
    }
    pub struct Cell(pub UnsafeCell<Stats>);
    unsafe impl Sync for Cell {}
    pub static S: Cell = Cell(UnsafeCell::new(Stats { tag: 126, live: [0; TAGS], count: [0; TAGS], total: 0, peak: 0, at_peak: [0; TAGS], big: [(0, 0, 0); BIGS], grow: [(0, 0, 0, 0); 256], pages: 0 }));
    pub fn s() -> &'static mut Stats { unsafe { &mut *S.0.get() } }

    struct Counting;
    fn hdr(l: Layout) -> (Layout, usize) {
        let off = H.max(l.align());
        (Layout::from_size_align(l.size() + off, l.align().max(8)).unwrap(), off)
    }
    fn add(size: usize, tag: u8, p: usize) { add2(size, tag, p, 0) }
    fn add2(size: usize, tag: u8, p: usize, old: usize) {
        let s = s();
        s.live[tag as usize] += size as i64;
        s.count[tag as usize] += 1;
        s.total += size as i64;
        if s.total > s.peak {
            s.peak = s.total;
            s.at_peak = s.live;
        }
        if size >= BIG {
            if let Some(e) = s.big.iter_mut().find(|e| e.0 == 0) {
                *e = (p, size as u32, tag);
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let pages = core::arch::wasm32::memory_size(0) as u32;
            if pages != s.pages {
                s.pages = pages;
                if let Some(e) = s.grow.iter_mut().find(|e| e.0 == 0) {
                    *e = (pages, s.total, size as u32 | if old > 0 { 1 << 31 } else { 0 }, tag);
                }
            }
        }
    }
    fn sub(size: usize, tag: u8, p: usize) {
        let s = s();
        s.live[tag as usize] -= size as i64;
        s.count[tag as usize] -= 1;
        s.total -= size as i64;
        if size >= BIG {
            if let Some(e) = s.big.iter_mut().find(|e| e.0 == p) {
                *e = (0, 0, 0);
            }
        }
    }
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let (hl, off) = hdr(l);
            let base = System.alloc(hl);
            if base.is_null() {
                return base;
            }
            let p = base.add(off);
            let tag = s().tag;
            *(p.sub(H) as *mut usize) = l.size();
            *p.sub(H - 8) = tag;
            add(l.size(), tag, p as usize);
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            let (hl, off) = hdr(l);
            sub(*(p.sub(H) as *const usize), *p.sub(H - 8), p as usize);
            System.dealloc(p.sub(off), hl)
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            let (hl, off) = hdr(l);
            let (old, tag) = (*(p.sub(H) as *const usize), *p.sub(H - 8));
            let base = System.realloc(p.sub(off), hl, new + off);
            if base.is_null() {
                return base;
            }
            let q = base.add(off);
            sub(old, tag, p as usize);
            // A grown block counts for whoever grew it.
            let tag = if new > old { s().tag } else { tag };
            *(q.sub(H) as *mut usize) = new;
            *q.sub(H - 8) = tag;
            add2(new, tag, q as usize, old.max(1));
            q
        }
    }
    #[global_allocator]
    static A: Counting = Counting;
}

/// Sets the current tag; returns the previous one.
#[inline]
pub fn set(tag: u8) -> u8 {
    #[cfg(feature = "memstat")]
    return std::mem::replace(&mut imp::s().tag, tag);
    #[cfg(not(feature = "memstat"))]
    {
        let _ = tag;
        0
    }
}

/// Runs `f` with `tag` current (finer tags: 64 and above).
#[inline]
pub fn with<R>(tag: u8, f: impl FnOnce() -> R) -> R {
    let old = set(tag);
    let r = f();
    set(old);
    r
}

/// `app.mem` (`reset` in args restarts the peak at the current total).
pub fn report(reset: bool) -> String {
    #[cfg(feature = "memstat")]
    {
        let s = imp::s();
        let names = |a: &[i64; imp::TAGS]| {
            let mut v: Vec<(usize, i64)> = a.iter().copied().enumerate().filter(|e| e.1 != 0).collect();
            v.sort_by_key(|e| -e.1);
            v.iter().map(|(t, b)| format!("[{},{}]", t, b)).collect::<Vec<_>>().join(",")
        };
        let mut big: Vec<(u32, u8)> = s.big.iter().filter(|e| e.0 != 0).map(|e| (e.1, e.2)).collect();
        big.sort_by_key(|e| std::cmp::Reverse(e.0));
        let big: Vec<String> = big.iter().take(40).map(|(n, t)| format!("[{n},{t}]")).collect();
        let out = format!(
            "{{\"total\":{},\"peak\":{},\"live\":[{}],\"count\":[{}],\"atPeak\":[{}],\"big\":[{}],\"grow\":[{}]}}",
            s.total,
            s.peak,
            names(&s.live),
            names(&s.count),
            names(&s.at_peak),
            big.join(","),
            s.grow.iter().filter(|g| g.0 != 0).map(|g| format!("[{},{},{},{}]", g.0, g.1, g.2, g.3)).collect::<Vec<_>>().join(",")
        );
        if reset {
            s.peak = s.total;
            s.at_peak = s.live;
        }
        return out;
    }
    #[cfg(not(feature = "memstat"))]
    {
        let _ = reset;
        "{}".into()
    }
}
