//! `skills` — agentic content registry worker. The binary in
//! `src/main.rs` is a thin wrapper that wires the modules below to the iii
//! engine.
//!
//! This worker owns two surfaces that were previously bundled inside
//! `iii-mcp`:
//!
//!   * **Skills** (`skills::*`): a state-backed markdown registry keyed
//!     by short skill ids. Skills are surfaced through the `iii://`
//!     resource URI scheme and read by the `mcp` worker when it serves
//!     `resources/list` / `resources/read`.
//!   * **Prompts** (`prompts::*`): a state-backed registry of
//!     slash-command templates. The `mcp` worker reads them via
//!     `prompts::mcp-list` / `prompts::mcp-get` when answering
//!     `prompts/list` and `prompts/get`.
//!
//! Mutations of either scope fan out through custom trigger types
//! `skills::on-change` and `prompts::on-change` so that the `mcp` worker
//! (or any other interested subscriber) can forward MCP
//! `notifications/*_list_changed` to its clients.

pub mod config;
pub mod functions;
pub mod manifest;
pub mod state;
pub mod trigger_types;
