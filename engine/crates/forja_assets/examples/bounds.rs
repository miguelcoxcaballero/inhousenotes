//! Prints bounds and triangle counts: `cargo run -p forja_assets --example bounds -- files...`
fn main() {
    for p in std::env::args().skip(1) {
        match forja_assets::load_model(std::path::Path::new(&p)) {
            Ok(s) => {
                let b = s.bounds();
                println!("{:<48} min {:>7.2?} max {:>7.2?} tris {:>6} nodes {:?}", std::path::Path::new(&p).file_name().unwrap().to_string_lossy(), b.min.to_array(), b.max.to_array(), s.triangle_count(), s.nodes.iter().map(|n| n.name.as_str()).take(8).collect::<Vec<_>>());
            }
            Err(e) => println!("{p}: {e}"),
        }
    }
}
