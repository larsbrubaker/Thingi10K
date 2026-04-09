/// replace_model — swaps one mesh entry in Thingi10K.zip and updates
/// the corresponding row in the embedded Summary.csv.
///
/// Usage:
///   cargo run --bin replace_model -- <old_id> <old_ext> <new_stl_path> <thing_id> <license>
///
/// Example (what we actually need):
///   cargo run --bin replace_model -- 376253 obj \
///       "C:\Users\LarsBrubaker\Downloads\Astronaut_Phil_A_Ment.STL" \
///       2557603 "Creative Commons - Attribution - Share Alike"

use std::{
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

const SUMMARY_ENTRY: &str = "Thingi10K/Thingi10K Summary.csv";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!("Usage: replace_model <old_id> <old_ext> <new_stl_path> <thing_id> <license>");
        std::process::exit(1);
    }

    let old_id: u64   = args[1].parse()?;
    let old_ext       = args[2].to_lowercase();
    let new_path      = PathBuf::from(&args[3]);
    let new_thing_id: u64 = args[4].parse()?;
    let license       = args[5].clone();

    let zip_path = PathBuf::from("Thingi10K.zip");
    let tmp_path = zip_path.with_extension("tmp.zip");

    // Derive model name from the filename stem.
    let new_name_raw = new_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .replace('_', " ")
        .replace('-', " ");

    // Build the new download link — encodes the name so name_from_link works.
    let new_link = format!(
        "https://www.thingiverse.com/thing:{}/files/{}.stl",
        new_thing_id,
        new_path.file_stem().unwrap_or_default().to_string_lossy()
    );

    println!("Replacing model {} ({}) with \"{}\" (thing {})…",
        old_id, old_ext, new_name_raw, new_thing_id);

    let old_mesh_entry = format!("Thingi10K/raw_meshes/{}.{}", old_id, old_ext);
    let new_mesh_entry = format!("Thingi10K/raw_meshes/{}.stl", old_id);

    let new_stl_bytes = fs::read(&new_path)?;
    println!("New STL: {} bytes ({:.1} MB)", new_stl_bytes.len(), new_stl_bytes.len() as f64 / 1e6);

    {
        let src_file = fs::File::open(&zip_path)?;
        let mut src  = ZipArchive::new(src_file)?;

        let dst_file = fs::File::create(&tmp_path)?;
        let mut dst  = ZipWriter::new(BufWriter::new(dst_file));

        let opts_deflate = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        let total = src.len();
        let mut done = 0usize;

        for i in 0..total {
            let name = src.by_index_raw(i)?.name().to_string();

            if name == old_mesh_entry {
                // Replace with new STL.
                println!("  Replacing mesh entry {} → {}", old_mesh_entry, new_mesh_entry);
                dst.start_file(&new_mesh_entry, opts_deflate)?;
                dst.write_all(&new_stl_bytes)?;
                done += 1;
                continue;
            }

            if name == SUMMARY_ENTRY {
                println!("  Updating Summary.csv row for id {}…", old_id);
                let csv = build_updated_csv(&mut src, old_id, new_thing_id, &license, &new_link)?;
                dst.start_file(SUMMARY_ENTRY, opts_deflate)?;
                dst.write_all(csv.as_bytes())?;
                done += 1;
                continue;
            }

            dst.raw_copy_file(src.by_index_raw(i)?)?;
            done += 1;

            if done % 1000 == 0 {
                println!("  {}/{} entries processed…", done, total);
            }
        }

        dst.finish()?;
    }

    let backup = zip_path.with_extension("bak.zip");
    fs::rename(&zip_path, &backup)?;
    fs::rename(&tmp_path, &zip_path)?;
    println!("Done. Original backed up to {}", backup.display());

    Ok(())
}

fn build_updated_csv(
    src: &mut ZipArchive<fs::File>,
    old_id: u64,
    new_thing_id: u64,
    license: &str,
    new_link: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let entry = src.by_name(SUMMARY_ENTRY)?;
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(entry);

    // Phil A Ment mesh properties (scanned externally).
    // Faces: 172652, Vertices: 86326
    // closed=T, edge_manifold=T, vertex_manifold=T,
    // single_component=T, pwn=T, dup_faces=F, degen_faces=F
    let replacement = [
        old_id.to_string(),
        new_thing_id.to_string(),
        license.to_string(),
        new_link.to_string(),
        "FALSE".to_string(), // duplicated_faces
        "TRUE".to_string(),  // closed
        "TRUE".to_string(),  // edge_manifold
        "FALSE".to_string(), // degenerate_faces
        "TRUE".to_string(),  // vertex_manifold
        "TRUE".to_string(),  // single_component
        "TRUE".to_string(),  // pwn
        "86326".to_string(), // vertices
        "172652".to_string(), // faces
    ];

    let mut out = String::with_capacity(1 << 20);
    out.push_str("ID,Thing ID,License,Link,Duplicated Faces,Closed,Edge manifold,\
                  Degenerate Faces,Vertex manifold,Single Component,PWN,Vertices,Faces\n");

    for result in rdr.records() {
        let rec = result?;
        let id: u64 = rec[0].trim().parse().unwrap_or(0);

        let fields: Vec<&str> = if id == old_id {
            replacement.iter().map(|s| s.as_str()).collect()
        } else {
            rec.iter().collect()
        };

        for (i, field) in fields.iter().enumerate() {
            if i > 0 { out.push(','); }
            if field.contains(',') || field.contains('"') {
                out.push('"');
                out.push_str(&field.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(field);
            }
        }
        out.push('\n');
    }

    Ok(out)
}
