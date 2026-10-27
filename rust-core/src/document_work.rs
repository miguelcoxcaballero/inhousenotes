//! Input-priority scheduling. No timers, browser imports or document payloads.
//! The host supplies one interaction snapshot for all waiting document tasks.
use std::collections::VecDeque;

struct Job { id: u32, session: f64, quiet_ms: f64 }
#[derive(Default)]
pub struct WorkQueue { jobs: VecDeque<Job> }

#[no_mangle]
pub extern "C" fn document_work_version() -> u32 { 1 }
#[no_mangle]
pub extern "C" fn document_work_new() -> *mut WorkQueue {
    Box::into_raw(Box::new(WorkQueue::default()))
}
// Handles and aligned output buffers belong exclusively to the private adapter.
#[no_mangle]
pub unsafe extern "C" fn document_work_add(q: *mut WorkQueue, id: u32, session: f64, quiet_ms: f64) -> u32 {
    let Some(q) = q.as_mut() else { return 0 };
    if id == 0 || !session.is_finite() || session < -1.0 || !quiet_ms.is_finite()
        || !(0.0..=60000.0).contains(&quiet_ms) || q.jobs.len() >= 4096
        || q.jobs.iter().any(|job| job.id == id) { return 0; }
    q.jobs.push_back(Job { id, session, quiet_ms });
    1
}
#[no_mangle]
pub unsafe extern "C" fn document_work_cancel(q: *mut WorkQueue, id: u32) {
    if let Some(q) = q.as_mut() { q.jobs.retain(|job| job.id != id); }
}

impl WorkQueue {
    // flags: 1 = interaction/input/parent busy, 2 = explicit exit or hidden host.
    // At most eight continuations per turn, including stale-session rejection.
    fn poll(&mut self, now: f64, last_activity: f64, session: f64, flags: u32, out: &mut [f64; 17]) -> usize {
        let bypass = flags & 2 != 0;
        let busy = flags & 1 != 0 && !bypass;
        let elapsed = (now - last_activity).max(0.0);
        let mut count = 0;
        let mut delay = f64::INFINITY;
        self.jobs.retain(|job| {
            let stale = job.session >= 0.0 && job.session != session;
            let remaining = if bypass { 0.0 } else { (job.quiet_ms - elapsed).max(0.0) };
            if (stale || (!busy && remaining == 0.0)) && count < 8 {
                out[1 + count * 2] = job.id as f64;
                out[2 + count * 2] = if stale { 0.0 } else { 1.0 };
                count += 1;
                return false;
            }
            delay = delay.min(if stale { 1.0 } else if !busy { remaining.max(1.0) } else { 100.0 });
            true
        });
        out[0] = if self.jobs.is_empty() { -1.0 } else { delay };
        count
    }
}
#[no_mangle]
pub unsafe extern "C" fn document_work_poll(q: *mut WorkQueue, now: f64, last_activity: f64,
    session: f64, flags: u32, out: *mut f64) -> i32 {
    let Some(q) = q.as_mut() else { return -1 };
    if out.is_null() || ![now, last_activity, session].iter().all(|v| v.is_finite()) { return -1; }
    q.poll(now, last_activity, session, flags, &mut *(out as *mut [f64; 17])) as i32
}
#[no_mangle]
pub unsafe extern "C" fn document_work_drop(q: *mut WorkQueue) {
    if !q.is_null() { drop(Box::from_raw(q)); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_pen_stale_sessions_and_bounded_resume() {
        let mut q = WorkQueue::default();
        for id in 1..=20 { q.jobs.push_back(Job { id, session: 3.0, quiet_ms: 1200.0 }); }
        let mut out = [0.0; 17];
        assert_eq!(q.poll(5000.0, 0.0, 3.0, 1, &mut out), 0);
        assert_eq!(out[0], 100.0);
        assert_eq!(q.poll(5000.0, 4900.0, 3.0, 0, &mut out), 0);
        assert_eq!(q.poll(6100.0, 4900.0, 3.0, 0, &mut out), 8);
        assert_eq!(out[1], 1.0); assert_eq!(out[15], 8.0);
        assert_eq!(q.poll(6101.0, 4900.0, 4.0, 1, &mut out), 8);
        assert_eq!(out[2], 0.0);
        assert_eq!(q.poll(6102.0, 4900.0, 3.0, 3, &mut out), 4);
        assert_eq!(out[0], -1.0);
    }
}
