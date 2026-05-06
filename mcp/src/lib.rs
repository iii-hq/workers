//! `mcp` — MCP 2025-06-18 Streamable HTTP bridge for the iii engine.
//!
//! The binary in [`main`](../mcp/index.html) is a thin wrapper that wires
//! the modules below to the iii engine. Tests under `tests/` reach into
//! these modules to register the bridge in-process against a shared
//! engine handle (the same code path the production binary takes).
//!
//! Surface:
//!
//!   * [`config`] — YAML-backed [`config::McpConfig`] (`hidden_prefixes`,
//!     `require_expose`, `api_path`).
//!   * [`functions`] — registers the `mcp::handler` function and bridges
//!     iii's HTTP trigger envelope into [`handler::handle_mcp_body`].
//!   * [`handler`] — JSON-RPC dispatch; pins the protocol version and
//!     fans out to `tools/*`, `resources/*`, `prompts/*`.
//!   * [`jsonrpc`] — JSON-RPC 2.0 wire types and helpers.
//!   * [`manifest`] — `--manifest` payload exposed to the iii registry.
//!   * [`skills_bridge`] — delegates `resources/*` and `prompts/*` to the
//!     `skills` worker and falls back to empty payloads when it isn't
//!     registered.
//!   * [`tools`] — `tools/list` + `tools/call` against the engine
//!     function registry; honours `hidden_prefixes` / `require_expose`.
//!   * [`transport`] — encodes JSON-RPC responses as a single SSE frame.

pub mod config;
pub mod functions;
pub mod handler;
pub mod jsonrpc;
pub mod manifest;
pub mod skills_bridge;
pub mod tools;
pub mod transport;
