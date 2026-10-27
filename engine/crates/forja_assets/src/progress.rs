//! Load progress shared between the thread that builds a world and the
//! one that draws the loading screen.
//!
//! Builders report the stage they enter with [`stage`] (a label and the
//! overall fraction reached when it starts); the loading screen reads
//! [`get`]. The fraction never goes back within one load ([`reset`] starts
//! a new one).

use std::sync::Mutex;

struct State {
    label: String,
    fraction: f32,
}

static STATE: Mutex<State> = Mutex::new(State { label: String::new(), fraction: 0.0 });

/// Starts a new load.
pub fn reset(label: &str) {
    if let Ok(mut s) = STATE.lock() {
        s.label = label.to_string();
        s.fraction = 0.0;
    }
}

/// Enters a stage at `fraction` (0..1) of the whole load.
pub fn stage(label: &str, fraction: f32) {
    if let Ok(mut s) = STATE.lock() {
        s.label = label.to_string();
        s.fraction = s.fraction.max(fraction.clamp(0.0, 1.0));
    }
}

/// Moves the bar within the current stage (keeps the label).
pub fn advance(fraction: f32) {
    if let Ok(mut s) = STATE.lock() {
        s.fraction = s.fraction.max(fraction.clamp(0.0, 1.0));
    }
}

/// Current label and fraction.
pub fn get() -> (String, f32) {
    STATE.lock().map(|s| (s.label.clone(), s.fraction)).unwrap_or_default()
}
