//! Planting the procedural city with generated trees.
//!
//! [`plant`] turns the layout's tree and palm props into instances of a
//! few generated variants per species: species follow the street type
//! (plane trees on boulevards, orange trees in plazas and narrow streets,
//! palms on the avenue median and the port), each `(species, variant)`
//! pair is one model so the renderer instances it with a single draw per
//! LOD, and all variants of a species share one impostor atlas.

use std::collections::BTreeMap;

use forja_city::layout::{PropKind, PropSite, Setting};
use forja_math::{Mat4, Quat};

use crate::rng::{Rng, hash_str};
use crate::{Billboard, Capsule, Params, Pruning, Season, SpeciesLibrary, Stats, TreeAsset, generate, impostor};

#[derive(Clone, Debug)]
pub struct PlantOptions {
    /// Generated variants per species.
    pub variants: usize,
    pub season: Season,
    pub seed: u64,
}

impl Default for PlantOptions {
    fn default() -> Self {
        PlantOptions { variants: 3, season: Season::Summer, seed: 1 }
    }
}

/// Generated trees plus where they stand.
#[derive(Default)]
pub struct CityForest {
    /// One asset per `(species, variant)`.
    pub models: Vec<TreeAsset>,
    /// `(model index, world transform)`.
    pub instances: Vec<(usize, Mat4)>,
    /// World-space trunk capsules.
    pub colliders: Vec<Capsule>,
    pub by_species: BTreeMap<String, usize>,
}

/// Species and pruning for a site; `pick` is a stable per-site random number.
pub fn species_for(kind: PropKind, site: &PropSite, pick: f32) -> Option<(&'static str, Pruning)> {
    Some(match (kind, site.setting) {
        (PropKind::Bench, _) => return None,
        (PropKind::Palm, Setting::Plaza) => ("phoenix", Pruning::Trimmed),
        (PropKind::Palm, Setting::Port) => (if pick < 0.7 { "washingtonia" } else { "phoenix" }, Pruning::Trimmed),
        (PropKind::Palm, _) => (if pick < 0.55 { "washingtonia" } else { "phoenix" }, Pruning::Trimmed),
        (PropKind::Tree, Setting::Plaza | Setting::NarrowStreet) => ("naranjo", Pruning::Lollipop),
        (PropKind::Tree, Setting::Boulevard) => {
            if pick < 0.7 {
                ("platanus", Pruning::Lifted)
            } else if pick < 0.85 {
                ("almez", Pruning::Lifted)
            } else {
                ("jacaranda", Pruning::Lifted)
            }
        }
        (PropKind::Tree, Setting::Street) => {
            if pick < 0.4 {
                ("naranjo", Pruning::Lollipop)
            } else if pick < 0.7 {
                ("almez", Pruning::Lifted)
            } else {
                ("jacaranda", Pruning::Lifted)
            }
        }
        (PropKind::Tree, Setting::Avenue) => ("platanus", Pruning::Lifted),
        (PropKind::Tree, Setting::Port) => ("washingtonia", Pruning::Trimmed),
        (PropKind::Tree, Setting::Park) => match (pick * 5.0) as u32 {
            0 => ("ficus", Pruning::Natural),
            1 => ("pino_pinonero", Pruning::Natural),
            2 => ("olivo", Pruning::Natural),
            3 => ("jacaranda", Pruning::Natural),
            _ => ("cipres", Pruning::Natural),
        },
    })
}

/// Stretches of street keep one species (as cities plant them): the pick
/// is hashed from a 36 m cell, not from the single tree.
fn site_pick(m: &Mat4, site: &PropSite, seed: u64) -> f32 {
    let p = m.w_axis.truncate();
    let key = match site.setting {
        Setting::Boulevard | Setting::Street | Setting::NarrowStreet => format!("{},{}", (p.x / 36.0).floor() as i64, (p.z / 36.0).floor() as i64),
        _ => format!("{:.0},{:.0}", p.x, p.z),
    };
    Rng::new(hash_str(&key) ^ seed).f32()
}

/// Generates the trees for a city layout's props (benches are skipped).
pub fn plant(props: &[(PropKind, Mat4)], sites: &[PropSite], lib: &SpeciesLibrary, opt: &PlantOptions) -> CityForest {
    let mut wanted: Vec<(usize, &'static str, Pruning)> = Vec::new();
    for (i, (kind, m)) in props.iter().enumerate() {
        let site = sites.get(i).copied().unwrap_or_default();
        if let Some((sp, pr)) = species_for(*kind, &site, site_pick(m, &site, opt.seed)) {
            wanted.push((i, sp, pr));
        }
    }
    // Unique (species, pruning) groups in a stable order.
    let mut groups: Vec<(&'static str, Pruning)> = Vec::new();
    for (_, sp, pr) in &wanted {
        if !groups.contains(&(*sp, *pr)) {
            groups.push((*sp, *pr));
        }
    }
    let variants = opt.variants.max(1);
    let key = forest_key(&groups, lib, opt);
    let t0 = std::time::Instant::now();
    let cached = forja_assets::cache::load("trees", &key).and_then(|b| forja_assets::cache::Reader::new(&b)).and_then(read_forest).filter(|b| b.len() == groups.len());
    let built = match cached {
        Some(b) => {
            log::info!("trees: {} models from the cache in {:.1}s", b.iter().map(Vec::len).sum::<usize>(), t0.elapsed().as_secs_f32());
            b
        }
        None => {
            let b = grow(&groups, lib, opt, variants);
            if let Some(bytes) = write_forest(&b) {
                forja_assets::cache::store("trees", &key, &bytes);
            }
            log::info!("trees: {} models grown in {:.1}s", b.iter().map(Vec::len).sum::<usize>(), t0.elapsed().as_secs_f32());
            b
        }
    };
    let mut forest = CityForest::default();
    let mut first = Vec::new();
    for (g, list) in groups.iter().zip(built) {
        first.push(forest.models.len());
        forest.by_species.entry(g.0.to_string()).or_insert(forest.models.len());
        forest.models.extend(list);
    }
    place(&mut forest, &first, &groups, wanted, props, opt);
    forest
}

/// Generates the variants of every `(species, pruning)` group.
fn grow(groups: &[(&'static str, Pruning)], lib: &SpeciesLibrary, opt: &PlantOptions, variants: usize) -> Vec<Vec<TreeAsset>> {
    // Species one after another (each builds its textures with all cores):
    // generating several atlases at once needs a lot of memory.
    let built: Vec<Vec<TreeAsset>> = groups
        .iter()
        .map(|(sp_id, pr)| {
            let Some(sp) = lib.get(sp_id) else { return Vec::new() };
            let mut out = Vec::new();
            for v in 0..variants {
                let mut rng = Rng::new(opt.seed ^ hash_str(sp_id) ^ (v as u64 * 7919));
                let params = Params {
                    age: rng.range(0.8, 1.1),
                    size: rng.range(0.9, 1.05),
                    season: opt.season,
                    health: rng.range(0.85, 1.0),
                    pruning: Some(*pr),
                    bake_impostor: v == 0,
                };
                let mut t = generate(sp, opt.seed.wrapping_add(v as u64 * 101), &params);
                if v > 0 {
                    impostor::share(&mut t, &out[0]);
                }
                out.push(t);
            }
            out
        })
        .collect();
    crate::clear_texture_cache();
    built
}

fn place(forest: &mut CityForest, first: &[usize], groups: &[(&'static str, Pruning)], wanted: Vec<(usize, &'static str, Pruning)>, props: &[(PropKind, Mat4)], opt: &PlantOptions) {
    for (i, sp, pr) in wanted {
        let gi = groups.iter().position(|g| *g == (sp, pr)).unwrap();
        let n = if gi + 1 < first.len() { first[gi + 1] - first[gi] } else { forest.models.len() - first[gi] };
        if n == 0 {
            continue;
        }
        let m = props[i].1;
        let p = m.w_axis.truncate();
        let r = Rng::new(hash_str(&format!("{:.2},{:.2}", p.x, p.z)) ^ opt.seed).next_u64();
        let model = first[gi] + (r as usize) % n;
        // Random yaw per instance (the variant already varies the shape).
        let yaw = Quat::from_rotation_y((r >> 20) as f32 / (1u64 << 44) as f32 * std::f32::consts::TAU);
        let mt = Mat4::from_translation(p) * Mat4::from_quat(yaw);
        let c = forest.models[model].capsule;
        forest.colliders.push(Capsule { a: mt.transform_point3(c.a), b: mt.transform_point3(c.b), radius: c.radius });
        forest.instances.push((model, mt));
    }
}

/// Cache key of a forest: the tree generator's code, the species
/// definitions used and the planting options.
fn forest_key(groups: &[(&'static str, Pruning)], lib: &SpeciesLibrary, opt: &PlantOptions) -> [u8; 32] {
    let mut k = forja_assets::cache::Key::new().str("trees 1");
    for src in [
        include_str!("city.rs"),
        include_str!("foliage.rs"),
        include_str!("impostor.rs"),
        include_str!("lib.rs"),
        include_str!("mesh.rs"),
        include_str!("palm.rs"),
        include_str!("rng.rs"),
        include_str!("skeleton.rs"),
        include_str!("species.rs"),
        include_str!("texture.rs"),
        include_str!("../../forja_assets/src/lod.rs"),
    ] {
        k = k.str(src);
    }
    k = k.u64(opt.variants as u64).u64(opt.seed).str(&format!("{:?}", opt.season));
    for (sp, pr) in groups {
        k = k.str(sp).str(&format!("{pr:?}")).str(&format!("{:?}", lib.get(sp)));
    }
    k.finish()
}

const SEASONS: [Season; 4] = [Season::Summer, Season::Spring, Season::Autumn, Season::Winter];
const PRUNINGS: [Pruning; 5] = [Pruning::Natural, Pruning::Pollard, Pruning::Lollipop, Pruning::Lifted, Pruning::Trimmed];

fn write_forest(built: &[Vec<TreeAsset>]) -> Option<Vec<u8>> {
    let mut w = forja_assets::cache::Writer::new();
    w.usize(built.len());
    for list in built {
        w.usize(list.len());
        for t in list {
            w.str(&t.species);
            w.u64(t.seed);
            let p = &t.params;
            w.f32(p.age);
            w.f32(p.size);
            w.u8(SEASONS.iter().position(|s| *s == p.season)? as u8);
            w.f32(p.health);
            w.u8(p.pruning.map_or(Some(255), |pr| PRUNINGS.iter().position(|x| *x == pr).map(|i| i as u8))?);
            w.bool(p.bake_impostor);
            if !w.scene(&t.scene) {
                return None;
            }
            w.usize(t.wind.len());
            for v in &t.wind {
                w.usize(v.len());
                w.f32s(v);
            }
            w.vec3(t.capsule.a);
            w.vec3(t.capsule.b);
            w.f32(t.capsule.radius);
            w.f32(t.height);
            w.vec3(t.crown_center);
            w.vec3(t.crown_radii);
            for n in t.lod_triangles {
                w.usize(n);
            }
            w.vec3(t.billboard.center);
            w.f32(t.billboard.half);
            let st = &t.stats;
            for n in [st.stems, st.cards, st.fruits, st.fronds] {
                w.usize(n);
            }
            w.f32(st.ms);
        }
    }
    Some(w.finish())
}

fn read_forest(mut r: forja_assets::cache::Reader) -> Option<Vec<Vec<TreeAsset>>> {
    let n = r.usize()?;
    let mut out = Vec::new();
    for _ in 0..n.min(4096) {
        let m = r.usize()?;
        let mut list = Vec::new();
        for _ in 0..m.min(4096) {
            let species = r.str()?;
            let seed = r.u64()?;
            let (age, size) = (r.f32()?, r.f32()?);
            let season = *SEASONS.get(r.u8()? as usize)?;
            let health = r.f32()?;
            let pruning = match r.u8()? {
                255 => None,
                i => Some(*PRUNINGS.get(i as usize)?),
            };
            let params = Params { age, size, season, health, pruning, bake_impostor: r.bool()? };
            let scene = r.scene()?;
            let nw = r.usize()?;
            let mut wind = Vec::new();
            for _ in 0..nw.min(64) {
                let k = r.usize()?;
                wind.push((0..k.min(1 << 26)).map(|_| r.f32()).collect::<Option<Vec<f32>>>()?);
            }
            let capsule = Capsule { a: r.vec3()?, b: r.vec3()?, radius: r.f32()? };
            let height = r.f32()?;
            let (crown_center, crown_radii) = (r.vec3()?, r.vec3()?);
            let lod_triangles = [r.usize()?, r.usize()?, r.usize()?];
            let billboard = Billboard { center: r.vec3()?, half: r.f32()? };
            let stats = Stats { stems: r.usize()?, cards: r.usize()?, fruits: r.usize()?, fronds: r.usize()?, ms: r.f32()? };
            list.push(TreeAsset { species, seed, params, scene, wind, capsule, height, crown_center, crown_radii, lod_triangles, billboard, stats });
        }
        out.push(list);
    }
    r.at_end().then_some(out)
}

impl CityForest {
    /// Instance counts per model (for stats).
    pub fn counts(&self) -> Vec<usize> {
        let mut c = vec![0; self.models.len()];
        for (m, _) in &self.instances {
            c[*m] += 1;
        }
        c
    }

    pub fn height_of(&self, model: usize) -> f32 {
        self.models[model].height
    }
}

