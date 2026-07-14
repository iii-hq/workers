//! `memory` worker — durable cross-session agent memory on iii primitives.
//!
//! Named **banks** hold two kinds of content: **blocks** (markdown documents
//! injected whole into every turn's system prompt) and **facts** (extracted
//! items recalled on demand via BM25 + entity scoring). Facts live in an
//! append-only `facts.jsonl` per bank — the file is the source of truth; the
//! search index is a RAM-only cache rebuilt from it at boot, so the two can
//! never diverge across restarts. Facts are superseded, never destroyed;
//! pinned facts are untouchable by any automatic path.

pub mod config;
pub mod configuration;
pub mod deps;
pub mod error;
pub mod events;
pub mod extract;
pub mod functions;
pub mod hooks;
pub mod index;
pub mod manifest;
pub mod store;
pub mod types;
