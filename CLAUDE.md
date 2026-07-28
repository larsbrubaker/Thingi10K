# Thingi10K Browser — Project Guide

## Project Overview

A Rust web server that reads `Thingi10K.zip` (a 9.1 GB dataset of 10,000 3D printing models
from Thingiverse) and serves a browser-based viewer. The UI has a searchable/filterable
sidebar list on the left and a Three.js STL 3D preview + details panel on the right.

**Dataset source:** https://ten-thousand-models.appspot.com

## Architecture

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
