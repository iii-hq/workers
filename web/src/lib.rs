//! `web` — outbound HTTP client on the iii bus (`web::fetch`).
//!
//! SSRF-guarded fetch with redirect handling, size/timeout caps, JSON
//! helpers, and a page-reading mode (HTML→Markdown/text, viewable images).
//! Standalone Rust port of the former `harness/src/web` TypeScript worker.

pub mod config;
pub mod manifest;
pub mod schemas;
pub mod ssrf;
