//! Populate / update the `weld_result` column in docs/data/models.json.
//!
//! For each model, this runs the exact import pipeline the manifold-rust
//! demo uses (larsbrubaker/manifold-rust, demo/src/thingi.ts +
//! demo/wasm/src/soup.rs): parse the STL as raw triangle soup, normalize
//! (center at origin, uniform-scale so the longest bounding-box side is 2),
//! weld coincident vertices with `MeshGL::merge`, and import via
//! `Manifold::from_mesh_gl_robust`. The outcome is recorded as:
//!
//!   "manifold"    — welds into a strictly manifold mesh
//!   "nonmanifold" — imports as closed-but-non-manifold triangle soup
//!   "not_closed"  — rejected (not geometrically closed after welding)
//!
//! The dataset's own `closed`/`edge_manifold`/`vertex_manifold` flags were
//! computed with different tolerances and regularly disagree with this
//! stricter pipeline; `weld_result` is what downstream consumers (the
//! manifold-rust Boolean Gallery) should trust for engine selection.
//!
//! Mesh files are fetched from the jsDelivr CDN (same URLs the browser
//! uses) and cached in ./mesh-cache so re-runs are cheap. Progress is
//! saved back to models.json every 100 models, so the run can be
//! interrupted and resumed; already-populated records are skipped unless
//! --refresh is given.
//!
//! Usage:
//!   cargo run --release --bin update_weld_status               # closed STL <= 20000 faces
//!   cargo run --release --bin update_weld_status -- --max-faces 50000
//!   cargo run --release --bin update_weld_status -- --all      # include closed=false records
//!   cargo run --release --bin update_weld_status -- --refresh  # recompute existing values

use std::io::Read;
use std::path::{Path, PathBuf};

use manifold_rust::manifold::Manifold;
use manifold_rust::types::{Error, MeshGL};
use serde_json::Value;

const MODELS_JSON: &str = "docs/data/models.json";
const MESH_CDN: &str = "https://cdn.jsdelivr.net/gh/larsbrubaker";
const SAVE_EVERY: usize = 100;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut max_faces: u64 = 20_000;
    let mut include_open = false;
    let mut refresh = false;
    let mut limit: usize = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--max-faces" => {
                i += 1;
                max_faces = args[i].parse().expect("--max-faces needs a number");
            }
            "--limit" => {
                i += 1;
                limit = args[i].parse().expect("--limit needs a number");
            }
            "--all" => include_open = true,
            "--refresh" => refresh = true,
            other => panic!("unknown argument: {other}"),
        }
        i += 1;
    }

    let json_path = Path::new(MODELS_JSON);
    let raw = std::fs::read_to_string(json_path).expect("cannot read models.json");
    let mut models: Vec<serde_json::Map<String, Value>> =
        serde_json::from_str(&raw).expect("models.json must be an array of objects");
    println!("{} records loaded", models.len());

    std::fs::create_dir_all("mesh-cache").expect("cannot create mesh-cache");

    let todo: Vec<usize> = models
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.get("format").and_then(Value::as_str) == Some("stl")
                && (include_open || m.get("closed").and_then(Value::as_bool) == Some(true))
                && m.get("faces").and_then(Value::as_u64).is_some_and(|f| f <= max_faces)
                && (refresh || !m.contains_key("weld_result"))
        })
        .map(|(i, _)| i)
        .take(limit)
        .collect();
    println!("{} records to process (max_faces={max_faces})", todo.len());

    // Worker pool: the run is dominated by CDN fetch latency, so 8 threads
    // pull (idx, id, repo) jobs from a shared cursor and send results back;
    // the main thread owns `models` and saves periodically.
    let jobs: Vec<(usize, u64, u64)> = todo
        .iter()
        .map(|&idx| {
            (
                idx,
                models[idx]["id"].as_u64().unwrap(),
                models[idx]["repo"].as_u64().unwrap(),
            )
        })
        .collect();
    let (tx, rx) = std::sync::mpsc::channel::<(usize, u64, Result<&'static str, String>)>();
    let cursor = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let jobs = std::sync::Arc::new(jobs);
    let workers = 8.min(jobs.len().max(1));
    let mut handles = Vec::new();
    for _ in 0..workers {
        let tx = tx.clone();
        let cursor = cursor.clone();
        let jobs = jobs.clone();
        handles.push(std::thread::spawn(move || loop {
            let i = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Some(&(idx, id, repo)) = jobs.get(i) else { break };
            let _ = tx.send((idx, id, process_model(id, repo)));
        }));
    }
    drop(tx);

    let (mut n_manifold, mut n_soup, mut n_rejected, mut n_error) = (0u32, 0u32, 0u32, 0u32);
    let mut done = 0usize;
    for (idx, id, result) in rx {
        done += 1;
        match result {
            Ok(r) => {
                match r {
                    "manifold" => n_manifold += 1,
                    "nonmanifold" => n_soup += 1,
                    _ => n_rejected += 1,
                }
                models[idx].insert("weld_result".into(), Value::String(r.into()));
            }
            Err(e) => {
                eprintln!("#{id}: {e}");
                n_error += 1;
            }
        }
        if done % SAVE_EVERY == 0 {
            save(json_path, &models);
            println!(
                "[{done}/{}] manifold={n_manifold} nonmanifold={n_soup} not_closed={n_rejected} errors={n_error}",
                jobs.len()
            );
        }
    }
    for h in handles {
        let _ = h.join();
    }
    save(json_path, &models);
    println!(
        "done: manifold={n_manifold} nonmanifold={n_soup} not_closed={n_rejected} errors={n_error}"
    );
}

fn save(path: &Path, models: &[serde_json::Map<String, Value>]) {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(models).unwrap()).expect("write failed");
    std::fs::rename(&tmp, path).expect("rename failed");
}

fn process_model(id: u64, repo: u64) -> Result<&'static str, String> {
    let bytes = fetch_zip(id, repo)?;
    let stl = unzip_first(&bytes)?;
    let mut positions = parse_stl(&stl)?;
    if positions.len() < 9 {
        return Err("no triangles parsed".into());
    }
    normalize(&mut positions);

    let mut mesh = MeshGL::default();
    mesh.num_prop = 3;
    mesh.tri_verts = (0..(positions.len() / 3) as u32).collect();
    mesh.vert_properties = positions;
    mesh.merge();
    let m = Manifold::from_mesh_gl_robust(&mesh);
    Ok(if m.status() != Error::NoError || m.is_empty() {
        "not_closed"
    } else if m.as_impl().is_soup {
        "nonmanifold"
    } else {
        "manifold"
    })
}

fn fetch_zip(id: u64, repo: u64) -> Result<Vec<u8>, String> {
    let cache: PathBuf = format!("mesh-cache/{id}.stl.zip").into();
    if let Ok(bytes) = std::fs::read(&cache) {
        return Ok(bytes);
    }
    let url = format!("{MESH_CDN}/Thingi10K-meshes-{repo}@main/meshes/{id}.stl.zip");
    let resp = ureq::get(&url).call().map_err(|e| format!("fetch: {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read: {e}"))?;
    std::fs::write(&cache, &bytes).map_err(|e| format!("cache write: {e}"))?;
    Ok(bytes)
}

fn unzip_first(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("zip: {e}"))?;
    if archive.is_empty() {
        return Err("empty zip".into());
    }
    let mut entry = archive.by_index(0).map_err(|e| format!("zip entry: {e}"))?;
    let mut out = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut out)
        .map_err(|e| format!("unzip: {e}"))?;
    Ok(out)
}

/// Binary or ASCII STL to xyz-interleaved f32 triangle soup, with the same
/// truncated-binary-header repair the browser applies (some dataset files
/// declare one more face than the data holds).
fn parse_stl(data: &[u8]) -> Result<Vec<f32>, String> {
    let head = String::from_utf8_lossy(&data[..data.len().min(512)]);
    if head.trim_start().starts_with("solid") && head.contains("facet") {
        return parse_ascii_stl(data);
    }
    if data.len() < 84 {
        return Err("binary STL too short".into());
    }
    let declared = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
    let actual = (data.len() - 84) / 50;
    let n_faces = declared.min(actual);
    let mut out = Vec::with_capacity(n_faces * 9);
    for f in 0..n_faces {
        let base = 84 + f * 50 + 12; // skip facet normal
        for v in 0..9 {
            let o = base + v * 4;
            out.push(f32::from_le_bytes(data[o..o + 4].try_into().unwrap()));
        }
    }
    Ok(out)
}

fn parse_ascii_stl(data: &[u8]) -> Result<Vec<f32>, String> {
    let text = String::from_utf8_lossy(data);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("vertex") {
            for tok in rest.split_whitespace().take(3) {
                out.push(tok.parse::<f32>().map_err(|e| format!("ascii stl: {e}"))?);
            }
        }
    }
    Ok(out)
}

/// Center at the bbox midpoint and uniform-scale so the longest side is 2 —
/// identical math to the demo's thingi.ts (f64 arithmetic on f32 values,
/// stored back as f32).
fn normalize(positions: &mut [f32]) {
    let n = positions.len() / 3;
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for i in 0..n {
        for k in 0..3 {
            let v = positions[i * 3 + k] as f64;
            min[k] = min[k].min(v);
            max[k] = max[k].max(v);
        }
    }
    let center = [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ];
    let max_side = (max[0] - min[0]).max(max[1] - min[1]).max(max[2] - min[2]);
    let scale = if max_side > 0.0 { 2.0 / max_side } else { 1.0 };
    for i in 0..n {
        for k in 0..3 {
            positions[i * 3 + k] = ((positions[i * 3 + k] as f64 - center[k]) * scale) as f32;
        }
    }
}
