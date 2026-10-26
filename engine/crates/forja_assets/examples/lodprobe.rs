//! Times LOD generation and prints LOD sizes: `cargo run -p forja_assets --example lodprobe -- file`
fn main() {
    let p = std::env::args().nth(1).expect("file");
    let t = std::time::Instant::now();
    let mut s = forja_assets::load_model(std::path::Path::new(&p)).unwrap();
    println!("load {:?}", t.elapsed());
    let t = std::time::Instant::now();
    forja_assets::lod::generate_lods(&mut s, 8, 2000);
    println!("lods {:?}", t.elapsed());
    for m in s.meshes.iter().flatten() {
        println!("tris {} -> {:?} (verts {})", m.triangle_count(), m.lods.iter().map(|l| l.len() / 3).collect::<Vec<_>>(), m.positions.len());
    }
}
