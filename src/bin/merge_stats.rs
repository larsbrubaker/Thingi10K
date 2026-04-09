/// merge_stats — rewrites Thingi10K.zip with geometry stats baked into
/// "Thingi10K/Thingi10K Summary.csv" and "Thingi10K Summary2.csv" removed.
///
/// Usage:
///   cargo run --bin merge_stats [<zip_path>] [<stats_csv>]
///
/// Defaults: Thingi10K.zip  mesh_stats.csv
///
/// All mesh entries are copied as raw compressed bytes — no decompression —
/// so the rewrite is fast (I/O-bound only).

use std::{
    collections::HashMap,
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

const SUMMARY_ENTRY:  &str = "Thingi10K/Thingi10K Summary.csv";
const SUMMARY2_ENTRY: &str = "Thingi10K/Thingi10K Summary2.csv";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let zip_path = PathBuf::from(
        std::env::args().nth(1).unwrap_or_else(|| "Thingi10K.zip".to_string()),
    );
    let stats_path = PathBuf::from(
        std::env::args().nth(2).unwrap_or_else(|| "mesh_stats.csv".to_string()),
    );

    println!("Reading stats from {} …", stats_path.display());
    let stats = load_stats(&stats_path)?;
    println!("Loaded stats for {} models.", stats.len());

    let tmp_path = zip_path.with_extension("tmp.zip");

    {
        let src_file = fs::File::open(&zip_path)?;
        let mut src = ZipArchive::new(src_file)?;

        let dst_file = fs::File::create(&tmp_path)?;
        let dst_buf = BufWriter::new(dst_file);
        let mut dst = ZipWriter::new(dst_buf);

        let total = src.len();
        let mut copied = 0usize;
        let mut skipped = 0usize;

        for i in 0..total {
            let name = src.by_index_raw(i)?.name().to_string();

            if name == SUMMARY2_ENTRY {
                println!("  Skipping {} (redundant)", name);
                skipped += 1;
                continue;
            }

            if name == SUMMARY_ENTRY {
                println!("  Replacing {} with merged version …", name);
                let merged = build_merged_csv(&mut src, &stats)?;
                let opts = SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                dst.start_file(SUMMARY_ENTRY, opts)?;
                dst.write_all(merged.as_bytes())?;
                copied += 1;
                continue;
            }

            // All other entries: raw byte copy, no decompression.
            let raw = src.by_index_raw(i)?;
            dst.raw_copy_file(raw)?;
            copied += 1;

            if copied % 500 == 0 {
                println!("  {}/{} entries copied …", copied, total - skipped);
            }
        }

        dst.finish()?;
        println!("Done. {} entries copied, {} removed.", copied, skipped);
    }

    // Atomically replace the original.
    let backup = zip_path.with_extension("bak.zip");
    fs::rename(&zip_path, &backup)?;
    fs::rename(&tmp_path, &zip_path)?;
    println!("Original backed up to {}", backup.display());
    println!("New zip written to {}", zip_path.display());

    Ok(())
}

/// Read mesh_stats.csv into a map: id → (vertices, faces).
fn load_stats(path: &PathBuf) -> Result<HashMap<u64, (u64, u64)>, Box<dyn std::error::Error>> {
    let mut rdr = csv::ReaderBuilder::new().has_headers(true).from_path(path)?;
    let mut map = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        if let (Ok(id), Ok(v), Ok(f)) = (
            rec[0].trim().parse::<u64>(),
            rec[1].trim().parse::<u64>(),
            rec[2].trim().parse::<u64>(),
        ) {
            map.insert(id, (v, f));
        }
    }
    Ok(map)
}

/// Read the existing Summary.csv out of the archive and produce a new CSV
/// string with Vertices and Faces columns appended.
fn build_merged_csv(
    src: &mut ZipArchive<fs::File>,
    stats: &HashMap<u64, (u64, u64)>,
) -> Result<String, Box<dyn std::error::Error>> {
    let entry = src.by_name(SUMMARY_ENTRY)?;
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(entry);

    let mut out = String::with_capacity(1 << 20); // ~1 MB initial

    // Header
    out.push_str("ID,Thing ID,License,Link,Duplicated Faces,Closed,Edge manifold,\
                  Degenerate Faces,Vertex manifold,Single Component,PWN,Vertices,Faces\n");

    for result in rdr.records() {
        let rec = result?;
        let id: u64 = rec[0].trim().parse().unwrap_or(0);
        let (vertices, faces) = stats.get(&id).copied().unwrap_or((0, 0));

        // Re-emit existing columns verbatim, then append the two new ones.
        for (i, field) in rec.iter().enumerate() {
            if i > 0 { out.push(','); }
            // Quote fields that contain commas or quotes.
            if field.contains(',') || field.contains('"') {
                out.push('"');
                out.push_str(&field.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(field);
            }
        }
        out.push(',');
        out.push_str(&vertices.to_string());
        out.push(',');
        out.push_str(&faces.to_string());
        out.push('\n');
    }

    Ok(out)
}
