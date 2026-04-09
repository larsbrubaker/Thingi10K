use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{cmp::Reverse, collections::HashMap, io::Read, path::PathBuf, sync::Arc};

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct Model {
    id: u64,
    thing_id: u64,
    name: String,   // derived from the filename in the download link
    license: String,
    link: String,
    duplicated_faces: bool,
    closed: bool,
    edge_manifold: bool,
    degenerate_faces: bool,
    vertex_manifold: bool,
    single_component: bool,
    pwn: bool,
    format: String,  // "stl", "obj", "ply", "off", or "unknown"
    vertices: u64,   // 0 = not yet scanned (run scan_meshes first)
    faces: u64,      // 0 = not yet scanned
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

struct AppState {
    models:       Vec<Model>,
    zip_path:     PathBuf,
    max_vertices: u64,
    max_faces:    u64,
}

// ── Startup: load models from zip ─────────────────────────────────────────────

fn parse_bool(s: &str) -> bool {
    s.trim().eq_ignore_ascii_case("true")
}

fn load_models(zip_path: &PathBuf) -> Result<Vec<Model>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Build a map of id → format by scanning all mesh filenames once.
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

    let mut models = Vec::with_capacity(10_000);
    for result in rdr.records() {
        let rec = result?;
        let id: u64 = rec[0].trim().parse()?;
        let format = format_map
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let link = rec[3].trim().to_string();
        let name = name_from_link(&link);
        // Columns 11 and 12 (Vertices, Faces) are present in the merged CSV.
        let vertices = rec.get(11).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let faces    = rec.get(12).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        models.push(Model {
            id,
            thing_id: rec[1].trim().parse()?,
            name,
            license: rec[2].trim().to_string(),
            link,
            duplicated_faces: parse_bool(&rec[4]),
            closed:           parse_bool(&rec[5]),
            edge_manifold:    parse_bool(&rec[6]),
            degenerate_faces: parse_bool(&rec[7]),
            vertex_manifold:  parse_bool(&rec[8]),
            single_component: parse_bool(&rec[9]),
            pwn:              parse_bool(&rec[10]),
            format,
            vertices,
            faces,
        });
    }

    Ok(models)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn serve_index() -> Response {
    serve_static_file("static/index.html", "text/html; charset=utf-8").await
}

async fn serve_css() -> Response {
    serve_static_file("static/style.css", "text/css; charset=utf-8").await
}

async fn serve_favicon() -> Response {
    serve_static_file("static/favicon.svg", "image/svg+xml").await
}

async fn serve_js() -> Response {
    serve_static_file("static/app.js", "application/javascript; charset=utf-8").await
}

async fn serve_static_file(path: &str, content_type: &'static str) -> Response {
    match std::fs::read_to_string(path) {
        Ok(body) => ([(header::CONTENT_TYPE, content_type)], body).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            format!("Could not read {path}: {e}"),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ModelQuery {
    search:       Option<String>,
    closed:       Option<bool>,
    edge_manifold: Option<bool>,
    vertex_manifold: Option<bool>,
    single_component: Option<bool>,
    pwn:          Option<bool>,
    min_vertices: Option<u64>,
    max_vertices: Option<u64>,
    min_faces:    Option<u64>,
    max_faces:    Option<u64>,
    /// id_asc | id_desc | thing_asc | thing_desc | name_asc | name_desc
    sort:         Option<String>,
}

#[derive(Serialize)]
struct ModelsResponse<'a> {
    total:  usize,
    models: Vec<&'a Model>,
}

#[derive(Serialize)]
struct StatsResponse {
    max_vertices: u64,
    max_faces:    u64,
    has_geometry: bool,
}

async fn get_stats(State(state): State<Arc<AppState>>) -> Json<StatsResponse> {
    Json(StatsResponse {
        max_vertices: state.max_vertices,
        max_faces:    state.max_faces,
        has_geometry: state.max_vertices > 0 || state.max_faces > 0,
    })
}

async fn get_models(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ModelQuery>,
) -> Json<ModelsResponse<'static>> {
    // SAFETY: AppState lives for the lifetime of the program (Arc kept alive by
    // the router). Returning references avoids cloning 10 K records per request.
    let state_ref: &'static AppState = unsafe { &*(Arc::as_ptr(&state)) };

    let search = q.search.as_deref().map(str::to_lowercase);

    let all_matching: Vec<&'static Model> = state_ref
        .models
        .iter()
        .filter(|m| {
            // Text search
            if let Some(ref s) = search {
                if !m.id.to_string().contains(s.as_str())
                    && !m.thing_id.to_string().contains(s.as_str())
                    && !m.name.to_lowercase().contains(s.as_str())
                {
                    return false;
                }
            }
            // Boolean property filters
            if let Some(v) = q.closed          { if m.closed != v           { return false; } }
            if let Some(v) = q.edge_manifold   { if m.edge_manifold != v    { return false; } }
            if let Some(v) = q.vertex_manifold { if m.vertex_manifold != v  { return false; } }
            if let Some(v) = q.single_component{ if m.single_component != v { return false; } }
            if let Some(v) = q.pwn             { if m.pwn != v              { return false; } }

            // Geometry range filters — only applied when the model has been scanned.
            if m.vertices > 0 {
                if let Some(v) = q.min_vertices { if m.vertices < v { return false; } }
                if let Some(v) = q.max_vertices { if m.vertices > v { return false; } }
            }
            if m.faces > 0 {
                if let Some(v) = q.min_faces { if m.faces < v { return false; } }
                if let Some(v) = q.max_faces { if m.faces > v { return false; } }
            }

            true
        })
        .collect();

    let total = all_matching.len();

    let mut all_matching = all_matching;
    match q.sort.as_deref().unwrap_or("id_asc") {
        "id_desc"    => all_matching.sort_unstable_by_key(|m| Reverse(m.id)),
        "thing_asc"  => all_matching.sort_unstable_by_key(|m| m.thing_id),
        "thing_desc" => all_matching.sort_unstable_by_key(|m| Reverse(m.thing_id)),
        "name_asc"   => all_matching.sort_unstable_by(|a, b| a.name.cmp(&b.name)),
        "name_desc"  => all_matching.sort_unstable_by(|a, b| b.name.cmp(&a.name)),
        _            => all_matching.sort_unstable_by_key(|m| m.id),
    }

    let models = all_matching.into_iter().take(100).collect();
    Json(ModelsResponse { total, models })
}

async fn get_mesh(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let disposition = state
        .models
        .iter()
        .find(|m| m.id == id)
        .map(|m| {
            let stem = m.name.replace(' ', "_");
            format!("attachment; filename=\"{}.{}\"", stem, m.format)
        })
        .unwrap_or_else(|| format!("attachment; filename=\"{id}.stl\""));

    let zip_path = state.zip_path.clone();

    let result = tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, &'static str), String> {
        let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

        let candidates: &[(&str, &str)] = &[
            ("stl", "model/stl"),
            ("obj", "text/plain"),
            ("ply", "application/octet-stream"),
            ("off", "text/plain"),
        ];

        for (ext, mime) in candidates {
            let entry_name = format!("Thingi10K/raw_meshes/{}.{}", id, ext);
            if let Ok(mut entry) = archive.by_name(&entry_name) {
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                return Ok((buf, mime));
            }
        }
        Err(format!("Model {} not found in archive", id))
    })
    .await;

    match result {
        Ok(Ok((bytes, mime))) => (
            [
                (header::CONTENT_TYPE, mime.to_string()),
                (header::CONTENT_DISPOSITION, disposition),
            ],
            bytes,
        )
            .into_response(),
        Ok(Err(msg)) => (StatusCode::NOT_FOUND, msg).into_response(),
        Err(e)       => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let zip_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "Thingi10K.zip".to_string()),
    );

    println!("Loading models from {} …", zip_path.display());
    let models = load_models(&zip_path).expect("Failed to load models from zip");
    println!("Loaded {} models.", models.len());

    let max_vertices = models.iter().map(|m| m.vertices).max().unwrap_or(0);
    let max_faces    = models.iter().map(|m| m.faces).max().unwrap_or(0);

    println!(
        "Geometry: max {} vertices, max {} faces.",
        max_vertices, max_faces
    );

    let state = Arc::new(AppState { models, zip_path, max_vertices, max_faces });

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/favicon.svg", get(serve_favicon))
        .route("/style.css", get(serve_css))
        .route("/app.js", get(serve_js))
        .route("/api/stats", get(get_stats))
        .route("/api/models", get(get_models))
        .route("/mesh/:id", get(get_mesh))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Server running at http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}
