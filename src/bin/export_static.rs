use serde::Serialize;
use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::BufWriter,
    path::{Path, PathBuf},
};

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct ModelRecord {
    id: u64,
    thing_id: u64,
    name: String,
    license: String,
    format: String,
    repo: u8,
    closed: bool,
    edge_manifold: bool,
    vertex_manifold: bool,
    single_component: bool,
    pwn: bool,
    duplicated_faces: bool,
    degenerate_faces: bool,
    vertices: u64,
    faces: u64,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_bool(s: &str) -> bool {
    s.trim().eq_ignore_ascii_case("true")
}

/// Extract a human-readable name from a download URL.
/// e.g. "…/Octocat-v1.stl" → "Octocat v1"
fn name_from_link(link: &str) -> String {
    let stem = link
        .rsplit('/')
        .next()
        .unwrap_or(link)
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(link);

    stem.replace('_', " ").replace('-', " ")
}

fn assign_repo(sorted_index: usize, total: usize) -> u8 {
    let third = total / 3;
    // First 3333 → repo 1, next 3333 → repo 2, remaining → repo 3
    if sorted_index < third {
        1
    } else if sorted_index < 2 * third {
        2
    } else {
        3
    }
}

// ── CSV loading ───────────────────────────────────────────────────────────────

fn load_models(
    zip_path: &Path,
) -> Result<Vec<ModelRecord>, Box<dyn std::error::Error>> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Build a map of id → (format, entry_index) by scanning mesh filenames.
    let format_map: HashMap<u64, String> = {
        let names: Vec<String> = archive.file_names().map(String::from).collect();
        let mut map = HashMap::new();
        for name in &names {
            if let Some(stem) = name.strip_prefix("Thingi10K/raw_meshes/") {
                if let Some(dot) = stem.rfind('.') {
                    let id_str = &stem[..dot];
                    let ext = stem[dot + 1..].to_lowercase();
                    if let Ok(id) = id_str.parse::<u64>() {
                        map.insert(id, ext);
                    }
                }
            }
        }
        map
    };

    // Parse CSV from inside the zip.
    let entry = archive.by_name("Thingi10K/Thingi10K Summary.csv")?;
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(entry);

    let mut models: Vec<ModelRecord> = Vec::with_capacity(10_000);
    for result in rdr.records() {
        let rec = result?;
        let id: u64 = rec[0].trim().parse()?;
        let format = format_map
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let link = rec[3].trim().to_string();
        let name = name_from_link(&link);
        let vertices = rec.get(11).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let faces = rec.get(12).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        models.push(ModelRecord {
            id,
            thing_id: rec[1].trim().parse()?,
            name,
            license: rec[2].trim().to_string(),
            format,
            repo: 0, // assigned after sorting
            closed: parse_bool(&rec[5]),
            edge_manifold: parse_bool(&rec[6]),
            vertex_manifold: parse_bool(&rec[8]),
            single_component: parse_bool(&rec[9]),
            pwn: parse_bool(&rec[10]),
            duplicated_faces: parse_bool(&rec[4]),
            degenerate_faces: parse_bool(&rec[7]),
            vertices,
            faces,
        });
    }

    // Sort by ID and assign repos.
    models.sort_unstable_by_key(|m| m.id);
    let total = models.len();
    for (i, m) in models.iter_mut().enumerate() {
        m.repo = assign_repo(i, total);
    }

    Ok(models)
}

// ── Mini-zip export ───────────────────────────────────────────────────────────

fn export_meshes(
    zip_path: &Path,
    models: &[ModelRecord],
    base_output: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Build a map from mesh entry name → repo number for fast lookup.
    let id_to_repo: HashMap<u64, u8> = models.iter().map(|m| (m.id, m.repo)).collect();

    // Create output directories.
    for repo in 1u8..=3 {
        let dir = base_output
            .join(format!("meshes-{}", repo))
            .join("meshes");
        fs::create_dir_all(&dir)?;
    }

    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    let count = archive.len();
    let mut processed = 0usize;

    for i in 0..count {
        // Peek at the name without borrowing the archive mutably yet.
        let (entry_name, is_dir) = {
            let entry = archive.by_index(i)?;
            (entry.name().to_string(), entry.is_dir())
        };

        if is_dir {
            continue;
        }

        let stem = match entry_name.strip_prefix("Thingi10K/raw_meshes/") {
            Some(s) => s,
            None => continue,
        };

        // Parse id and extension from stem like "32770.stl".
        let dot = match stem.rfind('.') {
            Some(d) => d,
            None => continue,
        };
        let id_str = &stem[..dot];
        let ext = &stem[dot + 1..];
        let id: u64 = match id_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let repo = match id_to_repo.get(&id) {
            Some(&r) => r,
            None => continue,
        };

        let out_dir = base_output
            .join(format!("meshes-{}", repo))
            .join("meshes");
        let out_path = out_dir.join(format!("{}.{}.zip", id, ext));

        // Copy raw compressed bytes without decompression.
        let raw = archive.by_index_raw(i)?;
        let out_file = File::create(&out_path)?;
        let buf_writer = BufWriter::new(out_file);
        let mut zip_writer = zip::ZipWriter::new(buf_writer);
        let new_name = format!("{}.{}", id, ext);
        zip_writer.raw_copy_file_rename(raw, &new_name)?;
        zip_writer.finish()?;

        processed += 1;
        if processed % 500 == 0 {
            println!("  Exported {} mesh files…", processed);
        }
    }

    println!("  Done — exported {} mesh files total.", processed);
    Ok(())
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let zip_path = PathBuf::from(
        env::args()
            .nth(1)
            .unwrap_or_else(|| "Thingi10K.zip".to_string()),
    );

    println!("Reading models from {} …", zip_path.display());
    let models = load_models(&zip_path)?;
    println!("Loaded {} models.", models.len());

    // ── 1. Write docs/data/models.json ────────────────────────────────────────
    let docs_data = Path::new("docs/data");
    fs::create_dir_all(docs_data)?;
    let json_path = docs_data.join("models.json");
    println!("Writing {} …", json_path.display());
    let json = serde_json::to_string(&models)?;
    fs::write(&json_path, &json)?;
    println!("  Wrote {} bytes.", json.len());

    // ── 2. Export mini-zips ───────────────────────────────────────────────────
    let mesh_export = Path::new("mesh-export");
    println!("Exporting mini-zips to {} …", mesh_export.display());
    export_meshes(&zip_path, &models, mesh_export)?;

    // ── Repo split summary ────────────────────────────────────────────────────
    let repo1: Vec<u64> = models.iter().filter(|m| m.repo == 1).map(|m| m.id).collect();
    let repo2: Vec<u64> = models.iter().filter(|m| m.repo == 2).map(|m| m.id).collect();
    let repo3: Vec<u64> = models.iter().filter(|m| m.repo == 3).map(|m| m.id).collect();

    println!("\nRepo split ({} models total):", models.len());
    println!(
        "  Repo 1 ({} models): ID {} … {}",
        repo1.len(),
        repo1.first().copied().unwrap_or(0),
        repo1.last().copied().unwrap_or(0),
    );
    println!(
        "  Repo 2 ({} models): ID {} … {}",
        repo2.len(),
        repo2.first().copied().unwrap_or(0),
        repo2.last().copied().unwrap_or(0),
    );
    println!(
        "  Repo 3 ({} models): ID {} … {}",
        repo3.len(),
        repo3.first().copied().unwrap_or(0),
        repo3.last().copied().unwrap_or(0),
    );

    println!("\nDone.");
    Ok(())
}
