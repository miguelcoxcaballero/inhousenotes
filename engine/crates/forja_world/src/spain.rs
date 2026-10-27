//! Procedural Spanish city (Madrid barrio + Valencian Ensanche) from
//! `forja_city`, with procedural trees from `forja_flora` (or the scanned
//! library tree) and scanned benches from the content library.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forja_city::layout::{CityConfig, PropKind, build_city};
use forja_city::{Kit, StyleLibrary};
use forja_math::{Iso, Mat4, Quat, Vec3};
use forja_physics::{ColliderDesc, CollisionGroups, Shape, TriMesh, layers};

use crate::library::ContentLibrary;
use crate::{SpawnKind, SpawnPoint, WorldDescription, WorldError, WorldSource};

/// Where the city's trees come from.
#[derive(Clone, Debug)]
pub enum CityTrees {
    /// The scanned library tree for every tree and palm (the old path).
    Scanned,
    /// `forja_flora` species chosen by street type, a few variants each.
    Procedural { variants: usize, season: forja_flora::Season, species: Option<PathBuf> },
}

impl Default for CityTrees {
    fn default() -> Self {
        CityTrees::Procedural { variants: 3, season: forja_flora::Season::Summer, species: None }
    }
}

pub struct SpanishCity {
    /// `assets/downloaded` (scanned materials and props); optional.
    pub downloaded: Option<PathBuf>,
    /// `data/city/styles.toml`; the built-in styles are used without it.
    pub styles: Option<PathBuf>,
    pub config: CityConfig,
    pub trees: CityTrees,
    /// Clock hour the content is built for: at night lamps, windows, shops
    /// and signs are lit and their lights fill
    /// [`WorldDescription::night_lights`]. `None` builds the daytime city.
    pub hour: Option<f32>,
}

impl SpanishCity {
    pub fn new(downloaded: impl AsRef<Path>, styles: impl AsRef<Path>) -> Self {
        let styles = styles.as_ref().to_path_buf();
        // data/city/styles.toml -> data/flora/species.toml
        let species = styles.parent().and_then(|p| p.parent()).map(|d| d.join("flora/species.toml")).filter(|p| p.exists());
        Self {
            downloaded: Some(downloaded.as_ref().to_path_buf()),
            styles: Some(styles),
            config: CityConfig::default(),
            trees: CityTrees::Procedural { variants: 3, season: forja_flora::Season::Summer, species },
            hour: None,
        }
    }

    /// A built-in city plan (`default`, `classic`, `carmen`, `austrias`,
    /// `organic`, `small`).
    pub fn preset(downloaded: impl AsRef<Path>, styles: impl AsRef<Path>, name: &str) -> Option<Self> {
        Some(Self { config: CityConfig::preset(name)?, ..Self::new(downloaded, styles) })
    }

    /// A city on a real OpenStreetMap extract (`map.json` or `.osm`):
    /// real streets with measured widths, real footprints as lots.
    pub fn config_from_osm(path: &Path) -> Result<CityConfig, WorldError> {
        let text = std::fs::read_to_string(path).map_err(|e| WorldError::Io(format!("{}: {e}", path.display())))?;
        let data = crate::osm::parse_any(&text)?;
        let lib = forja_city::profile::ProfileLibrary::builtin();
        let draft = crate::osm_city::draft(&data, &lib, &crate::osm_city::OsmOptions::default())?;
        Ok(CityConfig::from_draft(draft, crate::osm_city::city_of(&data)))
    }

    /// The Spanish city on an OpenStreetMap extract.
    pub fn from_osm(downloaded: Option<&Path>, styles: Option<&Path>, osm: &Path) -> Result<Self, WorldError> {
        Ok(Self { downloaded: downloaded.map(Path::to_path_buf), styles: styles.map(Path::to_path_buf), config: Self::config_from_osm(osm)?, trees: CityTrees::default(), hour: None })
    }
}

impl WorldSource for SpanishCity {
    fn name(&self) -> String {
        "Spanish city (procedural)".into()
    }

    fn build(&self) -> Result<WorldDescription, WorldError> {
        let mut kit = Kit::load(self.downloaded.as_deref());
        if let Some(h) = self.hour {
            kit.set_hour(h);
        }
        let styles = StyleLibrary::load(self.styles.as_deref());
        let city = build_city(&kit, &styles, &self.config);
        let mut world = WorldDescription::new("Ciudad española");
        let out = city.out;
        for s in out.scenes {
            world.register_scene(s);
        }
        // Placements first: the traffic lights index them.
        world.placements = out.placements;
        world.traffic = Some(city.traffic.clone());
        let groups = CollisionGroups::new(layers::WORLD, u32::MAX);
        let ext = city.extent;
        world.colliders.push((ColliderDesc::new(Shape::cuboid(Vec3::new(ext.x, 1.0, ext.y))).translation(Vec3::new(0.0, -1.0, 0.0)).friction(1.0).groups(groups), Iso::IDENTITY));
        if !out.col_tris.is_empty() {
            world.colliders.push((ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(out.col_verts, out.col_tris)))).friction(0.8).groups(groups), Iso::IDENTITY));
        }
        // Procedural trees: one model per species variant, instanced.
        let procedural = matches!(self.trees, CityTrees::Procedural { .. });
        if let CityTrees::Procedural { variants, season, species } = &self.trees {
            let lib = forja_flora::SpeciesLibrary::load(species.as_deref());
            let opt = forja_flora::city::PlantOptions { variants: *variants, season: *season, seed: self.config.seed };
            let forest = forja_flora::city::plant(&city.props, &city.prop_sites, &lib, &opt);
            let ids: Vec<usize> = forest.models.into_iter().map(|t| world.register_scene(Arc::new(t.scene))).collect();
            for (m, t) in &forest.instances {
                world.place(ids[*m], *t);
            }
            for c in &forest.colliders {
                let half = ((c.b - c.a).length() * 0.5).max(0.05);
                world.colliders.push((ColliderDesc::new(Shape::capsule(half, c.radius)).groups(groups), Iso::from_pos((c.a + c.b) * 0.5)));
            }
            world.stats.push(("trees".into(), forest.instances.len()));
            world.stats.push(("tree models".into(), ids.len()));
        }
        // Scanned trees (old path) and benches.
        if let Some(dl) = &self.downloaded {
            let lib = ContentLibrary::load(dl);
            let tree = if procedural { None } else { lib.prop("polyhaven_island_tree").map(|s| world.register_scene(s)) };
            let bench = lib.prop("polyhaven_street_bench").map(|s| world.register_scene(s));
            for (kind, m) in &city.props {
                match kind {
                    PropKind::Tree | PropKind::Palm => {
                        if let Some(t) = tree {
                            world.place(t, *m * Mat4::from_scale(Vec3::splat(1.5)));
                            let p = m.w_axis.truncate();
                            world.colliders.push((ColliderDesc::new(Shape::capsule(1.5, 0.3)).groups(groups), Iso::from_pos(p + Vec3::Y * 1.8)));
                        }
                    }
                    PropKind::Bench => {
                        if let Some(b) = bench {
                            world.place(b, *m);
                        }
                    }
                }
            }
        }
        for s in &city.spawns {
            world.spawns.push(SpawnPoint { kind: if s.vehicle { SpawnKind::Vehicle } else { SpawnKind::Player }, iso: Iso::new(s.pos, Quat::from_rotation_y(s.yaw)) });
        }
        world.lights = out.lights;
        world.night_lights = out.city_lights;
        world.extent = ext;
        let mut stats = city.stats;
        stats.append(&mut world.stats);
        world.stats = stats;
        world.attribution.push("Materials: Poly Haven (CC0). Buildings, streets and furniture generated by forja_city; trees by forja_flora.".into());
        if self.config.plan.given.as_ref().is_some_and(|d| d.footprints.iter().any(|f| f.osm != 0)) {
            world.attribution.push("Map data © OpenStreetMap contributors, ODbL 1.0".into());
        }
        Ok(world)
    }
}
