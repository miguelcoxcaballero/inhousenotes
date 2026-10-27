//! Real-world data written by the data team: `footprint.json` (OSM
//! outlines and parts, Catastro parcels with floor counts, neighbouring
//! context) and `sources.toml` (photos with attribution and cameras).
//! Parsing is lenient: unknown fields are ignored and numbers may come as
//! strings.

use std::collections::BTreeMap;
use std::path::Path;

use forja_math::Vec2;
use serde::Deserialize;
use serde_json::Value;

use super::parse::{Error, Result};
use super::poly::{self, Shape};

fn lenient_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.as_f64().map(|x| x as f32),
        Value::String(s) => parse_measure(s),
        _ => None,
    }
}

/// Leading number of a tag value ("34", "34 m", "12.5m", "~20").
pub fn parse_measure(s: &str) -> Option<f32> {
    let t = s.trim().trim_start_matches('~');
    let end = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(t.len());
    t[..end].parse::<f32>().ok().filter(|v| v.is_finite())
}

fn id_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        o => o.to_string(),
    }
}

/// `null` (common from Python writers) reads as the default.
fn nullable<'de, D: serde::Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> std::result::Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, Deserialize)]
struct RawPoly {
    #[serde(default)]
    id: Value,
    #[serde(default, deserialize_with = "nullable")]
    tags: BTreeMap<String, Value>,
    /// Points as raw JSON: non-numeric points are skipped with a warning.
    #[serde(default)]
    outer: Value,
    #[serde(default)]
    inners: Value,
    #[serde(default)]
    refcat: Value,
    #[serde(default)]
    floors_above: Value,
    #[serde(default)]
    floors_below: Value,
    #[serde(default)]
    source: Value,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct RawFootprint {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    frame: Option<String>,
    #[serde(default)]
    anchor: Value,
    #[serde(default)]
    attribution: Value,
    #[serde(default, deserialize_with = "nullable")]
    outlines: Vec<RawPoly>,
    #[serde(default, deserialize_with = "nullable")]
    parts: Vec<RawPoly>,
    #[serde(default, deserialize_with = "nullable")]
    catastro: Vec<RawPoly>,
    #[serde(default, deserialize_with = "nullable")]
    context: Vec<RawPoly>,
}

/// `[x, z]` from JSON (numbers or numeric strings).
fn point(v: &Value) -> Option<Vec2> {
    let a = v.as_array()?;
    let x = lenient_f32(a.first()?)?;
    let z = lenient_f32(a.get(1)?)?;
    (x.is_finite() && z.is_finite()).then(|| Vec2::new(x, z))
}

/// A ring from JSON, skipping (and counting) unusable points.
fn ring(v: &Value, bad: &mut usize) -> Vec<Vec2> {
    let Some(list) = v.as_array() else {
        if !v.is_null() {
            *bad += 1;
        }
        return Vec::new();
    };
    list.iter()
        .filter_map(|p| {
            let q = point(p);
            if q.is_none() {
                *bad += 1;
            }
            q
        })
        .collect()
}

/// One polygon from the footprint file, normalised.
#[derive(Clone, Debug)]
pub struct FpPoly {
    pub id: String,
    pub tags: BTreeMap<String, String>,
    pub shape: Shape,
    pub refcat: Option<String>,
    pub floors_above: Option<f32>,
    pub floors_below: Option<f32>,
    pub source: Option<String>,
}

impl FpPoly {
    pub fn tag(&self, k: &str) -> Option<&str> {
        self.tags.get(k).map(|s| s.as_str())
    }

    /// Top height from OSM tags (Simple 3D Buildings): `height`, else
    /// `building:levels` x 3.2 m + `roof:height`. Levels count from the
    /// ground, skipped ones included, so `building:min_level` only moves
    /// the bottom ([`Self::tag_min_height`]), never the top.
    pub fn tag_height(&self) -> Option<f32> {
        if let Some(h) = self.tag("height").and_then(parse_measure) {
            return Some(h);
        }
        let levels = self.tag("building:levels").and_then(parse_measure).filter(|l| *l > 0.0)?;
        let roof = self.tag("roof:height").and_then(parse_measure).unwrap_or(0.0);
        let top = levels * 3.2 + roof;
        // A `min_height` in metres with only a level count above it means
        // those levels stand on it.
        match (self.tag("min_height").and_then(parse_measure), self.tag("building:min_level")) {
            (Some(mh), None) if mh >= top => Some(mh + top),
            _ => Some(top),
        }
    }

    pub fn tag_min_height(&self) -> Option<f32> {
        self.tag("min_height").and_then(parse_measure).or_else(|| self.tag("building:min_level").and_then(parse_measure).map(|l| l * 3.2))
    }
}

/// Parsed `footprint.json`.
#[derive(Clone, Debug, Default)]
pub struct Footprint {
    pub slug: Option<String>,
    pub frame: Option<String>,
    /// City-frame position of the landmark-local origin.
    pub anchor: Vec2,
    pub attribution: Vec<String>,
    pub outlines: Vec<FpPoly>,
    pub parts: Vec<FpPoly>,
    pub catastro: Vec<FpPoly>,
    pub context: Vec<FpPoly>,
    /// Problems found while reading (degenerate rings...).
    pub warnings: Vec<String>,
}

fn convert(raw: &RawPoly, what: &str, i: usize, warnings: &mut Vec<String>) -> Option<FpPoly> {
    let mut bad = 0;
    let outer = ring(&raw.outer, &mut bad);
    let holes: Vec<Vec<Vec2>> = raw.inners.as_array().map(|l| l.iter().map(|h| ring(h, &mut bad)).collect()).unwrap_or_default();
    if bad > 0 {
        warnings.push(format!("footprint {what}[{i}] ({}): {bad} point(s) or ring(s) are not [x, z] numbers; skipped", id_string(&raw.id)));
    }
    if let Some((a, b)) = poly::self_intersection(&outer) {
        warnings.push(format!("footprint {what}[{i}] ({}): the outer ring crosses itself (edges {a} and {b}); fix the polygon or simplify it", id_string(&raw.id)));
    }
    let Some(shape) = (Shape { outer, holes }).normalized() else {
        warnings.push(format!("footprint {what}[{i}] ({}) has a degenerate outer ring; ignored", id_string(&raw.id)));
        return None;
    };
    let tags = raw.tags.iter().map(|(k, v)| (k.clone(), if let Value::String(s) = v { s.clone() } else { v.to_string() })).collect();
    let refcat = match &raw.refcat {
        Value::Null => None,
        v => Some(id_string(v)),
    };
    let source = match &raw.source {
        Value::Null => None,
        v => Some(id_string(v)),
    };
    Some(FpPoly { id: id_string(&raw.id), tags, shape, refcat, floors_above: lenient_f32(&raw.floors_above), floors_below: lenient_f32(&raw.floors_below), source })
}

impl Footprint {
    pub fn parse(text: &str, origin: &str) -> Result<Footprint> {
        let raw: RawFootprint = serde_json::from_str(text).map_err(|e| Error::msg(format!("{origin}:{}:{}: {e}", e.line(), e.column())))?;
        let mut warnings = Vec::new();
        if let Some(f) = &raw.frame
            && f != "utm30n-local-v1"
        {
            warnings.push(format!("footprint frame is `{f}`, expected `utm30n-local-v1`"));
        }
        let conv = |list: &[RawPoly], what: &str, w: &mut Vec<String>| list.iter().enumerate().filter_map(|(i, r)| convert(r, what, i, w)).collect::<Vec<_>>();
        let outlines = conv(&raw.outlines, "outlines", &mut warnings);
        let parts = conv(&raw.parts, "parts", &mut warnings);
        let catastro = conv(&raw.catastro, "catastro", &mut warnings);
        let context = conv(&raw.context, "context", &mut warnings);
        let attribution = match &raw.attribution {
            Value::Array(a) => a.iter().map(|v| if let Value::String(s) = v { s.clone() } else { v.to_string() }).collect(),
            Value::Null => Vec::new(),
            v => vec![v.to_string()],
        };
        let anchor = point(&raw.anchor).unwrap_or_else(|| {
            warnings.push("footprint has no usable anchor; using [0, 0]".into());
            Vec2::ZERO
        });
        Ok(Footprint { slug: raw.slug, frame: raw.frame, anchor, attribution, outlines, parts, catastro, context, warnings })
    }

    pub fn load(path: &Path) -> Result<Footprint> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::msg(format!("{}: {e}", path.display())))?;
        Self::parse(&text, &path.display().to_string())
    }

    /// Resolves `outline:0`, `outline:all`, `part:2`, `osm:way/123`,
    /// `catastro:<id or refcat>`, `catastro:all`, `context:3`.
    pub fn resolve(&self, reference: &str) -> std::result::Result<Vec<&FpPoly>, String> {
        let (kind, arg) = reference.split_once(':').unwrap_or((reference, "0"));
        let kind = kind.trim().to_ascii_lowercase();
        let arg = arg.trim();
        let list: &[FpPoly] = match kind.as_str() {
            "outline" | "outlines" => &self.outlines,
            "part" | "parts" => &self.parts,
            "catastro" => &self.catastro,
            "context" => &self.context,
            "osm" => {
                let hit: Vec<&FpPoly> = self.outlines.iter().chain(self.parts.iter()).filter(|p| p.id == arg || p.id.ends_with(&format!("/{arg}")) || p.id == format!("osm:{arg}")).collect();
                if hit.is_empty() {
                    let ids: Vec<&str> = self.outlines.iter().chain(self.parts.iter()).map(|p| p.id.as_str()).collect();
                    return Err(format!("no OSM outline or part with id `{arg}`{}; available: {}", super::parse::did_you_mean(arg, &ids), ids.join(", ")));
                }
                return Ok(hit);
            }
            _ => {
                return Err(format!(
                    "unknown polygon reference `{reference}`{}; use outline:<i|all>, part:<i|all>, osm:<way/id>, catastro:<id|i|all>, context:<i>, parent, or a shape",
                    super::parse::did_you_mean(&kind, &["outline", "part", "osm", "catastro", "context"])
                ));
            }
        };
        if arg == "all" || arg == "*" {
            if list.is_empty() {
                return Err(format!("footprint has no {kind} polygons"));
            }
            return Ok(list.iter().collect());
        }
        if let Ok(i) = arg.parse::<usize>() {
            return list.get(i).map(|p| vec![p]).ok_or_else(|| format!("`{reference}`: footprint has {} {kind} polygon(s) (0..{})", list.len(), list.len().saturating_sub(1)));
        }
        let hit: Vec<&FpPoly> = list.iter().filter(|p| p.id == arg || p.refcat.as_deref() == Some(arg)).collect();
        if hit.is_empty() {
            let ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
            return Err(format!("`{reference}`: no {kind} polygon with id `{arg}`{}", super::parse::did_you_mean(arg, &ids)));
        }
        Ok(hit)
    }

    /// Union bounding box of the outlines (or of everything).
    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let mut mn = Vec2::splat(f32::MAX);
        let mut mx = Vec2::splat(f32::MIN);
        let mut any = false;
        for p in self.outlines.iter().chain(self.parts.iter()).chain(self.catastro.iter()) {
            let (a, b) = poly::bbox(&p.shape.outer);
            mn = mn.min(a);
            mx = mx.max(b);
            any = true;
        }
        any.then_some((mn, mx))
    }
}

/// One reference photo from `sources.toml`.
#[derive(Clone, Debug, Default)]
pub struct Photo {
    pub file: String,
    pub title: String,
    pub url: String,
    pub author: String,
    pub license: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Landmark-local camera position [x, z].
    pub camera: Option<Vec2>,
    /// Compass heading of the camera (degrees).
    pub heading: Option<f32>,
}

/// The parts of `sources.toml` the tools use.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub photos: Vec<Photo>,
}

impl Sources {
    pub fn parse(text: &str) -> Result<Sources> {
        let v: toml::Table = toml::from_str(text).map_err(|e| Error::msg(format!("sources.toml: {}", e.message())))?;
        let mut photos = Vec::new();
        if let Some(toml::Value::Array(list)) = v.get("photo").or_else(|| v.get("photos")) {
            for p in list {
                let Some(t) = p.as_table() else { continue };
                let s = |k: &str| t.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                let num = |k: &str| t.get(k).and_then(|x| x.as_integer().map(|i| i as f64).or_else(|| x.as_float()));
                let camera = t.get("camera").and_then(|c| c.as_array()).and_then(|a| {
                    let f = |i: usize| a.get(i).and_then(|x| x.as_float().or_else(|| x.as_integer().map(|v| v as f64)));
                    Some(Vec2::new(f(0)? as f32, f(1)? as f32))
                });
                photos.push(Photo {
                    file: s("file"),
                    title: s("title"),
                    url: s("url"),
                    author: s("author"),
                    license: s("license"),
                    width: num("width").map(|v| v as u32),
                    height: num("height").map(|v| v as u32),
                    camera,
                    heading: num("heading").map(|v| v as f32),
                });
            }
        }
        Ok(Sources { photos })
    }

    pub fn load(path: &Path) -> Result<Sources> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::msg(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }

    pub fn photo(&self, file: &str) -> Option<&Photo> {
        let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
        self.photos.iter().find(|p| p.file == file || base(&p.file) == base(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = r#"{"slug":"t","frame":"utm30n-local-v1","origin":{"lat":39.4,"lon":-0.37,"E0":725000,"N0":4372000},
      "anchor":[120.5,-80.25],"attribution":["© OpenStreetMap contributors"],
      "outlines":[{"id":"way/1","tags":{"building":"church","height":"34 m"},"outer":[[0,0],[0,10],[20,10],[20,0]],"inners":[]}],
      "parts":[{"id":"way/2","tags":{"building:part":"tower","height":"60","min_height":"0"},"outer":[[0,0],[5,0],[5,5],[0,5]]}],
      "catastro":[{"id":"c1","refcat":"1234567YJ2713D","floors_above":3,"floors_below":null,"outer":[[0,0],[20,0],[20,10],[0,10]]}],
      "context":[{"source":"catastro","floors_above":"5","outer":[[30,0],[40,0],[40,10],[30,10]]}],
      "future_field": 1}"#;

    #[test]
    fn parses_and_resolves() {
        let f = Footprint::parse(FP, "fp").unwrap();
        assert_eq!(f.anchor, Vec2::new(120.5, -80.25));
        assert!(poly::area(&f.outlines[0].shape.outer) > 0.0, "normalised CCW");
        assert_eq!(f.outlines[0].tag_height(), Some(34.0));
        assert_eq!(f.parts[0].tag_height(), Some(60.0));
        assert_eq!(f.context[0].floors_above, Some(5.0));
        assert_eq!(f.resolve("osm:way/2").unwrap()[0].id, "way/2");
        assert_eq!(f.resolve("catastro:1234567YJ2713D").unwrap().len(), 1);
        assert!(f.resolve("outline:3").unwrap_err().contains("1 outline"));
        assert!(f.resolve("outlin:0").unwrap_err().contains("did you mean `outline`"));
    }

    #[test]
    fn nulls_and_bad_points_are_tolerated() {
        let t = r#"{"anchor":[1,2],"outlines":[{"id":"w","tags":null,"outer":[[0,0],[10,0],[10,"8"],[null,3],[0,8]],"inners":null}],"parts":null}"#;
        let f = Footprint::parse(t, "fp").unwrap();
        assert_eq!(f.outlines.len(), 1);
        assert_eq!(f.outlines[0].shape.outer.len(), 4);
        assert!(f.warnings.iter().any(|w| w.contains("1 point")), "{:?}", f.warnings);
    }

    #[test]
    fn levels_count_from_the_ground() {
        let t = r#"{"anchor":[0,0],"parts":[{"id":"p","tags":{"building:levels":"5","building:min_level":"3"},"outer":[[0,0],[4,0],[4,4],[0,4]]}]}"#;
        let f = Footprint::parse(t, "fp").unwrap();
        assert_eq!(f.parts[0].tag_height(), Some(16.0));
        assert_eq!(f.parts[0].tag_min_height(), Some(9.6));
    }

    #[test]
    fn sources_photos() {
        let s = Sources::parse("[[photo]]\nfile = \"photos/a.jpg\"\nwidth = 1600\nheight = 1200\ncamera = [10.0, 40]\nheading = 350\n[[photo]]\nfile = \"photos/b.jpg\"\n").unwrap();
        assert_eq!(s.photos.len(), 2);
        assert_eq!(s.photo("a.jpg").unwrap().camera, Some(Vec2::new(10.0, 40.0)));
    }
}
