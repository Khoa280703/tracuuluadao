# Phase 1: Project Bootstrap

## Priority: Critical | Effort: S | Status: completed

## Overview

Initialize Rust project with Axum, configure dependencies, establish directory structure.

## Current Reality

- `Cargo.toml`, `.env.example`, `src/main.rs`, `src/config.rs`, `src/error.rs`, and module tree all exist.
- Verified 2026-05-11: `cargo check` passes, `cargo run` starts, `GET /health` returns 200.
- Bootstrap scope is done; remaining warnings are dead-code/unused, not bootstrap blockers.

## Requirements

- Rust workspace with Axum web server
- All dependencies declared in Cargo.toml
- Project directory structure matching architecture decisions
- Basic health check endpoint to verify setup

## Architecture

```
tracuuluadao/
├── Cargo.toml
├── config/
│   └── agents/              # Agent configs (Phase 2)
├── src/
│   ├── main.rs              # Entry point, Axum router setup
│   ├── config.rs            # App config (env vars, ports)
│   ├── error.rs             # Error types
│   ├── agents/              # Agent system (Phase 2-3)
│   ├── scrapers/            # Scraping modules (Phase 4)
│   ├── pipeline/            # Orchestration (Phase 5)
│   └── api/                 # HTTP handlers (Phase 6)
├── plans/
├── scripts/
└── proxies/
```

## Implementation Steps

1. Create `Cargo.toml` with all dependencies:
   ```toml
   [package]
   name = "tracuuluadao"
   version = "0.1.0"
   edition = "2024"

[dependencies]
# Web framework
axum = { version = "0.8", features = ["macros"] }
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["rt"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace"] }

# HTTP clients
reqwest = { version = "0.12", features = ["json", "stream"] }  # direct vLLM API
rquest = "2"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# HTML parsing
scraper = "0.22"
regex = "1"

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "chrono"] }

# Config & utils
async-stream = "0.3"
toml = "0.8"
notify = "7"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
chrono = { version = "0.4", features = ["serde"] }
dotenvy = "0.15"
rand = "0.9"
sha2 = "0.10"
url = "2"
tokio-stream = { version = "0.1", features = ["sync"] }
futures = "0.3"
	   ```

2. Create `src/main.rs` — Axum server with health check
3. Create `src/config.rs` — Load env vars (DB URL, LLM endpoints, proxy path)
4. Create `src/error.rs` — AppError type implementing IntoResponse
5. Create empty module files for future phases
6. Create `.env.example` with required env vars
7. Verify `cargo check` passes

## Related Files

- Create: `Cargo.toml`, `src/main.rs`, `src/config.rs`, `src/error.rs`, `.env.example`
- Create: `src/agents/mod.rs`, `src/scrapers/mod.rs`, `src/pipeline/mod.rs`, `src/api/mod.rs`

## Success Criteria

- [x] `cargo check` passes without errors
- [x] `cargo run` starts Axum server on configured port
- [x] `GET /health` returns 200
- [x] All module directories created with mod.rs stubs

## Risk Assessment

	- **rquest crate compatibility** — If `rquest` doesn't compile, fallback to `reqwest` + manual TLS config for non-Cloudflare sources
- **Rust edition 2024** — If toolchain too old, use edition 2021
