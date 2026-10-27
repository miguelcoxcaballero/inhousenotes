//! Material palette of a landmark: `[material.<name>]` tables plus
//! built-in names (`stone`, `glass`, `roof`, `iron`...) usable without a
//! definition. Each entry maps to a material kind implemented in
//! [`super::materials`].

use std::collections::{BTreeMap, HashMap};

use super::spec::{Color, MaterialSpec};

/// Material kinds (the `kind` key).
pub const KINDS: &[&str] = &[
    "ashlar", "stone", "limestone", "sandstone", "brick", "stucco", "render", "plaster", "moulding", "granite", "marble", "travertine", "concrete", "terracotta", "roof_tiles", "clay_tiles", "slate", "zinc", "lead",
    "copper", "bronze", "gold", "iron", "steel", "white_steel", "glass", "curtain_wall", "stained_glass", "wood", "painted_wood", "azulejo", "glazed_tiles", "trencadis", "polychrome", "water", "interior", "dark",
    "paving", "cobble", "asphalt", "grass", "flat",
];

/// Built-in material names and their definitions.
pub fn builtin(name: &str) -> Option<MaterialSpec> {
    let m = |kind: &str, tint: Option<u32>| MaterialSpec {
        kind: kind.into(),
        tint: tint.map(|c| Color([((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0])),
        color: None,
        colors: Vec::new(),
        weights: Vec::new(),
        pattern: None,
        clearcoat: None,
        roughness: None,
        metallic: None,
        uv_scale: None,
        uv_rotate: None,
        normal_strength: None,
        weathering: None,
        block: None,
        emissive: None,
        seed: None,
        note: None,
    };
    Some(match name {
        "stone" | "ashlar" => m("ashlar", None),
        "limestone" => m("limestone", None),
        "sandstone" => m("sandstone", None),
        "brick" => m("brick", None),
        "plaster" => m("plaster", Some(0xf2ede4)),
        "stucco" | "render" => m("stucco", Some(0xece6da)),
        "moulding" => m("moulding", Some(0xf0ebe0)),
        "granite" => m("granite", None),
        "marble" => m("marble", None),
        "travertine" => m("travertine", None),
        "concrete" | "white_concrete" => m("concrete", None),
        "terracotta" => m("terracotta", None),
        "roof" | "roof_tiles" | "tiles" | "clay_tiles" => m("roof_tiles", None),
        "slate" => m("slate", None),
        "zinc" => m("zinc", None),
        "lead" => m("lead", None),
        "copper" => m("copper", None),
        "bronze" => m("bronze", None),
        "gold" => m("gold", None),
        "iron" => m("iron", None),
        "steel" | "white_steel" => m("white_steel", None),
        "glass" => m("glass", None),
        "curtain_wall" => m("curtain_wall", None),
        "stained_glass" => m("stained_glass", None),
        "wood" => m("wood", None),
        "door" => m("wood", Some(0x6a4428)),
        "frame" => m("painted_wood", Some(0xe9e4d8)),
        "azulejo" => m("azulejo", None),
        "glazed_tiles" | "blue_tiles" => m("glazed_tiles", None),
        "trencadis" => m("trencadis", None),
        "polychrome" => m("polychrome", None),
        "water" => m("water", None),
        "interior" => m("interior", None),
        "dark" => m("dark", None),
        "paving" | "pavement" => m("paving", None),
        "cobble" => m("cobble", None),
        "asphalt" => m("asphalt", None),
        "grass" => m("grass", None),
        "context" => m("stucco", Some(0xbdb6aa)),
        _ => return None,
    })
}

pub const BUILTIN_NAMES: &[&str] = &[
    "stone", "ashlar", "limestone", "sandstone", "brick", "plaster", "stucco", "render", "moulding", "granite", "marble", "travertine", "concrete", "white_concrete", "terracotta", "roof", "roof_tiles", "tiles", "slate",
    "zinc", "lead", "copper", "bronze", "gold", "iron", "steel", "white_steel", "glass", "curtain_wall", "stained_glass", "wood", "door", "frame", "azulejo", "glazed_tiles", "blue_tiles", "trencadis", "polychrome",
    "water", "interior", "dark", "paving", "pavement", "cobble", "asphalt", "grass", "context",
];

/// Texture repeat in metres when the spec does not set `uv_scale`.
pub fn default_uv_scale(kind: &str, spec: &MaterialSpec) -> f32 {
    match kind {
        "ashlar" | "stone" | "limestone" => {
            let b = spec.block.unwrap_or(if kind == "limestone" { [1.15, 0.52] } else { [0.95, 0.45] });
            // The generated texture holds about 8 blocks by 16 courses.
            (b[0] * 8.0).max(b[1] * 16.0).clamp(3.0, 12.0)
        }
        "sandstone" => 1.6,
        "brick" => 1.4,
        "stucco" | "render" | "plaster" => 2.5,
        "moulding" | "concrete" => 2.0,
        "granite" => 1.5,
        "marble" => 1.6,
        "travertine" => 1.8,
        "terracotta" => 1.0,
        "roof_tiles" | "clay_tiles" => 2.0,
        "slate" => 1.6,
        "zinc" | "lead" | "copper" => 2.0,
        "bronze" | "gold" => 1.0,
        "iron" | "steel" | "white_steel" => 1.0,
        "glass" => 1.0,
        "curtain_wall" => 3.0,
        "stained_glass" => 1.2,
        "wood" | "painted_wood" => 1.2,
        "azulejo" => 0.6,
        "glazed_tiles" => 1.2,
        "trencadis" => 1.0,
        "polychrome" => 0.9,
        "paving" => 2.0,
        "cobble" => 2.0,
        "asphalt" => 5.0,
        "grass" => 3.0,
        _ => 1.0,
    }
}

/// One palette entry.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub spec: MaterialSpec,
    /// Metres per texture repeat.
    pub uv_scale: f32,
    /// UVs are metric (scaled at assembly); false = already normalised.
    pub metric: bool,
}

/// Name → material id resolution, with built-ins added on first use.
#[derive(Clone, Debug, Default)]
pub struct Palette {
    pub entries: Vec<Entry>,
    map: HashMap<String, u32>,
    user: BTreeMap<String, MaterialSpec>,
}

pub fn check_kind(kind: &str) -> Result<(), String> {
    let k = kind.trim().to_ascii_lowercase();
    if KINDS.contains(&k.as_str()) {
        Ok(())
    } else {
        Err(format!("unknown material kind `{kind}`{}; expected one of {}", super::parse::did_you_mean(&k, KINDS), KINDS.join(", ")))
    }
}

impl Palette {
    pub fn new(user: &BTreeMap<String, MaterialSpec>) -> Result<Palette, String> {
        for (name, m) in user {
            check_kind(&m.kind).map_err(|e| format!("[material.{name}]: {e}"))?;
            if let Some(s) = m.uv_scale
                && !(s > 0.0 && s.is_finite())
            {
                return Err(format!("[material.{name}]: uv_scale must be positive"));
            }
            if let Some(p) = &m.pattern {
                let k = m.kind.trim().to_ascii_lowercase();
                let names: &[&str] = match k.as_str() {
                    "glazed_tiles" => &["scales", "chevron", "zigzag", "stripes", "stripes_h", "hstripes", "bands", "checker", "diamond", "plain"],
                    "azulejo" => &["checker_blue", "checker_green", "diamond_blue", "diamond", "frieze", "white"],
                    _ => return Err(format!("[material.{name}]: `pattern` only applies to glazed_tiles and azulejo")),
                };
                if !names.contains(&p.as_str()) {
                    return Err(format!("[material.{name}]: unknown {k} pattern `{p}`{}; expected one of {}", super::parse::did_you_mean(p, names), names.join(", ")));
                }
            }
            if !m.weights.is_empty() && m.weights.len() != m.colors.len() {
                return Err(format!("[material.{name}]: `weights` needs one value per colour ({} colours, {} weights)", m.colors.len(), m.weights.len()));
            }
        }
        Ok(Palette { entries: Vec::new(), map: HashMap::new(), user: user.clone() })
    }

    fn push(&mut self, name: &str, mut spec: MaterialSpec) -> u32 {
        spec.kind = spec.kind.trim().to_ascii_lowercase();
        let metric = spec.kind != "interior";
        let uv_scale = spec.uv_scale.unwrap_or_else(|| default_uv_scale(&spec.kind, &spec));
        let id = self.entries.len() as u32;
        self.entries.push(Entry { name: name.into(), spec, uv_scale, metric });
        self.map.insert(name.into(), id);
        id
    }

    /// Material id for a palette or built-in name.
    pub fn id(&mut self, name: &str) -> Result<u32, String> {
        if let Some(&i) = self.map.get(name) {
            return Ok(i);
        }
        if let Some(s) = self.user.get(name).cloned() {
            return Ok(self.push(name, s));
        }
        if let Some(s) = builtin(name) {
            return Ok(self.push(name, s));
        }
        let mut names: Vec<&str> = self.user.keys().map(|s| s.as_str()).collect();
        names.extend_from_slice(BUILTIN_NAMES);
        Err(format!(
            "unknown material `{name}`{}; define [material.{name}] or use a built-in ({})",
            super::parse::did_you_mean(name, &names),
            self.user.keys().map(|s| s.as_str()).chain(["stone", "glass", "roof", "iron", "wood", "..."]).collect::<Vec<_>>().join(", ")
        ))
    }

    pub fn get(&self, id: u32) -> &Entry {
        &self.entries[id as usize]
    }

    /// Id of a material already in use (no built-in lookup).
    pub fn find(&self, name: &str) -> Option<u32> {
        self.map.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
