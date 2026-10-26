//! Health with armor absorption.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Health {
    pub hp: f32,
    pub max_hp: f32,
    pub armor: f32,
    /// Fraction of damage absorbed by armor while it lasts.
    pub armor_absorb: f32,
}

impl Default for Health {
    fn default() -> Self {
        Self { hp: 100.0, max_hp: 100.0, armor: 0.0, armor_absorb: 0.7 }
    }
}

impl Health {
    pub fn is_dead(&self) -> bool {
        self.hp <= 0.0
    }

    /// Applies damage; returns the health actually lost.
    pub fn damage(&mut self, amount: f32) -> f32 {
        if self.is_dead() || amount <= 0.0 {
            return 0.0;
        }
        let absorbed = (amount * self.armor_absorb).min(self.armor);
        self.armor -= absorbed;
        let lost = (amount - absorbed).min(self.hp);
        self.hp -= lost;
        lost
    }

    pub fn heal(&mut self, amount: f32) {
        if !self.is_dead() {
            self.hp = (self.hp + amount).min(self.max_hp);
        }
    }
}
