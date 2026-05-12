//! `iii-directory` — engine introspection (functions / triggers /
//! workers), workers registry proxy, and filesystem-backed skill +
//! prompt reader. The binary in `src/main.rs` is a thin wrapper that
//! wires the modules below to the iii engine.
//!
//! Four surfaces, all MCP-agnostic:
//!
//!   * **Skills** (`skills::*`, `skill::fetch`): a filesystem-backed
//!     markdown reader keyed by short skill ids
//!     (slashed-path-relative-to-`skills_folder`). Skills are surfaced
//!     through the `iii://` resource URI scheme. `skill::fetch` is a
//!     batched read tool over one or more `iii://` URIs.
//!   * **Prompts** (`prompts::*`): filesystem-backed slash-command
//!     templates loaded from `<skills_folder>/<ns>/prompts/*.md` files
//!     with YAML frontmatter. `prompts::list` enumerates them;
//!     `prompts::get` reads one body + metadata.
//!   * **Directory** (`directory::*`): read-side enrichment over the
//!     engine's `engine::functions::list`, `engine::workers::list`,
//!     `engine::trigger-types::list`, `engine::triggers::list` plus
//!     bundled how-to skill discovery via [`how_to`].
//!   * **Registry** (`registry::*`): HTTP proxy over
//!     `api.workers.iii.dev` with the same `worker-list` /
//!     `worker-info` shape as `directory::*` so callers learn one
//!     envelope across local + registry surfaces.
//!
//! `skills::download` is the only write path. It pulls markdown either
//! from the workers registry (`worker=NAME version=X.Y.Z|tag=latest`) or
//! from a GitHub repo (`repo=URL skill=NAME`) and writes the contents
//! into `<skills_folder>/<namespace>/...`. After every successful
//! download the worker fires `skills::on-change` and/or
//! `prompts::on-change` so subscribers can forward change notifications
//! to their clients.

pub mod config;
pub mod fs_source;
pub mod functions;
pub mod how_to;
pub mod manifest;
pub mod sources;
pub mod trigger_types;
