//! `iii-directory` — function search, workers registry proxy, and
//! filesystem-backed directory content. The binary in `src/main.rs` wires the
//! modules below to the iii engine.
//!
//! Every public function sits under a single `directory::*` namespace,
//! split into five surfaces (all MCP-agnostic):
//!
//!   * **Search** (`directory::search_functions`): compact installed and
//!     installable function candidates for one to six capabilities.
//!   * **Skills** (`directory::skills::*`): a filesystem-backed markdown
//!     reader keyed by short skill ids
//!     (slashed-path-relative-to-`skills_folder`).
//!     `directory::skills::list` enumerates them with title/type/description
//!     pre-populated; `directory::skills::get` reads one body + metadata.
//!     `title` prefers the YAML frontmatter `title:` (then `name:`) over
//!     the body H1, and `type` is lifted verbatim from frontmatter `type:`.
//!     System-installed agent skills under the read-only
//!     `agents_skills_folder` (`~/.agents/skills` convention, shallow
//!     `<skill>/SKILL.md` scan) are served too, shadowed by the same
//!     namespace under the global or local root.
//!   * **System prompts** (`directory::system-prompts::*`): filesystem-backed
//!     prompts loaded from `system-prompts/` path segments with YAML frontmatter.
//!   * **Agent Profiles** (`directory::agents::*`): filesystem-backed
//!     profiles stored directly under `agents_folder` — reusable session
//!     identities whose file body is the system prompt, with display
//!     name / emoji logo / skill filter in required
//!     YAML frontmatter (see `docs/architecture/agent-profile-storage.md`).
//!   * **Registry** (`directory::registry::*`): HTTP proxy over
//!     `api.workers.iii.dev` with the same `workers::{list,info}` shape
//!     as the engine's `engine::workers::*` so callers learn one
//!     envelope across local + registry surfaces.
//!
//! Engine introspection is native. Call `engine::*` directly when possible;
//! `directory::engine::functions::info` remains as a narrow wrapper for
//! callers restricted to the `directory::` namespace.
//!
//! Write paths: `directory::skills::download*` pulls markdown either
//! from the workers registry (`worker=NAME version=X.Y.Z|tag=latest`;
//! defaults to `tag=latest`) or from a GitHub repo (`repo=URL
//! skill=NAME branch?=main`) and writes skills/system prompts into
//! `<skills_folder>/<namespace>/...`; legacy `prompts/` paths are ignored,
//! while registry agent-profile entries are routed to `agents_folder`.
//! `directory::skills::update` /
//! `directory::system-prompts::update` /
//! `directory::agents::update` overwrite one existing file with edited
//! full-file content; `directory::skills::{create,delete}`,
//! `directory::system-prompts::{create,delete}` and
//! `directory::agents::{create,delete}` manage skill/system-prompt/agent-profile
//! files (never touching the read-only `agents_skills_folder`). After
//! every successful write the worker fires the matching family's
//! `directory::*::on-change` (skills / system-prompts /
//! agent profiles) so subscribers can forward change notifications to their
//! clients.
//!
//! The worker also ships an injectable console UI (see [`ui`]): a
//! skills and system-prompts browser/editor page, a `directory::*`
//! function-trigger renderer, and a custom configuration form.

pub mod bundled;
pub mod config;
pub mod configuration;
pub mod fs_source;
pub mod functions;
pub mod hook;
pub mod manifest;
pub mod sources;
pub mod surface;
pub mod trigger_types;
pub mod ui;
pub mod watch;
