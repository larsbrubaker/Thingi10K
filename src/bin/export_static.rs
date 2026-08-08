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
    /// Number of split parts for oversized meshes (absent = single zip).
    #[serde(skip_serializing_if = "Option::is_none")]
    parts: Option<u32>,
}

/// jsDelivr refuses to serve files above ~20 MB from GitHub repos, so any
/// mini-zip larger than this is split into parts of `CHUNK_SIZE` raw bytes.
const ZIP_SIZE_LIMIT: u64 = 20 * 1024 * 1024;
/// Raw (decompressed) bytes per part — matches the existing part files.
const CHUNK_SIZE: usize = 15 * 1024 * 1024;

fn num_parts(raw_len: usize) -> u32 {
    (raw_len.div_ceil(CHUNK_SIZE)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn num_parts_matches_published_split_files() {
        // Real sizes from the dataset and the part counts published in the
        // Thingi10K-meshes-* repos.
        assert_eq!(num_parts(78_245_934), 5); // 112798.stl
        assert_eq!(num_parts(99_587_934), 7); // 1422991.stl
        assert_eq!(num_parts(220_004_755), 14); // 688370.stl
        assert_eq!(num_parts(CHUNK_SIZE), 1);
        assert_eq!(num_parts(CHUNK_SIZE + 1), 2);
    }
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
            parts: None, // filled in after export
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

/// Returns a map of model id → part count for meshes that had to be split.
fn export_meshes(
    zip_path: &Path,
    models: &[ModelRecord],
    base_output: &Path,
) -> Result<HashMap<u64, u32>, Box<dyn std::error::Error>> {
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
    let mut split_parts: HashMap<u64, u32> = HashMap::new();

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
        let new_name = format!("{}.{}", id, ext);

        let compressed_size = archive.by_index_raw(i)?.compressed_size();
        if compressed_size > ZIP_SIZE_LIMIT {
            // Too big for jsDelivr — split the raw mesh into CHUNK_SIZE slices,
            // each written as its own zip: ID.ext_1.zip, ID.ext_2.zip, …
            let mut raw_bytes = Vec::new();
            std::io::Read::read_to_end(&mut archive.by_index(i)?, &mut raw_bytes)?;
            let parts = num_parts(raw_bytes.len());
            for (p, chunk) in raw_bytes.chunks(CHUNK_SIZE).enumerate() {
                let out_path = out_dir.join(format!("{}.{}_{}.zip", id, ext, p + 1));
                let mut zip_writer =
                    zip::ZipWriter::new(BufWriter::new(File::create(&out_path)?));
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                zip_writer.start_file(&new_name, options)?;
                std::io::Write::write_all(&mut zip_writer, chunk)?;
                zip_writer.finish()?;
            }
            split_parts.insert(id, parts);
        } else {
            // Copy raw compressed bytes without decompression.
            let out_path = out_dir.join(format!("{}.{}.zip", id, ext));
            let raw = archive.by_index_raw(i)?;
            let out_file = File::create(&out_path)?;
            let buf_writer = BufWriter::new(out_file);
            let mut zip_writer = zip::ZipWriter::new(buf_writer);
            zip_writer.raw_copy_file_rename(raw, &new_name)?;
            zip_writer.finish()?;
        }

        processed += 1;
        if processed % 500 == 0 {
            println!("  Exported {} mesh files…", processed);
        }
    }

    println!(
        "  Done — exported {} mesh files total ({} split into parts).",
        processed,
        split_parts.len()
    );
    Ok(split_parts)
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let zip_path = PathBuf::from(
        env::args()
            .nth(1)
            .unwrap_or_else(|| "Thingi10K.zip".to_string()),
    );

    println!("Reading models from {} …", zip_path.display());
    let mut models = load_models(&zip_path)?;
    println!("Loaded {} models.", models.len());

    // ── 1. Export mini-zips (must run first — it determines part counts) ──────
    let mesh_export = Path::new("mesh-export");
    println!("Exporting mini-zips to {} …", mesh_export.display());
    let split_parts = export_meshes(&zip_path, &models, mesh_export)?;
    for m in models.iter_mut() {
        m.parts = split_parts.get(&m.id).copied();
    }

    // ── 2. Write docs/data/models.json ────────────────────────────────────────
    let docs_data = Path::new("docs/data");
    fs::create_dir_all(docs_data)?;
    let json_path = docs_data.join("models.json");
    println!("Writing {} …", json_path.display());
    let json = serde_json::to_string(&models)?;
    fs::write(&json_path, &json)?;
    println!("  Wrote {} bytes.", json.len());

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
