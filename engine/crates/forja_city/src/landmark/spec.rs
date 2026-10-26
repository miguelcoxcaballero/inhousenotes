//! The landmark spec language (`landmark.toml`), deserialised with serde.
//!
//! Every table rejects unknown keys; element tables dispatch on their
//! `type` key without buffering, so errors keep the exact line and column
//! of the offending key (see [`super::parse`]).

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, de::value::MapAccessDeserializer};

pub use toml::Spanned;

// ----------------------------------------------------------------- helpers

fn yes() -> bool {
    true
}

/// Values that accept a shorthand (string, bool or number) besides their
/// full table form.
pub trait Shorthand: Sized {
    const EXPECTING: &'static str;
    fn from_str(_s: &str) -> Result<Self, String> {
        Err(format!("expected {}", Self::EXPECTING))
    }
    fn from_bool(_b: bool) -> Result<Self, String> {
        Err(format!("expected {}", Self::EXPECTING))
    }
    fn from_num(_v: f64) -> Result<Self, String> {
        Err(format!("expected {}", Self::EXPECTING))
    }
}

/// `T` written either as a table or as a shorthand. `false` means absent.
#[derive(Clone, Debug)]
pub struct Flex<T>(pub Option<T>);

impl<T> Default for Flex<T> {
    fn default() -> Self {
        Flex(None)
    }
}

impl<T> Flex<T> {
    pub fn get(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<'de, T: Deserialize<'de> + Shorthand> Deserialize<'de> for Flex<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de> + Shorthand> Visitor<'de> for V<T> {
            type Value = Flex<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(T::EXPECTING)
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
                T::from_str(s).map(|t| Flex(Some(t))).map_err(E::custom)
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<Self::Value, E> {
                if !b {
                    return Ok(Flex(None));
                }
                T::from_bool(b).map(|t| Flex(Some(t))).map_err(E::custom)
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                T::from_num(v).map(|t| Flex(Some(t))).map_err(E::custom)
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                T::from_num(v as f64).map(|t| Flex(Some(t))).map_err(E::custom)
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                T::from_num(v as f64).map(|t| Flex(Some(t))).map_err(E::custom)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(|t| Flex(Some(t)))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// Picks an enum variant from a string, with a "did you mean" hint.
fn pick<T: Copy>(s: &str, table: &[(&str, T)], what: &str) -> Result<T, String> {
    let k = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    table.iter().find(|(n, _)| *n == k).map(|(_, v)| *v).ok_or_else(|| {
        let names: Vec<&str> = table.iter().map(|(n, _)| *n).collect();
        format!("unknown {what} `{s}`{}; expected one of {}", super::parse::did_you_mean(&k, &names), names.join(", "))
    })
}

macro_rules! str_enum {
    ($(#[$m:meta])* $name:ident, $what:expr, { $($v:ident => [$($s:expr),+]),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($v),* }
        impl $name {
            pub const NAMES: &'static [(&'static str, $name)] = &[$($(($s, $name::$v)),+),*];
            pub fn parse(s: &str) -> Result<Self, String> { pick(s, Self::NAMES, $what) }
            pub fn name(self) -> &'static str { Self::NAMES.iter().find(|(_, v)| *v == self).map(|(n, _)| *n).unwrap_or("?") }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                $name::parse(&s).map_err(de::Error::custom)
            }
        }
    };
}

/// A string restricted to `names` (lower-cased, `-`/space as `_`), with a
/// "did you mean" error on typos.
fn one_of<'de, D: Deserializer<'de>>(d: D, names: &[&str], what: &str) -> Result<String, D::Error> {
    let s = String::deserialize(d)?;
    let k = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    if names.contains(&k.as_str()) {
        return Ok(k);
    }
    Err(de::Error::custom(format!("unknown {what} `{s}`{}; expected one of {}", super::parse::did_you_mean(&k, names), names.join(", "))))
}

/// Deserialisers for free-text keys with a fixed vocabulary.
macro_rules! checked {
    ($names:ident, $f:ident, $fo:ident, $what:expr, [$($n:expr),* $(,)?]) => {
        pub const $names: &[&str] = &[$($n),*];
        #[allow(dead_code)]
        fn $f<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
            one_of(d, $names, $what)
        }
        #[allow(dead_code)]
        fn $fo<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
            one_of(d, $names, $what).map(Some)
        }
    };
}

checked!(PEDIMENTS, de_pediment, de_pediment_opt, "pediment", ["triangle", "segmental", "broken", "broken_segmental"]);
checked!(RAILINGS, de_railing, de_railing_opt, "railing", ["iron", "stone", "balustrade", "glass", "none"]);
checked!(MODILLION_STYLES, de_modillion, de_modillion_opt, "modillion style", ["bracket", "block", "corbel", "console"]);
checked!(FINIALS, de_finials, de_finials_opt, "balustrade finial", ["none", "urn", "ball", "pinecone"]);
checked!(MANSARD_UPPER, de_mansard, de_mansard_opt, "mansard upper part", ["hip", "flat", "pyramid"]);
checked!(RIB_DIRS, de_ribdir, de_ribdir_opt, "rib direction", ["u", "v"]);
checked!(BUTTRESS_CAPS, de_bcap, de_bcap_opt, "buttress cap", ["slope", "gable", "flat", "pinnacle"]);
checked!(LANTERN_CAPS, de_lcap, de_lcap_opt, "lantern cap", ["dome", "cone", "onion", "spire", "flat"]);
checked!(ALIGNS, de_align, de_align_opt, "wall alignment", ["center", "centre", "left", "right"]);
checked!(SIDES, de_side, de_side_opt, "side", ["left", "right"]);
checked!(PLANES, de_plane, de_plane_opt, "extrusion plane", ["xy", "xz", "yz", "zy"]);
checked!(SKIES, de_sky, de_sky_opt, "sky", ["clear", "sky", "street", "atmosphere"]);
checked!(TIMES, de_time, de_time_opt, "time of day", ["day", "morning", "evening", "dusk", "night"]);

/// Smoothing: `true` (40 degree crease), `false`, or a crease angle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Crease(pub f32);
impl<'de> Deserialize<'de> for Crease {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Crease;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("true, false or a crease angle in degrees")
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<Crease, E> {
                Ok(Crease(if b { 40.0 } else { 0.0 }))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Crease, E> {
                Ok(Crease(v as f32))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Crease, E> {
                Ok(Crease(v as f32))
            }
        }
        d.deserialize_any(V)
    }
}

// ------------------------------------------------------------------ colors

/// `"#rrggbb"`, `[r, g, b]` in 0..255, or `[r, g, b]` in 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub [f32; 3]);

impl Color {
    pub fn parse_hex(s: &str) -> Result<Color, String> {
        let h = s.trim().trim_start_matches('#');
        let v = u32::from_str_radix(h, 16).map_err(|_| format!("bad color `{s}` (use \"#rrggbb\")"))?;
        if h.len() != 6 {
            return Err(format!("bad color `{s}` (use \"#rrggbb\")"));
        }
        Ok(Color([((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0]))
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Color;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a color \"#rrggbb\" or [r, g, b]")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Color, E> {
                Color::parse_hex(s).map_err(E::custom)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Color, A::Error> {
                let mut c = [0.0f32; 3];
                for v in &mut c {
                    *v = a.next_element::<f32>()?.ok_or_else(|| de::Error::custom("a color needs 3 components"))?;
                }
                if a.next_element::<f32>()?.is_some() {
                    return Err(de::Error::custom("a color needs 3 components"));
                }
                if c.iter().any(|v| *v > 1.0) {
                    c = c.map(|v| v / 255.0);
                }
                Ok(Color(c))
            }
        }
        d.deserialize_any(V)
    }
}

// --------------------------------------------------------------- transform

/// Uniform or per-axis scale.
#[derive(Clone, Copy, Debug)]
pub struct Scale(pub [f32; 3]);

impl Default for Scale {
    fn default() -> Self {
        Scale([1.0; 3])
    }
}

impl<'de> Deserialize<'de> for Scale {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Scale;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a number or [sx, sy, sz]")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Scale, E> {
                Ok(Scale([v as f32; 3]))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Scale, E> {
                Ok(Scale([v as f32; 3]))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Scale, A::Error> {
                let v = <[f32; 3]>::deserialize(de::value::SeqAccessDeserializer::new(a))?;
                Ok(Scale(v))
            }
        }
        d.deserialize_any(V)
    }
}

/// Placement of an element in its parent's frame:
/// `translate · pivot · rotate_y · rotate(x, y, z) · scale · -pivot`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    #[serde(default)]
    pub translate: [f32; 3],
    /// Degrees, counter-clockwise seen from above (east turns to north).
    #[serde(default)]
    pub rotate_y: f32,
    /// Degrees about x, y, z (applied in that order).
    #[serde(default)]
    pub rotate: [f32; 3],
    #[serde(default)]
    pub scale: Scale,
    /// Point (parent frame) that rotation and scale act about.
    #[serde(default)]
    pub pivot: [f32; 3],
}

/// Extra mirrored copy: `"x"` (across the plane x = 0), `"z"`, `"xz"`,
/// or `{ axis = "x", at = 12.5 }`.
#[derive(Clone, Debug)]
pub struct Mirror {
    pub x: bool,
    pub z: bool,
    pub at: [f32; 2],
}

impl<'de> Deserialize<'de> for Mirror {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct T {
            axis: String,
            #[serde(default)]
            at: Option<f32>,
            #[serde(default)]
            origin: Option<[f32; 2]>,
        }
        fn axes(s: &str) -> Result<(bool, bool), String> {
            match s.trim().to_ascii_lowercase().as_str() {
                "x" => Ok((true, false)),
                "z" => Ok((false, true)),
                "xz" | "zx" | "both" => Ok((true, true)),
                o => Err(format!("unknown mirror axis `{o}`; expected x, z or xz")),
            }
        }
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Mirror;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("\"x\", \"z\", \"xz\" or { axis, at }")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Mirror, E> {
                let (x, z) = axes(s).map_err(E::custom)?;
                Ok(Mirror { x, z, at: [0.0; 2] })
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Mirror, A::Error> {
                let t = T::deserialize(MapAccessDeserializer::new(m))?;
                let (x, z) = axes(&t.axis).map_err(de::Error::custom)?;
                let at = t.origin.unwrap_or([t.at.unwrap_or(0.0); 2]);
                Ok(Mirror { x, z, at })
            }
        }
        d.deserialize_any(V)
    }
}

// ---------------------------------------------------------------- polygons

/// A plan polygon: literal points `[[x, z], ...]`, a reference string
/// (`"outline:0"`, `"osm:way/123"`, `"catastro:<id>"`, `"part:2"`,
/// `"parent"`, `"circle:4"`), or a table with a source and operations.
#[derive(Clone, Debug)]
pub enum PolySpec {
    Points(Vec<[f32; 2]>),
    Ref(String),
    Table(Box<PolyTable>),
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolyTable {
    /// Source reference (same syntax as the string form).
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    #[serde(default)]
    pub points: Option<Vec<[f32; 2]>>,
    /// square | rect | circle | ellipse | triangle | pentagon | hexagon |
    /// octagon | dodecagon | polygon (with `sides`).
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub center: Option<[f32; 2]>,
    /// Circumradius (regular polygons, circles).
    #[serde(default)]
    pub radius: Option<f32>,
    /// Width across flats (regular polygons) or side of a square.
    #[serde(default)]
    pub width: Option<f32>,
    /// [width along x, depth along z] for rect / ellipse.
    #[serde(default)]
    pub size: Option<[f32; 2]>,
    #[serde(default)]
    pub sides: Option<u32>,
    /// Degrees counter-clockwise seen from above.
    #[serde(default)]
    pub rotation: f32,
    #[serde(default)]
    pub holes: Vec<PolySpec>,
    /// Keep the source's inner rings (courtyards). Default true.
    #[serde(default = "yes")]
    pub keep_holes: bool,
    /// Shrink by metres (negative grows).
    #[serde(default)]
    pub inset: Option<f32>,
    /// Grow by metres (negative shrinks).
    #[serde(default)]
    pub offset: Option<f32>,
    /// Douglas-Peucker tolerance in metres.
    #[serde(default)]
    pub simplify: Option<f32>,
    /// Add these polygons (after inset/offset, holes and chamfer).
    #[serde(default)]
    pub union: Vec<PolySpec>,
    /// Remove these polygons (after `union`).
    #[serde(default)]
    pub subtract: Vec<PolySpec>,
    /// Keep only the part inside this polygon.
    #[serde(default)]
    pub clip: Option<PolySpec>,
    #[serde(default)]
    pub translate: Option<[f32; 2]>,
    /// Degrees counter-clockwise seen from above, about `about` or the centroid.
    #[serde(default)]
    pub rotate: Option<f32>,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub about: Option<[f32; 2]>,
    /// Replace by the convex hull.
    #[serde(default)]
    pub hull: bool,
    /// Replace by the minimum-area enclosing rectangle.
    #[serde(default)]
    pub rect: bool,
    /// Cut the corners of the result by this distance.
    #[serde(default)]
    pub chamfer: Option<f32>,
    /// Pick one ring of a multi-polygon reference (e.g. "outline:all").
    #[serde(default)]
    pub index: Option<usize>,
}

impl<'de> Deserialize<'de> for PolySpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = PolySpec;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a polygon: [[x, z], ...], a reference string like \"outline:0\", or a table")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<PolySpec, E> {
                Ok(PolySpec::Ref(s.to_string()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<PolySpec, A::Error> {
                let pts = Vec::<[f32; 2]>::deserialize(de::value::SeqAccessDeserializer::new(a))?;
                if pts.len() < 3 {
                    return Err(de::Error::custom(format!("a polygon needs at least 3 points, got {}", pts.len())));
                }
                Ok(PolySpec::Points(pts))
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<PolySpec, A::Error> {
                Ok(PolySpec::Table(Box::new(PolyTable::deserialize(MapAccessDeserializer::new(m))?)))
            }
        }
        d.deserialize_any(V)
    }
}

// ----------------------------------------------------------- edge selection

/// One token of an edge selection.
#[derive(Clone, Debug, PartialEq)]
pub enum EdgeTok {
    All,
    Outer,
    Inner,
    Index(i64),
    Range(i64, i64),
    Longest(usize),
    Shortest(usize),
    /// Facade normal near a compass azimuth: `None` = the half-open 90
    /// degree sector of a compass word (every edge gets exactly one of
    /// n / e / s / w), `Some(tol)` = within `tol` degrees (inclusive).
    Facing(f32, Option<f32>),
    MinLength(f32),
    Not(Box<EdgeTok>),
}

/// Which edges of a polygon: `"all"`, `"outer"`, `"inner"`, an index, a
/// list, `"longest"`, `"longest:2"`, `"facing:s"`, `"facing:135:30"`,
/// `"min:4"` (at least 4 m), `"2..5"`, `"not:3"`.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeSel(pub Vec<EdgeTok>);

impl Default for EdgeSel {
    fn default() -> Self {
        EdgeSel(vec![EdgeTok::All])
    }
}

pub fn compass_deg(s: &str) -> Option<f32> {
    Some(match s {
        "n" | "north" => 0.0,
        "ne" => 45.0,
        "e" | "east" => 90.0,
        "se" => 135.0,
        "s" | "south" => 180.0,
        "sw" => 225.0,
        "w" | "west" => 270.0,
        "nw" => 315.0,
        o => return o.parse::<f32>().ok(),
    })
}

impl EdgeTok {
    pub fn parse(s: &str) -> Result<EdgeTok, String> {
        let t = s.trim().to_ascii_lowercase();
        if let Some(rest) = t.strip_prefix("not:").or_else(|| t.strip_prefix('!')) {
            return Ok(EdgeTok::Not(Box::new(EdgeTok::parse(rest)?)));
        }
        if let Ok(i) = t.parse::<i64>() {
            return Ok(EdgeTok::Index(i));
        }
        if let Some((a, b)) = t.split_once("..") {
            let (a, b) = (a.trim().parse::<i64>(), b.trim().parse::<i64>());
            if let (Ok(a), Ok(b)) = (a, b) {
                return Ok(EdgeTok::Range(a, b));
            }
        }
        let mut parts = t.split(':');
        let head = parts.next().unwrap_or("");
        let arg = parts.next();
        let arg2 = parts.next();
        let bad = || format!("bad edge selector `{s}`");
        match head {
            "all" => Ok(EdgeTok::All),
            "outer" => Ok(EdgeTok::Outer),
            "inner" | "courtyard" => Ok(EdgeTok::Inner),
            "longest" => Ok(EdgeTok::Longest(arg.map(|a| a.parse().map_err(|_| bad())).transpose()?.unwrap_or(1))),
            "shortest" => Ok(EdgeTok::Shortest(arg.map(|a| a.parse().map_err(|_| bad())).transpose()?.unwrap_or(1))),
            "facing" => {
                let a = arg.ok_or_else(|| format!("`facing` needs a direction: facing:s, facing:135 ({s})"))?;
                let deg = compass_deg(a).ok_or_else(|| format!("bad direction `{a}` in `{s}`; use n, ne, e, se, s, sw, w, nw or degrees"))?;
                let tol = arg2.map(|v| v.parse::<f32>().map_err(|_| bad())).transpose()?;
                Ok(EdgeTok::Facing(deg, tol))
            }
            "min" | "min_length" => Ok(EdgeTok::MinLength(arg.ok_or_else(bad)?.parse().map_err(|_| bad())?)),
            n if compass_deg(n).is_some() && n.parse::<f32>().is_err() => Ok(EdgeTok::Facing(compass_deg(n).unwrap(), None)),
            _ => Err(format!(
                "unknown edge selector `{s}`{}; use all, outer, inner, <index>, a..b, longest[:k], shortest[:k], facing:<dir>[:tol], min:<metres>, not:<sel>",
                super::parse::did_you_mean(head, &["all", "outer", "inner", "longest", "shortest", "facing", "min", "not"])
            )),
        }
    }
}

impl<'de> Deserialize<'de> for EdgeSel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = EdgeSel;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an edge selector: \"all\", an index, \"facing:s\", or a list of them")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<EdgeSel, E> {
                Ok(EdgeSel(vec![EdgeTok::parse(s).map_err(E::custom)?]))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<EdgeSel, E> {
                Ok(EdgeSel(vec![EdgeTok::Index(v)]))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<EdgeSel, E> {
                Ok(EdgeSel(vec![EdgeTok::Index(v as i64)]))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<EdgeSel, A::Error> {
                #[derive(Deserialize)]
                #[serde(untagged)]
                enum One {
                    I(i64),
                    S(String),
                }
                let mut v = Vec::new();
                while let Some(o) = a.next_element::<One>()? {
                    v.push(match o {
                        One::I(i) => EdgeTok::Index(i),
                        One::S(s) => EdgeTok::parse(&s).map_err(de::Error::custom)?,
                    });
                }
                Ok(EdgeSel(v))
            }
        }
        d.deserialize_any(V)
    }
}

// ------------------------------------------------------- levels and bays

/// Opening rows: absolute heights `[4.0, 8.5]`, one number, or
/// `{ start = 4.0, step = 3.6, count = 3 }`.
#[derive(Clone, Debug)]
pub struct Levels(pub Vec<f32>);

impl<'de> Deserialize<'de> for Levels {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct T {
            start: f32,
            #[serde(default)]
            step: f32,
            #[serde(default = "one")]
            count: u32,
        }
        fn one() -> u32 {
            1
        }
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Levels;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("heights [y, ...], a number, or { start, step, count }")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Levels, E> {
                Ok(Levels(vec![v as f32]))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Levels, E> {
                Ok(Levels(vec![v as f32]))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Levels, A::Error> {
                Ok(Levels(Vec::<f32>::deserialize(de::value::SeqAccessDeserializer::new(a))?))
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Levels, A::Error> {
                let t = T::deserialize(MapAccessDeserializer::new(m))?;
                if t.count > 200 {
                    return Err(de::Error::custom("too many levels (max 200)"));
                }
                Ok(Levels((0..t.count).map(|i| t.start + t.step * i as f32).collect()))
            }
        }
        d.deserialize_any(V)
    }
}

/// Opening columns along each edge.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bays {
    /// Number of bays (evenly spread between the margins).
    #[serde(default)]
    pub count: Option<u32>,
    /// Centre-to-centre spacing (as many as fit, centred) or, with
    /// `count`, the spacing of a centred group.
    #[serde(default)]
    pub spacing: Option<f32>,
    /// Clear wall at both ends (metres).
    #[serde(default)]
    pub margin: Option<f32>,
    /// [left, right] margins seen from outside.
    #[serde(default)]
    pub margins: Option<[f32; 2]>,
    /// Explicit centres, metres from the left end seen from outside.
    #[serde(default)]
    pub at: Option<Vec<f32>>,
    /// Explicit centres relative to the facade centre (symmetric facades).
    #[serde(default)]
    pub offsets: Option<Vec<f32>>,
    /// Bay indices to leave solid.
    #[serde(default)]
    pub skip: Vec<i64>,
    /// One opening where the facade faces each compass azimuth (degrees;
    /// curved and round walls), within `azimuth_tolerance` (default 30).
    #[serde(default)]
    pub azimuth: Option<Vec<f32>>,
    #[serde(default)]
    pub azimuth_tolerance: Option<f32>,
}

impl Shorthand for Bays {
    const EXPECTING: &'static str = "a bay count or { count, spacing, margin, margins, at, offsets, skip, azimuth }";
    fn from_num(v: f64) -> Result<Self, String> {
        if v < 0.0 || v > 500.0 || v.fract() != 0.0 {
            return Err(format!("bay count must be a whole number 0..500, got {v}"));
        }
        Ok(Bays { count: Some(v as u32), ..Default::default() })
    }
}

// ---------------------------------------------------------------- openings

str_enum!(
    /// What kind of opening.
    OpeningKind, "opening kind", { Window => ["window"], Door => ["door"], Portal => ["portal"], Arch => ["arch"], Oculus => ["oculus"], Rose => ["rose"], Lancet => ["lancet"], Twin => ["twin", "ajimez"], Niche => ["niche"], Blind => ["blind"], Slit => ["slit"], Passage => ["passage", "gate", "tunnel"] }
);

str_enum!(
    /// Head shape of an opening.
    ArchKind, "arch shape", { Flat => ["flat"], Round => ["round", "semicircular"], Segmental => ["segmental"], Pointed => ["pointed", "equilateral"], Lancet => ["lancet"], Horseshoe => ["horseshoe"], PointedHorseshoe => ["pointed_horseshoe"], Ogee => ["ogee"], Basket => ["basket", "three_centred"], Tudor => ["tudor", "four_centred"], Trefoil => ["trefoil"], Circle => ["circle"], Quatrefoil => ["quatrefoil"], Shouldered => ["shouldered"] }
);

str_enum!(
    /// What fills an opening.
    Infill, "infill", { Glass => ["glass"], Stained => ["stained", "stained_glass"], Door => ["door"], Shutters => ["shutters"], Louvres => ["louvres", "louvers"], Grille => ["grille"], Void => ["void"], Dark => ["dark"], Blind => ["blind"], Tracery => ["tracery"], CurtainWall => ["curtain_wall"] }
);

str_enum!(
    /// Tracery in a window head.
    TraceryKind, "tracery", { Y => ["y"], Circle => ["circle"], Trefoil => ["trefoil"], Quatrefoil => ["quatrefoil"], Geometric => ["geometric"], Mullions => ["mullions"] }
);

/// Flat frame around an opening on the wall face.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurroundSpec {
    #[serde(default = "d_surround_w")]
    pub width: f32,
    #[serde(default = "d_surround_p")]
    pub projection: f32,
    #[serde(default)]
    pub material: Option<String>,
    /// Ears (crossettes) at the top corners.
    #[serde(default)]
    pub ears: bool,
    /// Pediment over the surround: triangle | segmental | broken |
    /// broken_segmental.
    #[serde(default, deserialize_with = "de_pediment_opt")]
    pub pediment: Option<String>,
}
fn d_surround_w() -> f32 {
    0.18
}
fn d_surround_p() -> f32 {
    0.06
}
impl Shorthand for SurroundSpec {
    const EXPECTING: &'static str = "true, a width, or { width, projection, material, ears, pediment }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(SurroundSpec { width: d_surround_w(), projection: d_surround_p(), material: None, ears: false, pediment: None })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(SurroundSpec { width: v as f32, ..Self::from_bool(true)? })
    }
}

/// Moulding following the arch above an opening (dripstone / label).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoodSpec {
    #[serde(default = "d_hood")]
    pub size: f32,
    #[serde(default)]
    pub material: Option<String>,
    /// Gap between the opening and the hood.
    #[serde(default = "d_hood_gap")]
    pub gap: f32,
    /// Label stops: horizontal returns at the springing.
    #[serde(default = "yes")]
    pub stops: bool,
}
fn d_hood() -> f32 {
    0.14
}
fn d_hood_gap() -> f32 {
    0.05
}
impl Shorthand for HoodSpec {
    const EXPECTING: &'static str = "true, a size, or { size, gap, material, stops }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(HoodSpec { size: d_hood(), material: None, gap: d_hood_gap(), stops: true })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(HoodSpec { size: v as f32, ..Self::from_bool(true)? })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SillSpec {
    #[serde(default = "d_sill_d")]
    pub depth: f32,
    #[serde(default = "d_sill_h")]
    pub height: f32,
    /// Extra length beyond each side of the opening.
    #[serde(default = "d_sill_e")]
    pub extend: f32,
    #[serde(default)]
    pub material: Option<String>,
}
fn d_sill_d() -> f32 {
    0.12
}
fn d_sill_h() -> f32 {
    0.08
}
fn d_sill_e() -> f32 {
    0.08
}
impl Shorthand for SillSpec {
    const EXPECTING: &'static str = "true, a depth, or { depth, height, extend, material }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(SillSpec { depth: d_sill_d(), height: d_sill_h(), extend: d_sill_e(), material: None })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(SillSpec { depth: v as f32, ..Self::from_bool(true)? })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BalconySpec {
    #[serde(default = "d_bal_d")]
    pub depth: f32,
    /// Extra width beyond each side of the opening.
    #[serde(default = "d_bal_e")]
    pub extend: f32,
    #[serde(default = "d_bal_t")]
    pub thickness: f32,
    /// iron | stone | balustrade | glass | none
    #[serde(default = "d_bal_r", deserialize_with = "de_railing")]
    pub railing: String,
    #[serde(default = "d_bal_h")]
    pub railing_height: f32,
    #[serde(default = "yes")]
    pub corbels: bool,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub railing_material: Option<String>,
    /// One continuous balcony along the whole edge at this level.
    #[serde(default)]
    pub continuous: bool,
}
fn d_bal_d() -> f32 {
    0.6
}
fn d_bal_e() -> f32 {
    0.25
}
fn d_bal_t() -> f32 {
    0.14
}
fn d_bal_r() -> String {
    "iron".into()
}
fn d_bal_h() -> f32 {
    1.0
}
impl Shorthand for BalconySpec {
    const EXPECTING: &'static str = "true, a depth, or { depth, extend, railing, corbels, continuous, ... }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(BalconySpec { depth: d_bal_d(), extend: d_bal_e(), thickness: d_bal_t(), railing: d_bal_r(), railing_height: d_bal_h(), corbels: true, material: None, railing_material: None, continuous: false })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(BalconySpec { depth: v as f32, ..Self::from_bool(true)? })
    }
}

/// Alternating voussoirs around the arch (e.g. two-colour Mudéjar arches).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoussoirSpec {
    #[serde(default = "d_vous_n")]
    pub count: u32,
    #[serde(default = "d_vous_w")]
    pub width: f32,
    /// Material of every other voussoir.
    #[serde(default)]
    pub alternate: Option<String>,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default = "d_vous_p")]
    pub projection: f32,
}
fn d_vous_n() -> u32 {
    13
}
fn d_vous_w() -> f32 {
    0.45
}
fn d_vous_p() -> f32 {
    0.03
}
impl Shorthand for VoussoirSpec {
    const EXPECTING: &'static str = "true, a count, or { count, width, alternate, material, projection }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(VoussoirSpec { count: d_vous_n(), width: d_vous_w(), alternate: None, material: None, projection: d_vous_p() })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(VoussoirSpec { count: v as u32, ..Self::from_bool(true)? })
    }
}

// ----------------------------------------------------------- mouldings etc.

/// Moulding profile: preset name or `[[out, up], ...]` (metres before
/// scaling by `size`).
#[derive(Clone, Debug)]
pub enum ProfileSpec {
    Named(String),
    Points(Vec<[f32; 2]>),
}

impl<'de> Deserialize<'de> for ProfileSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ProfileSpec;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a profile name or [[out, up], ...]")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<ProfileSpec, E> {
                super::profiles::check_name(s).map_err(E::custom)?;
                Ok(ProfileSpec::Named(s.to_string()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<ProfileSpec, A::Error> {
                let v = Vec::<[f32; 2]>::deserialize(de::value::SeqAccessDeserializer::new(a))?;
                if v.len() < 2 {
                    return Err(de::Error::custom("a profile needs at least 2 points"));
                }
                Ok(ProfileSpec::Points(v))
            }
        }
        d.deserialize_any(V)
    }
}

/// Brackets under a cornice.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModillionSpec {
    #[serde(default = "d_mod_s")]
    pub spacing: f32,
    #[serde(default = "d_mod_w")]
    pub width: f32,
    #[serde(default = "d_mod_h")]
    pub height: f32,
    /// Projection from the wall (default: most of the cornice's).
    #[serde(default)]
    pub depth: Option<f32>,
    /// bracket | block | corbel | console
    #[serde(default = "d_mod_k", deserialize_with = "de_modillion")]
    pub style: String,
}
fn d_mod_s() -> f32 {
    0.9
}
fn d_mod_w() -> f32 {
    0.16
}
fn d_mod_h() -> f32 {
    0.3
}
fn d_mod_k() -> String {
    "bracket".into()
}
impl Shorthand for ModillionSpec {
    const EXPECTING: &'static str = "true, a spacing, or { spacing, width, height, depth, style }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(ModillionSpec { spacing: d_mod_s(), width: d_mod_w(), height: d_mod_h(), depth: None, style: d_mod_k() })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(ModillionSpec { spacing: v as f32, ..Self::from_bool(true)? })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DentilSpec {
    #[serde(default = "d_den_s")]
    pub spacing: f32,
    #[serde(default = "d_den_w")]
    pub width: f32,
    #[serde(default = "d_den_h")]
    pub height: f32,
    #[serde(default = "d_den_d")]
    pub depth: f32,
}
fn d_den_s() -> f32 {
    0.18
}
fn d_den_w() -> f32 {
    0.1
}
fn d_den_h() -> f32 {
    0.12
}
fn d_den_d() -> f32 {
    0.1
}
impl Shorthand for DentilSpec {
    const EXPECTING: &'static str = "true or { spacing, width, height, depth }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(DentilSpec { spacing: d_den_s(), width: d_den_w(), height: d_den_h(), depth: d_den_d() })
    }
}

/// Inline moulding on a mass/stage: `cornice = "classical"` or a table.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MouldingSpec {
    #[serde(default)]
    pub profile: Option<ProfileSpec>,
    #[serde(default)]
    pub size: Option<f32>,
    #[serde(default)]
    pub projection: Option<f32>,
    /// Height of the moulding's base (default: top for cornices, base for plinths).
    #[serde(default)]
    pub y: Option<f32>,
    #[serde(default)]
    pub edges: Option<EdgeSel>,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub modillions: Flex<ModillionSpec>,
    #[serde(default)]
    pub dentils: Flex<DentilSpec>,
}
impl Shorthand for MouldingSpec {
    const EXPECTING: &'static str = "true, a profile name, a size, or { profile, size, y, edges, material, modillions, dentils }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(MouldingSpec { profile: None, size: None, projection: None, y: None, edges: None, material: None, modillions: Flex(None), dentils: Flex(None) })
    }
    fn from_str(s: &str) -> Result<Self, String> {
        super::profiles::check_name(s)?;
        Ok(MouldingSpec { profile: Some(ProfileSpec::Named(s.into())), ..Self::from_bool(true)? })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(MouldingSpec { size: Some(v as f32), ..Self::from_bool(true)? })
    }
}

str_enum!(
    /// Merlon outline.
    MerlonKind, "merlon shape", { Rect => ["rect", "rectangular"], Swallowtail => ["swallowtail", "ghibelline"], Stepped => ["stepped"], Pyramidal => ["pyramidal", "pointed"], Rounded => ["rounded"] }
);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParapetSpec {
    #[serde(default = "d_par_h")]
    pub height: f32,
    #[serde(default = "d_par_t")]
    pub thickness: f32,
    #[serde(default = "yes")]
    pub coping: bool,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub edges: Option<EdgeSel>,
}
fn d_par_h() -> f32 {
    1.0
}
fn d_par_t() -> f32 {
    0.3
}
impl Shorthand for ParapetSpec {
    const EXPECTING: &'static str = "true, a height, or { height, thickness, coping, material, edges }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(ParapetSpec { height: d_par_h(), thickness: d_par_t(), coping: true, material: None, edges: None })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(ParapetSpec { height: v as f32, ..Self::from_bool(true)? })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrenelSpec {
    #[serde(default = "d_mer_k")]
    pub shape: MerlonKind,
    #[serde(default = "d_mer_w")]
    pub width: f32,
    #[serde(default = "d_mer_g")]
    pub gap: f32,
    #[serde(default = "d_mer_h")]
    pub height: f32,
    #[serde(default = "d_mer_t")]
    pub thickness: f32,
    /// Solid parapet wall under the merlons.
    #[serde(default = "d_mer_wall")]
    pub wall: f32,
    #[serde(default)]
    pub loopholes: bool,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub edges: Option<EdgeSel>,
}
fn d_mer_k() -> MerlonKind {
    MerlonKind::Rect
}
fn d_mer_w() -> f32 {
    0.8
}
fn d_mer_g() -> f32 {
    0.6
}
fn d_mer_h() -> f32 {
    1.0
}
fn d_mer_t() -> f32 {
    0.5
}
fn d_mer_wall() -> f32 {
    0.9
}
impl Shorthand for CrenelSpec {
    const EXPECTING: &'static str = "true, a merlon shape name, or { shape, width, gap, height, thickness, wall, loopholes }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(CrenelSpec { shape: d_mer_k(), width: d_mer_w(), gap: d_mer_g(), height: d_mer_h(), thickness: d_mer_t(), wall: d_mer_wall(), loopholes: false, material: None, edges: None })
    }
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(CrenelSpec { shape: MerlonKind::parse(s)?, ..Self::from_bool(true)? })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BalustradeSpec {
    #[serde(default = "d_bal_height")]
    pub height: f32,
    /// Baluster spacing.
    #[serde(default = "d_bal_sp")]
    pub spacing: f32,
    /// Pedestal spacing (0 = only at corners and ends).
    #[serde(default = "d_bal_ped")]
    pub pedestals: f32,
    /// urn | ball | pinecone | none
    #[serde(default = "d_bal_fin", deserialize_with = "de_finials")]
    pub finials: String,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub edges: Option<EdgeSel>,
}
fn d_bal_height() -> f32 {
    0.95
}
fn d_bal_sp() -> f32 {
    0.27
}
fn d_bal_ped() -> f32 {
    3.6
}
fn d_bal_fin() -> String {
    "none".into()
}
impl Shorthand for BalustradeSpec {
    const EXPECTING: &'static str = "true, a height, or { height, spacing, pedestals, finials, material, edges }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(BalustradeSpec { height: d_bal_height(), spacing: d_bal_sp(), pedestals: d_bal_ped(), finials: d_bal_fin(), material: None, edges: None })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(BalustradeSpec { height: v as f32, ..Self::from_bool(true)? })
    }
}

// ------------------------------------------------------------------- roofs

str_enum!(
    /// Roof shape.
    RoofShape, "roof shape", { Flat => ["flat"], Gable => ["gable"], Hip => ["hip", "hipped"], Pyramid => ["pyramid"], Mansard => ["mansard"], Shed => ["shed", "lean_to"], Barrel => ["barrel", "vault"], Cone => ["cone", "conical"], Spire => ["spire"], Tent => ["tent"], Dome => ["dome"], None => ["none"] }
);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RibSpec {
    #[serde(default = "d_rib_n")]
    pub count: u32,
    #[serde(default = "d_rib_w")]
    pub width: f32,
    #[serde(default = "d_rib_d")]
    pub depth: f32,
    #[serde(default)]
    pub material: Option<String>,
}
fn d_rib_n() -> u32 {
    8
}
fn d_rib_w() -> f32 {
    0.25
}
fn d_rib_d() -> f32 {
    0.15
}
impl Shorthand for RibSpec {
    const EXPECTING: &'static str = "true, a rib count, or { count, width, depth, material }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(RibSpec { count: d_rib_n(), width: d_rib_w(), depth: d_rib_d(), material: None })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(RibSpec { count: v as u32, ..Self::from_bool(true)? })
    }
}

/// Inline roof on a mass or a tower top.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoofSpec {
    pub shape: RoofShape,
    #[serde(default)]
    pub pitch: Option<f32>,
    #[serde(default)]
    pub height: Option<f32>,
    #[serde(default)]
    pub ridge: Option<RidgeDir>,
    #[serde(default)]
    pub overhang: Option<f32>,
    #[serde(default)]
    pub thickness: Option<f32>,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub wall_material: Option<String>,
    #[serde(default)]
    pub ribs: Flex<RibSpec>,
    #[serde(default)]
    pub ridge_cap: Option<bool>,
    #[serde(default)]
    pub mansard: Option<MansardSpec>,
    #[serde(default)]
    pub y: Option<f32>,
}
impl Shorthand for RoofSpec {
    const EXPECTING: &'static str = "a roof shape name or { shape, pitch, height, ridge, overhang, ... }";
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(RoofSpec { shape: RoofShape::parse(s)?, pitch: None, height: None, ridge: None, overhang: None, thickness: None, material: None, wall_material: None, ribs: Flex(None), ridge_cap: None, mansard: None, y: None })
    }
}

/// Ridge direction: "long" (along the longer side), "short", or a
/// compass azimuth in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RidgeDir {
    Long,
    Short,
    Azimuth(f32),
}

impl<'de> Deserialize<'de> for RidgeDir {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = RidgeDir;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("\"long\", \"short\", a compass direction or degrees")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<RidgeDir, E> {
                match s.trim().to_ascii_lowercase().as_str() {
                    "long" => Ok(RidgeDir::Long),
                    "short" => Ok(RidgeDir::Short),
                    "ns" | "sn" => Ok(RidgeDir::Azimuth(0.0)),
                    "ew" | "we" => Ok(RidgeDir::Azimuth(90.0)),
                    o => compass_deg(o).map(RidgeDir::Azimuth).ok_or_else(|| E::custom(format!("bad ridge direction `{s}`; use long, short, ns, ew or degrees"))),
                }
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<RidgeDir, E> {
                Ok(RidgeDir::Azimuth(v as f32))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<RidgeDir, E> {
                Ok(RidgeDir::Azimuth(v as f32))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MansardSpec {
    /// Height of the steep lower slope.
    #[serde(default = "d_mans_h")]
    pub height: f32,
    /// Horizontal inset at the break.
    #[serde(default = "d_mans_i")]
    pub inset: f32,
    /// Upper part: flat | hip | pyramid
    #[serde(default = "d_mans_u", deserialize_with = "de_mansard")]
    pub upper: String,
    /// Pitch of the upper part (degrees).
    #[serde(default = "d_mans_p")]
    pub upper_pitch: f32,
}
fn d_mans_h() -> f32 {
    3.0
}
fn d_mans_i() -> f32 {
    1.0
}
fn d_mans_u() -> String {
    "hip".into()
}
fn d_mans_p() -> f32 {
    15.0
}

// ------------------------------------------------------------------ domes

str_enum!(
    /// Dome profile.
    DomeProfile, "dome profile", { Hemisphere => ["hemisphere", "half"], Pointed => ["pointed"], Onion => ["onion"], Ellipsoid => ["ellipsoid"], Segmental => ["segmental", "saucer"], Bell => ["bell"], Parabolic => ["parabolic"], Ribbed => ["ribbed"] }
);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrumSpec {
    #[serde(default = "d_drum_h")]
    pub height: f32,
    /// 0 = round; otherwise number of faces.
    #[serde(default)]
    pub sides: u32,
    /// Default: dome radius plus 0.3 m.
    #[serde(default)]
    pub radius: Option<f32>,
    #[serde(default)]
    pub material: Option<String>,
    /// Window count (one per face on polygonal drums).
    #[serde(default)]
    pub windows: u32,
    #[serde(default = "d_drum_ww")]
    pub window_width: f32,
    #[serde(default = "d_drum_wh")]
    pub window_height: f32,
    #[serde(default = "d_drum_arch")]
    pub window_arch: ArchKind,
    #[serde(default = "d_drum_inf")]
    pub window_infill: Infill,
    #[serde(default)]
    pub cornice: Flex<MouldingSpec>,
    /// Pilasters or buttresses between windows.
    #[serde(default)]
    pub pilasters: bool,
}
fn d_drum_h() -> f32 {
    3.0
}
fn d_drum_ww() -> f32 {
    1.0
}
fn d_drum_wh() -> f32 {
    1.8
}
fn d_drum_arch() -> ArchKind {
    ArchKind::Round
}
fn d_drum_inf() -> Infill {
    Infill::Glass
}
impl Shorthand for DrumSpec {
    const EXPECTING: &'static str = "a drum height or { height, sides, radius, windows, cornice, ... }";
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(DrumSpec {
            height: v as f32,
            sides: 0,
            radius: None,
            material: None,
            windows: 0,
            window_width: d_drum_ww(),
            window_height: d_drum_wh(),
            window_arch: d_drum_arch(),
            window_infill: d_drum_inf(),
            cornice: Flex(None),
            pilasters: false,
        })
    }
}

// ------------------------------------------------------ columns & supports

str_enum!(
    /// Column order.
    ColumnOrder, "column order", { Plain => ["plain"], Tuscan => ["tuscan"], Doric => ["doric"], Ionic => ["ionic"], Corinthian => ["corinthian", "composite"], Gothic => ["gothic"], Salomonic => ["salomonic", "solomonic"], Pier => ["pier"], Octagonal => ["octagonal"], Cylinder => ["cylinder", "modern"] }
);

str_enum!(
    /// Finial style.
    FinialStyle, "finial style", { Cross => ["cross"], Ball => ["ball"], Spike => ["spike"], Weathervane => ["weathervane"], Urn => ["urn"], Pinecone => ["pinecone"], Fleur => ["fleur"], Crown => ["crown"], Star => ["star"], Statue => ["statue"] }
);

str_enum!(
    /// Statue placeholder style.
    StatueStyle, "statue style", { Figure => ["figure"], Bust => ["bust"], Equestrian => ["equestrian"], Angel => ["angel"], Seated => ["seated"], Group => ["group"] }
);

/// Pedestal under a statue.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PedestalSpec {
    #[serde(default = "d_ped_h")]
    pub height: f32,
    #[serde(default = "d_ped_w")]
    pub width: f32,
    #[serde(default)]
    pub material: Option<String>,
}
fn d_ped_h() -> f32 {
    1.2
}
fn d_ped_w() -> f32 {
    0.9
}
impl Shorthand for PedestalSpec {
    const EXPECTING: &'static str = "true, a height, or { height, width, material }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(PedestalSpec { height: d_ped_h(), width: d_ped_w(), material: None })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(PedestalSpec { height: v as f32, ..Self::from_bool(true)? })
    }
}

/// Column used by arcades.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnSpec {
    #[serde(default = "d_col_r")]
    pub radius: f32,
    #[serde(default = "d_col_o")]
    pub order: ColumnOrder,
    #[serde(default)]
    pub flutes: u32,
    #[serde(default)]
    pub material: Option<String>,
    /// Pair of columns per pier (front and back).
    #[serde(default)]
    pub double: bool,
}
fn d_col_r() -> f32 {
    0.3
}
fn d_col_o() -> ColumnOrder {
    ColumnOrder::Tuscan
}
impl Shorthand for ColumnSpec {
    const EXPECTING: &'static str = "an order name, a radius, or { radius, order, flutes, material, double }";
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(ColumnSpec { radius: d_col_r(), order: ColumnOrder::parse(s)?, flutes: 0, material: None, double: false })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(ColumnSpec { radius: v as f32, order: d_col_o(), flutes: 0, material: None, double: false })
    }
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(ColumnSpec { radius: d_col_r(), order: d_col_o(), flutes: 0, material: None, double: false })
    }
}

/// Rectangular pier.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PierSpec {
    #[serde(default = "d_pier")]
    pub width: f32,
    #[serde(default)]
    pub depth: Option<f32>,
    #[serde(default)]
    pub material: Option<String>,
}
fn d_pier() -> f32 {
    0.8
}
impl Shorthand for PierSpec {
    const EXPECTING: &'static str = "true, a width, or { width, depth, material }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(PierSpec { width: d_pier(), depth: None, material: None })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(PierSpec { width: v as f32, depth: None, material: None })
    }
}

/// Buttress set-off.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepSpec {
    pub height: f32,
    pub depth: f32,
    #[serde(default)]
    pub width: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnacleSpec {
    #[serde(default = "d_pin_w")]
    pub width: f32,
    #[serde(default = "d_pin_h")]
    pub height: f32,
    #[serde(default = "d_pin_s")]
    pub sides: u32,
    #[serde(default = "d_pin_c")]
    pub crockets: u32,
    #[serde(default)]
    pub material: Option<String>,
}
fn d_pin_w() -> f32 {
    0.7
}
fn d_pin_h() -> f32 {
    3.5
}
fn d_pin_s() -> u32 {
    4
}
fn d_pin_c() -> u32 {
    3
}
impl Shorthand for PinnacleSpec {
    const EXPECTING: &'static str = "true, a height, or { width, height, sides, crockets, material }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(PinnacleSpec { width: d_pin_w(), height: d_pin_h(), sides: d_pin_s(), crockets: d_pin_c(), material: None })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(PinnacleSpec { height: v as f32, ..Self::from_bool(true)? })
    }
}

// ------------------------------------------------------- generic geometry

/// 2D section for sweeps and lofts: points `[[a, b], ...]` (a = side,
/// b = up) or a table `{ shape = "ellipse" | "rect" | "superellipse" |
/// "circle" | "polygon", size = [w, h], exponent, sides, rotate,
/// points }`.
#[derive(Clone, Debug)]
pub struct Section {
    pub points: Option<Vec<[f32; 2]>>,
    pub shape: Option<String>,
    pub size: [f32; 2],
    pub exponent: f32,
    pub sides: u32,
    pub rotate: f32,
    pub open: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SectionTable {
    #[serde(default)]
    points: Option<Vec<[f32; 2]>>,
    #[serde(default)]
    shape: Option<String>,
    #[serde(default)]
    size: Option<[f32; 2]>,
    #[serde(default)]
    radius: Option<f32>,
    #[serde(default)]
    exponent: Option<f32>,
    #[serde(default)]
    sides: Option<u32>,
    #[serde(default)]
    rotate: f32,
    #[serde(default)]
    open: bool,
}

impl<'de> Deserialize<'de> for Section {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Section;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a section: [[a, b], ...] or { shape, size, exponent, sides, rotate, points }")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Section, A::Error> {
                let v = Vec::<[f32; 2]>::deserialize(de::value::SeqAccessDeserializer::new(a))?;
                if v.len() < 2 {
                    return Err(de::Error::custom("a section needs at least 2 points"));
                }
                Ok(Section { points: Some(v), shape: None, size: [1.0, 1.0], exponent: 2.0, sides: 0, rotate: 0.0, open: false })
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Section, E> {
                super::shapes::check_section_shape(s).map_err(E::custom)?;
                Ok(Section { points: None, shape: Some(s.into()), size: [1.0, 1.0], exponent: 2.0, sides: 0, rotate: 0.0, open: false })
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Section, A::Error> {
                let t = SectionTable::deserialize(MapAccessDeserializer::new(m))?;
                if let Some(s) = &t.shape {
                    super::shapes::check_section_shape(s).map_err(de::Error::custom)?;
                }
                let size = t.size.unwrap_or(match t.radius {
                    Some(r) => [2.0 * r, 2.0 * r],
                    None => [1.0, 1.0],
                });
                Ok(Section { points: t.points, shape: t.shape, size, exponent: t.exponent.unwrap_or(2.0), sides: t.sides.unwrap_or(0), rotate: t.rotate, open: t.open })
            }
        }
        d.deserialize_any(V)
    }
}

/// Path for sweeps and path arrays.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathSpec {
    /// Polyline points [[x, y, z], ...].
    #[serde(default)]
    pub points: Option<Vec<[f32; 3]>>,
    /// Smooth Catmull-Rom curve through these points.
    #[serde(default)]
    pub curve: Option<Vec<[f32; 3]>>,
    /// Cubic Bezier control points (3k + 1).
    #[serde(default)]
    pub bezier: Option<Vec<[f32; 3]>>,
    /// Circular arc / circle: centre.
    #[serde(default)]
    pub center: Option<[f32; 3]>,
    #[serde(default)]
    pub radius: Option<f32>,
    /// Arc start and end angles in degrees, measured in the arc plane from
    /// its first axis (default: full circle).
    #[serde(default)]
    pub angles: Option<[f32; 2]>,
    /// Arc plane normal (default +y: horizontal circle).
    #[serde(default)]
    pub normal: Option<[f32; 3]>,
    /// First in-plane axis of the arc (default: +x, or +y for vertical arcs).
    #[serde(default)]
    pub axis: Option<[f32; 3]>,
    /// Samples for curves (default from length).
    #[serde(default)]
    pub samples: Option<u32>,
    #[serde(default)]
    pub closed: bool,
}

str_enum!(
    /// Parametric shell surfaces.
    SurfaceKind, "surface", { Ellipsoid => ["ellipsoid", "sphere"], Superellipsoid => ["superellipsoid"], Paraboloid => ["paraboloid"], Hypar => ["hypar", "bilinear"], Cylinder => ["cylinder"], Torus => ["torus"], Cone => ["cone"] }
);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellRibs {
    #[serde(default = "d_srib_n")]
    pub count: u32,
    /// "u" (along u at constant v) or "v".
    #[serde(default = "d_srib_dir", deserialize_with = "de_ribdir")]
    pub along: String,
    #[serde(default = "d_srib_w")]
    pub width: f32,
    #[serde(default = "d_srib_d")]
    pub depth: f32,
    #[serde(default)]
    pub material: Option<String>,
    /// Ribs stand on the outer face (default) or the inner face.
    #[serde(default)]
    pub inside: bool,
}
fn d_srib_n() -> u32 {
    12
}
fn d_srib_dir() -> String {
    "v".into()
}
fn d_srib_w() -> f32 {
    0.4
}
fn d_srib_d() -> f32 {
    0.3
}
impl Shorthand for ShellRibs {
    const EXPECTING: &'static str = "true, a count, or { count, along, width, depth, material, inside }";
    fn from_bool(_: bool) -> Result<Self, String> {
        Ok(ShellRibs { count: d_srib_n(), along: d_srib_dir(), width: d_srib_w(), depth: d_srib_d(), material: None, inside: false })
    }
    fn from_num(v: f64) -> Result<Self, String> {
        Ok(ShellRibs { count: v as u32, ..Self::from_bool(true)? })
    }
}

/// One loft station.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoftStation {
    pub at: [f32; 3],
    #[serde(default)]
    pub section: Option<Section>,
    /// Scale of the section [side, up] (multiplies its size).
    #[serde(default)]
    pub scale: Option<[f32; 2]>,
    /// Extra rotation of the section about the path (degrees).
    #[serde(default)]
    pub twist: f32,
    /// Explicit section-plane normal (default: path tangent).
    #[serde(default)]
    pub normal: Option<[f32; 3]>,
}

// --------------------------------------------------------- element types

macro_rules! element {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            /// Label used by `on = "..."`, metrics and error messages.
            #[serde(default)] pub name: Option<String>,
            /// Palette material (default: inherited / `[defaults].material`).
            #[serde(default)] pub material: Option<String>,
            #[serde(default)] pub transform: Option<Transform>,
            #[serde(default)] pub mirror: Option<Mirror>,
            /// Set false to skip the element while iterating.
            #[serde(default = "yes")] pub enabled: bool,
            /// Include in the collision mesh (default depends on the type).
            #[serde(default)] pub collide: Option<bool>,
            /// Free text for modellers (sources, reasoning).
            #[serde(default)] pub note: Option<String>,
            $($(#[$fm])* pub $f: $t),*
        }
        impl HasCommon for $name {
            fn common(&self) -> Common<'_> {
                Common { name: self.name.as_deref(), material: self.material.as_deref(), transform: self.transform.as_ref(), mirror: self.mirror.as_ref(), enabled: self.enabled, collide: self.collide }
            }
        }
    };
}

/// Fields shared by every element.
#[derive(Clone, Copy, Debug)]
pub struct Common<'a> {
    pub name: Option<&'a str>,
    pub material: Option<&'a str>,
    pub transform: Option<&'a Transform>,
    pub mirror: Option<&'a Mirror>,
    pub enabled: bool,
    pub collide: Option<bool>,
}

pub trait HasCommon {
    fn common(&self) -> Common<'_>;
}

fn d_depth() -> f32 {
    0.35
}
fn d_ow() -> f32 {
    1.2
}
fn d_oh() -> f32 {
    2.4
}
fn d_kind() -> OpeningKind {
    OpeningKind::Window
}
fn d_frame() -> f32 {
    0.07
}
fn d_order_step() -> f32 {
    0.22
}

element! {
    /// Openings cut into a mass, stage, wall or gable, laid out in rows
    /// (`levels`) and columns (`bays`) on the selected edges.
    pub struct Openings {
        /// Target element name (standalone `type = "openings"` only).
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub edges: EdgeSel,
        /// Lay bays out per plan edge instead of along whole facade runs.
        #[serde(default)] pub per_edge: bool,
        #[serde(default = "d_kind")] pub kind: OpeningKind,
        #[serde(default)] pub arch: Option<ArchKind>,
        #[serde(default = "d_ow")] pub width: f32,
        /// Total height from the bottom to the apex.
        #[serde(default = "d_oh")] pub height: f32,
        /// Arch rise (default per arch shape).
        #[serde(default)] pub rise: Option<f32>,
        /// Bottom (sill) heights, absolute.
        #[serde(default)] pub levels: Option<Levels>,
        #[serde(default)] pub bays: Flex<Bays>,
        /// [level, bay] pairs to leave solid.
        #[serde(default)] pub skip: Vec<[i64; 2]>,
        /// Reveal depth (wall thickness seen in the opening).
        #[serde(default = "d_depth")] pub depth: f32,
        /// Extra outward widening of the reveal (Romanesque/Gothic splays).
        #[serde(default)] pub splay: f32,
        #[serde(default)] pub infill: Option<Infill>,
        /// Joinery frame width (0 = none).
        #[serde(default = "d_frame")] pub frame: f32,
        #[serde(default)] pub mullions: u32,
        #[serde(default)] pub transoms: u32,
        /// Glazing bars [columns, rows] per light.
        #[serde(default)] pub panes: Option<[u32; 2]>,
        #[serde(default)] pub surround: Flex<SurroundSpec>,
        #[serde(default)] pub hood: Flex<HoodSpec>,
        #[serde(default)] pub sill: Flex<SillSpec>,
        /// Keystone size (true = 0.3 m).
        #[serde(default)] pub keystone: Option<KeystoneSize>,
        /// Recessed archivolt orders (portals).
        #[serde(default)] pub orders: u32,
        #[serde(default = "d_order_step")] pub order_step: f32,
        /// Colonnettes in the jambs of each order.
        #[serde(default)] pub colonnettes: bool,
        /// Stone tympanum in the arch over a door's lintel.
        #[serde(default)] pub tympanum: bool,
        #[serde(default)] pub tracery: Option<TraceryKind>,
        /// Rose windows: radial spokes and concentric rings.
        #[serde(default)] pub spokes: u32,
        #[serde(default)] pub rings: u32,
        #[serde(default)] pub balcony: Flex<BalconySpec>,
        #[serde(default)] pub voussoirs: Flex<VoussoirSpec>,
        /// Bells in belfry openings.
        #[serde(default)] pub bells: bool,
        #[serde(default)] pub glass_material: Option<String>,
        #[serde(default)] pub frame_material: Option<String>,
        #[serde(default)] pub door_material: Option<String>,
        #[serde(default)] pub iron_material: Option<String>,
        /// Material of tracery, mullions and the reveal (default: wall).
        #[serde(default)] pub stone_material: Option<String>,
    }
}

/// Keystone size: `true` or metres.
#[derive(Clone, Copy, Debug)]
pub struct KeystoneSize(pub f32);
impl<'de> Deserialize<'de> for KeystoneSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = KeystoneSize;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("true or a size in metres")
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<KeystoneSize, E> {
                Ok(KeystoneSize(if b { 0.3 } else { 0.0 }))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<KeystoneSize, E> {
                Ok(KeystoneSize(v as f32))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<KeystoneSize, E> {
                Ok(KeystoneSize(v as f32))
            }
        }
        d.deserialize_any(V)
    }
}

element! {
    /// Extruded plan polygon with facades, openings and inline details.
    pub struct Mass {
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub y0: f32,
        #[serde(default)] pub y1: Option<f32>,
        #[serde(default)] pub height: Option<f32>,
        /// Height as floors (x `floor_height`, default 3.2 m).
        #[serde(default)] pub floors: Option<f32>,
        #[serde(default)] pub floor_height: Option<f32>,
        /// Horizontal inset of the top (battered walls).
        #[serde(default)] pub batter: f32,
        #[serde(default = "yes")] pub top: bool,
        #[serde(default)] pub bottom: Option<bool>,
        /// Smooth shading around the walls (round towers, curved facades).
        #[serde(default)] pub smooth: bool,
        /// Edges meeting at less than this turn (degrees, default 25) form
        /// one facade run laid out by arc length; 0 = every edge alone.
        #[serde(default)] pub run_angle: Option<f32>,
        /// Sides left open (no wall), showing the inside: the mass becomes
        /// a shell of `wall_thickness` open on these edges.
        #[serde(default)] pub open_edges: Option<EdgeSel>,
        #[serde(default)] pub wall_thickness: Option<f32>,
        #[serde(default)] pub openings: Vec<Openings>,
        #[serde(default)] pub cornice: Flex<MouldingSpec>,
        #[serde(default)] pub plinth: Flex<MouldingSpec>,
        /// String courses: heights with the "string" profile, or tables.
        #[serde(default)] pub string_courses: Vec<f32>,
        #[serde(default)] pub parapet: Flex<ParapetSpec>,
        #[serde(default)] pub crenellations: Flex<CrenelSpec>,
        #[serde(default)] pub balustrade: Flex<BalustradeSpec>,
        #[serde(default)] pub roof: Flex<RoofSpec>,
        /// Material of the top cap (flat roofs, terraces).
        #[serde(default)] pub top_material: Option<String>,
    }
}

element! {
    /// Horizontal slab (terraces, platforms, floors, paving).
    pub struct Slab {
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub y: f32,
        #[serde(default = "d_slab")] pub thickness: f32,
    }
}
fn d_slab() -> f32 {
    0.3
}

element! {
    /// Free-standing wall of given thickness along a polyline (or polygon
    /// edges), with through openings.
    pub struct Wall {
        #[serde(default)] pub path: Option<Vec<[f32; 2]>>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub y0: f32,
        #[serde(default)] pub y1: Option<f32>,
        #[serde(default)] pub height: Option<f32>,
        #[serde(default = "d_wall_t")] pub thickness: f32,
        /// "center", "left" or "right" of the path (seen along it).
        #[serde(default, deserialize_with = "de_align_opt")] pub align: Option<String>,
        #[serde(default)] pub closed: bool,
        #[serde(default)] pub openings: Vec<Openings>,
        #[serde(default)] pub coping: bool,
    }
}
fn d_wall_t() -> f32 {
    0.6
}

element! {
    /// Moulding swept along polygon edges: `type` = cornice,
    /// string_course, plinth or moulding.
    pub struct Moulding {
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        /// Open polyline [[x, z], ...] instead of polygon edges.
        #[serde(default)] pub path: Option<Vec<[f32; 2]>>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub y: Option<f32>,
        #[serde(default)] pub profile: Option<ProfileSpec>,
        #[serde(default)] pub size: Option<f32>,
        #[serde(default)] pub projection: Option<f32>,
        /// Outward shift of the moulding's back from the polygon edge.
        #[serde(default)] pub offset: f32,
        #[serde(default)] pub modillions: Flex<ModillionSpec>,
        #[serde(default)] pub dentils: Flex<DentilSpec>,
    }
}

element! {
    /// Solid parapet wall along polygon edges.
    pub struct Parapet {
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub y: Option<f32>,
        #[serde(default = "d_par_h")] pub height: f32,
        #[serde(default = "d_par_t")] pub thickness: f32,
        #[serde(default = "yes")] pub coping: bool,
        /// Outward overhang of the outer face.
        #[serde(default)] pub overhang: f32,
    }
}

element! {
    /// Balustrade (balusters, rails, pedestals) along polygon edges.
    pub struct Balustrade {
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub y: Option<f32>,
        #[serde(default = "d_bal_height")] pub height: f32,
        #[serde(default = "d_bal_sp")] pub spacing: f32,
        #[serde(default = "d_bal_ped")] pub pedestals: f32,
        #[serde(default = "d_bal_fin", deserialize_with = "de_finials")] pub finials: String,
        /// Inward distance of the balustrade's centre line from the edge.
        #[serde(default = "d_bal_inset")] pub inset: f32,
    }
}
fn d_bal_inset() -> f32 {
    0.2
}

element! {
    /// Battlements along polygon edges.
    pub struct Crenellations {
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub y: Option<f32>,
        #[serde(default = "d_mer_k")] pub shape: MerlonKind,
        #[serde(default = "d_mer_w")] pub width: f32,
        #[serde(default = "d_mer_g")] pub gap: f32,
        #[serde(default = "d_mer_h")] pub height: f32,
        #[serde(default = "d_mer_t")] pub thickness: f32,
        #[serde(default = "d_mer_wall")] pub wall: f32,
        #[serde(default)] pub loopholes: bool,
        /// Outward overhang (e.g. over machicolations).
        #[serde(default)] pub overhang: f32,
    }
}

element! {
    /// Projecting parapet on stepped corbels (machicolations).
    pub struct Machicolations {
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub y: Option<f32>,
        /// Overhang of the parapet's outer face beyond the wall.
        #[serde(default = "d_mach_p")] pub projection: f32,
        /// Corbel spacing (centre to centre, by arc length).
        #[serde(default = "d_mach_s")] pub spacing: f32,
        /// Total corbel height.
        #[serde(default = "d_mach_h")] pub corbel_height: f32,
        /// Stepped courses per corbel.
        #[serde(default = "d_mach_c")] pub corbel_courses: u32,
        /// Width of each corbel along the wall.
        #[serde(default = "d_mach_w")] pub corbel_width: f32,
        /// Arches spanning between the corbels (Valencian arcuated
        /// machicolation); `false` for plain brackets.
        #[serde(default = "yes")] pub arches: bool,
        /// Arch shape between corbels (default pointed).
        #[serde(default = "d_mach_a")] pub arch: ArchKind,
        /// Parapet on top: height and thickness.
        #[serde(default = "d_mach_ph")] pub parapet: f32,
        #[serde(default = "d_par_t")] pub thickness: f32,
        /// Merlons on the parapet.
        #[serde(default)] pub merlons: Flex<CrenelSpec>,
    }
}
fn d_mach_p() -> f32 {
    0.9
}
fn d_mach_s() -> f32 {
    1.6
}
fn d_mach_h() -> f32 {
    1.5
}
fn d_mach_c() -> u32 {
    4
}
fn d_mach_w() -> f32 {
    0.45
}
fn d_mach_a() -> ArchKind {
    ArchKind::Pointed
}
fn d_mach_ph() -> f32 {
    1.1
}

element! {
    /// Gargoyles projecting along polygon edges.
    pub struct Gargoyles {
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub y: Option<f32>,
        #[serde(default = "d_garg_s")] pub spacing: f32,
        #[serde(default = "d_garg_l")] pub length: f32,
        #[serde(default = "d_garg_z")] pub size: f32,
        /// Also one at each corner (diagonal).
        #[serde(default = "yes")] pub corners: bool,
        /// Downward tilt in degrees.
        #[serde(default = "d_garg_t")] pub tilt: f32,
    }
}
fn d_garg_s() -> f32 {
    6.0
}
fn d_garg_l() -> f32 {
    1.1
}
fn d_garg_z() -> f32 {
    0.28
}
fn d_garg_t() -> f32 {
    12.0
}

element! {
    /// Roof over a polygon.
    pub struct Roof {
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        pub shape: RoofShape,
        /// Eave height (default: top of `on`).
        #[serde(default)] pub y: Option<f32>,
        /// Slope in degrees.
        #[serde(default)] pub pitch: Option<f32>,
        /// Rise from eaves to ridge/apex (overrides pitch).
        #[serde(default)] pub height: Option<f32>,
        #[serde(default)] pub ridge: Option<RidgeDir>,
        #[serde(default)] pub overhang: Option<f32>,
        #[serde(default)] pub thickness: Option<f32>,
        #[serde(default)] pub ribs: Flex<RibSpec>,
        /// Half-round tiles along ridges and hips (default on for tile roofs).
        #[serde(default)] pub ridge_cap: Option<bool>,
        /// Gable/end walls material (default: the element default).
        #[serde(default)] pub wall_material: Option<String>,
        /// Openings on gable walls (edge indices of the polygon).
        #[serde(default)] pub gable_openings: Vec<Openings>,
        #[serde(default)] pub mansard: Option<MansardSpec>,
        /// Barrel vault section: round | pointed | segmental.
        #[serde(default)] pub vault: Option<ArchKind>,
        #[serde(default)] pub segments: Option<u32>,
    }
}

element! {
    /// Dome with optional drum, ribs, lantern and finial.
    pub struct Dome {
        pub center: [f32; 2],
        /// Base height (bottom of the drum, or springing without drum).
        #[serde(default)] pub y: f32,
        #[serde(default)] pub radius: Option<f32>,
        #[serde(default)] pub rx: Option<f32>,
        #[serde(default)] pub rz: Option<f32>,
        /// Rise of the shell above the springing.
        #[serde(default)] pub height: Option<f32>,
        #[serde(default = "d_dome_p")] pub profile: DomeProfile,
        /// Pointed / onion sharpness (0..1).
        #[serde(default)] pub pointedness: Option<f32>,
        #[serde(default)] pub ribs: Flex<RibSpec>,
        /// Melon gores: scalloped segments.
        #[serde(default)] pub gores: u32,
        #[serde(default = "d_gore_d")] pub gore_depth: f32,
        #[serde(default)] pub segments: Option<u32>,
        #[serde(default)] pub drum: Flex<DrumSpec>,
        /// Open eye at the top (radius).
        #[serde(default)] pub oculus: Option<f32>,
        /// Ring moulding at the springing.
        #[serde(default)] pub base_ring: bool,
        /// Elements on top (origin at the apex).
        #[serde(default)] pub lantern: ElementList,
        /// Elements on top of the lantern (or the dome).
        #[serde(default)] pub finial: ElementList,
        /// Degrees: rotates ribs, drum faces and windows.
        #[serde(default)] pub rotation: f32,
        #[serde(default)] pub drum_material: Option<String>,
    }
}
fn d_dome_p() -> DomeProfile {
    DomeProfile::Hemisphere
}
fn d_gore_d() -> f32 {
    0.06
}

/// One tower stage.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    #[serde(default)]
    pub name: Option<String>,
    pub height: f32,
    /// Horizontal inset from the previous stage.
    #[serde(default)]
    pub inset: f32,
    /// New plan (shapes without `center` are centred on the tower).
    #[serde(default)]
    pub polygon: Option<PolySpec>,
    #[serde(default)]
    pub batter: f32,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub smooth: bool,
    #[serde(default)]
    pub openings: Vec<Openings>,
    #[serde(default)]
    pub cornice: Flex<MouldingSpec>,
    #[serde(default)]
    pub plinth: Flex<MouldingSpec>,
    #[serde(default)]
    pub string_courses: Vec<f32>,
    #[serde(default)]
    pub parapet: Flex<ParapetSpec>,
    #[serde(default)]
    pub crenellations: Flex<CrenelSpec>,
    #[serde(default)]
    pub balustrade: Flex<BalustradeSpec>,
    /// Shortcut: arched belfry openings on every face, with bells.
    #[serde(default)]
    pub belfry: bool,
    /// Corner pinnacles on top of the stage.
    #[serde(default)]
    pub pinnacles: Flex<PinnacleSpec>,
}

element! {
    /// Tower: stacked stages and a top.
    pub struct Tower {
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub y: f32,
        /// Smooth shading for every stage (round towers).
        #[serde(default)] pub smooth: bool,
        #[serde(default)] pub run_angle: Option<f32>,
        pub stages: Vec<Stage>,
        /// Elements on top (origin at the top centre; polygon "parent" = last stage).
        #[serde(default)] pub top: ElementList,
    }
}

element! {
    /// Column with base and capital.
    pub struct Column {
        #[serde(default)] pub at: [f32; 3],
        pub height: f32,
        #[serde(default = "d_col_r")] pub radius: f32,
        #[serde(default = "d_col_o")] pub order: ColumnOrder,
        /// Swelling of the shaft (fraction of the radius).
        #[serde(default)] pub entasis: Option<f32>,
        #[serde(default)] pub flutes: u32,
        #[serde(default = "yes")] pub base: bool,
        #[serde(default = "yes")] pub capital: bool,
        #[serde(default)] pub pedestal: Option<f32>,
        #[serde(default)] pub segments: Option<u32>,
    }
}

element! {
    /// Flat pilaster against a wall (local frame: x along the wall, z out).
    pub struct Pilaster {
        #[serde(default)] pub at: [f32; 3],
        /// Direction the pilaster faces (n, s, e, w or degrees).
        #[serde(default)] pub facing: Option<Facing>,
        pub height: f32,
        #[serde(default = "d_pil_w")] pub width: f32,
        #[serde(default = "d_pil_d")] pub depth: f32,
        #[serde(default = "d_col_o")] pub order: ColumnOrder,
        #[serde(default)] pub flutes: u32,
        #[serde(default = "yes")] pub base: bool,
        #[serde(default = "yes")] pub capital: bool,
    }
}
fn d_pil_w() -> f32 {
    0.6
}
fn d_pil_d() -> f32 {
    0.15
}

element! {
    /// Buttress projecting from a wall.
    pub struct Buttress {
        #[serde(default)] pub at: [f32; 3],
        #[serde(default)] pub facing: Option<Facing>,
        pub height: f32,
        #[serde(default = "d_but_w")] pub width: f32,
        #[serde(default = "d_but_d")] pub depth: f32,
        /// Number of set-offs (each reduces the depth with a sloped weathering).
        #[serde(default)] pub setoffs: u32,
        /// Explicit stages from the bottom.
        #[serde(default)] pub steps: Vec<StepSpec>,
        /// slope | gable | flat | pinnacle
        #[serde(default = "d_but_cap", deserialize_with = "de_bcap")] pub cap: String,
        #[serde(default)] pub pinnacle: Flex<PinnacleSpec>,
        /// Plinth at the foot.
        #[serde(default)] pub plinth: bool,
    }
}
fn d_but_w() -> f32 {
    1.2
}
fn d_but_d() -> f32 {
    1.8
}
fn d_but_cap() -> String {
    "slope".into()
}

element! {
    /// Flying buttress: inclined half arch from an outer pier to the wall.
    pub struct FlyingButtress {
        /// Top of the outer pier (where the arch starts).
        pub from: [f32; 3],
        /// Point on the wall where the arch lands (its top).
        pub to: [f32; 3],
        #[serde(default = "d_fb_w")] pub width: f32,
        /// Depth of the arch rib (vertical thickness).
        #[serde(default = "d_fb_t")] pub thickness: f32,
        /// Sag of the arch underside (fraction of the span).
        #[serde(default = "d_fb_s")] pub sag: f32,
        /// Build the outer pier down to the ground.
        #[serde(default)] pub pier: Flex<PierSpec>,
        #[serde(default)] pub pinnacle: Flex<PinnacleSpec>,
    }
}
fn d_fb_w() -> f32 {
    0.7
}
fn d_fb_t() -> f32 {
    0.8
}
fn d_fb_s() -> f32 {
    0.35
}

element! {
    /// Gothic pinnacle.
    pub struct Pinnacle {
        #[serde(default)] pub at: [f32; 3],
        #[serde(default = "d_pin_w")] pub width: f32,
        #[serde(default = "d_pin_h")] pub height: f32,
        #[serde(default = "d_pin_s")] pub sides: u32,
        #[serde(default = "d_pin_c")] pub crockets: u32,
        /// Fraction of the height that is the shaft (the rest is the spire).
        #[serde(default = "d_pin_shaft")] pub shaft: f32,
    }
}
fn d_pin_shaft() -> f32 {
    0.45
}

element! {
    /// Finial: cross, ball, spike, weathervane, urn, pinecone, fleur, crown, star, statue.
    pub struct Finial {
        #[serde(default)] pub at: [f32; 3],
        #[serde(default = "d_fin_s")] pub style: FinialStyle,
        /// Overall height.
        #[serde(default = "d_fin_h")] pub size: f32,
        #[serde(default)] pub facing: Option<Facing>,
    }
}
fn d_fin_s() -> FinialStyle {
    FinialStyle::Ball
}
fn d_fin_h() -> f32 {
    1.0
}

element! {
    /// Sculpture placeholder.
    pub struct Statue {
        #[serde(default)] pub at: [f32; 3],
        #[serde(default)] pub facing: Option<Facing>,
        /// Figure height (without pedestal).
        #[serde(default = "d_stat_h")] pub height: f32,
        #[serde(default = "d_stat_s")] pub style: StatueStyle,
        #[serde(default)] pub pedestal: Flex<PedestalSpec>,
    }
}
fn d_stat_h() -> f32 {
    2.0
}
fn d_stat_s() -> StatueStyle {
    StatueStyle::Figure
}

element! {
    /// Stairs: straight (`from` → `to`, ascending) or around a polygon.
    pub struct Stairs {
        #[serde(default)] pub from: Option<[f32; 2]>,
        #[serde(default)] pub to: Option<[f32; 2]>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub y: f32,
        #[serde(default = "d_st_w")] pub width: f32,
        #[serde(default)] pub steps: Option<u32>,
        /// Total rise (default steps x riser).
        #[serde(default)] pub rise: Option<f32>,
        #[serde(default = "d_st_r")] pub riser: f32,
        #[serde(default = "d_st_t")] pub tread: f32,
        /// Side walls (stairs from/to only).
        #[serde(default)] pub cheeks: bool,
    }
}
fn d_st_w() -> f32 {
    3.0
}
fn d_st_r() -> f32 {
    0.16
}
fn d_st_t() -> f32 {
    0.33
}

element! {
    /// Arcade / colonnade / loggia: arches on columns or piers along a line or polygon edges.
    pub struct Arcade {
        #[serde(default)] pub from: Option<[f32; 2]>,
        #[serde(default)] pub to: Option<[f32; 2]>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub y: f32,
        #[serde(default)] pub bays: Flex<Bays>,
        #[serde(default = "d_arc_a")] pub arch: ArchKind,
        #[serde(default)] pub rise: Option<f32>,
        /// Height of the springing (top of the column capitals).
        #[serde(default = "d_arc_imp")] pub impost: f32,
        /// Top of the arcade wall (default: springing + span/2 + 1 m).
        #[serde(default)] pub height: Option<f32>,
        /// Thickness of the arcade wall.
        #[serde(default = "d_arc_d")] pub depth: f32,
        #[serde(default)] pub column: Flex<ColumnSpec>,
        #[serde(default)] pub piers: Flex<PierSpec>,
        /// Gallery behind the arcade: depth to a back wall with a ceiling.
        #[serde(default)] pub gallery: Option<f32>,
        #[serde(default)] pub wall_material: Option<String>,
        /// Cornice on top.
        #[serde(default)] pub cornice: Flex<MouldingSpec>,
        /// Hood mouldings over the arches.
        #[serde(default)] pub hood: Flex<HoodSpec>,
        /// Which side of the line the arcade faces ("left" or "right").
        #[serde(default, deserialize_with = "de_side_opt")] pub face: Option<String>,
    }
}
fn d_arc_a() -> ArchKind {
    ArchKind::Round
}
fn d_arc_imp() -> f32 {
    3.5
}
fn d_arc_d() -> f32 {
    0.7
}

element! {
    /// Lantern: open turret with posts and a cap.
    pub struct Lantern {
        #[serde(default)] pub at: [f32; 3],
        #[serde(default = "d_lan_r")] pub radius: f32,
        #[serde(default = "d_lan_h")] pub height: f32,
        /// Faces (posts); 0 = 8 with round look.
        #[serde(default = "d_lan_s")] pub sides: u32,
        /// dome | cone | onion | spire | flat
        #[serde(default = "d_lan_c", deserialize_with = "de_lcap")] pub cap: String,
        #[serde(default)] pub cap_height: Option<f32>,
        #[serde(default)] pub cap_material: Option<String>,
        #[serde(default)] pub finial: ElementList,
    }
}
fn d_lan_r() -> f32 {
    1.2
}
fn d_lan_h() -> f32 {
    3.0
}
fn d_lan_s() -> u32 {
    8
}
fn d_lan_c() -> String {
    "dome".into()
}

element! {
    pub struct BoxEl {
        #[serde(default)] pub center: Option<[f32; 3]>,
        #[serde(default)] pub size: Option<[f32; 3]>,
        #[serde(default)] pub min: Option<[f32; 3]>,
        #[serde(default)] pub max: Option<[f32; 3]>,
    }
}

element! {
    /// Cylinder / cone / frustum: base point and height, or `from`/`to`.
    pub struct Cylinder {
        #[serde(default)] pub at: [f32; 3],
        #[serde(default)] pub height: Option<f32>,
        #[serde(default)] pub from: Option<[f32; 3]>,
        #[serde(default)] pub to: Option<[f32; 3]>,
        #[serde(default = "d_cyl_r")] pub radius: f32,
        /// Top radius (cones / frustums).
        #[serde(default)] pub radius_top: Option<f32>,
        #[serde(default)] pub segments: Option<u32>,
        #[serde(default = "yes")] pub caps: bool,
        /// Faceted (flat shaded) instead of smooth.
        #[serde(default)] pub faceted: bool,
    }
}
fn d_cyl_r() -> f32 {
    0.5
}

element! {
    /// Sphere or ellipsoid (optionally a latitude/longitude section).
    pub struct Sphere {
        #[serde(default)] pub center: [f32; 3],
        #[serde(default)] pub radius: Option<f32>,
        #[serde(default)] pub radii: Option<[f32; 3]>,
        /// Latitude range in degrees (-90 bottom .. 90 top).
        #[serde(default)] pub lat: Option<[f32; 2]>,
        /// Longitude range in degrees.
        #[serde(default)] pub lon: Option<[f32; 2]>,
        #[serde(default)] pub segments: Option<u32>,
    }
}

element! {
    pub struct Torus {
        #[serde(default)] pub center: [f32; 3],
        pub radius: f32,
        pub tube: f32,
        #[serde(default)] pub axis: Option<[f32; 3]>,
        #[serde(default)] pub segments: Option<u32>,
    }
}

element! {
    /// Surface of revolution: profile [[r, y], ...] about the vertical axis through `at`.
    pub struct Lathe {
        #[serde(default)] pub at: [f32; 3],
        pub profile: Vec<[f32; 2]>,
        #[serde(default)] pub segments: Option<u32>,
        /// Angle range in degrees (default full turn).
        #[serde(default)] pub angle: Option<[f32; 2]>,
    }
}

element! {
    /// 2D profile (with holes) extruded along an axis.
    pub struct Extrude {
        pub profile: PolySpec,
        /// "xy" (default: profile in x/y, extruded along +z), "xz" (plan, up), "yz", "zy".
        #[serde(default, deserialize_with = "de_plane_opt")] pub plane: Option<String>,
        #[serde(default)] pub origin: [f32; 3],
        pub length: f32,
        /// Centre the extrusion on the plane.
        #[serde(default)] pub center: bool,
        /// Smooth shading of the sides: true (40 degree crease) or an angle.
        #[serde(default)] pub smooth: Crease,
    }
}

element! {
    /// Profile swept along a path.
    pub struct Sweep {
        pub profile: Section,
        pub path: PathSpec,
        /// Total twist along the path (degrees).
        #[serde(default)] pub twist: f32,
        /// Section scale at the end (taper), [side, up] or uniform.
        #[serde(default)] pub scale_end: Option<Scale>,
        #[serde(default = "yes")] pub caps: bool,
        /// Up hint for the first frame.
        #[serde(default)] pub up: Option<[f32; 3]>,
        /// Keep section "up" vertical (mouldings, handrails).
        #[serde(default)] pub level: bool,
    }
}

element! {
    /// Smooth loft through sections placed at stations (organic shells, plumes, ribs).
    pub struct Loft {
        pub stations: Vec<LoftStation>,
        /// Default section for stations without one.
        #[serde(default)] pub section: Option<Section>,
        /// Interpolated sections between stations.
        #[serde(default = "d_loft_s")] pub samples: u32,
        /// Points per section.
        #[serde(default = "d_loft_r")] pub resolution: u32,
        #[serde(default = "yes")] pub smooth: bool,
        #[serde(default = "yes")] pub caps: bool,
        #[serde(default)] pub up: Option<[f32; 3]>,
        /// Loop back to the first station.
        #[serde(default)] pub closed: bool,
    }
}
fn d_loft_s() -> u32 {
    8
}
fn d_loft_r() -> u32 {
    32
}

element! {
    /// Parametric thin shell with thickness, clipped by planes.
    pub struct Shell {
        pub surface: SurfaceKind,
        #[serde(default)] pub center: [f32; 3],
        #[serde(default)] pub radii: Option<[f32; 3]>,
        #[serde(default)] pub radius: Option<f32>,
        /// Superellipsoid exponents [north-south, east-west] (2 = ellipsoid).
        #[serde(default)] pub exponents: Option<[f32; 2]>,
        /// Parameter ranges in degrees (ellipsoids: u = longitude, v = latitude).
        #[serde(default)] pub u_range: Option<[f32; 2]>,
        #[serde(default)] pub v_range: Option<[f32; 2]>,
        /// Hypar corners [[x, y, z] x 4] (in order around the patch).
        #[serde(default)] pub corners: Option<Vec<[f32; 3]>>,
        /// Keep the side where n·p <= d: [[nx, ny, nz, d], ...].
        #[serde(default)] pub clip: Vec<[f32; 4]>,
        #[serde(default = "d_shell_t")] pub thickness: f32,
        #[serde(default)] pub segments: Option<[u32; 2]>,
        #[serde(default)] pub ribs: Flex<ShellRibs>,
        /// Height / length for paraboloid, cylinder and cone surfaces.
        #[serde(default)] pub height: Option<f32>,
    }
}
fn d_shell_t() -> f32 {
    0.25
}

element! {
    /// Raw triangles (last resort).
    pub struct MeshEl {
        pub vertices: Vec<[f32; 3]>,
        #[serde(default)] pub triangles: Vec<[u32; 3]>,
        #[serde(default)] pub quads: Vec<[u32; 4]>,
        /// Smoothing angle in degrees (0 = flat).
        #[serde(default)] pub smooth: f32,
    }
}

element! {
    /// Group of child elements sharing a transform and material.
    pub struct Group {
        #[serde(default)] pub children: Vec<Element>,
        #[serde(default)] pub component: Option<String>,
    }
}

element! {
    /// Instance of a component.
    pub struct Instance {
        pub component: String,
        #[serde(default)] pub at: Option<[f32; 3]>,
        #[serde(default)] pub facing: Option<Facing>,
    }
}

str_enum!(
    /// Array layout.
    ArrayMode, "array mode", { Linear => ["linear"], Grid => ["grid"], Radial => ["radial"], Edges => ["edges", "along"], Path => ["path"], Points => ["points"] }
);

str_enum!(
    /// What to do at polygon corners in edge arrays.
    CornerMode, "corner mode", { Skip => ["skip"], Include => ["include"], Only => ["only"] }
);

element! {
    /// Repeats children (or a component) in a layout.
    pub struct Array {
        pub mode: ArrayMode,
        #[serde(default)] pub children: Vec<Element>,
        #[serde(default)] pub component: Option<String>,
        #[serde(default)] pub count: Option<u32>,
        /// Linear step [dx, dy, dz].
        #[serde(default)] pub step: Option<[f32; 3]>,
        /// Grid counts [nx, nz] or [nx, ny, nz] and steps.
        #[serde(default)] pub counts: Option<Vec<u32>>,
        #[serde(default)] pub steps: Option<Vec<f32>>,
        /// Radial: centre, axis, start angle and sweep (degrees), radius.
        #[serde(default)] pub center: Option<[f32; 3]>,
        #[serde(default)] pub axis: Option<[f32; 3]>,
        #[serde(default)] pub start: f32,
        #[serde(default)] pub sweep: Option<f32>,
        #[serde(default)] pub radius: Option<f32>,
        /// Rotate children with the layout (radial, edges, path).
        #[serde(default = "yes")] pub rotate: bool,
        /// Edges / path layouts.
        #[serde(default)] pub on: Option<String>,
        #[serde(default)] pub polygon: Option<PolySpec>,
        #[serde(default)] pub edges: EdgeSel,
        #[serde(default)] pub spacing: Option<f32>,
        #[serde(default)] pub margin: f32,
        #[serde(default)] pub corners: Option<CornerMode>,
        /// Outward offset from the edge.
        #[serde(default)] pub offset: f32,
        #[serde(default)] pub y: Option<f32>,
        #[serde(default)] pub path: Option<PathSpec>,
        /// Explicit positions [[x, y, z], ...] with optional `facings`.
        #[serde(default)] pub points: Option<Vec<[f32; 3]>>,
        #[serde(default)] pub facings: Option<Vec<f32>>,
        /// Skip these instance indices.
        #[serde(default)] pub skip: Vec<i64>,
        /// Scale of the last instance (the first is 1; linear in between),
        /// uniform or [x, y, z], about each instance's own origin.
        #[serde(default)] pub scale_end: Option<Scale>,
        /// Extra rotation about each instance's own vertical axis, per
        /// instance (degrees).
        #[serde(default)] pub rotate_step: f32,
        /// Extra offset per instance in the instance frame [x, y, z]
        /// (e.g. rising steps in a radial array).
        #[serde(default)] pub offset_step: Option<[f32; 3]>,
    }
}

/// Direction an attached element faces: compass name or degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facing(pub f32);
impl<'de> Deserialize<'de> for Facing {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Facing;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a compass direction (n, ne, e, ...) or degrees")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Facing, E> {
                compass_deg(&s.trim().to_ascii_lowercase()).map(Facing).ok_or_else(|| E::custom(format!("bad direction `{s}`")))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Facing, E> {
                Ok(Facing(v as f32))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Facing, E> {
                Ok(Facing(v as f32))
            }
        }
        d.deserialize_any(V)
    }
}

// ------------------------------------------------------------- dispatch

/// Any element. Tables dispatch on `type` (see [`Element::TYPES`]).
#[derive(Clone, Debug)]
pub enum Element {
    Mass(Mass),
    Slab(Slab),
    Wall(Wall),
    Openings(Openings),
    Moulding(Moulding, MouldingKind),
    Parapet(Parapet),
    Balustrade(Balustrade),
    Crenellations(Crenellations),
    Machicolations(Machicolations),
    Gargoyles(Gargoyles),
    Roof(Roof),
    Dome(Dome),
    Tower(Tower),
    Column(Column),
    Pilaster(Pilaster),
    Buttress(Buttress),
    FlyingButtress(FlyingButtress),
    Pinnacle(Pinnacle),
    Finial(Finial),
    Statue(Statue),
    Stairs(Stairs),
    Arcade(Arcade),
    Lantern(Lantern),
    Box(BoxEl),
    Cylinder(Cylinder, bool),
    Sphere(Sphere),
    Torus(Torus),
    Lathe(Lathe),
    Extrude(Extrude),
    Sweep(Sweep),
    Loft(Loft),
    Shell(Shell),
    Mesh(MeshEl),
    Group(Group),
    Array(Array),
    Instance(Instance),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouldingKind {
    Cornice,
    StringCourse,
    Plinth,
    Generic,
}

impl Element {
    /// Every accepted `type` name.
    pub const TYPES: &'static [&'static str] = &[
        "mass", "prism", "slab", "wall", "openings", "cornice", "string_course", "plinth", "moulding", "parapet", "balustrade", "crenellations", "machicolations", "gargoyles", "roof", "dome", "tower", "column",
        "pilaster", "buttress", "flying_buttress", "pinnacle", "finial", "statue", "stairs", "arcade", "colonnade", "loggia", "lantern", "box", "cylinder", "cone", "sphere", "ellipsoid", "torus", "lathe", "extrude",
        "sweep", "loft", "shell", "mesh", "group", "array", "instance",
    ];

    pub fn type_name(&self) -> &'static str {
        match self {
            Element::Mass(_) => "mass",
            Element::Slab(_) => "slab",
            Element::Wall(_) => "wall",
            Element::Openings(_) => "openings",
            Element::Moulding(_, k) => match k {
                MouldingKind::Cornice => "cornice",
                MouldingKind::StringCourse => "string_course",
                MouldingKind::Plinth => "plinth",
                MouldingKind::Generic => "moulding",
            },
            Element::Parapet(_) => "parapet",
            Element::Balustrade(_) => "balustrade",
            Element::Crenellations(_) => "crenellations",
            Element::Machicolations(_) => "machicolations",
            Element::Gargoyles(_) => "gargoyles",
            Element::Roof(_) => "roof",
            Element::Dome(_) => "dome",
            Element::Tower(_) => "tower",
            Element::Column(_) => "column",
            Element::Pilaster(_) => "pilaster",
            Element::Buttress(_) => "buttress",
            Element::FlyingButtress(_) => "flying_buttress",
            Element::Pinnacle(_) => "pinnacle",
            Element::Finial(_) => "finial",
            Element::Statue(_) => "statue",
            Element::Stairs(_) => "stairs",
            Element::Arcade(_) => "arcade",
            Element::Lantern(_) => "lantern",
            Element::Box(_) => "box",
            Element::Cylinder(_, c) => {
                if *c {
                    "cone"
                } else {
                    "cylinder"
                }
            }
            Element::Sphere(_) => "sphere",
            Element::Torus(_) => "torus",
            Element::Lathe(_) => "lathe",
            Element::Extrude(_) => "extrude",
            Element::Sweep(_) => "sweep",
            Element::Loft(_) => "loft",
            Element::Shell(_) => "shell",
            Element::Mesh(_) => "mesh",
            Element::Group(_) => "group",
            Element::Array(_) => "array",
            Element::Instance(_) => "instance",
        }
    }

    pub fn common(&self) -> Common<'_> {
        match self {
            Element::Mass(e) => e.common(),
            Element::Slab(e) => e.common(),
            Element::Wall(e) => e.common(),
            Element::Openings(e) => e.common(),
            Element::Moulding(e, _) => e.common(),
            Element::Parapet(e) => e.common(),
            Element::Balustrade(e) => e.common(),
            Element::Crenellations(e) => e.common(),
            Element::Machicolations(e) => e.common(),
            Element::Gargoyles(e) => e.common(),
            Element::Roof(e) => e.common(),
            Element::Dome(e) => e.common(),
            Element::Tower(e) => e.common(),
            Element::Column(e) => e.common(),
            Element::Pilaster(e) => e.common(),
            Element::Buttress(e) => e.common(),
            Element::FlyingButtress(e) => e.common(),
            Element::Pinnacle(e) => e.common(),
            Element::Finial(e) => e.common(),
            Element::Statue(e) => e.common(),
            Element::Stairs(e) => e.common(),
            Element::Arcade(e) => e.common(),
            Element::Lantern(e) => e.common(),
            Element::Box(e) => e.common(),
            Element::Cylinder(e, _) => e.common(),
            Element::Sphere(e) => e.common(),
            Element::Torus(e) => e.common(),
            Element::Lathe(e) => e.common(),
            Element::Extrude(e) => e.common(),
            Element::Sweep(e) => e.common(),
            Element::Loft(e) => e.common(),
            Element::Shell(e) => e.common(),
            Element::Mesh(e) => e.common(),
            Element::Group(e) => e.common(),
            Element::Array(e) => e.common(),
            Element::Instance(e) => e.common(),
        }
    }

    /// A finial preset as an element (`finial = "cross"` shorthand).
    pub fn finial_preset(style: FinialStyle) -> Element {
        Element::Finial(Finial { name: None, material: None, transform: None, mirror: None, enabled: true, collide: Some(false), note: None, at: [0.0; 3], style, size: d_fin_h(), facing: None })
    }
}

/// Map access that first replays entries buffered before `type` was seen.
struct Replay<'de, A: MapAccess<'de>> {
    pending: std::vec::IntoIter<(String, toml::Value)>,
    value: Option<toml::Value>,
    inner: A,
    _m: std::marker::PhantomData<&'de ()>,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Replay<'de, A> {
    type Error = A::Error;
    fn next_key_seed<K: de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error> {
        if let Some((k, v)) = self.pending.next() {
            self.value = Some(v);
            return seed.deserialize(de::value::StringDeserializer::<A::Error>::new(k)).map(Some);
        }
        self.inner.next_key_seed(seed)
    }
    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        if let Some(v) = self.value.take() {
            return seed.deserialize(v).map_err(|e| de::Error::custom(e.to_string()));
        }
        self.inner.next_value_seed(seed)
    }
}

impl<'de> Deserialize<'de> for Element {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Element;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an element table with a `type` key")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Element, A::Error> {
                let mut pending: Vec<(String, toml::Value)> = Vec::new();
                let ty: String = loop {
                    match map.next_key::<String>()? {
                        Some(k) if k == "type" => break map.next_value()?,
                        Some(k) => pending.push((k, map.next_value()?)),
                        None => {
                            return Err(de::Error::custom(format!("element is missing `type` (one of: {})", Element::TYPES.join(", "))));
                        }
                    }
                };
                let r = Replay { pending: pending.into_iter(), value: None, inner: map, _m: std::marker::PhantomData };
                let md = MapAccessDeserializer::new(r);
                let t = ty.trim().to_ascii_lowercase();
                Ok(match t.as_str() {
                    "mass" | "prism" => Element::Mass(Mass::deserialize(md)?),
                    "slab" => Element::Slab(Slab::deserialize(md)?),
                    "wall" => Element::Wall(Wall::deserialize(md)?),
                    "openings" => Element::Openings(Openings::deserialize(md)?),
                    "cornice" => Element::Moulding(Moulding::deserialize(md)?, MouldingKind::Cornice),
                    "string_course" => Element::Moulding(Moulding::deserialize(md)?, MouldingKind::StringCourse),
                    "plinth" => Element::Moulding(Moulding::deserialize(md)?, MouldingKind::Plinth),
                    "moulding" | "molding" => Element::Moulding(Moulding::deserialize(md)?, MouldingKind::Generic),
                    "parapet" => Element::Parapet(Parapet::deserialize(md)?),
                    "balustrade" => Element::Balustrade(Balustrade::deserialize(md)?),
                    "crenellations" | "battlements" => Element::Crenellations(Crenellations::deserialize(md)?),
                    "machicolations" => Element::Machicolations(Machicolations::deserialize(md)?),
                    "gargoyles" => Element::Gargoyles(Gargoyles::deserialize(md)?),
                    "roof" => Element::Roof(Roof::deserialize(md)?),
                    "dome" => Element::Dome(Dome::deserialize(md)?),
                    "tower" => Element::Tower(Tower::deserialize(md)?),
                    "column" => Element::Column(Column::deserialize(md)?),
                    "pilaster" => Element::Pilaster(Pilaster::deserialize(md)?),
                    "buttress" => Element::Buttress(Buttress::deserialize(md)?),
                    "flying_buttress" => Element::FlyingButtress(FlyingButtress::deserialize(md)?),
                    "pinnacle" => Element::Pinnacle(Pinnacle::deserialize(md)?),
                    "finial" => Element::Finial(Finial::deserialize(md)?),
                    "statue" | "sculpture" => Element::Statue(Statue::deserialize(md)?),
                    "stairs" | "steps" => Element::Stairs(Stairs::deserialize(md)?),
                    "arcade" | "colonnade" | "loggia" => Element::Arcade(Arcade::deserialize(md)?),
                    "lantern" => Element::Lantern(Lantern::deserialize(md)?),
                    "box" => Element::Box(BoxEl::deserialize(md)?),
                    "cylinder" => Element::Cylinder(Cylinder::deserialize(md)?, false),
                    "cone" => Element::Cylinder(Cylinder::deserialize(md)?, true),
                    "sphere" | "ellipsoid" => Element::Sphere(Sphere::deserialize(md)?),
                    "torus" => Element::Torus(Torus::deserialize(md)?),
                    "lathe" => Element::Lathe(Lathe::deserialize(md)?),
                    "extrude" => Element::Extrude(Extrude::deserialize(md)?),
                    "sweep" => Element::Sweep(Sweep::deserialize(md)?),
                    "loft" => Element::Loft(Loft::deserialize(md)?),
                    "shell" => Element::Shell(Shell::deserialize(md)?),
                    "mesh" => Element::Mesh(MeshEl::deserialize(md)?),
                    "group" => Element::Group(Group::deserialize(md)?),
                    "array" => Element::Array(Array::deserialize(md)?),
                    "instance" => Element::Instance(Instance::deserialize(md)?),
                    _ => return Err(de::Error::custom(format!("unknown element type `{ty}`{}; expected one of: {}", super::parse::did_you_mean(&t, Element::TYPES), Element::TYPES.join(", ")))),
                })
            }
        }
        d.deserialize_map(V)
    }
}

/// Nested elements: an array of element tables, one table, or a finial
/// preset name (`"cross"`).
#[derive(Clone, Debug, Default)]
pub struct ElementList(pub Vec<Element>);

impl<'de> Deserialize<'de> for ElementList {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ElementList;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("element tables or a finial preset name")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<ElementList, E> {
                Ok(ElementList(vec![Element::finial_preset(FinialStyle::parse(s).map_err(E::custom)?)]))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<ElementList, A::Error> {
                Ok(ElementList(Vec::<Element>::deserialize(de::value::SeqAccessDeserializer::new(a))?))
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<ElementList, A::Error> {
                Ok(ElementList(vec![Element::deserialize(MapAccessDeserializer::new(m))?]))
            }
        }
        d.deserialize_any(V)
    }
}

// --------------------------------------------------------------- the file

/// `[landmark]`: identity and global settings.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    /// Footprint file relative to the spec (default footprint.json).
    #[serde(default)]
    pub footprint: Option<String>,
    /// Raw photo folder (default assets/downloaded/landmarks/valencia/<slug>).
    #[serde(default)]
    pub raw_dir: Option<String>,
    /// Main facade for the street view: edge selector of outline 0 or degrees.
    #[serde(default)]
    pub main_facade: Option<EdgeSel>,
    /// City-frame anchor override [x, z] when there is no footprint.
    #[serde(default)]
    pub anchor: Option<[f32; 2]>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Free-form metadata (architect, year, style, sources...).
    #[serde(default)]
    pub info: Option<toml::Table>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub roof_material: Option<String>,
    #[serde(default)]
    pub glass_material: Option<String>,
    #[serde(default)]
    pub frame_material: Option<String>,
    #[serde(default)]
    pub door_material: Option<String>,
    #[serde(default)]
    pub iron_material: Option<String>,
    /// Curve resolution multiplier (1 = default).
    #[serde(default)]
    pub detail: Option<f32>,
    /// Default storey height for `floors`.
    #[serde(default)]
    pub floor_height: Option<f32>,
}

/// `[material.<name>]`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialSpec {
    pub kind: String,
    #[serde(default)]
    pub tint: Option<Color>,
    #[serde(default)]
    pub color: Option<Color>,
    #[serde(default)]
    pub colors: Vec<Color>,
    /// Relative amount of each of `colors` (random per tile).
    #[serde(default)]
    pub weights: Vec<f32>,
    #[serde(default)]
    pub pattern: Option<String>,
    /// Glossy coat over the texture (glazes, varnish): 0..1.
    #[serde(default)]
    pub clearcoat: Option<f32>,
    #[serde(default)]
    pub roughness: Option<f32>,
    #[serde(default)]
    pub metallic: Option<f32>,
    /// Metres per texture repeat.
    #[serde(default)]
    pub uv_scale: Option<f32>,
    #[serde(default)]
    pub uv_rotate: Option<f32>,
    #[serde(default)]
    pub normal_strength: Option<f32>,
    /// Dirt and streaks (0..1).
    #[serde(default)]
    pub weathering: Option<f32>,
    /// Ashlar blocks [length, course height] in metres.
    #[serde(default)]
    pub block: Option<[f32; 2]>,
    #[serde(default)]
    pub emissive: Option<f32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub children: Vec<Element>,
    #[serde(default)]
    pub note: Option<String>,
}

/// `[[view]]`: a camera matching a reference photo.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewSpec {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub photo: Option<String>,
    #[serde(default)]
    pub eye: Option<[f32; 3]>,
    #[serde(default)]
    pub target: Option<[f32; 3]>,
    #[serde(default)]
    pub fov_y: Option<f32>,
    #[serde(default)]
    pub focal_35mm: Option<f32>,
    #[serde(default)]
    pub roll: f32,
    /// [azimuth, elevation] in degrees.
    #[serde(default)]
    pub sun: Option<[f32; 2]>,
    /// day | morning | evening | dusk | night (sky and lights).
    #[serde(default, deserialize_with = "de_time_opt")]
    pub time: Option<String>,
    #[serde(default)]
    pub context: Option<bool>,
    /// Image-to-world correspondences (at least 4): the camera is solved
    /// from them and printed, overriding eye / target / fov.
    #[serde(default, rename = "match")]
    pub matches: Vec<MatchPoint>,
    #[serde(default)]
    pub note: Option<String>,
}

/// One correspondence: pixel `[u, v]` of the photo (from its top-left
/// corner) seeing landmark-local `[x, y, z]`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchPoint {
    pub px: [f32; 2],
    pub world: [f32; 3],
    #[serde(default)]
    pub note: Option<String>,
}

/// `[[light]]`: a point light for dusk and night renders.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightSpec {
    pub at: [f32; 3],
    #[serde(default)]
    pub color: Option<Color>,
    /// Relative intensity (1 = a street lamp).
    #[serde(default = "d_light_i")]
    pub intensity: f32,
    /// Reach in metres.
    #[serde(default = "d_light_r")]
    pub radius: f32,
    #[serde(default)]
    pub note: Option<String>,
}
fn d_light_i() -> f32 {
    1.0
}
fn d_light_r() -> f32 {
    12.0
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderSpec {
    /// Sun [azimuth, elevation] for the standard views.
    #[serde(default)]
    pub sun: Option<[f32; 2]>,
    /// Distance of the street-level camera from the main facade.
    #[serde(default)]
    pub street_distance: Option<f32>,
    /// Explicit street-view camera (overrides the automatic one).
    #[serde(default)]
    pub street_eye: Option<[f32; 3]>,
    #[serde(default)]
    pub street_target: Option<[f32; 3]>,
    #[serde(default)]
    pub street_fov: Option<f32>,
    /// clear (default: sky-only HDRI) | street (the street HDRI) |
    /// atmosphere (analytic sky following the sun).
    #[serde(default, deserialize_with = "de_sky_opt")]
    pub sky: Option<String>,
    /// day | morning | evening | dusk | night.
    #[serde(default, deserialize_with = "de_time_opt")]
    pub time: Option<String>,
    #[serde(default)]
    pub exposure: Option<f32>,
    /// Paving radius around the building (metres).
    #[serde(default)]
    pub paving: Option<f32>,
}

/// A whole `landmark.toml`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub landmark: Meta,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub material: BTreeMap<String, MaterialSpec>,
    #[serde(default)]
    pub component: BTreeMap<String, Component>,
    #[serde(default)]
    pub element: Vec<Spanned<Element>>,
    #[serde(default)]
    pub view: Vec<ViewSpec>,
    #[serde(default)]
    pub light: Vec<LightSpec>,
    #[serde(default)]
    pub render: RenderSpec,
}
