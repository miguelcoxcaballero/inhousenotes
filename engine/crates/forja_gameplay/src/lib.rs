//! # forja_gameplay
//!
//! Gameplay building blocks that only depend on data files and the physics
//! API: swap the weapon list, the item catalogue or a car's handling by
//! editing TOML in `data/`.

pub mod health;
pub mod inventory;
pub mod items;
pub mod vehicles;
pub mod weapons;

pub use health::Health;
pub use inventory::{Inventory, ItemStack, WeaponWheel};
pub use items::{ItemCategory, ItemDef, ItemRegistry};
pub use vehicles::{BoundVehicle, HandlingRegistry, auto_orient, auto_scale, bind_vehicle_model, bind_vehicle_model_oriented, straighten_yaw};
pub use weapons::{FireMode, Hit, WeaponCategory, WeaponDef, WeaponRegistry, WeaponState};

/// Loads every `*.toml` in a directory and merges the parsed values.
pub(crate) fn load_dir<T: serde::de::DeserializeOwned>(dir: &std::path::Path, key: &str) -> Result<Vec<T>, String> {
    #[derive(serde::Deserialize)]
    struct File<T> {
        #[serde(default = "Vec::new")]
        items: Vec<T>,
    }
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let value: toml::Table = toml::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        let Some(list) = value.get(key) else { continue };
        let wrapped = toml::Value::Table([("items".to_string(), list.clone())].into_iter().collect());
        let f: File<T> = wrapped.try_into().map_err(|e| format!("{}: {e}", p.display()))?;
        out.extend(f.items);
    }
    Ok(out)
}
