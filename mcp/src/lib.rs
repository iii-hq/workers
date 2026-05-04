//! `mcp` — Model Context Protocol bridge worker. The binary in
//! `src/main.rs` is a thin wrapper that wires the modules below to the iii
//! engine.
//!
//! This worker exposes a single iii function (`mcp::handler`) bound to
//! `POST /mcp` on the engine's HTTP trigger port. Inside the handler each
//! MCP 2025-06-18 core method dispatches to either an inline reply or a
//! single `iii.trigger` call:
//!
//!   * `tools/list` / `tools/call`           → `engine::functions::list` / `iii.trigger`
//!   * `resources/list`                      → `skills::resources-list`
//!   * `resources/read`                      → `skills::resources-read`
//!   * `resources/templates/list`            → `skills::resources-templates`
//!   * `prompts/list`                        → `prompts::mcp-list`
//!   * `prompts/get`                         → `prompts::mcp-get`
//!
//! Skills + prompts therefore live in the sibling [`skills`](../skills)
//! worker; this crate is a stateless protocol surface.

pub mod config;
pub mod functions;
pub mod manifest;
pub mod protocol;
