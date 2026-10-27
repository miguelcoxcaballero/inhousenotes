//! `--frame-trace N` (or `FORJA_FRAME_TRACE=N`): a per-frame CPU timeline
//! of the main thread, logged as one summary every N frames.
//!
//! Each frame is cut into consecutive segments by [`FrameTrace::mark`]
//! (event-loop wait, simulation, extraction, swapchain acquire, render,
//! UI, present, GPU waits); the segments add up to the wall time between
//! two frame starts, so whatever is not covered shows up as `other`. Nested
//! details (each game system, each renderer scope and encoded pass) are
//! averaged next to their segment. The summary also has the main thread's
//! and the whole process's CPU time over the window (a main thread that is
//! busy for much less than the wall time is blocked somewhere: GPU, driver,
//! swapchain or a lock), the log records written and the time spent writing
//! them, blocking upload flushes, the fixed-step count, background work
//! (world build, tile streaming) and the slowest frame's own timeline.
//!
//! Overhead: a handful of `Instant::now()` per frame and one log line per
//! window (two with the slowest frame).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Log records written so far and the nanoseconds spent writing them.
static LOG_RECORDS: AtomicU64 = AtomicU64::new(0);
static LOG_NANOS: AtomicU64 = AtomicU64::new(0);

/// `env_logger` with a counter around it (records and time spent in the
/// console or file write, read by the trace).
struct CountingLogger(env_logger::Logger);

impl log::Log for CountingLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.0.enabled(m)
    }

    fn log(&self, r: &log::Record) {
        if !self.0.matches(r) {
            return;
        }
        let t = Instant::now();
        self.0.log(r);
        LOG_NANOS.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        LOG_RECORDS.fetch_add(1, Ordering::Relaxed);
    }

    fn flush(&self) {
        self.0.flush();
    }
}

/// Installs the logger (`RUST_LOG`, else `default_filter`).
pub fn init_logger(default_filter: &str) {
    let logger = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter)).build();
    let max = logger.filter();
    if log::set_boxed_logger(Box::new(CountingLogger(logger))).is_ok() {
        log::set_max_level(max);
    }
}

/// `(log records, ms spent logging)` so far.
pub fn log_counters() -> (u64, f64) {
    (LOG_RECORDS.load(Ordering::Relaxed), LOG_NANOS.load(Ordering::Relaxed) as f64 * 1e-6)
}

/// CPU time (ms) of the calling thread and of the whole process.
pub fn cpu_times() -> (f64, f64) {
    os::cpu_times()
}

#[cfg(windows)]
mod os {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn GetCurrentProcess() -> isize;
        fn GetThreadTimes(h: isize, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn GetProcessTimes(h: isize, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
    }

    pub fn cpu_times() -> (f64, f64) {
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        let (mut pk, mut pu) = (0u64, 0u64);
        // SAFETY: pseudo handles of this thread and process; the four
        // out-pointers are valid FILETIME-sized locations.
        unsafe {
            GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u);
            GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut pk, &mut pu);
        }
        // 100 ns units.
        ((k + u) as f64 * 1e-4, (pk + pu) as f64 * 1e-4)
    }
}

#[cfg(all(unix, not(windows)))]
mod os {
    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }

    unsafe extern "C" {
        fn clock_gettime(clock: i32, ts: *mut Timespec) -> i32;
    }

    fn clock_ms(id: i32) -> f64 {
        let mut ts = Timespec { sec: 0, nsec: 0 };
        // SAFETY: `ts` is a valid timespec (64-bit targets).
        unsafe {
            clock_gettime(id, &mut ts);
        }
        ts.sec as f64 * 1000.0 + ts.nsec as f64 * 1e-6
    }

    pub fn cpu_times() -> (f64, f64) {
        // CLOCK_THREAD_CPUTIME_ID, CLOCK_PROCESS_CPUTIME_ID.
        (clock_ms(3), clock_ms(2))
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    pub fn cpu_times() -> (f64, f64) {
        (0.0, 0.0)
    }
}

/// How long `sleep(1 ms)` and `recv_timeout(1 ms)` really take here
/// (Windows' default 15.6 ms timer turns every short wait into ~16 ms).
pub fn timer_report() -> String {
    let median = |f: &dyn Fn()| {
        let mut v: Vec<f64> = (0..9)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        v.sort_by(f64::total_cmp);
        (v[4], v[8])
    };
    let (sleep, sleep_max) = median(&|| std::thread::sleep(Duration::from_millis(1)));
    let (_tx, rx) = std::sync::mpsc::channel::<()>();
    let (recv, recv_max) = median(&|| {
        let _ = rx.recv_timeout(Duration::from_millis(1));
    });
    let (yield_, _) = median(&std::thread::yield_now);
    format!("sleep(1ms) {sleep:.2} ms (max {sleep_max:.2}), recv_timeout(1ms) {recv:.2} ms (max {recv_max:.2}), yield {yield_:.3} ms")
}

/// Segment and detail sums of the current window.
#[derive(Default)]
struct Acc {
    /// `(name, sum ms, max ms)` in first-seen order.
    entries: Vec<(&'static str, f64, f64)>,
}

impl Acc {
    fn add(&mut self, name: &'static str, ms: f64) {
        match self.entries.iter_mut().find(|e| e.0 == name) {
            Some(e) => {
                e.1 += ms;
                e.2 = e.2.max(ms);
            }
            None => self.entries.push((name, ms, ms)),
        }
    }
}

pub struct FrameTrace {
    pub every: usize,
    label: &'static str,
    frame: u64,
    /// Start of the current frame and of the current segment.
    start: Instant,
    seg: Instant,
    started: bool,
    segments: Vec<(&'static str, f64)>,
    details: Vec<(&'static str, &'static str, f64)>,
    acc: Acc,
    detail_acc: Vec<(&'static str, Acc)>,
    frames: usize,
    wall: f64,
    worst: (f64, Vec<(&'static str, f64)>),
    window_start: Instant,
    cpu0: (f64, f64),
    log0: (u64, f64),
    waits0: (u32, f64),
    fixed_steps: u64,
    events: u64,
    notes: Vec<String>,
}

impl FrameTrace {
    /// `--frame-trace N`, else `FORJA_FRAME_TRACE=N` (0: off).
    pub fn from_args(n: usize, label: &'static str) -> Option<Self> {
        let n = if n > 0 { n } else { std::env::var("FORJA_FRAME_TRACE").ok().and_then(|v| v.parse().ok()).unwrap_or(0) };
        (n > 0).then(|| {
            let timers = format!("frame trace: every {n} frames ({label}); timers: {}", timer_report());
            log::info!("{timers}");
            if let Some(path) = std::env::var_os("FORJA_TRACE_FILE") {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "{timers}");
                }
            }
            let now = Instant::now();
            Self {
                every: n,
                label,
                frame: 0,
                start: now,
                seg: now,
                started: false,
                segments: Vec::new(),
                details: Vec::new(),
                acc: Acc::default(),
                detail_acc: Vec::new(),
                frames: 0,
                wall: 0.0,
                worst: (0.0, Vec::new()),
                window_start: now,
                cpu0: cpu_times(),
                log0: log_counters(),
                waits0: (0, 0.0),
                fixed_steps: 0,
                events: 0,
                notes: Vec::new(),
            }
        })
    }

    /// Ends the time since the last mark as segment `name`.
    pub fn mark(&mut self, name: &'static str) {
        let now = Instant::now();
        self.segments.push((name, (now - self.seg).as_secs_f64() * 1000.0));
        self.seg = now;
    }

    /// A nested measurement shown under `segment`.
    pub fn detail(&mut self, segment: &'static str, name: &'static str, ms: f64) {
        self.details.push((segment, name, ms));
    }

    pub fn details(&mut self, segment: &'static str, list: &[(&'static str, f32)]) {
        for (n, ms) in list {
            self.details.push((segment, n, *ms as f64));
        }
    }

    pub fn fixed_steps(&mut self, n: u32) {
        self.fixed_steps += n as u64;
    }

    /// Frames recorded so far in the current window.
    pub fn frame_in_window(&self) -> usize {
        self.frames
    }

    /// A window event handled between frames.
    pub fn event(&mut self) {
        self.events += 1;
    }

    /// Free text for the next summary (present mode, background work).
    pub fn note(&mut self, s: String) {
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }

    /// Starts a frame: the time since the previous frame's end mark is the
    /// event-loop wait.
    pub fn begin(&mut self, upload_waits: (u32, f64)) {
        let now = Instant::now();
        if !self.started {
            self.started = true;
            self.start = now;
            self.seg = now;
            self.window_start = now;
            self.cpu0 = cpu_times();
            self.log0 = log_counters();
            self.waits0 = upload_waits;
            return;
        }
        // Whatever ran after the last mark of the previous frame (event
        // loop, other window events).
        self.mark("loop wait");
        let wall = (now - self.start).as_secs_f64() * 1000.0;
        self.frame += 1;
        self.frames += 1;
        self.wall += wall;
        let covered: f64 = self.segments.iter().map(|s| s.1).sum();
        for &(n, ms) in &self.segments {
            self.acc.add(n, ms);
        }
        self.acc.add("other", (wall - covered).max(0.0));
        for &(seg, n, ms) in &self.details {
            match self.detail_acc.iter_mut().find(|d| d.0 == seg) {
                Some(d) => d.1.add(n, ms),
                None => {
                    let mut a = Acc::default();
                    a.add(n, ms);
                    self.detail_acc.push((seg, a));
                }
            }
        }
        if wall > self.worst.0 {
            self.worst = (wall, std::mem::take(&mut self.segments));
        }
        self.segments.clear();
        self.details.clear();
        self.start = now;
        self.seg = now;
        if self.frames >= self.every {
            self.flush(upload_waits);
        }
    }

    fn flush(&mut self, upload_waits: (u32, f64)) {
        let n = self.frames.max(1) as f64;
        let span = self.window_start.elapsed().as_secs_f64() * 1000.0;
        let cpu = cpu_times();
        let (main_cpu, proc_cpu) = (cpu.0 - self.cpu0.0, cpu.1 - self.cpu0.1);
        let logs = log_counters();
        let fmt = |a: &Acc| a.entries.iter().filter(|e| e.1 / n >= 0.005).map(|(k, s, m)| format!("{k} {:.2}/{:.1}", s / n, m)).collect::<Vec<_>>().join(", ");
        let mut segs = String::new();
        for (k, s, m) in &self.acc.entries {
            segs.push_str(&format!(" | {k} {:.2}/{:.1}", s / n, m));
            if let Some(d) = self.detail_acc.iter().find(|d| d.0 == *k) {
                let mut v = d.1.entries.clone();
                v.sort_by(|a, b| b.1.total_cmp(&a.1));
                let s = fmt(&Acc { entries: v.into_iter().take(10).collect() });
                if !s.is_empty() {
                    segs.push_str(&format!(" [{s}]"));
                }
            }
        }
        let fps = 1000.0 * n / span.max(1e-3);
        let line = format!(
            "trace {} f{}-{}: {:.2} ms/frame ({fps:.1} fps), max {:.1}{segs} || main thread cpu {:.0}% of wall, process cpu {:.2} cores | fixed steps {:.2}/frame | log {} lines {:.2} ms/frame | upload flushes {} ({:.1} ms) | events {:.1}/frame{}{}",
            self.label,
            self.frame - self.frames as u64,
            self.frame - 1,
            self.wall / n,
            self.worst.0,
            100.0 * main_cpu / span.max(1e-3),
            proc_cpu / span.max(1e-3),
            self.fixed_steps as f64 / n,
            logs.0 - self.log0.0,
            (logs.1 - self.log0.1) / n,
            upload_waits.0 - self.waits0.0,
            upload_waits.1 - self.waits0.1,
            self.events as f64 / n,
            if self.notes.is_empty() { "" } else { " | " },
            self.notes.join(" | ")
        );
        let slowest = format!("trace {} slowest frame {:.1} ms: {}", self.label, self.worst.0, self.worst.1.iter().filter(|s| s.1 >= 0.05).map(|(k, ms)| format!("{k} {ms:.1}")).collect::<Vec<_>>().join(", "));
        log::info!("{line}");
        log::info!("{slowest}");
        // `FORJA_TRACE_FILE`: the summaries also go to this file (runs
        // whose console output is not captured).
        if let Some(path) = std::env::var_os("FORJA_TRACE_FILE") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{line}\n{slowest}");
            }
        }
        // The two log lines above belong to the next window; count them out.
        let after = log_counters();
        self.log0 = after;
        self.acc = Acc::default();
        self.detail_acc.clear();
        self.frames = 0;
        self.wall = 0.0;
        self.worst = (0.0, Vec::new());
        self.window_start = Instant::now();
        self.cpu0 = cpu_times();
        self.waits0 = upload_waits;
        self.fixed_steps = 0;
        self.events = 0;
        self.notes.clear();
        // The summary itself is not part of the next frame.
        self.start = self.window_start;
        self.seg = self.window_start;
    }
}
