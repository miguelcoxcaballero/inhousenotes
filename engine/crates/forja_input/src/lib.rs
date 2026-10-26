//! Action-based input. The platform layer reports raw keys by name
//! (`"KeyW"`, `"Space"`, `"MouseLeft"`, ...) and gameplay asks for actions
//! (`"accelerate"`, `"fire"`), so bindings live in `data/input.toml`.

use std::collections::{HashMap, HashSet};

use forja_math::Vec2;
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ActionMap {
    /// Action name -> any of these keys.
    pub actions: HashMap<String, Vec<String>>,
    #[serde(default = "sensitivity")]
    pub mouse_sensitivity: f32,
}

fn sensitivity() -> f32 {
    0.0025
}

impl ActionMap {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

#[derive(Default)]
pub struct Input {
    pub map: ActionMap,
    held: HashSet<String>,
    pressed: HashSet<String>,
    released: HashSet<String>,
    pub mouse_delta: Vec2,
    pub scroll: f32,
}

impl Input {
    pub fn new(map: ActionMap) -> Self {
        Self { map, ..Default::default() }
    }

    pub fn key_down(&mut self, key: &str) {
        if self.held.insert(key.to_string()) {
            self.pressed.insert(key.to_string());
        }
    }

    pub fn key_up(&mut self, key: &str) {
        if self.held.remove(key) {
            self.released.insert(key.to_string());
        }
    }

    pub fn mouse_moved(&mut self, d: Vec2) {
        self.mouse_delta += d;
    }

    /// Clears per-frame state; call after gameplay consumed it.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.mouse_delta = Vec2::ZERO;
        self.scroll = 0.0;
    }

    /// Drops all held keys (focus lost).
    pub fn reset(&mut self) {
        self.held.clear();
        self.end_frame();
    }

    fn keys(&self, action: &str) -> &[String] {
        self.map.actions.get(action).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn held(&self, action: &str) -> bool {
        self.keys(action).iter().any(|k| self.held.contains(k))
    }

    pub fn pressed(&self, action: &str) -> bool {
        self.keys(action).iter().any(|k| self.pressed.contains(k))
    }

    pub fn released(&self, action: &str) -> bool {
        self.keys(action).iter().any(|k| self.released.contains(k))
    }

    /// `positive` minus `negative`, in -1..1.
    pub fn axis(&self, negative: &str, positive: &str) -> f32 {
        self.held(positive) as i32 as f32 - self.held(negative) as i32 as f32
    }

    pub fn look(&self) -> Vec2 {
        self.mouse_delta * self.map.mouse_sensitivity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_from_shipped_bindings() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/input.toml")).unwrap();
        let mut input = Input::new(ActionMap::from_toml(&text).unwrap());
        input.key_down("KeyW");
        assert!(input.pressed("forward") && input.held("forward"));
        assert_eq!(input.axis("back", "forward"), 1.0);
        input.end_frame();
        assert!(!input.pressed("forward") && input.held("forward"));
        input.key_up("KeyW");
        assert!(input.released("forward"));
        for a in ["fire", "aim", "jump", "enter_vehicle", "reload", "inventory", "handbrake"] {
            assert!(input.map.actions.contains_key(a), "unbound action {a}");
        }
    }
}
