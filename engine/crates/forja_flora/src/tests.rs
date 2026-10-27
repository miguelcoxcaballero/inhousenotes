use super::*;
use crate::mesh::{MeshBuilder, TubeOpts, tube, welded_edges_closed};

fn lib() -> SpeciesLibrary {
    SpeciesLibrary::builtin()
}

fn quick() -> Params {
    Params { bake_impostor: false, ..Default::default() }
}

#[test]
fn deterministic_by_seed() {
    let lib = lib();
    for id in ["platanus", "phoenix", "ficus"] {
        let sp = lib.get(id).unwrap();
        let a = generate(sp, 7, &quick());
        let b = generate(sp, 7, &quick());
        let c = generate(sp, 8, &quick());
        for k in 0..2 {
            assert_eq!(a.scene.meshes[0][k].positions, b.scene.meshes[0][k].positions, "{id}");
            assert_eq!(a.scene.meshes[0][k].indices, b.scene.meshes[0][k].indices, "{id}");
            assert_eq!(a.wind[k], b.wind[k]);
        }
        assert_ne!(a.scene.meshes[0][0].positions, c.scene.meshes[0][0].positions, "{id}: seeds must differ");
    }
}

#[test]
fn all_species_are_finite_and_within_lod_budgets() {
    let lib = lib();
    for sp in &lib.species {
        let t = generate(sp, 3, &quick());
        for (k, m) in t.scene.meshes[0].iter().enumerate() {
            assert!(m.positions.iter().all(|p| p.is_finite()), "{} NaN position", sp.id);
            assert!(m.normals.iter().all(|n| n.is_finite() && (n.length() - 1.0).abs() < 1e-3), "{} bad normal", sp.id);
            assert!(m.uvs.iter().all(|u| u.is_finite()), "{} NaN uv", sp.id);
            assert!(m.tangents.iter().all(|t| t.is_finite()), "{} NaN tangent", sp.id);
            assert_eq!(t.wind[k].len(), m.positions.len());
            assert!(t.wind[k].iter().all(|w| (0.0..=1.0).contains(w)));
            let n = m.positions.len() as u32;
            assert!(m.indices.iter().chain(m.lods.iter().flatten()).all(|i| *i < n));
        }
        let [near, mid, far] = t.lod_triangles;
        println!("{:14} near {near:6} mid {mid:5} far {far:4}  h {:.1} m  {} stems {} cards {} fruit {} fronds  {:.0} ms", sp.id, t.height, t.stats.stems, t.stats.cards, t.stats.fruits, t.stats.fronds, t.stats.ms);
        assert!((20_000..=80_000).contains(&near), "{} near LOD {near}", sp.id);
        assert!((3_000..=8_000).contains(&mid), "{} mid LOD {mid}", sp.id);
        assert_eq!(t.triangles(FOLIAGE, 2), 16, "{}: billboard is 8 quads", sp.id);
        // The bark reaches its far level (trunk stub) no later than the
        // crown reaches its billboard, so no limb pokes through it.
        if sp.kind != Kind::Palm {
            assert!(t.triangles(BARK, 1) >= t.triangles(FOLIAGE, 1).min(800), "{}: bark mid {} < foliage mid {}", sp.id, t.triangles(BARK, 1), t.triangles(FOLIAGE, 1));
        }
        assert!(t.height > 2.0 && t.height < 40.0);
        assert!(t.capsule.radius > 0.05 && t.capsule.b.y > t.capsule.a.y);
    }
}

#[test]
fn trunk_and_branch_tubes_are_closed() {
    let lib = lib();
    for id in ["platanus", "olivo", "ficus", "washingtonia"] {
        let sp = lib.get(id).unwrap();
        let g = Growth { height: sp.height[1], trunk_radius: sp.trunk.radius, age: 1.0, health: 1.0, pruning: Pruning::Natural, clearance: 0.0 };
        let mut rng = Rng::new(5);
        let sk = if sp.kind == Kind::Palm {
            palm::build(sp, sp.palm.as_ref().unwrap(), &g, &mut rng).skeleton
        } else if sp.model == Model::Colonization {
            skeleton::colonization(sp, &g, &mut rng)
        } else {
            skeleton::parametric(sp, &g, &mut rng)
        };
        let shape = TrunkShape::new(&sp.trunk, &mut Rng::new(1), 1.0);
        for (i, s) in sk.stems.iter().enumerate().take(40) {
            let mut mb = MeshBuilder::default();
            let o = TubeOpts { sides_per_m: 30.0, min_sides: 5, max_sides: 32, step: 1, max_seg: 0.3, tile: [1.0, 1.0] };
            tube(&mut mb, s, &o, if i == 0 { Some(&shape) } else { None }, &|_, _| 0.0);
            assert!(welded_edges_closed(&mb, 0..mb.idx.len()), "{id}: stem {i} ({:?}) not closed", s.kind);
        }
    }
}

#[test]
fn seasons_and_pruning_change_the_tree() {
    let lib = lib();
    let sp = lib.get("platanus").unwrap();
    let summer = generate(sp, 1, &quick());
    let winter = generate(sp, 1, &Params { season: Season::Winter, ..quick() });
    let pollard = generate(sp, 1, &Params { pruning: Some(Pruning::Pollard), ..quick() });
    assert!(winter.stats.cards < summer.stats.cards);
    assert!(pollard.height < summer.height);
    let jac = lib.get("jacaranda").unwrap();
    let bloom = generate(jac, 1, &Params { season: Season::Spring, ..quick() });
    // Bloom atlases differ from summer ones.
    let plain = generate(jac, 1, &quick());
    assert_ne!(bloom.scene.textures[FOLIAGE_ALBEDO].rgba8[..], plain.scene.textures[FOLIAGE_ALBEDO].rgba8[..]);
    let young = generate(sp, 1, &Params { age: 0.3, ..quick() });
    assert!(young.height < summer.height * 0.6);
}

#[test]
fn impostor_bake_fills_the_billboard_tiles() {
    let lib = lib();
    let sp = lib.get("naranjo").unwrap();
    let t = generate(sp, 2, &Params::default());
    let tex = &t.scene.textures[FOLIAGE_ALBEDO];
    let w = tex.width as usize;
    let mut covered = 0usize;
    for k in 0..8 {
        let r = texture::impostor_rect(k);
        let (x0, y0) = ((r[0] * w as f32) as usize, (r[1] * w as f32) as usize);
        let n = ((r[2] - r[0]) * w as f32) as usize;
        let c = (0..n * n).filter(|i| tex.rgba8[((y0 + i / n) * w + x0 + i % n) * 4 + 3] > 128).count();
        assert!(c > n * n / 20, "view {k} nearly empty: {c}");
        covered += c;
    }
    assert!(covered > 0);
}

#[test]
fn glb_export_loads_back() {
    let lib = lib();
    let t = generate(lib.get("naranjo").unwrap(), 4, &quick());
    let dir = std::env::temp_dir().join("forja_flora_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("naranjo.glb");
    gltf::write_glb(&t, &path).unwrap();
    let back = forja_assets::load_gltf(&path).unwrap();
    assert_eq!(back.meshes.len(), 3);
    assert_eq!(back.roots, vec![0]);
    assert_eq!(back.meshes[0].iter().map(|m| m.triangle_count()).sum::<usize>(), t.lod_triangles[0]);
    assert!(matches!(back.materials[1].alpha, AlphaMode::Mask(_)));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn city_planting_instances_species_variants() {
    let lib = lib();
    use forja_city::layout::{PropKind, PropSite, Setting};
    use forja_math::Mat4;
    let props: Vec<(PropKind, Mat4)> = (0..12).map(|i| (if i < 4 { PropKind::Palm } else if i < 6 { PropKind::Bench } else { PropKind::Tree }, Mat4::from_translation(Vec3::new(i as f32 * 9.0, 0.15, 0.0)))).collect();
    let sites: Vec<PropSite> = (0..12)
        .map(|i| PropSite { setting: if i < 4 { Setting::Avenue } else if i < 9 { Setting::Boulevard } else { Setting::NarrowStreet }, valencia: true, street_width: 20.0 })
        .collect();
    let f = city::plant(&props, &sites, &lib, &city::PlantOptions { variants: 2, ..Default::default() });
    assert_eq!(f.instances.len(), 10, "benches are skipped");
    assert_eq!(f.colliders.len(), 10);
    assert!(f.by_species.contains_key("naranjo"));
    assert!(f.models.len() <= 2 * 4);
    // Variants of one species share the impostor atlas buffer.
    let first = &f.models[0];
    for m in &f.models[1..] {
        if m.species == first.species && m.params.pruning == first.params.pruning {
            assert!(std::sync::Arc::ptr_eq(&m.scene.textures[FOLIAGE_ALBEDO].rgba8, &first.scene.textures[FOLIAGE_ALBEDO].rgba8));
        }
    }
}
