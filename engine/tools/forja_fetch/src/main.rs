//! Downloads and verifies the sandbox test content listed in `data/assets.toml`.
//!
//! ```text
//! cargo run -p forja_fetch -- list
//! cargo run -p forja_fetch -- fetch --all
//! cargo run -p forja_fetch -- fetch kenney_car_kit map_miami_beach_osm
//! cargo run -p forja_fetch -- fetch --kind animation
//! cargo run -p forja_fetch -- import quaternius_universal_animation_library ~/Downloads/ual.zip
//! ```

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use forja_assets::manifest::{AssetEntry, AssetKind, Lock, Manifest, asset_dir, find_engine_root};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, String>;

struct Ctx {
    root: PathBuf,
    lock: Lock,
    agent: ureq::Agent,
    force: bool,
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent("ForjaEngine/0.1 (https://github.com/miguelcoxcaballero/inhousenotes)")
        .build()
        .into()
}

fn get(ctx: &Ctx, url: &str) -> Result<Vec<u8>> {
    let mut last = String::new();
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(1 << attempt));
        }
        match ctx.agent.get(url).call() {
            Ok(mut resp) => {
                return resp
                    .body_mut()
                    .with_config()
                    .limit(512 * 1024 * 1024)
                    .read_to_vec()
                    .map_err(|e| format!("{url}: {e}"));
            }
            Err(ureq::Error::StatusCode(code)) if code == 404 || code == 403 => return Err(format!("{url}: HTTP {code}")),
            Err(e) => last = format!("{url}: {e}"),
        }
    }
    Err(last)
}

fn sha256(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verifies against the lock (or pins a new hash) and writes the file.
fn store(ctx: &mut Ctx, entry: &AssetEntry, rel: &str, bytes: &[u8]) -> Result<()> {
    let key = format!("{}/{}", entry.id, rel);
    let hash = sha256(bytes);
    match ctx.lock.files.get(&key) {
        Some(pinned) if *pinned != hash && !ctx.force => {
            return Err(format!("{key}: SHA-256 mismatch (pinned {pinned}, got {hash}); rerun with --force to accept"));
        }
        _ => {
            ctx.lock.files.insert(key, hash);
        }
    }
    let path = asset_dir(&ctx.root, &entry.id).join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Rejects absolute paths and `..` so archives cannot write outside the asset dir.
fn safe_rel(p: &str) -> Option<String> {
    let p = p.replace('\\', "/");
    if p.starts_with('/') || p.split('/').any(|c| c == ".." || c.contains(':')) {
        return None;
    }
    Some(p)
}

fn extract_zip(ctx: &mut Ctx, entry: &AssetEntry, bytes: &[u8]) -> Result<usize> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("{}: bad zip: {e}", entry.id))?;
    let mut count = 0;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        if !entry.include.is_empty() && !entry.include.iter().any(|p| name.starts_with(p.as_str())) {
            continue;
        }
        let rel = entry.strip.as_deref().and_then(|s| name.strip_prefix(s)).unwrap_or(&name);
        let Some(rel) = safe_rel(rel) else {
            eprintln!("  skipping unsafe path {name}");
            continue;
        };
        let mut data = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        store(ctx, entry, &rel, &data)?;
        count += 1;
    }
    Ok(count)
}

fn scrape_kenney_zip(ctx: &Ctx, page: &str) -> Option<String> {
    let html = String::from_utf8(get(ctx, page).ok()?).ok()?;
    let start = html.find("https://kenney.nl/media/pages/assets/")?;
    let end = html[start..].find(".zip")? + start + 4;
    Some(html[start..end].to_string())
}

fn fetch_polyhaven(ctx: &mut Ctx, entry: &AssetEntry) -> Result<usize> {
    let ph = entry.polyhaven.as_ref().expect("checked");
    let api = format!("https://api.polyhaven.com/files/{}", ph.id);
    let json: serde_json::Value = serde_json::from_slice(&get(ctx, &api)?).map_err(|e| format!("{api}: {e}"))?;
    let mut count = 0;
    if ph.format == "texture" {
        // Color, OpenGL normal and packed AO/roughness/metal (glTF ORM order).
        for (key, name) in [("Diffuse", "color"), ("nor_gl", "normal"), ("arm", "arm")] {
            let url = json[key][&ph.res]["jpg"]["url"].as_str().ok_or(format!("{api}: no {key}/{}", ph.res))?;
            let bytes = get(ctx, url)?;
            store(ctx, entry, &format!("{}_{name}.jpg", ph.id), &bytes)?;
            count += 1;
        }
        return Ok(count);
    }
    if ph.format == "hdr" {
        let url = json["hdri"][&ph.res]["hdr"]["url"].as_str().ok_or(format!("{api}: no hdri/{}/hdr", ph.res))?;
        let bytes = get(ctx, url)?;
        store(ctx, entry, &format!("{}_{}.hdr", ph.id, ph.res), &bytes)?;
        return Ok(1);
    }
    let node = &json[&ph.format][&ph.res][&ph.format];
    let url = node["url"].as_str().ok_or(format!("{api}: no {}/{}", ph.format, ph.res))?;
    let main_name = url.rsplit('/').next().unwrap_or("model.gltf").to_string();
    let bytes = get(ctx, url)?;
    store(ctx, entry, &main_name, &bytes)?;
    count += 1;
    if let Some(includes) = node["include"].as_object() {
        for (rel, info) in includes {
            let Some(u) = info["url"].as_str() else { continue };
            let Some(rel) = safe_rel(rel) else { continue };
            let bytes = get(ctx, u)?;
            store(ctx, entry, &rel, &bytes)?;
            count += 1;
        }
    }
    Ok(count)
}

/// Sketchfab: verify the license with the public API, then download the
/// glTF archive with the user's token (`SKETCHFAB_TOKEN`).
fn fetch_sketchfab(ctx: &mut Ctx, entry: &AssetEntry, uid: &str) -> Result<usize> {
    let token = std::env::var("SKETCHFAB_TOKEN").map_err(|_| {
        format!("needs a Sketchfab API token: set SKETCHFAB_TOKEN (Sketchfab > Settings > Password & API) and rerun `forja_fetch fetch {}`", entry.id)
    })?;
    let info: serde_json::Value = serde_json::from_slice(&get(ctx, &format!("https://api.sketchfab.com/v3/models/{uid}"))?).map_err(|e| e.to_string())?;
    let license = info["license"]["slug"].as_str().unwrap_or("");
    if !["by", "cc0"].contains(&license) {
        return Err(format!("{uid}: license '{license}' is not CC-BY or CC0; skipping"));
    }
    let author = info["user"]["displayName"].as_str().unwrap_or("?").to_string();
    let resp = ctx
        .agent
        .get(&format!("https://api.sketchfab.com/v3/models/{uid}/download"))
        .header("Authorization", &format!("Token {token}"))
        .call()
        .map_err(|e| format!("{uid}: download request failed ({e}); check the token"))?
        .body_mut()
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_slice(&resp).map_err(|e| e.to_string())?;
    let url = json["gltf"]["url"].as_str().ok_or(format!("{uid}: no glTF download offered"))?;
    let bytes = get(ctx, url)?;
    let n = extract_zip(ctx, entry, &bytes)?;
    // Credit as required by CC-BY.
    let credit = format!("{} by {} ({}), {}\n", info["name"].as_str().unwrap_or(uid), author, info["viewerUrl"].as_str().unwrap_or(""), info["license"]["label"].as_str().unwrap_or(""));
    store(ctx, entry, "CREDIT.txt", credit.as_bytes())?;
    Ok(n + 1)
}

fn fetch_entry(ctx: &mut Ctx, entry: &AssetEntry) -> Result<usize> {
    if let Some(url) = &entry.zip {
        let bytes = match get(ctx, url) {
            Ok(b) => b,
            Err(e) => {
                let fresh = entry.page.as_deref().and_then(|p| scrape_kenney_zip(ctx, p)).ok_or(e.clone())?;
                eprintln!("  archive moved, using {fresh}");
                get(ctx, &fresh)?
            }
        };
        return extract_zip(ctx, entry, &bytes);
    }
    if !entry.files.is_empty() {
        for f in &entry.files {
            let rel = safe_rel(&f.path).ok_or(format!("unsafe path {}", f.path))?;
            let mut bytes = get(ctx, &f.url)?;
            // A zipped single file (e.g. a BVH clip) stored unzipped.
            if f.url.ends_with(".zip") && !rel.ends_with(".zip") {
                let mut zip = zip::ZipArchive::new(Cursor::new(&bytes[..])).map_err(|e| format!("{}: bad zip: {e}", entry.id))?;
                let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                let idx = (0..zip.len()).find(|&i| zip.by_index(i).map(|z| z.name().to_ascii_lowercase().ends_with(&ext)).unwrap_or(false)).ok_or(format!("{}: no .{ext} inside {}", entry.id, f.url))?;
                let mut out = Vec::new();
                std::io::Read::read_to_end(&mut zip.by_index(idx).map_err(|e| e.to_string())?, &mut out).map_err(|e| e.to_string())?;
                bytes = out;
            }
            store(ctx, entry, &rel, &bytes)?;
        }
        return Ok(entry.files.len());
    }
    if entry.polyhaven.is_some() {
        return fetch_polyhaven(ctx, entry);
    }
    if let Some(sf) = &entry.sketchfab {
        return fetch_sketchfab(ctx, entry, &sf.uid);
    }
    if let Some(acg) = &entry.ambientcg {
        let url = format!("https://ambientcg.com/get?file={}_{}.zip", acg.id, acg.res);
        let bytes = get(ctx, &url)?;
        // Keep the maps the renderer uses; skip previews and displacement.
        let mut e = entry.clone();
        e.include = Vec::new();
        let mut zip = zip::ZipArchive::new(Cursor::new(&bytes[..])).map_err(|err| format!("{}: bad zip: {err}", entry.id))?;
        let mut n = 0;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|err| err.to_string())?;
            let name = f.name().to_string();
            let keep = ["_Color", "_NormalGL", "_Roughness", "_AmbientOcclusion", "_Metalness", "_Emission", "_Opacity"].iter().any(|k| name.contains(k));
            if f.is_dir() || !keep {
                continue;
            }
            let Some(rel) = safe_rel(&name) else { continue };
            let mut data = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut data).map_err(|err| err.to_string())?;
            store(ctx, &e, &rel, &data)?;
            n += 1;
        }
        return Ok(n);
    }
    if let Some(osm) = &entry.osm {
        let [a, b, c, d] = osm.bbox;
        let url = format!("https://api.openstreetmap.org/api/0.6/map?bbox={a},{b},{c},{d}");
        let bytes = get(ctx, &url)?;
        // Live map data changes every day: record the hash, never enforce it.
        ctx.lock.files.remove(&format!("{}/map.osm", entry.id));
        store(ctx, entry, "map.osm", &bytes)?;
        ctx.lock.files.remove(&format!("{}/map.osm", entry.id));
        return Ok(1);
    }
    if let Some(t) = &entry.terrarium {
        let mut n = 0;
        for dy in 0..t.tiles {
            for dx in 0..t.tiles {
                let (x, y) = (t.x + dx, t.y + dy);
                let url = format!("https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{}/{x}/{y}.png", t.z);
                let bytes = get(ctx, &url)?;
                store(ctx, entry, &format!("{}_{dx}_{dy}.png", t.z), &bytes)?;
                n += 1;
            }
        }
        return Ok(n);
    }
    if let Some(m) = &entry.manual {
        return Err(format!("manual download: {}\n  {}", m.url, m.note));
    }
    unreachable!("manifest validated")
}

fn write_attribution(root: &Path, manifest: &Manifest) {
    let mut out = String::from(
        "# Third-party content\n\nGenerated by forja_fetch. Ship this file with any build that includes these assets.\n\n",
    );
    for a in &manifest.assets {
        if !asset_dir(root, &a.id).exists() {
            continue;
        }
        let tag = if a.needs_attribution() { "**attribution required**" } else { "no attribution required" };
        out += &format!("- **{}** — {} — {} ({tag}) — <{}>\n", a.name, a.author, a.license, a.homepage);
    }
    let path = root.join("assets/downloaded/ATTRIBUTION.md");
    let _ = std::fs::create_dir_all(path.parent().expect("has parent"));
    let _ = std::fs::write(path, out);
}

fn parse_kind(s: &str) -> Option<AssetKind> {
    toml::Value::String(s.to_string()).try_into().ok()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(find_engine_root)
    else {
        eprintln!("could not find data/assets.toml; run inside the engine folder or pass --root");
        return ExitCode::FAILURE;
    };
    let manifest = match Manifest::load(&root.join("data/assets.toml")) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let lock_path = root.join("data/assets.lock.toml");
    let mut ctx = Ctx { root: root.clone(), lock: Lock::load(&lock_path), agent: agent(), force: args.iter().any(|a| a == "--force") };
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let positional: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
    let mut failed = false;
    match cmd {
        "list" => {
            for a in &manifest.assets {
                let have = if asset_dir(&root, &a.id).exists() { "✓" } else { " " };
                println!("[{have}] {:<40} {:<10} {:<12} {}", a.id, format!("{:?}", a.kind).to_lowercase(), a.license, a.name);
            }
        }
        "fetch" => {
            let kind = args.iter().position(|a| a == "--kind").and_then(|i| args.get(i + 1)).and_then(|k| parse_kind(k));
            let all = args.iter().any(|a| a == "--all");
            let selected: Vec<&AssetEntry> = manifest
                .assets
                .iter()
                .filter(|a| {
                    if let Some(k) = kind {
                        return a.kind == k;
                    }
                    all || positional.iter().any(|p| **p == a.id)
                })
                .collect();
            if selected.is_empty() {
                eprintln!("nothing selected; pass ids, --kind <kind> or --all");
                return ExitCode::FAILURE;
            }
            for a in selected {
                if a.sketchfab.is_some() && all && std::env::var("SKETCHFAB_TOKEN").is_err() {
                    println!("- {}: Sketchfab (set SKETCHFAB_TOKEN to download)", a.id);
                    continue;
                }
                if a.manual.is_some() && all {
                    let m = a.manual.as_ref().expect("checked");
                    println!("- {}: manual download from {}", a.id, m.url);
                    continue;
                }
                print!("- {} ... ", a.id);
                match fetch_entry(&mut ctx, a) {
                    Ok(n) => println!("{n} files"),
                    Err(e) => {
                        println!("FAILED\n  {e}");
                        failed = true;
                    }
                }
                let _ = ctx.lock.save(&lock_path);
            }
            write_attribution(&root, &manifest);
        }
        "import" => {
            let (Some(id), Some(file)) = (positional.first(), positional.get(1)) else {
                eprintln!("usage: forja_fetch import <asset_id> <archive.zip>");
                return ExitCode::FAILURE;
            };
            let Some(entry) = manifest.get(id) else {
                eprintln!("unknown asset {id}");
                return ExitCode::FAILURE;
            };
            let result = std::fs::read(file.as_str()).map_err(|e| e.to_string()).and_then(|b| extract_zip(&mut ctx, entry, &b));
            match result {
                Ok(n) => println!("imported {n} files into {}", asset_dir(&root, id).display()),
                Err(e) => {
                    eprintln!("{e}");
                    failed = true;
                }
            }
            let _ = ctx.lock.save(&lock_path);
            write_attribution(&root, &manifest);
        }
        _ => {
            println!("forja_fetch list | fetch <ids..> [--all] [--kind <kind>] [--force] | import <id> <zip>  [--root <dir>]");
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
