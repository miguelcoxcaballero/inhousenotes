//! Item catalogue loaded from `data/items/*.toml` (`[[item]]` tables).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemCategory {
    Weapon,
    Ammo,
    Consumable,
    Armor,
    Key,
    Misc,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemDef {
    pub id: String,
    pub name: String,
    pub category: ItemCategory,
    #[serde(default = "one")]
    pub max_stack: u32,
    /// Kilograms per unit.
    #[serde(default)]
    pub weight: f32,
    /// Weapon definition id for weapon items.
    #[serde(default)]
    pub weapon: Option<String>,
    /// Health restored (consumables) or armor granted (armor).
    #[serde(default)]
    pub value: f32,
    /// Model path relative to `assets/downloaded`.
    #[serde(default)]
    pub model: Option<String>,
}

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, Default)]
pub struct ItemRegistry {
    items: HashMap<String, ItemDef>,
}

impl ItemRegistry {
    pub fn load(dir: &Path) -> Result<Self, String> {
        Ok(Self::from_defs(crate::load_dir(dir, "item")?))
    }

    pub fn from_defs(defs: Vec<ItemDef>) -> Self {
        Self { items: defs.into_iter().map(|d| (d.id.clone(), d)).collect() }
    }

    pub fn get(&self, id: &str) -> Option<&ItemDef> {
        self.items.get(id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ItemDef> {
        self.items.values()
    }
}
