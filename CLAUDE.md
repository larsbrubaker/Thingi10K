# Thingi10K Browser — Project Guide

## Project Overview

A Rust web server that reads `Thingi10K.zip` (a 9.1 GB dataset of 10,000 3D printing models
from Thingiverse) and serves a browser-based viewer. The UI has a searchable/filterable
sidebar list on the left and a Three.js STL 3D preview + details panel on the right.

**Dataset source:** https://ten-thousand-models.appspot.com

## Running the Server

```bash
cd C:\Development\rust\Thingi10K
cargo run
```

Then open **http://localhost:3000** in a browser.

To point at a zip in a different location:
```bash
cargo run -- path/to/Thingi10K.zip
```

## Build & Test

```bash
cargo build          # debug build
cargo build --release  # optimised build
cargo test           # run all tests
cargo clippy         # lint
```

## Architecture

| File | Purpose |
|------|---------|
| `src/main.rs` | Server entry point, all routes and handlers |
| `static/index.html` | Single-page frontend (Three.js + vanilla JS) |
| `Thingi10K.zip` | Dataset — read at runtime, never extracted |

### API Routes

| Route | Description |
|-------|-------------|
| `GET /` | Serves `static/index.html` |
| `GET /api/models` | Filtered JSON list (max 100). Query params: `search`, `closed`, `edge_manifold`, `vertex_manifold`, `single_component`, `pwn` |
| `GET /mesh/:id` | Streams the mesh file (STL/OBJ/PLY/OFF) from the zip |

### Key design decisions

- The zip is **never extracted to disk** — all mesh reads use `zip::ZipArchive` with
  `tokio::task::spawn_blocking` to avoid blocking the async executor.
- Model metadata (10 K records) is loaded into memory at startup and filtered in-memory
  on each API request — no database needed.
- The frontend is a single static HTML file embedded via `include_str!` — no build step.

## Coding Standards

### File length
- **Hard limit: 800 lines.** Files that reach this must be refactored by splitting into
  focused modules before adding more code.
- Never reduce a file's line count by removing comments or blank lines to meet the limit —
  that is not refactoring. Split real logic into separate files/modules.

### Bug workflow — always follow this order
1. **Write a failing test** that reproduces the bug.
2. **Fix the bug.**
3. **Confirm the test passes** (`cargo test`).

Never commit a bug fix that isn't covered by a test.

### General style
- Prefer `Result`/`Option` over `unwrap` in library code; `expect` is acceptable in
  `main` for startup failures with a clear message.
- Keep handler functions focused — if a handler grows complex, extract helpers.
- Avoid unsafe code unless there is no alternative; document every `unsafe` block.
