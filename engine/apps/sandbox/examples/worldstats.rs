//! Geometry and texture budget of a map, without a GPU:
//! `cargo run --release -p sandbox --example worldstats -- [spain|kit|osm|track]`.
//!
//! Prints unique and instanced triangles, the heaviest scenes, LOD
//! coverage, the placement size distribution and texture memory (RGBA8 with
//! mips, as the renderer uploads them).

use std::collections::HashMap;
use std::path::PathBuf;

use forja_world::{KitCity, OsmWorld, SpanishCity, TestTrack, WorldSource};

fn main() {
    let map = std::env::args().nth(1).unwrap_or_else(|| "spain".into());
    let root = forja_assets::manifest::find_engine_root().unwrap_or_else(|| PathBuf::from("."));
    let dl = root.join("assets/downloaded");
    let t = std::time::Instant::now();
    let w = match map.as_str() {
        "kit" => KitCity::new(&dl).build(),
        "osm" => OsmWorld::new(dl.join("map_miami_beach_osm/map.osm")).with_content(&dl).build(),
        "track" => TestTrack::default().build(),
        _ => SpanishCity::new(&dl, root.join("data/city/styles.toml")).build(),
    }
    .expect("world");
    println!("{map}: built in {:.1}s, {} scenes, {} placements", t.elapsed().as_secs_f32(), w.scenes.len(), w.placements.len());
    let mut uses = vec![0usize; w.scenes.len()];
    for (s, _) in &w.placements {
        uses[*s] += 1;
    }
    let (mut uniq_tris, mut uniq_verts, mut inst_tris, mut lod_scenes, mut lod_tris) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut rows = Vec::new();
    for (i, s) in w.scenes.iter().enumerate() {
        let tris = s.triangle_count();
        let verts: usize = s.meshes.iter().flatten().map(|m| m.positions.len()).sum();
        let has_lod = s.meshes.iter().flatten().any(|m| !m.lods.is_empty());
        uniq_tris += tris;
        uniq_verts += verts;
        inst_tris += tris * uses[i];
        if has_lod {
            lod_scenes += 1;
            lod_tris += tris * uses[i];
        }
        rows.push((tris * uses[i], tris, uses[i], s.name.clone(), has_lod));
    }
    // Vertex layout of the renderer: 48 B + 4 B per index.
    let mesh_mb = (uniq_verts * 48 + uniq_tris * 12) as f64 / 1048576.0;
    println!("unique: {:.2} M tris, {:.2} M verts, ~{mesh_mb:.0} MB of vertex+index buffers", uniq_tris as f64 / 1e6, uniq_verts as f64 / 1e6);
    println!("instanced: {:.1} M tris; scenes with LODs: {lod_scenes} ({:.1}% of instanced tris)", inst_tris as f64 / 1e6, lod_tris as f64 * 100.0 / inst_tris.max(1) as f64);
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    println!("heaviest (instanced tris, tris/instance, instances, lod):");
    for r in rows.iter().take(25) {
        println!("  {:>10} {:>8} x{:<6} {} {}", r.0, r.1, r.2, r.4, r.3);
    }
    // LOD chains (triangles per level) of the three heaviest scenes.
    for r in rows.iter().take(3) {
        let Some(s) = w.scenes.iter().find(|s| s.name == r.3) else { continue };
        for m in s.meshes.iter().flatten() {
            let chain: Vec<usize> = std::iter::once(m.indices.len() / 3).chain(m.lods.iter().map(|l| l.len() / 3)).collect();
            println!("    lods {}: {chain:?}", r.3);
        }
    }
    // By name prefix.
    let mut by: HashMap<String, (usize, usize, usize)> = HashMap::new();
    for r in &rows {
        let k: String = r.3.split(['_', '-', ' ']).next().unwrap_or("").to_string();
        let e = by.entry(k).or_default();
        e.0 += r.0;
        e.1 += r.2;
        e.2 += r.1;
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    println!("by kind (instanced tris, instances, unique tris):");
    for (k, v) in by.iter().take(20) {
        println!("  {k:<20} {:>10} {:>7} {:>9}", v.0, v.1, v.2);
    }
    // Textures (shared buffers counted once).
    let mut seen = std::collections::HashSet::new();
    let (mut tex_bytes, mut tex_n) = (0u64, 0);
    let mut sizes: HashMap<(u32, u32), usize> = HashMap::new();
    for s in &w.scenes {
        for t in &s.textures {
            if seen.insert(t.rgba8.as_ptr() as usize) {
                tex_n += 1;
                tex_bytes += t.width as u64 * t.height as u64 * 4 * 4 / 3;
                *sizes.entry((t.width, t.height)).or_default() += 1;
            }
        }
    }
    let mut sizes: Vec<_> = sizes.into_iter().collect();
    sizes.sort_by(|a, b| (b.0.0 * b.0.1).cmp(&(a.0.0 * a.0.1)));
    println!("textures: {tex_n} unique, {:.0} MB RGBA8 with mips; sizes {:?}", tex_bytes as f64 / 1048576.0, sizes);
    println!("lights: {}", w.lights.len());
}
