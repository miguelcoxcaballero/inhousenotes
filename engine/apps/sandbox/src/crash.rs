//! Crash report: a panic anywhere in the game writes `forja_crash.txt`
//! next to the executable (the message, where it happened, the thread, what
//! the game was doing, a backtrace and the last log lines) and, on Windows,
//! shows a message box saying so, instead of the console closing with the
//! evidence.
//!
//! Only a panic of the main thread shows the box; a worker's panic writes
//! its report and the main thread decides whether it is fatal ([`fatal`]
//! keeps that report and shows the box). Unattended runs (`--smoke`,
//! screenshots, `FORJA_NO_CRASH_BOX=1`) never show it: CI would hang.
//!
//! The log lines come from the game's logger ([`crate::trace::init_logger`]
//! hands every record to [`remember`]; the last [`LOG_LINES`] are kept).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Log records kept for the crash report.
pub const LOG_LINES: usize = 200;
/// File name of the report.
pub const FILE_NAME: &str = "forja_crash.txt";

static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// Only the first fatal error shows the message box.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Unattended run: no message box.
static QUIET: AtomicBool = AtomicBool::new(false);
/// What the game is doing (map, GPU), written into the report.
static CONTEXT: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());
/// Where the last report went.
static LAST_REPORT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Keeps a log line for the report (called by the logger). Never blocks: a
/// line is dropped rather than wait on a panicking writer.
pub fn remember(line: String) {
    let mut q = match RECENT.try_lock() {
        Ok(q) => q,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    if q.len() >= LOG_LINES {
        q.pop_front();
    }
    q.push_back(line);
}

/// The last log lines, oldest first.
pub fn recent_lines() -> Vec<String> {
    match RECENT.try_lock() {
        Ok(q) => q.iter().cloned().collect(),
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().iter().cloned().collect(),
        Err(std::sync::TryLockError::WouldBlock) => vec!["(log buffer busy)".into()],
    }
}

/// Records what the game is doing (`key` replaces its earlier value).
pub fn set_context(key: &'static str, value: impl Into<String>) {
    let mut c = CONTEXT.lock().unwrap_or_else(|e| e.into_inner());
    let value = value.into();
    match c.iter_mut().find(|(k, _)| *k == key) {
        Some(e) => e.1 = value,
        None => c.push((key, value)),
    }
}

/// `true`: never show the message box (the report is written either way).
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Where the report goes: next to the executable, else the working
/// directory, else the temp folder.
pub fn report_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        v.push(dir.join(FILE_NAME));
    }
    v.push(PathBuf::from(FILE_NAME));
    v.push(std::env::temp_dir().join(FILE_NAME));
    v
}

/// Installs the panic hook (at start, before anything can panic). `quiet`:
/// never show a message box (unattended runs).
pub fn install(quiet: bool) {
    set_quiet(quiet || std::env::var_os("FORJA_NO_CRASH_BOX").is_some());
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The default hook still prints the panic to the console.
        default(info);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let main = std::thread::current().name() == Some("main");
        let path = write_report(&info.to_string(), &backtrace.to_string());
        log::error!("panic: {info}");
        if main {
            dialog(&info.to_string(), path.as_deref());
        }
    }));
}

/// A fatal error the main thread found (a worker thread panicked): logs
/// it, keeps the worker's report (its backtrace is the useful one) or
/// writes one, shows the message box and exits with `code`.
pub fn fatal(what: &str, code: i32) -> ! {
    log::error!("{what}");
    eprintln!("Forja: {what}");
    let previous = LAST_REPORT.lock().ok().and_then(|p| p.clone());
    let path = previous.or_else(|| write_report(what, &std::backtrace::Backtrace::force_capture().to_string()));
    dialog(what, path.as_deref());
    std::process::exit(code);
}

/// Writes the report; returns where it went.
fn write_report(panic: &str, backtrace: &str) -> Option<PathBuf> {
    let report = format_report(panic, backtrace, &recent_lines());
    for p in report_paths() {
        if std::fs::write(&p, &report).is_ok() {
            eprintln!("Forja: crash report -> {}", p.display());
            *LAST_REPORT.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
            return Some(p);
        }
    }
    eprintln!("Forja: could not write {FILE_NAME}");
    None
}

/// The text of `forja_crash.txt`.
pub fn format_report(panic: &str, backtrace: &str, log: &[String]) -> String {
    use std::fmt::Write;
    let thread = std::thread::current();
    let mut s = String::new();
    let _ = writeln!(s, "Forja crash report");
    let _ = writeln!(s, "version: {} ({})", env!("CARGO_PKG_VERSION"), option_env!("FORJA_BUILD").unwrap_or("local build"));
    let _ = writeln!(s, "time (unix): {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()));
    let _ = writeln!(s, "os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(s, "args: {}", std::env::args().collect::<Vec<_>>().join(" "));
    let _ = writeln!(s, "thread: {}", thread.name().unwrap_or("<unnamed>"));
    if let Ok(c) = CONTEXT.try_lock() {
        for (k, v) in c.iter() {
            let _ = writeln!(s, "{k}: {v}");
        }
    }
    let _ = writeln!(s, "\npanic: {panic}\n\nbacktrace:\n{backtrace}");
    let _ = writeln!(s, "last {} log lines:", log.len());
    for l in log {
        let _ = writeln!(s, "{l}");
    }
    s
}

fn dialog(what: &str, path: Option<&std::path::Path>) {
    if QUIET.load(Ordering::Relaxed) || SHOWN.swap(true, Ordering::Relaxed) {
        return;
    }
    let first: String = what.lines().take(3).collect::<Vec<_>>().join("\n").chars().take(400).collect();
    let shown = path.map_or_else(|| "(no se pudo escribir)".to_string(), |p| p.display().to_string());
    message_box("Forja se ha cerrado por un error", &format!("Forja ha encontrado un error y tiene que cerrarse.\n\n{first}\n\nSe ha guardado un informe en:\n{shown}\n\nEnvía ese archivo para que podamos arreglarlo."));
}

#[cfg(windows)]
fn message_box(title: &str, text: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (t, c) = (wide(text), wide(title));
    // MB_OK | MB_ICONERROR | MB_TASKMODAL | MB_SETFOREGROUND | MB_TOPMOST:
    // above the (borderless fullscreen) game window.
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call.
    unsafe {
        MessageBoxW(0, t.as_ptr(), c.as_ptr(), 0x10 | 0x2000 | 0x1_0000 | 0x4_0000);
    }
}

#[cfg(not(windows))]
fn message_box(_title: &str, _text: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_panic_context_backtrace_and_log() {
        for i in 0..LOG_LINES + 5 {
            remember(format!("line {i}"));
        }
        let lines = recent_lines();
        assert_eq!(lines.len(), LOG_LINES);
        assert_eq!(lines.last().map(String::as_str), Some(format!("line {}", LOG_LINES + 4).as_str()));
        set_context("map", "Test track");
        let r = format_report("boom at main.rs:1", "0: frame", &lines);
        assert!(r.contains("panic: boom at main.rs:1"));
        assert!(r.contains("map: Test track"));
        assert!(r.contains("backtrace:\n0: frame"));
        assert!(r.contains(&format!("line {}", LOG_LINES + 4)));
    }
}
