/// One-time scanner that reads every mesh in Thingi10K.zip and writes
/// mesh_stats.csv (sibling of the zip) with columns: ID, Vertices, Faces.
///
/// Usage:
///   cargo run --bin scan_meshes
///   cargo run --bin scan_meshes -- path/to/Thingi10K.zip

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::PathBuf;

fn main() {
    let zip_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "Thingi10K.zip".to_string()),
    )
    .canonicalize()
    .expect("Cannot resolve zip path — does the file exist?");

    let stats_path = zip_path.with_file_name("mesh_stats.csv");
    println!("Zip  : {}", zip_path.display());
    println!("Out  : {}", stats_path.display());

    // ── Pass 1: collect entry metadata ──────────────────────────────────────
    // We must finish borrowing the archive before we can borrow it again.
    let file = std::fs::File::open(&zip_path).expect("Cannot open zip");
    let mut archive = zip::ZipArchive::new(file).expect("Cannot parse zip");

    struct EntryMeta {
        index: usize,
        id:    u64,
        ext:   String,
        size:  u64, // uncompressed size
    }

    let mut entries: Vec<EntryMeta> = Vec::new();

    for i in 0..archive.len() {
        let entry = archive.by_index(i).unwrap();
        let name  = entry.name().to_owned();
        let size  = entry.size();
        drop(entry); // release borrow before next by_index call

        let Some(stem_ext) = name.strip_prefix("Thingi10K/raw_meshes/") else {
            continue;
        };
        let Some(dot) = stem_ext.rfind('.') else { continue };
        let stem = &stem_ext[..dot];
        let ext  = stem_ext[dot + 1..].to_lowercase();

        let Ok(id) = stem.parse::<u64>() else { continue };

        entries.push(EntryMeta { index: i, id, ext, size });
    }

    println!("Found {} mesh entries. Scanning...", entries.len());

    // ── Pass 2: read & parse each mesh ──────────────────────────────────────
    let out_file = std::fs::File::create(&stats_path)
        .expect("Cannot create mesh_stats.csv");
    let mut out = std::io::BufWriter::new(out_file);
    writeln!(out, "ID,Vertices,Faces").unwrap();

    let total = entries.len();
    let mut done = 0usize;

    for meta in &entries {
        let mut entry = archive.by_index(meta.index).unwrap();
        let mut bytes = Vec::with_capacity(meta.size as usize);
        entry.read_to_end(&mut bytes).unwrap_or(0);
        drop(entry);

        let (vertices, faces) = count_mesh(&bytes, &meta.ext, meta.size);
        writeln!(out, "{},{},{}", meta.id, vertices, faces).unwrap();

        done += 1;
        if done % 500 == 0 || done == total {
            println!("  {}/{} scanned…", done, total);
        }
    }

    out.flush().unwrap();
    println!("Done — wrote {} rows to {}", done, stats_path.display());
    println!("Restart the server to load the geometry data.");
}

// ── Mesh parsing ─────────────────────────────────────────────────────────────

fn count_mesh(bytes: &[u8], ext: &str, uncompressed_size: u64) -> (u64, u64) {
    match ext {
        "stl" => try_binary_stl(bytes, uncompressed_size)
            .unwrap_or_else(|| ascii_stl(bytes)),
        "obj" => count_obj(bytes),
        "ply" => count_ply(bytes),
        "off" => count_off(bytes),
        _     => (0, 0),
    }
}

/// Returns Some((vertices, faces)) if the buffer matches the binary STL format.
/// Falls back to None so the caller can try ASCII parsing instead.
///
/// Detection relies solely on the size invariant: binary STL files are exactly
/// 84 + (face_count * 50) bytes long.  The "starts with 'solid'" heuristic is
/// deliberately NOT used here because many CAD tools write binary STLs that
/// begin with "solid" in their 80-byte header, which would cause false negatives
/// and produce zero counts.
fn try_binary_stl(bytes: &[u8], size: u64) -> Option<(u64, u64)> {
    if bytes.len() < 84 {
        return None;
    }
    let header_count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as u64;
    // The size check is the definitive binary STL test.  An ASCII STL accidentally
    // satisfying this equation is astronomically unlikely.
    //
    // Some dataset files are truncated by exactly one 50-byte record (header
    // count is 1 larger than the data) — accept those too, clamped to the
    // complete records, matching fixTruncatedBinaryStl in the viewer.
    let expected = 84 + header_count * 50;
    let face_count = if expected == size {
        header_count
    } else if header_count > 0 && expected == size + 50 && (size - 84) % 50 == 0 {
        header_count - 1
    } else {
        return None;
    };

    // Deduplicate vertices by their raw float bytes.  Each face has 3 vertices
    // stored as 3 × f32 (12 bytes each) at offset 84 + i*50 + 12 + v*12.
    let mut unique: HashSet<[u8; 12]> = HashSet::with_capacity(face_count as usize * 3);
    for i in 0..face_count as usize {
        let base = 84 + i * 50 + 12; // skip header + normal vector
        for v in 0..3usize {
            let off = base + v * 12;
            if off + 12 <= bytes.len() {
                let key: [u8; 12] = bytes[off..off + 12].try_into().unwrap();
                unique.insert(key);
            }
        }
    }

    Some((unique.len() as u64, face_count))
}

/// Parse ASCII STL, counting unique vertex positions and faces.
fn ascii_stl(bytes: &[u8]) -> (u64, u64) {
    let mut unique: HashSet<[u8; 12]> = HashSet::new();
    let mut faces = 0u64;

    for line in bytes.split(|&b| b == b'\n') {
        let s = line.iter().copied()
            .skip_while(|&b| b == b' ' || b == b'\t')
            .collect::<Vec<u8>>();

        if s.starts_with(b"facet ") {
            faces += 1;
        } else if s.starts_with(b"vertex ") {
            // Parse "vertex x y z" — convert each f32 to its raw bytes as the key.
            if let Ok(text) = std::str::from_utf8(&s[7..]) {
                let mut parts = text.split_ascii_whitespace();
                let x = parts.next().and_then(|v| v.parse::<f32>().ok());
                let y = parts.next().and_then(|v| v.parse::<f32>().ok());
                let z = parts.next().and_then(|v| v.parse::<f32>().ok());
                if let (Some(x), Some(y), Some(z)) = (x, y, z) {
                    let mut key = [0u8; 12];
                    key[0..4].copy_from_slice(&x.to_le_bytes());
                    key[4..8].copy_from_slice(&y.to_le_bytes());
                    key[8..12].copy_from_slice(&z.to_le_bytes());
                    unique.insert(key);
                }
            }
        }
    }

    (unique.len() as u64, faces)
}

fn count_obj(bytes: &[u8]) -> (u64, u64) {
    let mut vertices = 0u64;
    let mut faces    = 0u64;
    for line in bytes.split(|&b| b == b'\n') {
        if line.starts_with(b"v ") {
            vertices += 1;
        } else if line.starts_with(b"f ") {
            faces += 1;
        }
    }
    (vertices, faces)
}

fn count_ply(bytes: &[u8]) -> (u64, u64) {
    let mut vertices = 0u64;
    let mut faces    = 0u64;
    for line in bytes.split(|&b| b == b'\n') {
        let s = String::from_utf8_lossy(line);
        let s = s.trim();
        if s == "end_header" {
            break;
        }
        if let Some(rest) = s.strip_prefix("element vertex ") {
            vertices = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = s.strip_prefix("element face ") {
            faces = rest.trim().parse().unwrap_or(0);
        }
    }
    (vertices, faces)
}

fn count_off(bytes: &[u8]) -> (u64, u64) {
    let mut lines = bytes
        .split(|&b| b == b'\n')
        .map(|l| String::from_utf8_lossy(l).trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'));

    // First line is either "OFF" alone or "OFF V F E" or "V F E".
    let first = lines.next().unwrap_or_default();
    let data = if first == "OFF" {
        lines.next().unwrap_or_default()
    } else if let Some(rest) = first.strip_prefix("OFF") {
        rest.trim().to_string()
    } else {
        first
    };

    let parts: Vec<&str> = data.split_whitespace().collect();
    let vertices = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let faces    = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (vertices, faces)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a binary STL with `faces` triangles, each using distinct vertices.
    fn binary_stl(faces: u32) -> Vec<u8> {
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&faces.to_le_bytes());
        for i in 0..faces {
            out.extend_from_slice(&[0u8; 12]); // normal
            for v in 0..3u32 {
                out.extend_from_slice(&((i * 3 + v) as f32).to_le_bytes());
                out.extend_from_slice(&1.0f32.to_le_bytes());
                out.extend_from_slice(&2.0f32.to_le_bytes());
            }
            out.extend_from_slice(&[0u8; 2]); // attribute byte count
        }
        out
    }

    #[test]
    fn exact_binary_stl_counts() {
        let stl = binary_stl(4);
        let (v, f) = try_binary_stl(&stl, stl.len() as u64).unwrap();
        assert_eq!((v, f), (12, 4));
    }

    #[test]
    fn truncated_binary_stl_clamps_to_complete_records() {
        // Header claims 4 faces but the last 50-byte record is missing —
        // the defect model 77942 has.  Must count the 3 complete faces.
        let mut stl = binary_stl(4);
        stl.truncate(stl.len() - 50);
        let (v, f) = try_binary_stl(&stl, stl.len() as u64).unwrap();
        assert_eq!(f, 3);
        assert_eq!(v, 9);
    }

    #[test]
    fn garbage_is_not_binary_stl() {
        let bytes = vec![7u8; 200];
        assert!(try_binary_stl(&bytes, 200).is_none());
    }
}
