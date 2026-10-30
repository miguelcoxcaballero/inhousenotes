//! Thread priorities and pools that keep the frame responsive.
//!
//! Work that runs while the game plays (the dusk/dawn world build, tile
//! loaders) must never take CPU time from the main thread:
//!
//! - it runs below normal priority ([`lower_priority`]), and every thread
//!   it spawns inherits that ([`inherit`]; Windows does not do it on its
//!   own, Linux does);
//! - its parallel stages use a pool of all cores but two
//!   ([`background_pool`]), so the main thread, the driver's thread and the
//!   OS keep a core each on a 4-core laptop;
//! - the frame's own parallel work (culling, pass recording, texture
//!   preparation on the main thread) runs on [`frame_pool`], a pool of its
//!   own: a frame job never queues behind a long build job in rayon's
//!   global pool.

use std::cell::Cell;
use std::sync::OnceLock;

thread_local! {
    static BACKGROUND: Cell<bool> = const { Cell::new(false) };
}

/// Lowers the calling thread's scheduling priority (Windows: below normal;
/// Linux: nice +10) and marks it as background work, so threads spawned
/// through [`inherit`] are lowered too. Best effort: OS failures are
/// ignored.
pub fn lower_priority() {
    BACKGROUND.with(|b| b.set(true));
    #[cfg(windows)]
    {
        // THREAD_PRIORITY_BELOW_NORMAL
        const BELOW_NORMAL: i32 = -1;
        unsafe extern "system" {
            fn GetCurrentThread() -> isize;
            fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        }
        // SAFETY: plain Win32 calls on the pseudo handle of this thread.
        unsafe {
            SetThreadPriority(GetCurrentThread(), BELOW_NORMAL);
        }
    }
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn gettid() -> i32;
            fn setpriority(which: i32, who: u32, prio: i32) -> i32;
        }
        // PRIO_PROCESS with a thread id sets that thread's nice value.
        // SAFETY: plain libc calls on this thread's id.
        unsafe {
            let tid = gettid();
            setpriority(0, tid as u32, 10);
        }
    }
}

/// Whether the calling thread was lowered with [`lower_priority`].
pub fn is_background() -> bool {
    BACKGROUND.with(|b| b.get())
}

/// Wraps the body of a thread about to be spawned from the calling one:
/// when the caller is background work, the new thread is lowered too.
///
/// `std::thread::scope(|s| s.spawn(threads::inherit(move || ...)))`.
pub fn inherit<R>(f: impl FnOnce() -> R) -> impl FnOnce() -> R {
    let background = is_background();
    move || {
        if background {
            lower_priority();
        }
        f()
    }
}

/// Threads for a pool that works while the game runs: all cores but two
/// (at least one).
pub fn background_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).max(1)
}

/// A rayon pool of [`background_threads`] low-priority threads.
pub fn background_pool(name: &'static str) -> Option<rayon::ThreadPool> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(background_threads())
        .thread_name(move |i| format!("{name} {i}"))
        .start_handler(|_| lower_priority())
        .build()
        .ok()
}

/// The frame's own rayon pool (one normal-priority thread per core),
/// separate from rayon's global pool, which loaders and builds share.
pub fn frame_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| rayon::ThreadPoolBuilder::new().thread_name(|i| format!("frame {i}")).build().ok()).as_ref()
}

/// Runs `f` on [`frame_pool`] (its rayon calls stay in that pool), or in
/// place when the pool could not be created.
pub fn on_frame_pool<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    match frame_pool() {
        Some(pool) => pool.install(f),
        None => f(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_pool_leaves_cores_and_runs() {
        assert!(background_threads() >= 1);
        let pool = background_pool("test").expect("pool");
        assert_eq!(pool.current_num_threads(), background_threads());
        let sum: u64 = pool.install(|| {
            use rayon::prelude::*;
            (0..1000u64).into_par_iter().sum()
        });
        assert_eq!(sum, 499500);
        // Lowering a thread's own priority is harmless.
        std::thread::spawn(lower_priority).join().unwrap();
    }

    #[test]
    fn spawned_threads_inherit_background() {
        let (parent, child, plain) = std::thread::spawn(|| {
            lower_priority();
            let child = std::thread::scope(|s| s.spawn(inherit(is_background)).join().unwrap());
            (is_background(), child, std::thread::spawn(is_background).join().unwrap())
        })
        .join()
        .unwrap();
        assert!(parent && child);
        assert!(!plain, "a thread not spawned through inherit stays normal");
        assert!(!std::thread::spawn(|| std::thread::scope(|s| s.spawn(inherit(is_background)).join().unwrap())).join().unwrap());
    }

    #[test]
    fn frame_pool_runs_parallel_work() {
        let sum: u64 = on_frame_pool(|| {
            use rayon::prelude::*;
            assert!(rayon::current_thread_index().is_some(), "runs on a pool worker");
            (0..1000u64).into_par_iter().sum()
        });
        assert_eq!(sum, 499500);
    }
}
