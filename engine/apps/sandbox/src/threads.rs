//! Worker threads that run while the game plays (the dusk/dawn world
//! build, tile loaders) must never take CPU time from the main thread.
//!
//! They run below normal priority, and a background pool leaves cores to
//! the frame: on a 4-core laptop the main thread, the driver's own thread
//! and the OS keep a core each.

/// Lowers the calling thread's scheduling priority (Windows: below normal;
/// Linux: nice +10). Best effort: failures are ignored.
pub fn lower_priority() {
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
        // Lowering this thread's own priority is harmless.
        std::thread::spawn(lower_priority).join().unwrap();
    }
}
