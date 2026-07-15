//! `memory` worker — durable cross-session agent memory on iii primitives.
//!
//! Named **banks** hold two kinds of content: **rules** (markdown documents
//! injected whole into every turn's system prompt) and **memories** (extracted
//! items recalled on demand via BM25 + entity scoring). Memories live in an
//! append-only `memories.jsonl` per bank — the file is the source of truth; the
//! search index is a RAM-only cache rebuilt from it at boot, so the two can
//! never diverge across restarts. Memories are superseded, never destroyed;
//! pinned memories are untouchable by any automatic path.

pub mod config;
pub mod configuration;
pub mod deps;
pub mod embed_client;
pub mod error;
pub mod events;
pub mod extract;
pub mod functions;
pub mod hooks;
pub mod index;
pub mod manifest;
pub mod store;
pub mod types;
