//! Street profiles: what a street *is* (cross-section, paving, dressing,
//! traffic control) and what it allows the facades facing it
//! (`FrontRules`), as data in `data/city/streets.toml`.
//!
//! Nothing downstream knows about alleys, boulevards or Ensanche streets by
//! name: a 3 m callejón has `carriageway = 0` and wall lamps, so the ground
//! becomes a shared cobbled surface, no paint or poles are planted and the
//! facades get tight limits. All of that is numbers in this table.

use std::path::Path;

use serde::Deserialize;

use crate::kit::Mat;
use crate::lot::FrontRules;

pub const BUILTIN: &str = include_str!("../../../data/city/streets.toml");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ProfileId(pub u16);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Avenue,
    Street,
    Lane,
    Alley,
    Plaza,
    Limit,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Parking {
    #[default]
    None,
    One,
    Both,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Markings {
    #[default]
    None,
    /// Dashed centre line.
    Centre,
    /// Lane arrows only.
    OneWay,
    /// Parking bays on both sides, a dashed divider and arrows.
    ParkingOneWay,
    /// Several lanes each way with edge lines along a median.
    Avenue,
    /// A single yellow no-parking line along one kerb.
    KerbLine,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    #[default]
    None,
    Signs,
    Signals,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Gutter {
    #[default]
    None,
    Centre,
    Sides,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mount {
    #[default]
    None,
    Pole,
    Candelabra,
    Road,
    /// Lanterns on the facades (`Front.lamps`).
    Wall,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Lamps {
    #[serde(default)]
    pub mount: Mount,
    #[serde(default = "d_pitch")]
    pub pitch: f32,
}

fn d_pitch() -> f32 {
    20.0
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Bollards {
    #[default]
    None,
    /// A row along the kerb (Madrid).
    Kerb,
    /// Across the mouth where this vehicle-free street meets a car street.
    Mouth,
}

/// Material names of the three ground surfaces of a profile.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Surfaces {
    pub carriage: String,
    pub walk: String,
    pub shared: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub class: Class,
    /// Style pool key in a zone (`street` when absent).
    #[serde(default)]
    pub pool: Option<String>,
    /// Hierarchy: higher wins at collinear duplicates and has priority at
    /// junctions.
    pub rank: u8,
    /// Building line to building line (m): the range a generator or a
    /// measurement picks from.
    pub width: [f32; 2],
    /// Roadway width (m); 0 means a shared surface with no kerbs.
    #[serde(default)]
    pub carriageway: f32,
    #[serde(default)]
    pub vehicles: bool,
    #[serde(default)]
    pub median: f32,
    #[serde(default)]
    pub lanes: u8,
    #[serde(default)]
    pub parking: Parking,
    #[serde(default)]
    pub markings: Markings,
    /// Zebra crossings at the mouths.
    #[serde(default)]
    pub crossings: bool,
    #[serde(default)]
    pub control: Control,
    pub surface: Surfaces,
    /// Height of the raised sidewalk (m).
    #[serde(default = "d_kerb")]
    pub kerb: f32,
    /// Height of a shared surface (m).
    #[serde(default)]
    pub shared_height: f32,
    /// Narrowest sidewalk the street keeps (m), for measured widths.
    #[serde(default)]
    pub walk_min: f32,
    #[serde(default)]
    pub kerb_radius: f32,
    #[serde(default)]
    pub gutter: Gutter,
    /// Chaflán length at block corners (m).
    #[serde(default)]
    pub chamfer: f32,
    /// Chance that a corner is a fillet instead of a chord.
    #[serde(default)]
    pub round_corners: f32,
    #[serde(default)]
    pub lamps: Lamps,
    /// Tree pitch (m), 0 for none.
    #[serde(default)]
    pub trees: f32,
    #[serde(default)]
    pub palms: bool,
    #[serde(default)]
    pub bollards: Bollards,
    /// Litter bin pitch (m).
    #[serde(default)]
    pub bins: f32,
    #[serde(default)]
    pub containers: bool,
    /// Bench pitch (m).
    #[serde(default)]
    pub benches: f32,
    #[serde(default)]
    pub terraces: bool,
    /// Hanging lanterns and laundry across the street.
    #[serde(default)]
    pub catenary: bool,
    /// Painted bike lane along the median (boulevards).
    #[serde(default)]
    pub bike: bool,
    #[serde(default)]
    pub speed: Option<u8>,
    #[serde(default)]
    pub front: FrontRules,
}

fn d_kerb() -> f32 {
    0.15
}

impl Profile {
    /// Typical width (m).
    pub fn default_width(&self) -> f32 {
        (self.width[0] + self.width[1]) * 0.5
    }

    pub fn pool(&self) -> &str {
        self.pool.as_deref().unwrap_or(match self.class {
            Class::Alley | Class::Lane => "alley",
            Class::Plaza => "plaza",
            Class::Avenue => "avenue",
            _ => "street",
        })
    }

    /// Whether the street has a roadway with kerbs.
    pub fn has_carriage(&self) -> bool {
        self.carriageway > 0.0
    }

    pub fn carriage_mat(&self) -> Mat {
        mat(&self.surface.carriage).unwrap_or(Mat::Asphalt)
    }

    pub fn walk_mat(&self) -> Mat {
        mat(&self.surface.walk).unwrap_or(Mat::SidewalkSlab)
    }

    pub fn shared_mat(&self) -> Mat {
        mat(&self.surface.shared).unwrap_or(Mat::Cobble)
    }
}

/// Ground material by name.
pub fn mat(name: &str) -> Option<Mat> {
    Some(match name {
        "Asphalt" => Mat::Asphalt,
        "Cobble" => Mat::Cobble,
        "Granite" => Mat::Granite,
        "SidewalkHex" => Mat::SidewalkHex,
        "SidewalkSlab" => Mat::SidewalkSlab,
        "Curb" => Mat::Curb,
        "Grass" => Mat::Grass,
        "Soil" => Mat::Soil,
        _ => return None,
    })
}

/// How an OSM `highway` class maps to profiles by measured clearance.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OsmRule {
    pub highway: Vec<String>,
    /// `(profile, clearance upper bound)` in increasing order.
    pub profiles: Vec<(String, f32)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    profile: Vec<Profile>,
    #[serde(default)]
    osm: Vec<OsmRule>,
}

#[derive(Clone, Debug)]
pub struct ProfileLibrary {
    pub list: Vec<Profile>,
    pub osm: Vec<OsmRule>,
}

impl ProfileLibrary {
    pub fn parse(text: &str) -> Result<Self, String> {
        let f: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let lib = ProfileLibrary { list: f.profile, osm: f.osm };
        for p in &lib.list {
            for m in [&p.surface.carriage, &p.surface.walk, &p.surface.shared] {
                if mat(m).is_none() {
                    return Err(format!("profile {}: unknown surface {m}", p.id));
                }
            }
            if !(p.width[0] >= 0.0 && p.width[0] <= p.width[1]) || p.carriageway < 0.0 || p.carriageway > p.width[1] {
                return Err(format!("profile {}: bad widths", p.id));
            }
            if lib.list.iter().filter(|q| q.id == p.id).count() > 1 {
                return Err(format!("profile {} defined twice", p.id));
            }
        }
        for r in &lib.osm {
            for (name, _) in &r.profiles {
                if lib.id(name).is_none() {
                    return Err(format!("osm rule names unknown profile {name}"));
                }
            }
        }
        if lib.list.len() >= u16::MAX as usize {
            return Err("too many profiles".into());
        }
        Ok(lib)
    }

    pub fn builtin() -> Self {
        Self::parse(BUILTIN).expect("built-in street profiles parse")
    }

    /// `data/city/streets.toml` from disk, else the built-in copy.
    pub fn load(path: Option<&Path>) -> Self {
        path.and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| Self::parse(&t).map_err(|e| log::warn!("streets: {e}")).ok())
            .unwrap_or_else(Self::builtin)
    }

    pub fn id(&self, name: &str) -> Option<ProfileId> {
        self.list.iter().position(|p| p.id == name).map(|i| ProfileId(i as u16))
    }

    pub fn get(&self, id: ProfileId) -> &Profile {
        &self.list[id.0 as usize]
    }

    pub fn by_name(&self, name: &str) -> Option<&Profile> {
        self.id(name).map(|i| self.get(i))
    }

    /// The profile for an OSM `highway` with this measured clearance.
    pub fn by_clearance(&self, highway: &str, clear: f32) -> Option<ProfileId> {
        let rule = self.osm.iter().find(|r| r.highway.iter().any(|h| h == highway))?;
        let name = rule.profiles.iter().find(|(_, up)| clear <= *up).or(rule.profiles.last())?;
        self.id(&name.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_parse_and_are_consistent() {
        let lib = ProfileLibrary::builtin();
        for name in ["bulevar", "avenida", "calle_ancha", "ensanche", "madrid_calle", "madrid_borde", "ronda", "calle_vieja", "callejuela_aceras", "callejuela", "callejon", "calle_peatonal", "plaza", "limit"] {
            assert!(lib.id(name).is_some(), "{name}");
        }
        let alley = lib.by_name("callejon").unwrap();
        assert_eq!(alley.carriageway, 0.0);
        assert_eq!(alley.lamps.mount, Mount::Wall);
        assert!(!alley.vehicles);
        let ens = lib.by_name("ensanche").unwrap();
        assert!(ens.vehicles && ens.chamfer > 0.0 && ens.control == Control::Signals);
        // Vehicle streets have room for their roadway.
        for p in &lib.list {
            assert!(!p.vehicles || p.carriageway > 0.0 || p.class == Class::Lane, "{}", p.id);
        }
        assert_eq!(lib.by_clearance("footway", 3.0), lib.id("callejon"));
        assert_eq!(lib.by_clearance("footway", 5.0), lib.id("callejuela"));
        assert_eq!(lib.by_clearance("footway", 50.0), lib.id("calle_peatonal"));
        assert!(ProfileLibrary::parse("[[profile]]\nid='x'\nclass='street'\nrank=1\nwidth=[3.0,4.0]\nsurface={carriage='Mud',walk='Cobble',shared='Cobble'}").is_err());
    }
}
