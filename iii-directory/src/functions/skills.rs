//! Filesystem-backed skills reader.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::skills::list` — enriched listing of every markdown
//!     skill under `skills_folder`, sorted by id. Each row carries
//!     `id`, `title`, `type`, `description`, `bytes`, and `modified_at`
//!     so a consumer can render a picker / index in one round trip
//!     without follow-up `get` calls per row.
//!   * `directory::skills::get`  — fetch one skill by id. Returns
//!     `{ id, title, type, function_id, body, modified_at }`. The
//!     teaser `description` field that `list` rows carry is omitted
//!     here on purpose: the full `body` is already in the response,
//!     and repeating its first paragraph wastes ~200 tokens per fetch
//!     on local models that pay for every token (session z0mudsgu).
//!
//! Title resolution precedence (shared by `list` and `get`): the YAML
//! frontmatter `title:` wins when present and non-empty, then the
//! first `# H1` line in the body, with the bare id as final fallback.
//! `type` is read straight from the frontmatter `type:` key (e.g.
//! `index`, `how-to`, `reference`) and serialised as `null` when the
//! file omits it.
//!
//! There are no write paths in this module. Files arrive on disk via
//! `directory::skills::download` (see [`crate::functions::download`])
//! or by direct editing under `skills_folder`. Mutations fan out
//! through the `directory::skills::on-change` trigger type which is
//! fired from the download function on success.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use tokio::sync::Mutex;

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source::{self, FsSkill, SkillFrontmatter};
use crate::functions::error::{invalid_input_message, not_found_message, NextAction, SuggestEntry};

/// Soft-cap on a single skill body (matches the historic state-backed
/// limit the registry enforced).
pub const SKILL_BODY_MAX_BYTES: usize = 256 * 1024;

/// Per-segment cap for both skill ids and section URI segments. The
/// total id is allowed to chain many segments via `/`, but each
/// individual segment stays short so directory listings stay readable.
const ID_SEGMENT_MAX_LEN: usize = 64;

/// Soft ceiling on the slashed id length. With the per-segment cap above
/// this allows depth ~16 in practice — far deeper than any reasonable
/// tree, while preventing pathological inputs.
const ID_TOTAL_MAX_LEN: usize = 1024;

/// `iii://` prefix accepted on `get` inputs as a convenience so callers
/// can paste a link target verbatim. The prefix is stripped before id
/// validation; any other URI scheme (`https://`, `ftp://`, ...) is
/// rejected.
const URI_PREFIX: &str = "iii://";

/// Description for the `directory::skills::get` registration.
const GET_DESCRIPTION: &str =
    "Fetch one filesystem-backed skill by id and return its raw markdown body plus \
     id, title, type, function_id, and modified_at. A worker overview is addressed \
     by the bare worker name (e.g. \"iii-sandbox\") — that is the id `list`/`index` \
     hand back. Input is forgiving: \"iii-sandbox/index\", \"iii-sandbox/SKILL.md\", a \
     trailing \".md\", and an iii:// prefix all resolve to the same overview; and if \
     the exact id misses, the worker name is matched case-insensitively as a \
     substring (\"sandbox\" finds \"iii-sandbox\"). `title` prefers frontmatter \
     `title:` over the body H1; `type` is the frontmatter `type:`. There is no \
     `description` field here (the body already opens with that paragraph) — use \
     directory::skills::list for the teaser-only view. On a miss you get a \
     `D110 not_found` message naming the closest ids and the next function to call.";

/// Recovery pointers attached to every `directory::skills::*` not-found
/// message: where the agent should look to find a valid id.
const SKILL_NOT_FOUND_NEXT: &[NextAction] = &[
    NextAction::new("directory::skills::list", "browse skill ids"),
    NextAction::new("directory::skills::index", "see the per-worker overview"),
];

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListSkillsInput {
    /// Case-insensitive substring match against `id`, `title`, and (when
    /// `include_description` is true) the first body paragraph. Omitted
    /// rows are filtered out cheaply on the FsSkill { id } pass before
    /// the per-file frontmatter read, so a narrowed list is dramatically
    /// cheaper for the caller than the unfiltered one.
    #[serde(default)]
    search: Option<String>,
    /// Exact prefix match against `id`. Combine with `search` to scope a
    /// fuzzy match to one worker namespace, e.g. `prefix: "sandbox/"`.
    #[serde(default)]
    prefix: Option<String>,
    /// Exact match against the frontmatter `type:` field (`index`,
    /// `how-to`, `reference`, ...). `null` for entries with no
    /// frontmatter `type:`.
    #[serde(default, rename = "type")]
    kind: Option<String>,
    /// When `false`, the response omits the first-paragraph
    /// `description` field on every row. Useful for token-light pickers
    /// that only need `id` + `title` + `type`. Default `true`.
    #[serde(default)]
    include_description: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct SkillEntry {
    id: String,
    /// On-disk id before `display_id` stripping (e.g. `iii-sandbox/index`).
    /// Internal only — used to classify worker-overview rows for
    /// `directory::skills::index`; never serialized, never in the schema.
    #[serde(skip)]
    on_disk_id: String,
    /// Frontmatter `title:` when present and non-empty, otherwise the
    /// first `# H1` line in the body, otherwise the bare `id`.
    title: String,
    /// Frontmatter `type:` (e.g. `index`, `how-to`, `reference`).
    /// `null` when the file has no frontmatter or omits the key.
    #[serde(rename = "type")]
    kind: Option<String>,
    /// Frontmatter `function_id:` when present — the canonical bus
    /// function id this skill documents (e.g. `sandbox::create`). The
    /// row's `id` field is the SKILL path on disk (e.g.
    /// `sandbox/skills/sandbox/create`); `function_id` is what an
    /// agent should pass to `agent_trigger`. `null` for skills that
    /// aren't 1:1 with a single function (index/reference).
    function_id: Option<String>,
    /// First paragraph of the body, empty when the file has only
    /// headings. Also empty when the caller passed
    /// `list { include_description: false }` for a token-light row.
    description: String,
    bytes: usize,
    /// File mtime as RFC 3339 (best effort; empty if unavailable).
    modified_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListSkillsOutput {
    skills: Vec<SkillEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct IndexSkillsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct IndexSkillsOutput {
    /// Rendered markdown document — one short `## <title>` block per
    /// installed worker (each worker's root overview doc, whether or not
    /// it declares frontmatter `type: index`), carrying the worker's
    /// first-paragraph overview and a `directory::skills::get` call to
    /// read the full reference. Sorted lex by id.
    body: String,
    /// Number of worker entries rendered (i.e. the count of worker
    /// overview rows that survived the filter). Cheap sanity check that
    /// doesn't require re-parsing the body.
    workers_count: usize,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SkillGetInput {
    /// Skill id (the same string returned by `directory::skills::list`,
    /// e.g. `"directory/skills/list"`). Two ergonomic variants are also
    /// accepted: the file-path form `<id>.md` (the trailing `.md` is
    /// stripped) and the legacy `iii://{id}` URI form. Other URI
    /// schemes are rejected. The filename `SKILLS.md` is aliased to
    /// `index.md` to match the filesystem scanner.
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillGetOutput {
    pub id: String,
    /// Frontmatter `title:` when present and non-empty, otherwise the
    /// first `# H1` line in the body, otherwise the bare `id`.
    pub title: String,
    /// Frontmatter `type:` (e.g. `index`, `how-to`, `reference`).
    /// `null` when the file has no frontmatter or omits the key.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Frontmatter `function_id:` when present — the canonical bus
    /// function id this skill documents (e.g. `sandbox::create`). The
    /// response's `id` field is the SKILL path on disk; `function_id`
    /// is what the agent should pass to `agent_trigger`. `null` when
    /// the skill isn't 1:1 with a single function.
    pub function_id: Option<String>,
    /// Raw markdown body (post-frontmatter) from disk.
    ///
    /// Note: there is no `description` field. `description` is the
    /// body's first paragraph, which is already inside `body` — every
    /// caller asking for the body would otherwise pay for the prefix
    /// twice. Use `directory::skills::list` rows when you want the
    /// teaser without the full body.
    pub body: String,
    /// File mtime as RFC 3339.
    pub modified_at: String,
}

// ────────────────── registered-workers cache ──────────────────────────
//
// Caches the set of installed worker names so `resolve_visible_skills`
// doesn't hit `worker::list` on every read.  The cache is:
//
//   1. Populated lazily on first read.
//   2. Invalidated when the `worker` trigger fires an `add`/`remove`.
//   3. On error / daemon-down, falls back to the last-known set.
//      If no cached set exists yet, returns `None` (meaning: unfiltered).

/// Internal cache entry. `pub(crate)` so tests in sibling modules
/// can populate / inspect it without refactoring the cache.
pub(crate) struct CacheEntry {
    pub(crate) workers: HashSet<String>,
    pub(crate) fetched_at: Instant,
}

/// Thread-safe cache of installed worker names.
pub struct RegisteredWorkersCache {
    /// `pub(crate)` so tests in sibling modules can inspect / populate.
    pub(crate) inner: Mutex<Option<CacheEntry>>,
    /// Live TTL in ms, shared with the registry cache and updated on a
    /// `configuration:updated` reload (see `configuration::apply_config`).
    ttl_ms: Arc<AtomicU64>,
}

impl RegisteredWorkersCache {
    /// Construct with a fixed TTL (used by unit tests).
    pub fn new(ttl_ms: u64) -> Self {
        Self::new_shared(Arc::new(AtomicU64::new(ttl_ms)))
    }

    /// Construct sharing a live TTL cell with the rest of the worker.
    pub fn new_shared(ttl_ms: Arc<AtomicU64>) -> Self {
        Self {
            inner: Mutex::new(None),
            ttl_ms,
        }
    }

    /// Invalidate the cache so the next `get_or_fetch` call re-fetches.
    pub async fn invalidate(&self) {
        let mut lock = self.inner.lock().await;
        *lock = None;
    }

    /// Get the cached set if fresh, or fetch from the engine via
    /// `worker::list`. Returns `None` when both the fetch fails AND
    /// there is no stale cached set — caller should fall back to
    /// unfiltered.
    ///
    /// The mutex is NOT held across the `iii.trigger` await — the lock
    /// is acquired only to check / update the cache entry. A brief
    /// duplicate fetch on simultaneous cache misses is acceptable and
    /// far cheaper than serialising all reads behind a 5 s RPC.
    pub async fn get_or_fetch(&self, iii: &IIIClient) -> Option<HashSet<String>> {
        // Phase 1: check for a fresh cache entry under the lock.
        {
            let lock = self.inner.lock().await;
            if let Some(entry) = lock.as_ref() {
                if entry.fetched_at.elapsed().as_millis()
                    < self.ttl_ms.load(Ordering::Relaxed) as u128
                {
                    return Some(entry.workers.clone());
                }
            }
            // Drop the guard before the async fetch.
        }

        self.fetch_and_store(iii).await
    }

    /// Always fetch `worker::list` fresh (ignoring the TTL), refresh the
    /// cache, and return the current registered set (falling back to the
    /// last-known set on error). Used by `directory::skills::index` so a
    /// just-registered worker shows up immediately instead of waiting on
    /// the TTL or a worker-add cache invalidation.
    pub async fn get_fresh(&self, iii: &IIIClient) -> Option<HashSet<String>> {
        self.fetch_and_store(iii).await
    }

    /// Fetch `worker::list` from the engine WITHOUT holding the lock,
    /// then store the result (or fall back to the stale set on error).
    async fn fetch_and_store(&self, iii: &IIIClient) -> Option<HashSet<String>> {
        let result = iii
            .trigger(TriggerRequest {
                function_id: "worker::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await;

        // Re-acquire the lock and store or fall back.
        let mut lock = self.inner.lock().await;
        match result {
            Ok(val) => {
                let names = parse_worker_names(&val);
                let entry = CacheEntry {
                    workers: names.clone(),
                    fetched_at: Instant::now(),
                };
                *lock = Some(entry);
                Some(names)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "worker::list failed; using last-known registered set"
                );
                // Return stale cache if available.
                lock.as_ref().map(|entry| entry.workers.clone())
            }
        }
    }
}

/// Parse worker names from the `worker::list` response.
///
/// Expected shape: `{ workers: [{ name: "foo", ... }, ...] }`.
/// Falls back to an empty set on unexpected shapes.
fn parse_worker_names(val: &serde_json::Value) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Some(workers) = val.get("workers").and_then(|w| w.as_array()) {
        for w in workers {
            if let Some(name) = w.get("name").and_then(|n| n.as_str()) {
                names.insert(name.to_string());
            }
        }
    }
    names
}

// ────────────────── resolve_visible_skills pipeline ──────────────────
//
// Single entry point for the read view. All three read functions
// (list, get, index) go through this so they can't drift.
//
// Pipeline (ASCII):
//
//   ┌───────────────┐   ┌───────────────┐
//   │ global root   │   │  local root   │
//   └───────┬───────┘   └───────┬───────┘
//           │                   │
//           └─────┬─────────────┘
//                 ▼
//     scan_skills_merged(global, local)
//        whole-namespace local-wins
//                 │
//                 ▼
//     ┌── filter_unregistered? ──┐
//     │  YES: fetch/cache        │  NO: pass through
//     │  worker::list            │
//     │  keep only matched ns    │
//     └──────────┬───────────────┘
//                ▼
//         Vec<FsSkill> (visible)

/// Resolve the visible set of skills given config and engine handle.
///
/// When `cfg.filter_unregistered` is true, only skills whose top
/// namespace segment matches a registered (installed) worker name are
/// returned. On daemon-down or first-boot-no-cache, falls back to
/// the unfiltered set.
/// Build the canonical skills-index markdown plus its worker-overview
/// count. Shared by the directory::skills::index handler and the
/// pre-generate injection hook (crate::inject_index) so the index has
/// exactly one semantics: same overview classification, same title and
/// teaser resolution, same never-drop-a-worker budget policy.
pub(crate) async fn build_index(
    cfg: &SkillsConfig,
    cache: &RegisteredWorkersCache,
    iii: &IIIClient,
    fresh: bool,
) -> (String, usize) {
    let entries = resolve_visible_skills(cfg, cache, iii, fresh).await;
    let siblings = id_set(&entries);
    let rows: Vec<SkillEntry> = entries
        .into_iter()
        .map(|fs| skill_entry_from_fs(fs, &siblings))
        .collect();
    let workers_count = rows.iter().filter(|e| is_index_overview(e)).count();
    (render_index_markdown(&rows), workers_count)
}

pub async fn resolve_visible_skills(
    cfg: &SkillsConfig,
    cache: &RegisteredWorkersCache,
    iii: &IIIClient,
    fresh: bool,
) -> Vec<FsSkill> {
    let (merged, _skipped) =
        fs_source::scan_skills_merged(&cfg.resolved_skills_folder(), &cfg.local_skills_folder());

    if !cfg.filter_unregistered {
        return merged;
    }

    // `fresh` callers (the index) re-fetch `worker::list` every call so a
    // just-registered worker is never hidden by a stale registered-workers
    // cache; cached callers (`list`/`get`) keep the TTL fast path.
    let registered = if fresh {
        cache.get_fresh(iii).await
    } else {
        cache.get_or_fetch(iii).await
    };

    match registered {
        Some(registered) => filter_to_registered(merged, &registered),
        None => {
            tracing::info!(
                "no cached registered workers and daemon unreachable; \
                 returning unfiltered skill set"
            );
            merged
        }
    }
}

/// The engine's own skill namespace. The iii engine is not a worker, so
/// it never appears in `worker::list` / the registered-workers set; its
/// skill is reconciled unconditionally (see `spawn_boot_reconcile`) and
/// kept visible regardless of `filter_unregistered`.
pub const ENGINE_NAMESPACE: &str = "iii";

/// Filter a merged skill set to only those visible given a set of
/// registered worker names. A skill is kept if:
///
/// 1. It has no namespace separator (single-segment id like `index`) —
///    these are root/bundle docs that belong to everyone.
/// 2. Its top namespace segment is `directory` — the iii-directory
///    worker's OWN docs namespace; always visible regardless of what
///    other workers are installed.
/// 3. Its top namespace segment is `iii` — the engine's own skill
///    namespace; the engine is not a worker, so it is never in the
///    `registered` set, but its skill is always visible.
/// 4. Its top namespace segment is in the `registered` set (i.e. it
///    belongs to an installed worker).
///
/// Everything else (skills from uninstalled workers) is dropped.
pub(crate) fn filter_to_registered(
    merged: Vec<FsSkill>,
    registered: &HashSet<String>,
) -> Vec<FsSkill> {
    merged
        .into_iter()
        .filter(|s| {
            let top_seg = s.id.split('/').next().unwrap_or("");
            // Single-segment ids (no `/`) are root/bundle docs — always keep.
            !s.id.contains('/')
                // The iii-directory worker's own docs namespace.
                || top_seg == "directory"
                // The engine's own skill namespace (not a worker).
                || top_seg == ENGINE_NAMESPACE
                // Belongs to a registered (installed) worker.
                || registered.contains(top_seg)
        })
        .collect()
}

pub fn register(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    let cache = Arc::new(RegisteredWorkersCache::new(
        cfg.load().registry_cache_ttl_ms,
    ));
    register_list_skills(iii, cfg, &cache);
    register_get_skill(iii, cfg, &cache);
    register_index_skills(iii, cfg, &cache);
}

/// Expose the cache so main.rs can share it with the event handler. Takes
/// the live TTL cell so a `configuration:updated` reload changes the
/// effective freshness window in place.
pub fn make_registered_cache(ttl_ms: Arc<AtomicU64>) -> Arc<RegisteredWorkersCache> {
    Arc::new(RegisteredWorkersCache::new_shared(ttl_ms))
}

/// Register all skills functions with a shared cache instance.
pub fn register_with_cache(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    cache: &Arc<RegisteredWorkersCache>,
) {
    register_list_skills(iii, cfg, cache);
    register_get_skill(iii, cfg, cache);
    register_index_skills(iii, cfg, cache);
}

fn register_list_skills(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    cache: &Arc<RegisteredWorkersCache>,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let cache_inner = cache.clone();
    iii.register_function(
        "directory::skills::list",
        RegisterFunction::new_async(move |input: ListSkillsInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let cache = cache_inner.clone();
            async move {
                let entries = resolve_visible_skills(&cfg, &cache, &iii, false).await;
                let out = list_skills_filtered(entries, &input);
                Ok::<_, Error>(ListSkillsOutput { skills: out })
            }
        })
        .description(
            "List skills as one row PER SKILL (id, title, type, function_id, description, \
             bytes, modified_at) from skills_folder — use this when you need individual \
             skill ids. A worker overview row's `id` is the bare worker name (e.g. \
             `iii-sandbox`); pass it straight to directory::skills::get. For a per-WORKER \
             overview instead, call directory::skills::index. Filters: `search` \
             (case-insens. substring vs id+title+description), `prefix` (worker-namespace \
             prefix; matches the overview row and its sub-skills), `type` (exact \
             frontmatter type match). Pass `include_description: false` for token-light \
             id+title+type rows (default: descriptions included). `title` prefers \
             frontmatter `title:` over the body H1. Each row's `function_id` is the \
             callable bus id (e.g. `sandbox::create`) — pass THAT to agent_trigger, not \
             the row's `id` (which is a documentation address).",
        ),
    );
}

/// Apply ListSkillsInput filters to the raw FsSkill stream. The cheap
/// id-only filters (`prefix`, id substring) run BEFORE the expensive
/// per-row frontmatter read so a narrowed list pays per surviving row,
/// not per file in skills_folder.
fn list_skills_filtered(entries: Vec<FsSkill>, input: &ListSkillsInput) -> Vec<SkillEntry> {
    let include_description = input.include_description.unwrap_or(true);
    let search_lc = input.search.as_deref().map(|s| s.to_lowercase());
    let prefix = input.prefix.as_deref();
    let kind_filter = input.kind.as_deref();

    // Cheap pre-screen on FsSkill { id } — `prefix` is the only filter
    // we can apply without reading the file. `search` and `type` still
    // need the per-row frontmatter read because they hit title/body or
    // the frontmatter `type:` field respectively.
    // Sibling set for display_id is the FULL view (pre-prefix-filter) so a
    // narrowing `prefix` can't hide a literal `<ns>` doc and wrongly strip
    // the `<ns>/index` overview's id.
    let siblings = id_set(&entries);
    let candidates: Vec<FsSkill> = entries
        .into_iter()
        .filter(|fs| match prefix {
            Some(p) => fs.id.starts_with(p),
            None => true,
        })
        .collect();

    let mut rows: Vec<SkillEntry> = candidates
        .into_iter()
        .map(|fs| skill_entry_from_fs(fs, &siblings))
        .filter(|row| match kind_filter {
            Some(k) => row.kind.as_deref() == Some(k),
            None => true,
        })
        .filter(|row| match &search_lc {
            Some(needle) => {
                row.id.to_lowercase().contains(needle)
                    || row.title.to_lowercase().contains(needle)
                    || row.description.to_lowercase().contains(needle)
            }
            None => true,
        })
        .collect();

    if !include_description {
        for row in &mut rows {
            row.description.clear();
        }
    }

    rows
}

fn register_get_skill(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    cache: &Arc<RegisteredWorkersCache>,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let cache_inner = cache.clone();
    iii.register_function(
        "directory::skills::get",
        RegisterFunction::new_async(move |req: SkillGetInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let cache = cache_inner.clone();
            async move {
                get_skill_visible(&cfg, &cache, &iii, req)
                    .await
                    .map_err(Error::Handler)
            }
        })
        .description(GET_DESCRIPTION)
        .metadata(json!({"tool": {"label": "Get skill"}})),
    );
}

fn register_index_skills(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    cache: &Arc<RegisteredWorkersCache>,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let cache_inner = cache.clone();
    iii.register_function(
        "directory::skills::index",
        RegisterFunction::new_async(move |_input: IndexSkillsInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let cache = cache_inner.clone();
            async move {
                let (body, workers_count) = build_index(&cfg, &cache, &iii, true).await;
                Ok::<_, Error>(IndexSkillsOutput {
                    body,
                    workers_count,
                })
            }
        })
        .description(
            "Render a per-WORKER overview: one short markdown block per installed worker \
             (each worker's root overview doc `<ns>/index`, whether or not it declares \
             frontmatter `type: index`). Each block is a `## <worker title>` heading, the \
             first paragraph of that worker's overview, and a `directory::skills::get` call \
             to read the full reference. Token-light by design and intended for \
             system-prompt injection; for individual per-SKILL rows call \
             directory::skills::list.",
        ),
    );
}

// ---------- core handler ----------

/// Agent-facing display id. A worker overview's on-disk id carries a
/// trailing `/index` (e.g. `iii-sandbox/index`, `resend/emails/index`), but
/// agents address a worker by its bare name (`iii-sandbox`, `resend/emails`).
/// Strip that trailing segment for everything RETURNED to the agent. The
/// on-disk id, lookups, and registry layout keep the `/index` form — this is
/// presentation only, and input still accepts both forms (see
/// [`find_fs_skill_in`], which aliases `<id>` back to `<id>/index`). A bare
/// single-segment `index` (a root bundle doc) is left untouched.
///
/// `siblings` is the set of on-disk ids in the same view. The `/index`
/// suffix is stripped ONLY when the bare form does not collide with a
/// literal sibling doc of that exact id: a worker that ships BOTH
/// `<ns>.md` (id `<ns>`) and `<ns>/index.md` (id `<ns>/index`) keeps the
/// overview's id as `<ns>/index`, so the two stay distinct and the overview
/// remains addressable.
fn display_id(on_disk: &str, siblings: &std::collections::HashSet<String>) -> String {
    match on_disk.strip_suffix("/index") {
        Some(bare) if !siblings.contains(bare) => bare.to_string(),
        _ => on_disk.to_string(),
    }
}

/// Build the sibling id-set [`display_id`] needs from a skill view.
fn id_set(skills: &[FsSkill]) -> std::collections::HashSet<String> {
    skills.iter().map(|s| s.id.clone()).collect()
}

/// Read one filesystem skill into a `SkillGetOutput`. Shared by the
/// happy path and the engine-skill fallback. The returned `id` is the
/// agent-facing [`display_id`] (no `/index`); the title falls back to the
/// same display id so an untitled overview reads as the worker name.
fn read_skill_output(
    fs: &FsSkill,
    siblings: &std::collections::HashSet<String>,
) -> Result<SkillGetOutput, String> {
    let (fm, body) = fs_source::read_skill_with_frontmatter(&fs.abs_path)?;
    let display = display_id(&fs.id, siblings);
    let title = resolve_title(&fm, &body, &display);
    let kind = clean_optional(fm.kind);
    let function_id = clean_optional(fm.function_id);
    let (_, modified_at) = fs_metadata(fs);
    Ok(SkillGetOutput {
        id: display,
        title,
        kind,
        function_id,
        body,
        modified_at,
    })
}

/// Decide whether a missed skill id should fall back to the engine
/// overview. Returns `Some(worker)` when the missed id's top segment
/// resolves (via [`resolve_worker`]) to an installed worker that ships NO
/// skill doc at all — e.g. `iii-sandbox`, an engine-builtin with no
/// published skills. In that case `get` serves `iii/index` with a note
/// pointing at engine introspection instead of dead-ending. Resolution is
/// shared with [`worker_overview_fallback`] so a colloquial name
/// (`sandbox` → `iii-sandbox`) reaches the engine overview just like the
/// exact name does. When the worker DOES ship skills (the caller asked for
/// a wrong sub-path), returns `None` so the closest-id suggestions apply.
fn engine_fallback_worker(
    id: &str,
    visible: &[FsSkill],
    registered: &std::collections::HashSet<String>,
) -> Option<String> {
    let top = id.split('/').next().unwrap_or("");
    let ns = resolve_worker(top, registered)?;
    // Only fall back when the resolved worker ships no skill doc at all.
    let prefix = format!("{ns}/");
    let has_doc = visible
        .iter()
        .any(|s| s.id == ns || s.id.starts_with(&prefix));
    if has_doc {
        None
    } else {
        Some(ns)
    }
}

/// Markdown note prepended to the engine overview when `get` falls back
/// because the requested worker ships no skill doc.
fn skilless_worker_note(worker: &str) -> String {
    format!(
        "> Note: the worker `{worker}` is installed but ships no skill doc, so \
         `{worker}` has no overview. Showing the iii engine overview below. \
         For `{worker}`'s own API, call `directory::engine::functions::list` with \
         worker={worker}, or `directory::engine::workers::info` with name={worker}.\n\n---\n\n"
    )
}

/// Error used when a skill-less installed worker is requested AND the
/// engine overview itself isn't on disk to fall back to.
fn skilless_worker_message(worker: &str, missed: &str) -> String {
    format!(
        "D110 not_found: \"{missed}\" does not exist — the worker `{worker}` is installed \
         but ships no skill doc. Next: call directory::engine::functions::list with \
         worker={worker} to list its functions; or directory::engine::workers::info with \
         name={worker}."
    )
}

/// Match a query's top segment to a single installed worker NAME by
/// case-insensitive substring (`sandbox` ⊂ `iii-sandbox`, `memory` ⊂
/// `agent-memory`). The corpus is the registered worker names plus the
/// always-visible `directory` namespace.
///
/// Resolution is INDEPENDENT of whether the worker ships a doc: an exact
/// (case-insensitive) worker-name match wins outright — even a skill-less
/// worker — so a substring can never hijack a query that exactly names an
/// installed worker (a query `box` resolves the installed `box`, not the
/// longer `iii-sandbox` that merely contains "box"). Whether the resolved
/// worker has an overview is decided by the CALLERS
/// ([`worker_overview_fallback`] serves the overview; [`engine_fallback_worker`]
/// serves the engine overview for a skill-less worker).
///
/// Cardinality: exact wins; else the uniquely shortest substring match wins
/// (most specific); a length tie is ambiguous → `None`, so the caller falls
/// through to the ranked-suggestion list. The engine namespace (`iii`) is
/// excluded — a bare `iii` already resolves on the happy path, and as a
/// substring it would match every `iii-*` worker.
fn resolve_worker(top: &str, registered: &std::collections::HashSet<String>) -> Option<String> {
    if top.is_empty() || top == ENGINE_NAMESPACE {
        return None;
    }
    // Installed workers + the directory worker (always visible, never in
    // the registered set).
    let mut cands: Vec<String> = registered.iter().cloned().collect();
    cands.push("directory".to_string());
    cands.sort();
    cands.dedup();

    // An exact (case-insensitive) worker-name match wins outright — overview
    // or not — so a substring can't shadow an exactly-named installed worker.
    if let Some(hit) = cands.iter().find(|c| c.eq_ignore_ascii_case(top)) {
        return Some(hit.clone());
    }
    // Otherwise, case-insensitive substring matches.
    let lc = top.to_lowercase();
    let mut subs: Vec<String> = cands
        .into_iter()
        .filter(|c| c.to_lowercase().contains(&lc))
        .collect();
    subs.sort_by_key(|c| c.len());
    match subs.as_slice() {
        [only] => Some(only.clone()),
        // Uniquely shortest match wins; a length tie at the front is
        // ambiguous → defer to the suggester.
        [first, second, ..] if first.len() < second.len() => Some(first.clone()),
        _ => None,
    }
}

/// Resolve a missed skill id to a worker's overview doc by worker NAME,
/// covering the two ways agents reach for a worker they can't id exactly:
///
///   * the bare colloquial name — `get id=sandbox` — and
///   * a path built from a function id — `sandbox/create`,
///     `iii-sandbox/sandbox/create`.
///
/// The top segment is mapped to a worker namespace via
/// [`resolve_worker_ns`] (case-insensitive substring), which counts a
/// namespace only when its `<ns>/index` overview is present in `visible`.
///
/// A BARE worker name asks for the worker itself, so its overview is
/// served regardless of how many skills the worker ships. A SUB-PATH only
/// collapses when the worker's ONLY visible doc is that overview (a
/// single-skill worker like `iii-sandbox`); multi-skill workers like
/// `directory` keep precise closest-id suggestions, which beat a coarse
/// worker-root collapse. The engine namespace, unmatched namespaces,
/// and workers that ship NO doc at all return `None` (the last are left to
/// [`engine_fallback_worker`]).
fn worker_overview_fallback(
    id: &str,
    visible: &[FsSkill],
    registered: &std::collections::HashSet<String>,
) -> Option<String> {
    let top = id.split('/').next().unwrap_or("");
    // Map the query's top segment to a worker (case-insensitive substring,
    // exact-name-wins), so a colloquial name (`sandbox` → `iii-sandbox`) lands.
    let ns = resolve_worker(top, registered)?;
    // The overview must exist for THIS path; a skill-less worker is left to
    // engine_fallback_worker (which resolves the same `ns`).
    let overview = format!("{ns}/index");
    let fs = find_fs_skill_in(visible, &overview)?;
    // A sub-path only collapses for a single-skill worker; a bare worker
    // name always resolves to the overview.
    if id.contains('/') {
        let prefix = format!("{ns}/");
        let mut docs = visible
            .iter()
            .filter(|s| s.id == ns || s.id.starts_with(&prefix));
        let first = docs.next()?;
        if docs.next().is_some() || first.id != overview {
            return None;
        }
    }
    Some(fs.id)
}

/// Markdown note prepended to a worker overview when `get` collapses a
/// missed sub-path to it. Keeps the redirect honest: names what was asked,
/// what is being served, and how to find the worker's other surfaces.
fn worker_overview_redirect_note(
    missed: &str,
    overview: &str,
    siblings: &std::collections::HashSet<String>,
) -> String {
    // `prefix` keeps the on-disk `<ns>/` form (the list filter matches raw
    // ids); the shown overview id is the agent-facing bare name.
    let prefix = overview.strip_suffix("index").unwrap_or(overview);
    let shown = display_id(overview, siblings);
    format!(
        "> Note: no skill `{missed}`. Showing `{shown}` (the worker overview) instead. \
         For this worker's callable functions use `directory::engine::functions::list`; \
         for any other skills it ships call `directory::skills::list` with \
         prefix=\"{prefix}\".\n\n---\n\n"
    )
}

/// Visible-skills-aware `get`. Used by the registered handler. Resolves
/// skills through the merged + filtered pipeline so `get` can't return
/// a skill that `list`/`index` would hide. On a miss whose namespace is
/// an installed-but-skill-less worker, falls back to the engine overview.
async fn get_skill_visible(
    cfg: &SkillsConfig,
    cache: &RegisteredWorkersCache,
    iii: &IIIClient,
    req: SkillGetInput,
) -> Result<SkillGetOutput, String> {
    let id = normalize_get_id(&req.id)?;
    reject_function_id_shaped(&id)?;
    validate_id(&id)?;
    let visible = resolve_visible_skills(cfg, cache, iii, false).await;
    let siblings = id_set(&visible);

    if let Some(fs) = find_fs_skill_in(&visible, &id) {
        return read_skill_output(&fs, &siblings);
    }

    // Miss. Two recovery paths, in order of specificity:
    let registered = cache.get_or_fetch(iii).await.unwrap_or_default();

    // 1. A wrong sub-path under a single-skill worker (agents fabricate
    //    skill paths from function ids, e.g. `iii-sandbox/sandbox/create`)
    //    collapses straight to that worker's overview — one call, not three.
    if let Some(overview_id) = worker_overview_fallback(&id, &visible, &registered) {
        if let Some(fs) = find_fs_skill_in(&visible, &overview_id) {
            let mut out = read_skill_output(&fs, &siblings)?;
            out.body = format!(
                "{}{}",
                worker_overview_redirect_note(&id, &overview_id, &siblings),
                out.body
            );
            return Ok(out);
        }
    }

    // 2. If the requested namespace is an installed worker that ships
    //    no skill doc, serve the engine overview (iii/index) with a note +
    //    pointer to engine introspection rather than dead-ending the caller.
    if let Some(worker) = engine_fallback_worker(&id, &visible, &registered) {
        let engine_id = format!("{ENGINE_NAMESPACE}/index");
        if let Some(eng) = find_fs_skill_in(&visible, &engine_id) {
            let mut out = read_skill_output(&eng, &siblings)?;
            out.body = format!("{}{}", skilless_worker_note(&worker), out.body);
            return Ok(out);
        }
        return Err(skilless_worker_message(&worker, &id));
    }

    let candidates: Vec<String> = rank_suggestions_in(&visible, &id, 3)
        .into_iter()
        .map(|s| display_id(&s.id, &siblings))
        .collect();
    Err(not_found_message(
        "D110",
        "skill",
        &id,
        &candidates,
        SKILL_NOT_FOUND_NEXT,
    ))
}

/// Standalone `get_skill` for unit tests that don't have an engine.
/// Scans the single-root skills folder (no merged view, no filter).
pub async fn get_skill(cfg: &SkillsConfig, req: SkillGetInput) -> Result<SkillGetOutput, String> {
    let id = normalize_get_id(&req.id)?;
    reject_function_id_shaped(&id)?;
    validate_id(&id)?;
    let (fs_all, _skipped) = fs_source::scan_skills(&cfg.resolved_skills_folder());
    let siblings = id_set(&fs_all);
    let Some(fs) = find_fs_skill_in(&fs_all, &id) else {
        let candidates: Vec<String> = rank_suggestions_in(&fs_all, &id, 3)
            .into_iter()
            .map(|s| display_id(&s.id, &siblings))
            .collect();
        return Err(not_found_message(
            "D110",
            "skill",
            &id,
            &candidates,
            SKILL_NOT_FOUND_NEXT,
        ));
    };
    let (fm, body) = fs_source::read_skill_with_frontmatter(&fs.abs_path)?;
    let display = display_id(&fs.id, &siblings);
    let title = resolve_title(&fm, &body, &display);
    let kind = clean_optional(fm.kind);
    let function_id = clean_optional(fm.function_id);
    let (_, modified_at) = fs_metadata(&fs);
    Ok(SkillGetOutput {
        id: display,
        title,
        kind,
        function_id,
        body,
        modified_at,
    })
}

/// Trim and strip an optional `iii://` prefix; reject any other URI
/// scheme. Also accepts a file-path form: a trailing `.md` is stripped
/// so callers can paste either `hello-worker/index` or
/// `hello-worker/index.md` and get the same id. The literal filename
/// `SKILLS.md` (final path component) is aliased to `index.md` — same
/// rule the filesystem scanner uses. The remaining string still has to
/// satisfy [`validate_id`]; this function only handles the prefix /
/// suffix ergonomics.
pub fn normalize_get_id(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("id must be non-empty".into());
    }
    let without_scheme = if let Some(rest) = trimmed.strip_prefix(URI_PREFIX) {
        rest
    } else if trimmed.contains("://") {
        return Err(format!(
            "Invalid id (must be a bare skill path, a path ending in .md, or an iii:// URI): {trimmed}"
        ));
    } else {
        trimmed
    };
    let aliased = if let Some(stem) = without_scheme.strip_suffix("/SKILLS.md") {
        format!("{stem}/index")
    } else if let Some(stem) = without_scheme.strip_suffix("/SKILL.md") {
        format!("{stem}/index")
    } else if without_scheme == "SKILLS.md" || without_scheme == "SKILL.md" {
        "index".to_string()
    } else {
        without_scheme
            .strip_suffix(".md")
            .unwrap_or(without_scheme)
            .to_string()
    };
    Ok(aliased)
}

// ---------- validation ----------

/// Validate a single id segment.
pub fn validate_id_segment(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("segment must be non-empty".into());
    }
    if s.len() > ID_SEGMENT_MAX_LEN {
        return Err(format!(
            "segment too long ({} chars; max {ID_SEGMENT_MAX_LEN}): {s:?}",
            s.len()
        ));
    }
    for c in s.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if !ok {
            return Err(format!(
                "segment may only contain lowercase ASCII letters, digits, '-' and '_': {s:?}"
            ));
        }
    }
    Ok(())
}

/// Validate a full skill id. Accepts 1+ segments separated by `/`.
pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("id must be non-empty".into());
    }
    if id.starts_with('/') || id.ends_with('/') {
        return Err(format!("id may not have a leading or trailing '/': {id:?}"));
    }
    if id.len() > ID_TOTAL_MAX_LEN {
        return Err(format!(
            "id too long ({} chars; max {ID_TOTAL_MAX_LEN}): {id:?}",
            id.len()
        ));
    }
    let segments: Vec<&str> = id.split('/').collect();
    for (i, seg) in segments.iter().enumerate() {
        validate_id_segment(seg)
            .map_err(|e| format!("invalid id (segment {} of {:?}): {e}", i + 1, id))?;
    }
    Ok(())
}

/// Common dumb-agent mistake: passing a FUNCTION id (`service::name`, e.g.
/// `database::execute`) to `get`, which takes a SKILL id (`database/index`).
/// `::` can never appear in a valid skill id, so detect it and return a
/// targeted, self-correcting message instead of a raw "invalid segment"
/// rejection the agent can't act on.
fn reject_function_id_shaped(id: &str) -> Result<(), String> {
    if id.contains("::") {
        return Err(invalid_input_message(
            "D112",
            &format!(
                "{id:?} looks like a FUNCTION id (service::name), not a skill id. \
                 Skill ids use '/' (e.g. \"database/index\"). To CALL that function pass \
                 the id to agent_trigger; to READ its skill doc, look up the skill id."
            ),
            SKILL_NOT_FOUND_NEXT,
        ));
    }
    Ok(())
}

// ---------- markdown helpers ----------

pub fn extract_title(markdown: &str) -> Option<&str> {
    markdown.lines().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed.strip_prefix("# ").map(|s| s.trim())
    })
}

/// Pick the best title for a skill: frontmatter `title:` (when present
/// and non-empty after trim), then the first body `# H1`, then the
/// bare `id` so the response field is never empty.
pub fn resolve_title(fm: &SkillFrontmatter, body: &str, id: &str) -> String {
    if let Some(t) = fm.title.as_deref() {
        let trimmed = t.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(h1) = extract_title(body) {
        if !h1.is_empty() {
            return h1.to_string();
        }
    }
    id.to_string()
}

/// Trim, then drop the value when the result is empty. Used to keep
/// the response `type` field as `null` rather than an empty string
/// when the frontmatter declares `type:` with no value.
pub fn clean_optional(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn extract_description(markdown: &str) -> Option<String> {
    let mut buf = String::new();
    let mut in_blockquote = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        let is_quote = trimmed.starts_with('>');
        // A blockquote ends at the first line that doesn't continue it.
        // Clear the flag eagerly so the rest of the body parses normally
        // — including the case where a heading appears immediately
        // after the callout with no blank-line separator.
        if in_blockquote && !is_quote {
            in_blockquote = false;
        }
        if trimmed.is_empty() {
            if !buf.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            if !buf.is_empty() {
                break;
            }
            continue;
        }
        // Skip leading blockquote callouts (`> ...`) — used for
        // "Function id:" hints under the frontmatter so agents see the
        // callable name. The picker / list UI wants the first real
        // paragraph, not the operator-side preamble.
        if is_quote {
            if !buf.is_empty() {
                break;
            }
            in_blockquote = true;
            continue;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(trimmed);
    }
    if buf.is_empty() {
        return None;
    }
    Some(buf)
}

/// Character budget cap for the per-worker DESCRIPTION paragraphs in the
/// rendered index. Every worker's heading and `get` pointer are always
/// emitted (the index must list every installed worker); only the
/// description paragraph is dropped once the running body would exceed
/// this limit, with a note pointing at `directory::skills::get` for the
/// full reference. Workers themselves are never truncated away.
const INDEX_CHAR_BUDGET: usize = 3000;

/// True when `on_disk_id` is the root overview doc of a top-level worker
/// namespace, i.e. exactly `<ns>/index` (one `/`, ends with `/index`).
/// Nested indexes (`<ns>/sub/index`) and the bare root bundle doc (`index`)
/// are NOT worker overviews.
fn is_worker_overview(on_disk_id: &str) -> bool {
    on_disk_id.ends_with("/index") && on_disk_id.matches('/').count() == 1
}

/// A row counts as a worker overview for `directory::skills::index` when it
/// EITHER declares frontmatter `type: index` OR is the namespace-root
/// overview doc (`<ns>/index`). The second clause surfaces legacy bundles
/// that predate the `type: index` convention: their overview ships as a
/// bare `<ns>/index.md` with `name:`/`description:` frontmatter and no
/// `type:`, so without it those workers never appeared in the index at all.
fn is_index_overview(entry: &SkillEntry) -> bool {
    entry.kind.as_deref() == Some("index") || is_worker_overview(&entry.on_disk_id)
}

/// Render a `directory::skills::index` markdown document from already
/// title/description-resolved rows. Keeps one block per installed worker
/// — every worker's root overview doc (`<ns>/index`), whether or not it
/// declares frontmatter `type: index` (see [`is_index_overview`]) — and
/// emits a compact per-worker block:
///
/// ```markdown
/// # Skills index
///
/// N worker(s).
///
/// ## <resolved title>
///
/// <first paragraph from the worker's overview>
///
/// Full reference: call `directory::skills::get { "id": "<id>" }` (legacy `iii://<id>`).
/// ```
///
/// The pointer names the directory's own `get` function — the in-engine
/// way to read the full doc — rather than a file path or an external URL
/// the agent can't open. The legacy `iii://<id>` token is retained so
/// harnesses that grep for the old URI scheme keep working.
///
/// The description block is omitted (no extra blank line) when the
/// overview body has no paragraph. Entries must already be sorted lex
/// by `id` (the order `fs_source::scan_skills` returns); this function
/// does not re-sort.
///
/// Every worker overview row is ALWAYS rendered (heading + `get` pointer)
/// so the index lists every installed worker. Only the optional
/// description paragraph is budget-sensitive: once the running body would
/// exceed [`INDEX_CHAR_BUDGET`], later descriptions are omitted (with a
/// note pointing at `directory::skills::get`) while the remaining workers
/// are still listed.
fn render_index_markdown(entries: &[SkillEntry]) -> String {
    let workers: Vec<&SkillEntry> = entries.iter().filter(|e| is_index_overview(e)).collect();

    let mut out = String::new();
    out.push_str("# Skills index\n\n");
    out.push_str(&format!("{} worker(s).\n", workers.len()));

    let mut omitted_descriptions = false;

    for worker in &workers {
        out.push('\n');
        out.push_str(&format!("## {}\n", worker.title));

        // The heading and the get-pointer are ALWAYS emitted so every
        // installed worker appears — the index is the discovery surface, so
        // dropping a worker entirely is never acceptable. Only the optional
        // description paragraph is budget-sensitive: keep it while the body
        // stays under INDEX_CHAR_BUDGET, otherwise omit it (recoverable via
        // directory::skills::get) and keep listing the remaining workers.
        if !worker.description.is_empty() {
            let desc_cost = "\n".len() + worker.description.len() + "\n".len();
            if out.len() + desc_cost <= INDEX_CHAR_BUDGET {
                out.push('\n');
                out.push_str(&format!("{}\n", worker.description));
            } else {
                omitted_descriptions = true;
            }
        }

        out.push('\n');
        out.push_str(&format!(
            "Full reference: call `directory::skills::get {{ \"id\": \"{id}\" }}` \
             (legacy `iii://{id}`).\n",
            id = worker.id
        ));
    }

    if omitted_descriptions {
        out.push_str(
            "\n(some descriptions omitted to save space; call directory::skills::get for the full reference)\n",
        );
    }

    out
}

// ---------- fs lookup ----------

/// Targeted lookup for the read path against a pre-scanned list.
/// Returns `None` if no entry matches `id`.
///
/// An **overview shorthand** is resolved by aliasing `<id>` to
/// `<id>/index`: a bare worker name (`iii-sandbox`) resolves to
/// `iii-sandbox/index`, and a nested overview shorthand (`resend/emails`)
/// to `resend/emails/index`. This is the inverse of [`display_id`], so the
/// `/index`-stripped ids `get`/`list` hand back round-trip cleanly. An
/// exact literal match always wins over the alias (so a literal `sandbox`
/// doc shadows `sandbox/index`). The alias resolves ONLY to a real
/// `<id>/index` overview that exists, never to a sibling skill — so a
/// function-shaped typo like `sandbox/exec` (no `sandbox/exec/index`)
/// still misses rather than silently resolving wrong.
fn find_fs_skill_in(skills: &[FsSkill], id: &str) -> Option<FsSkill> {
    let alias = format!("{id}/index");
    let mut exact: Option<FsSkill> = None;
    let mut aliased: Option<FsSkill> = None;
    for skill in skills {
        if skill.id == id {
            exact = Some(skill.clone());
            continue;
        }
        if skill.id == alias {
            aliased = Some(skill.clone());
        }
    }
    exact.or(aliased)
}

/// Rank candidate skill ids by closeness to a missed id and return the
/// top `limit`, fully resolved (title + type) for the structured
/// `skill_not_found` envelope.
///
/// Scoring: `shared_segments * 100 - levenshtein(missed, candidate)`.
/// Shared-segments dominates (a candidate sharing a worker namespace
/// always outranks one with the same string distance but no shared
/// segment), so a request for `iii/skills/sandbox/index` against a
/// catalog with `sandbox/index` ranks `sandbox/index` (shared seg
/// `sandbox`) above `iii/index` (shared seg `iii` AND closer string
/// distance — but loses on segment specificity when bigram weighting
/// also boosts `sandbox`).
///
/// Single-segment misses (the bare-worker case) bypass the bare-name
/// alias already handled in [`find_fs_skill_in`]. Still run the
/// ranker — it does the right thing by finding the closest worker id.
///
/// Returns at most `limit` entries; empty when the catalog itself is
/// empty. Never errors — a `read_skill_with_frontmatter` failure on a
/// candidate just demotes that row to (`id`, kind=None, title=id).
fn rank_suggestions_in(skills: &[FsSkill], missed: &str, limit: usize) -> Vec<SuggestEntry> {
    if skills.is_empty() {
        return Vec::new();
    }
    let missed_segs: Vec<&str> = missed.split('/').filter(|s| !s.is_empty()).collect();
    let missed_lc = missed.to_lowercase();

    let mut scored: Vec<(i32, &FsSkill)> = skills
        .iter()
        .map(|skill| {
            let cand_segs: Vec<&str> = skill.id.split('/').collect();
            let shared: i32 = missed_segs
                .iter()
                .filter(|seg| cand_segs.contains(seg))
                .count() as i32;
            let dist = levenshtein(&missed_lc, &skill.id.to_lowercase()) as i32;
            let score = shared * 100 - dist;
            (score, skill)
        })
        .collect();

    scored.sort_by_key(|b| std::cmp::Reverse(b.0));

    scored
        .into_iter()
        .take(limit)
        .filter(|(score, _)| *score > 0)
        .map(
            |(score, skill)| match fs_source::read_skill_with_frontmatter(&skill.abs_path) {
                Ok((fm, body)) => SuggestEntry {
                    id: skill.id.clone(),
                    title: resolve_title(&fm, &body, &skill.id),
                    kind: clean_optional(fm.kind),
                    score,
                },
                Err(_) => SuggestEntry {
                    id: skill.id.clone(),
                    title: skill.id.clone(),
                    kind: None,
                    score,
                },
            },
        )
        .collect()
}

/// Iterative two-row Levenshtein distance. Used by [`rank_suggestions_in`]
/// to break ties on shared-segment count, and re-used by the prompts
/// not-found ranker. Allocates two `usize` rows of size
/// `b.chars().count() + 1`; cost is O(|a| * |b|) which is fine for skill
/// ids / prompt names (capped at [`ID_TOTAL_MAX_LEN`] = 1024).
pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    if a_chars.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a_chars.len();
    }
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr: Vec<usize> = vec![0; b_chars.len() + 1];
    for (i, ca) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b_chars.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (curr[j] + 1) // insertion
                .min(prev[j + 1] + 1) // deletion
                .min(prev[j] + cost); // substitution
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

/// Build a `SkillEntry` for `list` output. Reads the file body and
/// frontmatter so the row carries title + type + function_id +
/// description; on read failure the row still surfaces the id with
/// empty title / null type / null function_id / empty description so a
/// single broken file doesn't hide every other skill from the picker.
///
/// Description precedence:
/// 1. Frontmatter `description:` when present and non-empty (after trim).
/// 2. Body first-paragraph via [`extract_description`] (fallback).
fn skill_entry_from_fs(fs: FsSkill, siblings: &std::collections::HashSet<String>) -> SkillEntry {
    let (bytes, modified_at) = fs_metadata(&fs);
    // Agent-facing id drops the `/index` overview suffix; title falls back
    // to the same display id. Filtering already ran against the raw on-disk
    // id (see list_skills_filtered), so stripping here is display-only.
    let display = display_id(&fs.id, siblings);
    let (title, kind, function_id, description) =
        match fs_source::read_skill_with_frontmatter(&fs.abs_path) {
            Ok((fm, body)) => {
                let title = resolve_title(&fm, &body, &display);
                let kind = clean_optional(fm.kind);
                let function_id = clean_optional(fm.function_id);
                // Prefer frontmatter description; fall back to body
                // first-paragraph so skills with NO frontmatter
                // description still get the body-derived text.
                let description = fm
                    .description
                    .as_deref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| extract_description(&body))
                    .unwrap_or_default();
                (title, kind, function_id, description)
            }
            Err(_) => (display.clone(), None, None, String::new()),
        };
    SkillEntry {
        id: display,
        on_disk_id: fs.id,
        title,
        kind,
        function_id,
        description,
        bytes,
        modified_at,
    }
}

/// Cheap metadata for `skills::list` rows. Bytes is the on-disk file
/// size; `modified_at` is the file's mtime as RFC 3339.
fn fs_metadata(skill: &FsSkill) -> (usize, String) {
    match std::fs::metadata(&skill.abs_path) {
        Ok(meta) => {
            let bytes = meta.len() as usize;
            let modified = meta
                .modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                .unwrap_or_default();
            (bytes, modified)
        }
        Err(_) => (0, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    // ── normalize_get_id ────────────────────────────────────────────────

    #[test]
    fn normalize_accepts_bare_id() {
        assert_eq!(
            normalize_get_id("agent-memory/observe").unwrap(),
            "agent-memory/observe"
        );
    }

    #[test]
    fn normalize_strips_iii_prefix() {
        assert_eq!(
            normalize_get_id("iii://agent-memory/observe").unwrap(),
            "agent-memory/observe"
        );
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_get_id("  iii://foo  ").unwrap(), "foo");
        assert_eq!(normalize_get_id("\nfoo\t").unwrap(), "foo");
    }

    #[test]
    fn normalize_rejects_empty() {
        assert!(normalize_get_id("").is_err());
        assert!(normalize_get_id("   ").is_err());
    }

    #[test]
    fn normalize_rejects_other_uri_schemes() {
        let err = normalize_get_id("https://example.com").unwrap_err();
        assert!(err.contains("iii://"), "got: {err}");
        assert!(normalize_get_id("ftp://nope").is_err());
    }

    #[test]
    fn normalize_strips_md_suffix_on_bare_path() {
        assert_eq!(
            normalize_get_id("hello-worker/index.md").unwrap(),
            "hello-worker/index"
        );
    }

    #[test]
    fn normalize_aliases_skills_md_to_index() {
        assert_eq!(
            normalize_get_id("hello-worker/SKILLS.md").unwrap(),
            "hello-worker/index"
        );
    }

    #[test]
    fn normalize_aliases_nested_skills_md_to_index() {
        assert_eq!(
            normalize_get_id("resend/emails/SKILLS.md").unwrap(),
            "resend/emails/index"
        );
    }

    #[test]
    fn normalize_strips_md_after_iii_prefix() {
        assert_eq!(
            normalize_get_id("iii://hello-worker/index.md").unwrap(),
            "hello-worker/index"
        );
    }

    #[test]
    fn normalize_does_not_strip_md_in_middle_of_path() {
        // ".md" inside a segment is a real id, not a file suffix.
        assert_eq!(
            normalize_get_id("hello-worker/index_md").unwrap(),
            "hello-worker/index_md"
        );
    }

    // ── iii:// back-compat ─────────────────────────────────────────────

    #[test]
    fn normalize_iii_prefix_with_skills_md_aliases_to_index() {
        // `iii://` + `SKILLS.md` filename composes through both transforms.
        assert_eq!(normalize_get_id("iii://ns/SKILLS.md").unwrap(), "ns/index");
    }

    #[test]
    fn normalize_iii_prefix_with_nested_skills_md_aliases_to_index() {
        assert_eq!(
            normalize_get_id("iii://resend/emails/SKILLS.md").unwrap(),
            "resend/emails/index"
        );
    }

    #[test]
    fn normalize_iii_prefix_round_trips_with_render_emitted_id() {
        // The `iii://<id>` token render_index_markdown emits for the
        // legacy-pointer footer now carries the bare worker name (the
        // overview's display id). It must parse back through normalize_get_id
        // to that bare name, which then resolves via the find_fs_skill_in
        // `<id>/index` alias.
        let emitted = "iii://agent-memory";
        assert_eq!(normalize_get_id(emitted).unwrap(), "agent-memory");
        // The legacy `iii://<ns>/index` form must still parse too (back-compat
        // for anything that cached the old pointer).
        assert_eq!(
            normalize_get_id("iii://agent-memory/index").unwrap(),
            "agent-memory/index"
        );
    }

    // ── validate_id: happy paths ────────────────────────────────────────

    #[test]
    fn id_validation_accepts_single_segment() {
        assert!(validate_id("brain").is_ok());
        assert!(validate_id("agent_memory").is_ok());
        assert!(validate_id("my-skill-1").is_ok());
        assert!(validate_id("a").is_ok());
    }

    #[test]
    fn id_validation_accepts_multi_segment() {
        assert!(validate_id("a/b").is_ok());
        assert!(validate_id("a/b/c").is_ok());
        assert!(validate_id("a/b/c/d/e").is_ok());
        assert!(validate_id("resend/email/send").is_ok());
    }

    #[test]
    fn id_validation_allows_fn_segment_anywhere() {
        assert!(validate_id("fn").is_ok());
        assert!(validate_id("fn/anything").is_ok());
        assert!(validate_id("docs/fn-reference").is_ok());
        assert!(validate_id("a/fn/c").is_ok());
    }

    // ── validate_id: error cases ────────────────────────────────────────

    #[test]
    fn id_validation_rejects_bad_chars() {
        assert!(validate_id("").is_err());
        assert!(validate_id("UpperCase").is_err());
        assert!(validate_id("with space").is_err());
        assert!(validate_id("with::colon").is_err());
    }

    #[test]
    fn id_validation_rejects_leading_or_trailing_slash() {
        assert!(validate_id("/a").is_err());
        assert!(validate_id("a/").is_err());
        assert!(validate_id("a//b").is_err());
    }

    #[test]
    fn id_validation_enforces_per_segment_length() {
        let too_long = "x".repeat(ID_SEGMENT_MAX_LEN + 1);
        assert!(validate_id(&too_long).is_err());
        let nested_with_long_segment = format!("ok/{too_long}");
        assert!(validate_id(&nested_with_long_segment).is_err());
        let max_segment = "x".repeat(ID_SEGMENT_MAX_LEN);
        assert!(validate_id(&max_segment).is_ok());
    }

    #[test]
    fn id_validation_enforces_total_length() {
        let too_long: String = "ab/".repeat((ID_TOTAL_MAX_LEN / 3) + 5);
        let trimmed = too_long.trim_end_matches('/').to_string();
        assert!(trimmed.len() > ID_TOTAL_MAX_LEN);
        assert!(validate_id(&trimmed).is_err());
    }

    // ── extract_title / extract_description ─────────────────────────────

    #[test]
    fn extract_title_finds_h1() {
        let md = "# my skill\n\nbody\n";
        assert_eq!(extract_title(md), Some("my skill"));
    }

    #[test]
    fn extract_title_ignores_h2() {
        let md = "## sub\n\nbody\n";
        assert_eq!(extract_title(md), None);
    }

    #[test]
    fn extract_description_grabs_first_paragraph() {
        let md = "# title\n\nfirst paragraph here.\n\nsecond paragraph.\n";
        assert_eq!(
            extract_description(md).as_deref(),
            Some("first paragraph here.")
        );
    }

    #[test]
    fn extract_description_skips_subheadings() {
        let md = "# title\n\n## sub\n\n### deeper\n\nfinally text.\n";
        assert_eq!(extract_description(md).as_deref(), Some("finally text."));
    }

    #[test]
    fn extract_description_handles_missing_paragraph() {
        let md = "# only a title\n";
        assert_eq!(extract_description(md), None);
    }

    #[test]
    fn extract_description_skips_leading_blockquote_callout() {
        // The "Function id:" callouts ship as blockquotes right under
        // the frontmatter so agents see the callable name without
        // hunting through the body. They must NOT become the picker's
        // description — that's reserved for the first real paragraph.
        let md = "\
> **Function id:** `sandbox::create` — pass this to agent_trigger.

# When to use

Boot a sandbox to run untrusted code.
";
        assert_eq!(
            extract_description(md),
            Some("Boot a sandbox to run untrusted code.".into()),
        );
    }

    #[test]
    fn extract_description_skips_multi_line_blockquote_callout() {
        let md = "\
> **Function id:** `sandbox::create`
> Pass this to agent_trigger, not the skill path.

The actual description starts here.
";
        assert_eq!(
            extract_description(md),
            Some("The actual description starts here.".into()),
        );
    }

    #[test]
    fn extract_description_after_blockquote_with_no_blank_separator() {
        // Edge case: blockquote followed directly by a heading (no
        // blank line). Still treats the heading as a separator and
        // returns the first paragraph after it.
        let md = "\
> **Function id:** `x::y`
# Heading
First paragraph.
";
        assert_eq!(extract_description(md), Some("First paragraph.".into()),);
    }

    #[test]
    fn extract_description_keeps_long_first_paragraph() {
        let body = "x".repeat(200);
        let md = format!("# t\n\n{body}\n");
        let desc = extract_description(&md).unwrap();
        assert_eq!(desc, body);
        assert!(!desc.contains("..."));
    }

    #[test]
    fn extract_description_stops_at_blank_line() {
        let md = "# t\n\nfirst paragraph here.\n\nsecond paragraph.\n";
        assert_eq!(
            extract_description(md).as_deref(),
            Some("first paragraph here.")
        );
    }

    // ── resolve_title / clean_optional ──────────────────────────────────

    #[test]
    fn resolve_title_prefers_frontmatter_over_h1() {
        let fm = SkillFrontmatter {
            title: Some("Frontmatter wins".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_title(&fm, "# Body H1\n\nbody", "ns/foo"),
            "Frontmatter wins"
        );
    }

    #[test]
    fn resolve_title_trims_frontmatter_whitespace() {
        let fm = SkillFrontmatter {
            title: Some("   spaced   ".into()),
            ..Default::default()
        };
        assert_eq!(resolve_title(&fm, "# H1", "id"), "spaced");
    }

    #[test]
    fn resolve_title_falls_back_to_h1_when_frontmatter_missing() {
        let fm = SkillFrontmatter::default();
        assert_eq!(resolve_title(&fm, "# Body H1\n\nbody", "ns/foo"), "Body H1");
    }

    #[test]
    fn resolve_title_falls_back_to_h1_when_frontmatter_blank() {
        let fm = SkillFrontmatter {
            title: Some("   ".into()),
            ..Default::default()
        };
        assert_eq!(resolve_title(&fm, "# Body H1", "ns/foo"), "Body H1");
    }

    #[test]
    fn resolve_title_falls_back_to_id_when_no_h1_or_frontmatter() {
        let fm = SkillFrontmatter::default();
        assert_eq!(resolve_title(&fm, "no heading here", "ns/foo"), "ns/foo");
    }

    #[test]
    fn clean_optional_drops_blank_strings() {
        assert_eq!(clean_optional(None), None);
        assert_eq!(clean_optional(Some("".into())), None);
        assert_eq!(clean_optional(Some("   ".into())), None);
        assert_eq!(
            clean_optional(Some(" how-to ".into())),
            Some("how-to".into())
        );
    }

    // ── skill_entry_from_fs ─────────────────────────────────────────────

    #[test]
    fn list_row_pulls_title_and_description_from_body() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.md");
        std::fs::write(&path, "# My title\n\nFirst paragraph.\n").unwrap();
        let fs = FsSkill {
            id: "foo".into(),
            abs_path: path,
        };
        let entry = skill_entry_from_fs(fs, &HashSet::new());
        assert_eq!(entry.id, "foo");
        assert_eq!(entry.title, "My title");
        assert_eq!(entry.kind, None);
        assert_eq!(entry.description, "First paragraph.");
        assert!(entry.bytes > 0);
        assert!(!entry.modified_at.is_empty());
    }

    #[test]
    fn list_row_prefers_frontmatter_title_and_carries_type() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.md");
        std::fs::write(
            &path,
            "---\ntitle: Real title\ntype: how-to\n---\n# Body H1\n\nFirst paragraph.\n",
        )
        .unwrap();
        let fs = FsSkill {
            id: "foo".into(),
            abs_path: path,
        };
        let entry = skill_entry_from_fs(fs, &HashSet::new());
        assert_eq!(entry.title, "Real title");
        assert_eq!(entry.kind.as_deref(), Some("how-to"));
        assert_eq!(entry.description, "First paragraph.");
    }

    #[test]
    fn list_row_falls_back_to_id_when_h1_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bare.md");
        std::fs::write(&path, "no heading at all\n").unwrap();
        let fs = FsSkill {
            id: "bare".into(),
            abs_path: path,
        };
        let entry = skill_entry_from_fs(fs, &HashSet::new());
        assert_eq!(entry.title, "bare");
        assert_eq!(entry.kind, None);
        assert_eq!(entry.description, "no heading at all");
    }

    #[test]
    fn list_row_survives_unreadable_body() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing.md");
        let fs = FsSkill {
            id: "missing".into(),
            abs_path: missing,
        };
        let entry = skill_entry_from_fs(fs, &HashSet::new());
        assert_eq!(entry.title, "missing");
        assert_eq!(entry.kind, None);
        assert_eq!(entry.description, "");
        assert_eq!(entry.bytes, 0);
    }

    // ── get_skill (full handler) ────────────────────────────────────────

    fn cfg_with_skills_folder(root: &std::path::Path) -> SkillsConfig {
        SkillsConfig {
            skills_folder: root.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn get_prefers_frontmatter_title_and_returns_type() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("ns");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(
            ns.join("doc.md"),
            "---\ntitle: Real title\ntype: how-to\n---\n# Body H1\n\nThe body.\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "ns/doc".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out.id, "ns/doc");
        assert_eq!(out.title, "Real title");
        assert_eq!(out.kind.as_deref(), Some("how-to"));
        assert!(out.body.contains("Body H1"));
    }

    #[tokio::test]
    async fn get_falls_back_to_h1_without_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("ns");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("plain.md"), "# Just an H1\n\nbody.\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "ns/plain".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out.title, "Just an H1");
        assert_eq!(out.kind, None);
    }

    #[tokio::test]
    async fn get_skill_not_found_error_points_agent_at_directory_skills_list() {
        // LLM agents calling directory::skills::get tend to guess skill
        // ids (observed: "sandbox/create" hallucinated). The miss must be a
        // self-sufficient prose sentence: a stable D110 / not_found token,
        // the missed id, and the exact next functions to call — so the agent
        // recovers in one read instead of doubling down on the wrong path.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let err = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/create".into(),
            },
        )
        .await
        .expect_err("should error on missing skill");
        assert!(err.contains("D110"), "missing code: {err}");
        assert!(err.contains("not_found"), "missing class word: {err}");
        assert!(err.contains("sandbox/create"), "missing id: {err}");
        assert!(err.contains("directory::skills::list"), "got: {err}");
        assert!(err.contains("directory::skills::index"), "got: {err}");
    }

    #[tokio::test]
    async fn get_function_id_shaped_id_gets_targeted_hint() {
        // Dumb-agent mistake: passing a FUNCTION id (`database::execute`) to
        // `get`, which wants a SKILL id (`database/index`). Must get a
        // targeted D112 hint, not a raw "invalid segment" rejection.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        for fid in ["database::execute", "shell::fs::mv"] {
            let err = get_skill(&cfg, SkillGetInput { id: fid.into() })
                .await
                .expect_err("function id must be rejected");
            assert!(err.contains("D112"), "got: {err}");
            assert!(err.contains("invalid_input"), "got: {err}");
            assert!(err.contains("FUNCTION id"), "got: {err}");
            assert!(err.contains("directory::skills::list"), "got: {err}");
        }
    }

    // ── rank_suggestions (multi-candidate, ranked) ──────────────────────

    #[tokio::test]
    async fn get_suggests_nested_skill_id_on_two_segment_miss() {
        // Reported case: agent calls `directory::skills::get { id:
        // "sandbox/exec" }` by analogy with the iii-directory layout
        // (`directory/skills/get`), but the sandbox worker lays its
        // skills one folder deeper. The prose miss must name the
        // canonical id in its "Did you mean" list.
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("sandbox").join("skills").join("sandbox");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("exec.md"),
            "---\nfunction_id: sandbox::exec\n---\n# Exec\n\nRun a command.\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let err = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/exec".into(),
            },
        )
        .await
        .expect_err("two-segment shorthand must miss");
        assert!(err.contains("D110"), "got: {err}");
        assert!(err.contains("sandbox/exec"), "got: {err}");
        assert!(err.contains("Did you mean"), "got: {err}");
        assert!(
            err.contains("sandbox/skills/sandbox/exec"),
            "expected canonical id in suggestions, got: {err}",
        );
    }

    #[tokio::test]
    async fn get_returns_multiple_candidates_when_ambiguous() {
        // Two skills under `sandbox/...` whose paths both end in `exec`.
        // The ranked suggester returns BOTH in the prose so the agent can
        // pick — this is the P0 fix from the session analysis.
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("sandbox").join("skills").join("sandbox");
        let b = tmp.path().join("sandbox").join("skills").join("legacy");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("exec.md"), "# A\n").unwrap();
        std::fs::write(b.join("exec.md"), "# B\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let err = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/exec".into(),
            },
        )
        .await
        .expect_err("multi-segment id must miss");
        assert!(err.contains("sandbox/skills/sandbox/exec"), "got: {err}");
        assert!(err.contains("sandbox/skills/legacy/exec"), "got: {err}");
    }

    #[tokio::test]
    async fn get_session_209nqqr4_regression_iii_skills_sandbox_index() {
        // Session 209nqqr4 line 137: agent asks for `iii/skills/sandbox/index`
        // (built on a wrong prior — most workers ship skills at the root,
        // not nested under `iii/skills/`). The old suggester pointed at
        // `iii/index` (wrong worker). The ranked suggester must surface
        // `sandbox/index` as the TOP hit (first in the "Did you mean" list)
        // because the discriminating segment is `sandbox`, not `iii`.
        let tmp = tempfile::tempdir().unwrap();
        // Layout mirrors a real install: iii bundle nested, sandbox flat.
        std::fs::create_dir_all(tmp.path().join("iii").join("skills").join("iii")).unwrap();
        std::fs::write(
            tmp.path()
                .join("iii")
                .join("skills")
                .join("iii")
                .join("quick-reference.md"),
            "# iii quick reference\n\nAll the things.\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("iii")).unwrap();
        std::fs::write(
            tmp.path().join("iii").join("index.md"),
            "---\ntype: index\n---\n# iii\n\nCore bundle overview.\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("sandbox")).unwrap();
        std::fs::write(
            tmp.path().join("sandbox").join("index.md"),
            "---\ntype: index\n---\n# Sandbox\n\nBoot a sandbox to run code.\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let err = get_skill(
            &cfg,
            SkillGetInput {
                id: "iii/skills/sandbox/index".into(),
            },
        )
        .await
        .expect_err("nested guess must miss");
        // `sandbox/index` shares the discriminating `sandbox` segment AND
        // `index` segment with the miss — must be the FIRST candidate in
        // the prose "Did you mean" list, ahead of any `iii/...` candidate.
        // It is surfaced by its bare display id `sandbox` (no `/index`).
        let dym = err
            .split("Did you mean: ")
            .nth(1)
            .unwrap_or_else(|| panic!("prose miss must list candidates, got: {err}"));
        assert!(
            dym.starts_with("sandbox"),
            "ranked suggester must put the sandbox overview first, got: {err}",
        );
        assert!(
            !dym.starts_with("sandbox/index"),
            "suggestion ids must drop the /index suffix, got: {err}",
        );
    }

    #[tokio::test]
    async fn get_returns_no_suggestions_when_catalog_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let err = get_skill(
            &cfg,
            SkillGetInput {
                id: "anything/here".into(),
            },
        )
        .await
        .expect_err("empty catalog");
        assert!(err.contains("D110"), "got: {err}");
        // No candidates -> no misleading "Did you mean".
        assert!(!err.contains("Did you mean"), "got: {err}");
    }

    #[test]
    fn levenshtein_basic_cases() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("sandbox/index", "sandbox/index"), 0);
        assert_eq!(levenshtein("sandbox/exec", "sandbox/index"), 4);
    }

    #[tokio::test]
    async fn get_serialises_type_field_with_correct_json_key() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("ns");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(
            ns.join("doc.md"),
            "---\ntitle: T\ntype: index\n---\n# H\n\nb\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "ns/doc".into(),
            },
        )
        .await
        .unwrap();
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["type"].as_str(), Some("index"));
        assert!(v.get("kind").is_none(), "kind should be renamed to type");
        assert!(v["title"].as_str() == Some("T"));
    }

    // ── bare worker name → <worker>/index alias ─────────────────────────

    #[tokio::test]
    async fn get_accepts_bare_worker_name_as_alias_for_index() {
        // The user-facing requirement: agents reach for the worker name
        // (e.g. `sandbox`) when they want the worker overview. That call
        // resolves to `<worker>/index.md` on disk, and the response carries
        // the BARE worker name as the agent-facing id (the `/index` suffix
        // is a filesystem detail, stripped for display).
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("index.md"), "# Sandbox\n\nWorker overview.\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            out.id, "sandbox",
            "response id is the bare worker name, not the on-disk /index path"
        );
        assert!(out.body.contains("Worker overview."));
    }

    #[tokio::test]
    async fn bare_name_and_explicit_index_return_same_body() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(
            ns.join("index.md"),
            "---\ntitle: Sandbox\ntype: index\n---\n# Sandbox\n\nShared body.\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let bare = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox".into(),
            },
        )
        .await
        .unwrap();
        let explicit = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/index".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            bare.id, "sandbox",
            "both forms display the bare worker name"
        );
        assert_eq!(bare.id, explicit.id);
        assert_eq!(bare.title, explicit.title);
        assert_eq!(bare.body, explicit.body);
        assert_eq!(bare.kind, explicit.kind);
    }

    #[tokio::test]
    async fn multi_segment_id_only_aliases_to_a_real_index_overview() {
        // A multi-segment id aliases to `<id>/index` ONLY when that overview
        // actually exists; it never resolves to a sibling skill. So a
        // function-shaped typo like `sandbox/exec` (no `sandbox/exec/index`
        // on disk) still misses rather than silently resolving wrong.
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("index.md"), "# Sandbox\n\nOverview.\n").unwrap();
        // Note: we deliberately do NOT create sandbox/exec/index.md.
        let cfg = cfg_with_skills_folder(tmp.path());
        let err = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/exec".into(),
            },
        )
        .await
        .expect_err("multi-segment id with no /index overview must miss");
        assert!(
            err.contains("not_found") && err.contains("sandbox/exec"),
            "expected literal-id miss, got: {err}"
        );
    }

    #[tokio::test]
    async fn get_accepts_nested_overview_shorthand() {
        // A nested overview `resend/emails/index` displays as `resend/emails`;
        // that bare form must round-trip back through `get` (the inverse of
        // the display strip), so an agent can paste what `list` showed.
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("resend").join("emails");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("index.md"), "# Emails\n\nEmail ops.\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "resend/emails".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out.id, "resend/emails");
        assert!(out.body.contains("Email ops."));
    }

    #[tokio::test]
    async fn bare_id_with_literal_root_skill_wins_over_index_alias() {
        // When both `<root>/sandbox.md` and `<root>/sandbox/index.md`
        // exist, the literal root skill takes precedence over the
        // bare-name → index alias. Documents the precedence rule so a
        // future refactor doesn't silently flip it.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("sandbox.md"), "# Root\n\nRoot body.\n").unwrap();
        let ns = tmp.path().join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("index.md"), "# Index\n\nIndex body.\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            out.id, "sandbox",
            "literal root skill must win over the index alias"
        );
        assert!(out.body.contains("Root body."));
    }

    // ── function_id surfacing (fix a — eliminates skill-id vs function-id confusion) ──

    #[tokio::test]
    async fn get_surfaces_function_id_from_frontmatter() {
        // Documents the canonical bus function id alongside the skill
        // id so callers don't conflate the two. Models call
        // `agent_trigger { function: <skill_id> }` when only the id is
        // visible, then loop on `function_not_found`.
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("sandbox").join("skills").join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(
            ns.join("create.md"),
            "---\nfunction_id: sandbox::create\ntype: how-to\ntitle: Boot a sandbox\n---\n# Create\n\nBody.\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/skills/sandbox/create".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out.id, "sandbox/skills/sandbox/create");
        assert_eq!(
            out.function_id.as_deref(),
            Some("sandbox::create"),
            "frontmatter function_id must surface verbatim",
        );
    }

    #[tokio::test]
    async fn get_function_id_is_null_when_frontmatter_omits_it() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("ns");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("readme.md"), "# Just a readme\n\nNo function.\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "ns/readme".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out.function_id, None);
        // Serialises as JSON null — same contract as `type`.
        let v = serde_json::to_value(&out).unwrap();
        assert!(v["function_id"].is_null());
    }

    #[test]
    fn list_row_surfaces_function_id_from_frontmatter() {
        // A picker / agent listing the catalog must be able to read off
        // the function id without a follow-up `get` call. Without this,
        // models conflate the row's `id` (skill path) with the callable
        // function id and burn turns on `function_not_found`.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("create.md");
        std::fs::write(
            &path,
            "---\nfunction_id: sandbox::create\ntype: how-to\n---\n# Create\n\nBoot a VM.\n",
        )
        .unwrap();
        let entry = skill_entry_from_fs(
            FsSkill {
                id: "sandbox/skills/sandbox/create".into(),
                abs_path: path,
            },
            &HashSet::new(),
        );
        assert_eq!(entry.function_id.as_deref(), Some("sandbox::create"));
        assert_eq!(entry.kind.as_deref(), Some("how-to"));
        // Serialises as JSON with both fields visible to the agent.
        let v = serde_json::to_value(&entry).unwrap();
        assert_eq!(v["function_id"].as_str(), Some("sandbox::create"));
        assert_eq!(v["id"].as_str(), Some("sandbox/skills/sandbox/create"));
    }

    #[test]
    fn list_row_function_id_is_null_when_frontmatter_omits_it() {
        // Index-type skills and free-form references aren't 1:1 with a
        // single function. Their row must carry null, not a guess.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("index.md");
        std::fs::write(&path, "---\ntype: index\n---\n# Sandbox\n\nOverview.\n").unwrap();
        let entry = skill_entry_from_fs(
            FsSkill {
                id: "sandbox/index".into(),
                abs_path: path,
            },
            &HashSet::new(),
        );
        assert_eq!(entry.function_id, None);
        let v = serde_json::to_value(&entry).unwrap();
        assert!(v["function_id"].is_null());
    }

    // ── render_index_markdown ───────────────────────────────────────────

    /// Build a `SkillEntry` for renderer tests. `on_disk_id` mirrors `id`,
    /// so passing an `<ns>/index` id exercises the namespace-root overview
    /// branch and any other id with a non-`index` `kind` exercises the
    /// "should be filtered out" path.
    fn entry(id: &str, title: &str, kind: Option<&str>, description: &str) -> SkillEntry {
        SkillEntry {
            id: id.into(),
            on_disk_id: id.into(),
            title: title.into(),
            kind: kind.map(String::from),
            function_id: None,
            description: description.into(),
            bytes: 0,
            modified_at: String::new(),
        }
    }

    #[test]
    fn render_index_starts_with_h1_and_worker_count() {
        let body = render_index_markdown(&[
            entry(
                "agent-memory/index",
                "agent-memory",
                Some("index"),
                "Memory tier.",
            ),
            entry(
                "iii-directory/index",
                "iii-directory",
                Some("index"),
                "Directory worker.",
            ),
        ]);
        assert!(
            body.starts_with("# Skills index\n\n2 worker(s).\n"),
            "got: {body}"
        );
    }

    #[test]
    fn render_index_empty_input_still_emits_header() {
        let body = render_index_markdown(&[]);
        assert_eq!(body, "# Skills index\n\n0 worker(s).\n");
    }

    #[test]
    fn render_index_filters_to_type_index() {
        let body = render_index_markdown(&[
            entry(
                "agent-memory/index",
                "agent-memory",
                Some("index"),
                "Worker overview.",
            ),
            entry(
                "agent-memory/observe",
                "Observe",
                Some("how-to"),
                "Record an event.",
            ),
            entry("agent-memory/strays", "Stray", None, "Untyped skill."),
        ]);
        assert!(body.contains("## agent-memory"), "missing h2; got: {body}");
        assert!(
            !body.contains("## Observe"),
            "how-to should be filtered out; got: {body}"
        );
        assert!(
            !body.contains("## Stray"),
            "untyped skill should be filtered out; got: {body}"
        );
        // Filtered-out skills must not leak into the read-more pointers either.
        assert!(
            !body.contains("agent-memory/observe.md"),
            "filtered-out how-to leaked a link; got: {body}"
        );
        assert!(body.contains("1 worker(s).\n"), "wrong count; got: {body}");
    }

    #[test]
    fn render_index_emits_h2_per_worker_using_resolved_title() {
        let body = render_index_markdown(&[
            entry(
                "agent-memory/index",
                "agent-memory",
                Some("index"),
                "Memory tier.",
            ),
            entry(
                "iii-directory/index",
                "iii-directory",
                Some("index"),
                "Directory worker.",
            ),
        ]);
        assert_eq!(
            body.matches("\n## ").count(),
            2,
            "expected exactly two `##` headings; got: {body}"
        );
        assert!(body.contains("\n## agent-memory\n"), "got: {body}");
        assert!(body.contains("\n## iii-directory\n"), "got: {body}");
    }

    #[test]
    fn render_index_includes_description_paragraph() {
        let body = render_index_markdown(&[entry(
            "iii-directory/index",
            "iii-directory",
            Some("index"),
            "Engine introspection and filesystem-backed skill reader.",
        )]);
        // Description sits between the `## title` and the read-more line,
        // separated by blank lines on either side.
        assert!(
            body.contains(
                "\n## iii-directory\n\nEngine introspection and filesystem-backed skill reader.\n\nFull reference: call `directory::skills::get "
            ),
            "description not framed correctly; got: {body}"
        );
        assert!(
            !body.contains("workers.iii.dev"),
            "external dive-deeper URL should be gone; got: {body}"
        );
    }

    #[test]
    fn render_index_emits_get_pointer() {
        let body = render_index_markdown(&[entry(
            "agent-memory",
            "agent-memory",
            Some("index"),
            "Memory tier.",
        )]);
        assert!(
            body.contains(
                "Full reference: call `directory::skills::get { \"id\": \"agent-memory\" }` (legacy `iii://agent-memory`).\n"
            ),
            "missing directory::skills::get pointer; got: {body}"
        );
        assert!(
            !body.contains("workers.iii.dev"),
            "external dive-deeper URL should be gone; got: {body}"
        );
    }

    #[test]
    fn render_index_skips_blank_description() {
        let body = render_index_markdown(&[entry(
            "bare",
            "bare",
            Some("index"),
            "", // body has no paragraph
        )]);
        // Title comes immediately before the read-more line — no extra
        // blank paragraph in the middle.
        assert!(
            body.contains(
                "\n## bare\n\nFull reference: call `directory::skills::get { \"id\": \"bare\" }`"
            ),
            "blank-description block should compress; got: {body}"
        );
        assert!(
            !body.contains("workers.iii.dev"),
            "no external URL; got: {body}"
        );
        // And the rest of the document still has the header.
        assert!(body.contains("1 worker(s).\n"));
    }

    #[test]
    fn render_index_ordering_follows_input_lex_order() {
        // Input is already lex-sorted by `scan_skills`; the renderer
        // emits sections in the same order.
        let body = render_index_markdown(&[
            entry("agent-memory/index", "agent-memory", Some("index"), "a"),
            entry("iii-directory/index", "iii-directory", Some("index"), "b"),
            entry("resend/index", "resend", Some("index"), "c"),
        ]);
        let am = body.find("## agent-memory").expect("am missing");
        let iii = body.find("## iii-directory").expect("iii missing");
        let resend = body.find("## resend").expect("resend missing");
        assert!(
            am < iii && iii < resend,
            "headings out of order; got: {body}"
        );
    }

    #[test]
    fn render_index_emits_get_pointer_and_legacy_iii() {
        let entries = vec![SkillEntry {
            id: "agent-memory".into(),
            on_disk_id: "agent-memory/index".into(),
            title: "agent-memory".into(),
            kind: Some("index".into()),
            function_id: None,
            description: "Memory worker overview.".into(),
            bytes: 10,
            modified_at: String::new(),
        }];
        let body = render_index_markdown(&entries);
        assert!(
            body.contains("`directory::skills::get { \"id\": \"agent-memory\" }`"),
            "expected directory::skills::get pointer, got:\n{body}"
        );
        assert!(
            body.contains("legacy `iii://agent-memory`"),
            "expected legacy iii:// token for back-compat, got:\n{body}"
        );
        assert!(
            !body.contains("workers.iii.dev"),
            "external dive-deeper URL should be gone, got:\n{body}"
        );
    }

    // ── description precedence (Task 5 regression) ─────────────────────

    #[test]
    fn list_row_prefers_frontmatter_description_over_body() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.md");
        std::fs::write(
            &path,
            "---\ndescription: Frontmatter desc.\n---\n# Title\n\nBody paragraph.\n",
        )
        .unwrap();
        let entry = skill_entry_from_fs(
            FsSkill {
                id: "s".into(),
                abs_path: path,
            },
            &HashSet::new(),
        );
        assert_eq!(
            entry.description, "Frontmatter desc.",
            "frontmatter description should win over body paragraph"
        );
    }

    #[test]
    fn list_row_falls_back_to_body_when_no_frontmatter_description() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.md");
        std::fs::write(&path, "---\ntitle: T\n---\n# Title\n\nBody fallback.\n").unwrap();
        let entry = skill_entry_from_fs(
            FsSkill {
                id: "s".into(),
                abs_path: path,
            },
            &HashSet::new(),
        );
        assert_eq!(
            entry.description, "Body fallback.",
            "body first-paragraph must be used when frontmatter description is absent"
        );
    }

    #[test]
    fn untyped_namespace_root_overview_classifies_as_index() {
        // Reproduces the live bug: a legacy worker overview shipped as
        // `<ns>/index.md` with `name:`/`description:` frontmatter and NO
        // `type:` must still be treated as the worker's overview row so
        // directory::skills::index lists it.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("iii-sandbox");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("index.md");
        std::fs::write(
            &path,
            "---\nname: sandbox\ndescription: Ephemeral microVMs.\n---\n# Sandbox\n\nOverview.\n",
        )
        .unwrap();
        let entry = skill_entry_from_fs(
            FsSkill {
                id: "iii-sandbox/index".into(),
                abs_path: path,
            },
            &HashSet::new(),
        );
        assert_eq!(entry.kind, None, "legacy overview declares no `type:`");
        assert_eq!(entry.on_disk_id, "iii-sandbox/index");
        assert!(
            is_index_overview(&entry),
            "a namespace-root overview must classify as an index row even without `type: index`"
        );
    }

    // ── render_index character budget cap ──────────────────────────────

    #[test]
    fn render_index_lists_every_worker_when_over_budget() {
        // Enough workers (with big descriptions) to blow past
        // INDEX_CHAR_BUDGET. Every worker must still appear; only the
        // descriptions get dropped once the budget is hit.
        let mut entries = Vec::new();
        for i in 0..50 {
            entries.push(entry(
                &format!("worker-{i:02}/index"),
                &format!("Worker {i:02}"),
                Some("index"),
                &"x".repeat(200),
            ));
        }
        let body = render_index_markdown(&entries);

        // Count header reflects ALL workers.
        assert!(
            body.contains("50 worker(s).\n"),
            "count must reflect every worker; got: {body}"
        );
        // Every worker heading is present — none dropped.
        for i in 0..50 {
            let heading = format!("## Worker {i:02}\n");
            assert!(
                body.contains(&heading),
                "worker {i:02} must be listed even over budget; missing {heading:?}"
            );
        }
        // Over budget, the omission note appears and points at get.
        assert!(
            body.contains("descriptions omitted"),
            "should note omitted descriptions; got: {body}"
        );
        assert!(
            body.contains("directory::skills::get"),
            "omission note should reference the get function; got: {body}"
        );
        // The old whole-worker truncation behaviour is gone.
        assert!(
            !body.contains("truncated"),
            "workers must never be truncated away; got: {body}"
        );
    }

    #[test]
    fn render_index_get_pointer_uses_row_id() {
        let body = render_index_markdown(&[entry(
            "my-worker/index",
            "My Worker",
            Some("index"),
            "A worker.",
        )]);
        assert!(
            body.contains("`directory::skills::get { \"id\": \"my-worker/index\" }`"),
            "get pointer should carry the row id; got: {body}"
        );
        assert!(
            !body.contains("workers.iii.dev"),
            "no external URL; got: {body}"
        );
    }

    #[test]
    fn render_index_includes_untyped_namespace_root_overview() {
        // Legacy bundles ship `<ns>/index.md` with `name:`/`description:`
        // frontmatter and NO `type:`. The worker-root overview must still
        // render as a worker block; a non-root sub-skill must not.
        let body = render_index_markdown(&[
            entry("iii-sandbox/index", "sandbox", None, "Ephemeral microVMs."),
            entry("iii-sandbox/exec", "exec", None, "Run a command."),
        ]);
        assert!(
            body.contains("## sandbox"),
            "untyped namespace-root overview must render; got: {body}"
        );
        assert!(
            !body.contains("## exec"),
            "a non-root sub-skill must not render as a worker; got: {body}"
        );
        assert!(body.contains("1 worker(s).\n"), "wrong count; got: {body}");
        assert!(
            body.contains("`directory::skills::get { \"id\": \"iii-sandbox/index\" }`"),
            "should instruct directory::skills::get; got: {body}"
        );
    }

    // ── SKILL.md alias in normalize_get_id ─────────────────────────────

    #[test]
    fn normalize_aliases_skill_md_to_index() {
        assert_eq!(
            normalize_get_id("hello-worker/SKILL.md").unwrap(),
            "hello-worker/index"
        );
    }

    #[test]
    fn normalize_aliases_nested_skill_md_to_index() {
        assert_eq!(
            normalize_get_id("resend/emails/SKILL.md").unwrap(),
            "resend/emails/index"
        );
    }

    // ── parse_worker_names ──────────────────────────────────────────

    #[test]
    fn parse_worker_names_well_formed() {
        let val = json!({
            "workers": [
                {"name": "resend", "version": "1.0.0"},
                {"name": "agent-memory"}
            ]
        });
        let names = parse_worker_names(&val);
        assert_eq!(names.len(), 2);
        assert!(names.contains("resend"));
        assert!(names.contains("agent-memory"));
    }

    #[test]
    fn parse_worker_names_missing_workers_key() {
        let val = json!({"something_else": true});
        let names = parse_worker_names(&val);
        assert!(names.is_empty());
    }

    #[test]
    fn parse_worker_names_workers_not_array() {
        let val = json!({"workers": "not an array"});
        let names = parse_worker_names(&val);
        assert!(names.is_empty());
    }

    #[test]
    fn parse_worker_names_entry_missing_name() {
        let val = json!({
            "workers": [
                {"name": "good"},
                {"version": "1.0.0"},
                {"name": "also-good"}
            ]
        });
        let names = parse_worker_names(&val);
        assert_eq!(names.len(), 2);
        assert!(names.contains("good"));
        assert!(names.contains("also-good"));
    }

    // ── filter_to_registered ──────────────────────────────────────────

    fn fs_skill(id: &str) -> FsSkill {
        FsSkill {
            id: id.into(),
            abs_path: PathBuf::from(format!("/fake/{id}.md")),
        }
    }

    #[test]
    fn filter_keeps_root_doc_without_namespace() {
        let registered = HashSet::from(["resend".to_string()]);
        let merged = vec![fs_skill("index")];
        let result = filter_to_registered(merged, &registered);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "index");
    }

    #[test]
    fn filter_keeps_directory_namespace_docs() {
        let registered = HashSet::new(); // nothing registered
        let merged = vec![fs_skill("directory/engine/functions/info")];
        let result = filter_to_registered(merged, &registered);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "directory/engine/functions/info");
    }

    // ── engine_fallback_worker ─────────────────────────────────────────

    #[test]
    fn engine_fallback_for_installed_skill_less_worker() {
        // iii-sandbox is installed but ships no skill doc → fall back to engine.
        let registered = HashSet::from(["iii-sandbox".to_string(), "iii-http".to_string()]);
        let visible = vec![
            fs_skill("iii-http/index"),
            fs_skill("iii/index"),
            fs_skill("directory/skills/list"),
        ];
        assert_eq!(
            engine_fallback_worker("iii-sandbox/index", &visible, &registered),
            Some("iii-sandbox".to_string())
        );
        // Any sub-path under the skill-less worker triggers the same fallback.
        assert_eq!(
            engine_fallback_worker("iii-sandbox/anything/here", &visible, &registered),
            Some("iii-sandbox".to_string())
        );
    }

    #[test]
    fn no_engine_fallback_when_worker_has_skills() {
        // iii-http HAS a skill; a wrong sub-path should fall through to the
        // normal closest-id suggestions, not the engine overview.
        let registered = HashSet::from(["iii-http".to_string()]);
        let visible = vec![fs_skill("iii-http/index")];
        assert_eq!(
            engine_fallback_worker("iii-http/typo", &visible, &registered),
            None
        );
    }

    #[test]
    fn no_engine_fallback_for_unregistered_or_engine_namespace() {
        let registered = HashSet::from(["iii-http".to_string()]);
        let visible = vec![fs_skill("iii-http/index")];
        // Not an installed worker → no fallback.
        assert_eq!(
            engine_fallback_worker("totally-unknown/x", &visible, &registered),
            None
        );
        // The engine namespace itself never falls back to itself.
        let reg2 = HashSet::from(["iii".to_string()]);
        assert_eq!(engine_fallback_worker("iii/missing", &[], &reg2), None);
    }

    #[test]
    fn filter_keeps_engine_namespace_docs() {
        let registered = HashSet::new(); // nothing registered; `iii` is not a worker
        let merged = vec![fs_skill("iii/index"), fs_skill("iii/SKILL")];
        let result = filter_to_registered(merged, &registered);
        let ids: Vec<&str> = result.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"iii/index"));
        assert!(ids.contains(&"iii/SKILL"));
    }

    // ── worker_overview_fallback ───────────────────────────────────────

    #[test]
    fn worker_overview_collapses_fabricated_subpath_on_single_skill_worker() {
        // Session 3y005kju: agent turned the function id `sandbox::create`
        // into the skill path `iii-sandbox/sandbox/create`. iii-sandbox ships
        // exactly one doc (its overview), so the miss collapses straight to
        // `iii-sandbox/index` — one call instead of get→list→get.
        let registered = HashSet::from(["iii-sandbox".to_string()]);
        let visible = vec![fs_skill("iii-sandbox/index"), fs_skill("iii/index")];
        assert_eq!(
            worker_overview_fallback("iii-sandbox/sandbox/create", &visible, &registered),
            Some("iii-sandbox/index".to_string())
        );
        // A wrong two-segment guess collapses identically.
        assert_eq!(
            worker_overview_fallback("iii-sandbox/create", &visible, &registered),
            Some("iii-sandbox/index".to_string())
        );
    }

    #[test]
    fn worker_overview_resolves_colloquial_name_via_substring() {
        // Agents type the function-namespace name `sandbox`, but the worker
        // (and published skill) is `iii-sandbox`. `get id=sandbox` must
        // resolve to `iii-sandbox/index` instead of dead-ending with no
        // suggestion (the ranker scores it negative and drops it).
        let registered = HashSet::from(["iii-sandbox".to_string()]);
        let visible = vec![fs_skill("iii-sandbox/index"), fs_skill("iii/index")];
        assert_eq!(
            worker_overview_fallback("sandbox", &visible, &registered),
            Some("iii-sandbox/index".to_string())
        );
        // A colloquial sub-path (function id under the short name) maps too.
        assert_eq!(
            worker_overview_fallback("sandbox/create", &visible, &registered),
            Some("iii-sandbox/index".to_string())
        );
        // Case-insensitive: `Sandbox` matches `iii-sandbox`.
        assert_eq!(
            worker_overview_fallback("Sandbox", &visible, &registered),
            Some("iii-sandbox/index".to_string())
        );
    }

    #[test]
    fn worker_overview_substring_match_is_not_just_a_prefix() {
        // The match is a substring, not the old `iii-` prefix heuristic:
        // `memory` resolves the worker `agent-memory` (matches in the middle).
        let registered = HashSet::from(["agent-memory".to_string()]);
        let visible = vec![fs_skill("agent-memory/index"), fs_skill("iii/index")];
        assert_eq!(
            worker_overview_fallback("memory", &visible, &registered),
            Some("agent-memory/index".to_string())
        );
    }

    #[test]
    fn worker_overview_ambiguous_substring_defers_to_suggester() {
        // Two equally-specific workers both contain the query → genuinely
        // ambiguous, so resolution declines (None) and the caller falls
        // through to the ranked-suggestion list.
        let registered = HashSet::from(["iii-foobar".to_string(), "iii-fooqux".to_string()]);
        let visible = vec![
            fs_skill("iii-foobar/index"),
            fs_skill("iii-fooqux/index"),
            fs_skill("iii/index"),
        ];
        assert_eq!(worker_overview_fallback("foo", &visible, &registered), None);
        // But an EXACT (case-insensitive) name still wins even amid others.
        assert_eq!(
            worker_overview_fallback("iii-foobar", &visible, &registered),
            Some("iii-foobar/index".to_string())
        );
    }

    #[test]
    fn worker_overview_bare_name_serves_overview_even_for_multi_skill_worker() {
        // A bare worker name asks for the worker itself → serve its overview
        // regardless of skill count (only wrong SUB-paths defer to the
        // suggester). `iii-foo` is colloquially `foo`.
        let registered = HashSet::from(["iii-foo".to_string()]);
        let visible = vec![
            fs_skill("iii-foo/index"),
            fs_skill("iii-foo/a"),
            fs_skill("iii-foo/b"),
        ];
        assert_eq!(
            worker_overview_fallback("foo", &visible, &registered),
            Some("iii-foo/index".to_string())
        );
        // ...but a wrong sub-path under that multi-skill worker does NOT
        // collapse — the suggester names the exact intended skill.
        assert_eq!(
            worker_overview_fallback("foo/c", &visible, &registered),
            None
        );
    }

    #[test]
    fn display_id_strips_only_trailing_index_segment() {
        let none = HashSet::new();
        // Worker-root and nested overviews drop the trailing `/index`.
        assert_eq!(display_id("iii-sandbox/index", &none), "iii-sandbox");
        assert_eq!(display_id("resend/emails/index", &none), "resend/emails");
        // Non-overview ids are untouched.
        assert_eq!(
            display_id("database/iii-database/query", &none),
            "database/iii-database/query"
        );
        // A bare single-segment `index` (root bundle doc) is left as-is —
        // only a `/index` SUFFIX strips.
        assert_eq!(display_id("index", &none), "index");
        // An id that merely contains `index` mid-path is untouched.
        assert_eq!(display_id("indexer/run", &none), "indexer/run");
    }

    #[test]
    fn display_id_keeps_index_suffix_on_collision_with_literal_sibling() {
        // A worker that ships BOTH `sandbox.md` (id `sandbox`) and
        // `sandbox/index.md` (id `sandbox/index`) must NOT collapse the
        // overview onto the root doc's id, or the overview becomes
        // unaddressable and two list rows share an id.
        let siblings: HashSet<String> = ["sandbox".to_string(), "sandbox/index".to_string()]
            .into_iter()
            .collect();
        assert_eq!(display_id("sandbox/index", &siblings), "sandbox/index");
        assert_eq!(display_id("sandbox", &siblings), "sandbox");
        // No literal sibling → strips normally.
        let only_index: HashSet<String> = ["sandbox/index".to_string()].into_iter().collect();
        assert_eq!(display_id("sandbox/index", &only_index), "sandbox");
    }

    #[test]
    fn resolve_worker_exact_name_wins_over_substring_even_when_skill_less() {
        // `box` is installed (skill-less); `iii-sandbox` is installed with a
        // doc and CONTAINS "box". An exact-name query must resolve `box`, not
        // be hijacked by the longer substring match — resolution is
        // independent of whether the worker ships a doc.
        let registered = HashSet::from(["box".to_string(), "iii-sandbox".to_string()]);
        assert_eq!(resolve_worker("box", &registered), Some("box".to_string()));
        // A non-exact colloquial query still resolves via substring.
        assert_eq!(
            resolve_worker("sandbox", &registered),
            Some("iii-sandbox".to_string())
        );
        // Engine namespace and empty are excluded.
        assert_eq!(resolve_worker("iii", &registered), None);
        assert_eq!(resolve_worker("", &registered), None);
    }

    #[test]
    fn engine_fallback_resolves_colloquial_name_for_skill_less_worker() {
        // Defect #2: `get sandbox` for a skill-less `iii-sandbox` must reach
        // the engine-overview fallback just like the exact name does, instead
        // of dead-ending. engine_fallback_worker resolves the colloquial name.
        let registered = HashSet::from(["iii-sandbox".to_string()]);
        let visible = vec![fs_skill("iii/index")]; // iii-sandbox ships no doc
        assert_eq!(
            engine_fallback_worker("sandbox", &visible, &registered),
            Some("iii-sandbox".to_string())
        );
        // The exact name resolves to the same worker.
        assert_eq!(
            engine_fallback_worker("iii-sandbox", &visible, &registered),
            Some("iii-sandbox".to_string())
        );
    }

    #[test]
    fn substring_does_not_hijack_an_exact_skill_less_worker() {
        // Defect #3: `box` is installed but skill-less; `iii-sandbox` has an
        // overview and contains "box". `get box` must NOT serve iii-sandbox's
        // overview — the exact `box` wins resolution, has no overview, so the
        // overview path declines and the engine fallback serves `box`.
        let registered = HashSet::from(["box".to_string(), "iii-sandbox".to_string()]);
        let visible = vec![fs_skill("iii-sandbox/index"), fs_skill("iii/index")];
        assert_eq!(
            worker_overview_fallback("box", &visible, &registered),
            None,
            "must not collapse `box` onto the unrelated iii-sandbox overview"
        );
        assert_eq!(
            engine_fallback_worker("box", &visible, &registered),
            Some("box".to_string()),
            "skill-less exact worker `box` reaches the engine overview, named `box`"
        );
    }

    #[test]
    fn worker_overview_redirect_note_shows_bare_id_and_raw_prefix() {
        let none = HashSet::new();
        let note =
            worker_overview_redirect_note("iii-sandbox/sandbox/create", "iii-sandbox/index", &none);
        assert!(
            note.contains("no skill `iii-sandbox/sandbox/create`"),
            "got: {note}"
        );
        // Shown overview id is the bare worker name (display form)...
        assert!(note.contains("Showing `iii-sandbox`"), "got: {note}");
        assert!(!note.contains("Showing `iii-sandbox/index`"), "got: {note}");
        // ...but the list-prefix hint keeps the on-disk `<ns>/` form.
        assert!(note.contains("prefix=\"iii-sandbox/\""), "got: {note}");
    }

    #[test]
    fn worker_overview_no_collapse_for_multi_skill_worker() {
        // `directory` ships per-function sub-skills; a wrong sub-path keeps
        // the precise closest-id suggester rather than collapsing to a
        // worker overview (the suggester names the exact intended skill).
        let registered = HashSet::new();
        let visible = vec![
            fs_skill("directory/index"),
            fs_skill("directory/skills/get"),
            fs_skill("directory/skills/list"),
        ];
        assert_eq!(
            worker_overview_fallback("directory/skills/got", &visible, &registered),
            None
        );
    }

    #[test]
    fn worker_overview_no_collapse_when_worker_skill_less() {
        // Worker installed but ships no doc → not this fallback's job;
        // `engine_fallback_worker` serves the engine overview instead.
        let registered = HashSet::from(["iii-sandbox".to_string()]);
        let visible = vec![fs_skill("iii/index")];
        assert_eq!(
            worker_overview_fallback("iii-sandbox/sandbox/create", &visible, &registered),
            None
        );
    }

    #[test]
    fn worker_overview_no_collapse_for_engine_or_unregistered_namespace() {
        let registered = HashSet::from(["iii-http".to_string()]);
        let visible = vec![fs_skill("iii-http/index"), fs_skill("iii/index")];
        // Engine namespace never collapses via this path.
        assert_eq!(
            worker_overview_fallback("iii/skills/sandbox/index", &visible, &registered),
            None
        );
        // Unregistered top namespace → no collapse (keep typo suggestions).
        assert_eq!(
            worker_overview_fallback("totally-unknown/x", &visible, &registered),
            None
        );
        // A bare name that is a substring of no installed worker → no
        // resolution; keep suggester behavior.
        assert_eq!(
            worker_overview_fallback("nope", &visible, &registered),
            None
        );
    }

    #[test]
    fn filter_keeps_registered_worker_skills() {
        let registered = HashSet::from(["resend".to_string()]);
        let merged = vec![fs_skill("resend/index"), fs_skill("resend/emails/send")];
        let result = filter_to_registered(merged, &registered);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_drops_unregistered_worker_skills() {
        let registered = HashSet::from(["resend".to_string()]);
        let merged = vec![
            fs_skill("resend/index"),
            fs_skill("otherworker/x"),
            fs_skill("index"),
            fs_skill("directory/skills/list"),
        ];
        let result = filter_to_registered(merged, &registered);
        let ids: Vec<&str> = result.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"resend/index"));
        assert!(ids.contains(&"index"));
        assert!(ids.contains(&"directory/skills/list"));
        assert!(!ids.contains(&"otherworker/x"));
    }

    #[test]
    fn filter_drops_resend_when_not_registered() {
        let registered = HashSet::from(["agent-memory".to_string()]);
        let merged = vec![fs_skill("resend/index"), fs_skill("agent-memory/index")];
        let result = filter_to_registered(merged, &registered);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "agent-memory/index");
    }
}
