//! `iii-directory` — engine introspection (functions / triggers /
//! workers), workers registry proxy, and filesystem-backed skill +
//! prompt reader. The binary in `src/main.rs` is a thin wrapper that
//! wires the modules below to the iii engine.
//!
//! Every public function sits under a single `directory::*` namespace,
//! split into four surfaces (all MCP-agnostic):
//!
//!   * **Skills** (`directory::skills::*`): a filesystem-backed markdown
//!     reader keyed by short skill ids
//!     (slashed-path-relative-to-`skills_folder`).
//!     `directory::skills::list` enumerates them with title/description
//!     pre-populated; `directory::skills::get` reads one body + metadata
//!     (same flat shape as `directory::prompts::get`).
//!   * **Prompts** (`directory::prompts::*`): filesystem-backed
//!     slash-command templates loaded from
//!     `<skills_folder>/<ns>/prompts/*.md` files with YAML frontmatter.
//!     `directory::prompts::list` enumerates them;
//!     `directory::prompts::get` reads one body + metadata.
//!   * **Engine** (`directory::engine::*`): read-side enrichment over
//!     the engine's `engine::functions::list`, `engine::workers::list`,
//!     `engine::trigger-types::list`, `engine::triggers::list` plus
//!     bundled how-to skill discovery via [`how_to`].
//!   * **Registry** (`directory::registry::*`): HTTP proxy over
//!     `api.workers.iii.dev` with the same `workers::{list,info}` shape
//!     as `directory::engine::workers::*` so callers learn one
//!     envelope across local + registry surfaces.
//!
//! `directory::skills::download` is the only write path. It pulls
//! markdown either from the workers registry (`worker=NAME
//! version=X.Y.Z|tag=latest`; defaults to `tag=latest`) or from a
//! GitHub repo (`repo=URL skill=NAME branch?=main`) and writes the
//! contents into `<skills_folder>/<namespace>/...`. After every
//! successful download the worker fires `directory::skills::on-change`
//! and/or `directory::prompts::on-change` so subscribers can forward
//! change notifications to their clients.

pub mod config;
pub mod fs_source;
pub mod functions;
pub mod how_to;
pub mod manifest;
pub mod sources;
pub mod trigger_types;
