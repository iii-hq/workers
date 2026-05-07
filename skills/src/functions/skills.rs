//! State-backed skills registry, ported from the `mcp` worker.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `skills::register`   — store a markdown skill body keyed by id.
//!   * `skills::unregister` — delete one by id (idempotent).
//!   * `skills::list`       — metadata-only listing, sorted by id.
//!   * `skills::fetch_skill` — batched read over one or more `iii://` URIs.
//!     Internal id only; MCP clients never see it because the bridge
//!     hard-floors `skills::*`.
//!
//! Public alias on a non-hidden namespace (visible as the MCP tool
//! `skill__fetch`):
//!
//!   * `skill::fetch` — same handler as `skills::fetch_skill`. Lets agents
//!     resolve `iii://` links in skill bodies on demand without changes
//!     to the mcp worker.
//!
//! Internal RPC called only by the `mcp` worker (hard-floored under
//! `skills::*` so never an MCP tool):
//!
//!   * `skills::resources-list`      — `{ resources: [...] }` for MCP `resources/list`.
//!   * `skills::resources-read`      — `{ contents: [...] }` for MCP `resources/read`.
//!   * `skills::resources-templates` — `{ resourceTemplates: [...] }` for MCP `resources/templates/list`.
//!
//! Each mutation fans out through [`trigger_types::dispatch`] on the
//! `skills::on-change` trigger type so interested workers (the `mcp`
//! worker today) can forward MCP notifications.

use std::collections::HashSet;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, TriggerRequest, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::SkillsConfig;
use crate::fs_source::{self, FsSkill};
use crate::state;
use crate::trigger_types::{self, SubscriberSet};

pub const SKILL_BODY_MAX_BYTES: usize = 256 * 1024;

/// Per-segment cap for both skill ids and section URI segments.
/// The total id is allowed to chain many segments via `/`, but each
/// individual segment stays short so directory listings stay readable.
const ID_SEGMENT_MAX_LEN: usize = 64;

/// Soft ceiling on the slashed id length. With the per-segment cap above
/// this allows depth ~16 in practice — far deeper than any reasonable
/// tree, while preventing pathological inputs.
const ID_TOTAL_MAX_LEN: usize = 1024;

/// Reserved as the first path segment of any URI. `iii://fn/...` is the
/// section-URI marker; `iii://anything-else/...` is a state-backed
/// skill body lookup. The reservation only applies to the first segment
/// — `iii://docs/fn-reference` (the literal `fn` deeper in the path) is
/// a perfectly valid skill id.
const FN_PREFIX: &str = "fn";
const INDEX_URI: &str = "iii://skills";
const URI_PREFIX: &str = "iii://";

/// Description shared by both `skills::fetch_skill` and its public alias
/// `skill::fetch`. Phrased to nudge an MCP client that sees an
/// `iii://...` link in a skill body to call the tool with that URI.
const FETCH_DESCRIPTION: &str = "Fetches the content of one or more skill resources identified by iii:// URIs. \
     When you encounter iii:// links in skill instructions, use this tool to retrieve their contents \
     (batch with `uris` when helpful).";

/// Prefixes that are NEVER allowed as the `function_id` half of an
/// `iii://{skill}/{fn}` resource URI. Mirrors the hard-floor list in
/// [mcp/src/handler.rs](../../mcp/src/handler.rs); duplicated here so
/// `skills` can enforce the recursion guard without importing from
/// the mcp crate (each worker is its own cargo workspace). Keep the
/// two lists in sync when adding an infra namespace.
pub const ALWAYS_HIDDEN_PREFIXES: &[&str] = &[
    "engine::",
    "state::",
    "stream::",
    "iii.",
    "iii::",
    "mcp::",
    "a2a::",
    "skills::",
    "prompts::",
];

pub fn is_always_hidden(function_id: &str) -> bool {
    ALWAYS_HIDDEN_PREFIXES
        .iter()
        .any(|p| function_id.starts_with(p))
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RegisterSkillInput {
    /// Unique skill id (lowercase ASCII, kebab/underscore, max 64 chars).
    id: String,
    /// Markdown body served at iii://{id}.
    skill: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct RegisterSkillOutput {
    id: String,
    registered_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UnregisterSkillInput {
    id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct UnregisterSkillOutput {
    id: String,
    removed: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListSkillsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct SkillEntry {
    id: String,
    bytes: usize,
    registered_at: String,
    /// `"state"` for state-backed skills (`skills::register`), `"fs"`
    /// for filesystem-backed entries loaded via the `skills:` glob
    /// patterns. Lets clients distinguish the two without a separate
    /// query.
    origin: &'static str,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListSkillsOutput {
    skills: Vec<SkillEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadResourceInput {
    uri: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FetchSkillInput {
    /// A single iii:// URI to read. Must start with "iii://".
    #[serde(default)]
    pub uri: Option<String>,
    /// Multiple iii:// URIs to read and concatenate into one response.
    #[serde(default)]
    pub uris: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct StoredSkill {
    id: String,
    skill: String,
    registered_at: String,
}

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &SubscriberSet) {
    register_register_skill(iii, cfg, subscribers);
    register_unregister_skill(iii, cfg, subscribers);
    register_list_skills(iii, cfg);
    register_resources_list(iii, cfg);
    register_resources_read(iii, cfg);
    register_resources_templates(iii);
    register_fetch_skill(iii, cfg);
    register_fetch_skill_public_alias(iii, cfg);
}

fn register_register_skill(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &SubscriberSet) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let subs_inner = subscribers.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::register", move |req: RegisterSkillInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            let subs = subs_inner.clone();
            async move {
                validate_id(&req.id).map_err(IIIError::Handler)?;
                if req.skill.is_empty() {
                    return Err(IIIError::Handler("skill must be non-empty".into()));
                }
                if req.skill.len() > SKILL_BODY_MAX_BYTES {
                    return Err(IIIError::Handler(format!(
                        "skill body too large ({} bytes; max {SKILL_BODY_MAX_BYTES})",
                        req.skill.len()
                    )));
                }

                let stored = StoredSkill {
                    id: req.id.clone(),
                    skill: req.skill,
                    registered_at: chrono::Utc::now().to_rfc3339(),
                };
                let value = serde_json::to_value(&stored)
                    .map_err(|e| IIIError::Handler(format!("encode skill: {e}")))?;
                state::state_set(
                    &iii,
                    &cfg.scopes.skills,
                    &req.id,
                    value,
                    cfg.state_timeout_ms,
                )
                .await?;
                tracing::info!(skill_id = %req.id, "skill registered");

                // Fan out to any `skills::on-change` subscribers with a
                // Void dispatch so the write path doesn't block on
                // downstream latency.
                trigger_types::dispatch(&iii, &subs, json!({ "op": "register", "id": req.id }))
                    .await;

                Ok::<_, IIIError>(RegisterSkillOutput {
                    id: req.id,
                    registered_at: stored.registered_at,
                })
            }
        })
        .description("Register a markdown skill so it appears as iii://{id} in resources/list."),
    );
}

fn register_unregister_skill(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &SubscriberSet) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let subs_inner = subscribers.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::unregister", move |req: UnregisterSkillInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            let subs = subs_inner.clone();
            async move {
                validate_id(&req.id).map_err(IIIError::Handler)?;
                let prior =
                    state::state_delete(&iii, &cfg.scopes.skills, &req.id, cfg.state_timeout_ms)
                        .await?;
                let removed = !prior.is_null();
                tracing::info!(skill_id = %req.id, removed, "skill unregister");

                // Only fan out when the state actually changed so
                // idempotent deletes stay quiet.
                if removed {
                    trigger_types::dispatch(
                        &iii,
                        &subs,
                        json!({ "op": "unregister", "id": req.id }),
                    )
                    .await;
                }

                Ok::<_, IIIError>(UnregisterSkillOutput {
                    id: req.id,
                    removed,
                })
            }
        })
        .description("Remove a registered skill by id."),
    );
}

fn register_list_skills(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::list", move |_input: ListSkillsInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move {
                let entries = list_stored(&iii, &cfg).await?;
                let mut out: Vec<SkillEntry> = entries
                    .into_iter()
                    .map(|s| SkillEntry {
                        bytes: s.skill.len(),
                        id: s.id,
                        registered_at: s.registered_at,
                        origin: "state",
                    })
                    .collect();
                let state_ids: HashSet<String> = out.iter().map(|e| e.id.clone()).collect();
                for fs in non_colliding_fs_skills(&cfg, &state_ids) {
                    let (bytes, registered_at) = fs_metadata(&fs);
                    out.push(SkillEntry {
                        id: fs.id,
                        bytes,
                        registered_at,
                        origin: "fs",
                    });
                }
                out.sort_by(|a, b| a.id.cmp(&b.id));
                Ok::<_, IIIError>(ListSkillsOutput { skills: out })
            }
        })
        .description(
            "List registered skills (id, body length, registered_at, origin) without bodies. \
             `origin` is `state` for state-backed entries, `fs` for filesystem-backed.",
        ),
    );
}

fn register_resources_list(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "skills::resources-list",
            move |_input: EmptyInput| {
                let iii = iii_inner.clone();
                let cfg = cfg_inner.clone();
                async move { Ok::<_, IIIError>(list_resources(&iii, &cfg).await) }
            },
        )
        .description(
            "Internal: returns the MCP resources/list envelope with the iii://skills index + one iii://{id} entry per registered skill.",
        ),
    );
}

fn register_resources_read(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::resources-read", move |req: ReadResourceInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move { read(&iii, &cfg, &req.uri).await.map_err(IIIError::Handler) }
        })
        .description(
            "Internal: resolves an iii:// URI and returns the MCP resources/read contents envelope.",
        ),
    );
}

fn register_resources_templates(iii: &Arc<III>) {
    iii.register_function(
        RegisterFunction::new_async(
            "skills::resources-templates",
            move |_input: EmptyInput| async move { Ok::<_, IIIError>(list_templates()) },
        )
        .description("Internal: returns the MCP resources/templates/list envelope."),
    );
}

/// Internal id, hidden from MCP `tools/list` because the bridge
/// hard-floors every `skills::` prefix. Sibling workers can still call
/// it directly via `iii.trigger`.
fn register_fetch_skill(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::fetch_skill", move |req: FetchSkillInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move {
                fetch_skill(&iii, &cfg, req)
                    .await
                    .map_err(IIIError::Handler)
            }
        })
        .description(FETCH_DESCRIPTION),
    );
}

/// Public alias on a non-hidden namespace so MCP clients see the tool
/// (as `skill__fetch`) without changing the mcp worker. Delegates to
/// the same shared core fn as `skills::fetch_skill`.
fn register_fetch_skill_public_alias(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("skill::fetch", move |req: FetchSkillInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move {
                fetch_skill(&iii, &cfg, req)
                    .await
                    .map_err(IIIError::Handler)
            }
        })
        .description(FETCH_DESCRIPTION)
        .metadata(json!({"tool": {"label": "Fetch skill"}})),
    );
}

// ---------- core resource helpers (also usable by in-process tests) ----------

pub async fn list_resources(iii: &III, cfg: &SkillsConfig) -> Value {
    let mut resources: Vec<Value> = vec![json!({
        "uri": INDEX_URI,
        "name": "skills",
        "description": "Index of every registered skill",
        "mimeType": "text/markdown",
    })];
    let state_skills = list_stored(iii, cfg).await.unwrap_or_default();
    let state_ids: HashSet<String> = state_skills.iter().map(|s| s.id.clone()).collect();
    for s in &state_skills {
        let title = extract_title(&s.skill).unwrap_or(&s.id);
        resources.push(json!({
            "uri": format!("{URI_PREFIX}{}", s.id),
            "name": title,
            "mimeType": "text/markdown",
        }));
    }
    for fs in non_colliding_fs_skills(cfg, &state_ids) {
        // Cheap title resolution: read the file once. On any IO error
        // we still surface the entry under its id so the resource
        // listing isn't suddenly missing rows because of a transient
        // read failure.
        let title = match fs_source::read_body(&fs.abs_path) {
            Ok(body) => extract_title(&body)
                .map(String::from)
                .unwrap_or_else(|| fs.id.clone()),
            Err(_) => fs.id.clone(),
        };
        resources.push(json!({
            "uri": format!("{URI_PREFIX}{}", fs.id),
            "name": title,
            "mimeType": "text/markdown",
        }));
    }
    json!({ "resources": resources })
}

pub fn list_templates() -> Value {
    json!({
        "resourceTemplates": [
            {
                "uriTemplate": "iii://{skill_id}",
                "name": "Skill",
                "description": "Markdown body of a registered skill (1+ segments separated by '/')",
                "mimeType": "text/markdown"
            },
            {
                "uriTemplate": "iii://fn/{function_path}",
                "name": "Skill section",
                "description": "Trigger the function at `function_path` (each '/' becomes '::') with empty input and serve its output. e.g. `iii://fn/scope/echo` triggers `scope::echo`.",
                "mimeType": "text/markdown"
            }
        ]
    })
}

pub async fn read(iii: &III, cfg: &SkillsConfig, uri: &str) -> Result<Value, String> {
    let parsed = parse_uri(uri)?;
    match parsed {
        ParsedUri::Index => {
            let body = render_index(iii, cfg).await;
            Ok(wrap_contents(uri, "text/markdown", &body))
        }
        ParsedUri::Skill(id) => {
            // The slashed path is the state key. Re-validate so a
            // crafted `iii://Foo` URI fails fast even if it slipped
            // past the section-prefix check.
            validate_id(&id)?;
            // State always wins on lookup so a runtime registration
            // covering an fs id stays authoritative (matches the
            // collision policy: fs entries colliding with state are
            // never served).
            if let Some(stored) = read_skill(iii, cfg, &id).await? {
                return Ok(wrap_contents(uri, "text/markdown", &stored.skill));
            }
            if let Some(fs) = find_fs_skill(cfg, &id) {
                let body = fs_source::read_body(&fs.abs_path)?;
                return Ok(wrap_contents(uri, "text/markdown", &body));
            }
            Err(format!("Skill not found: {id}"))
        }
        ParsedUri::Section { function_id } => {
            // Recursion guard — a client that crafts iii://fn/state/set
            // would otherwise tunnel into infra. We also block
            // skills::* / prompts::* so the resource resolver can't
            // drive the admin API.
            if is_always_hidden(&function_id) {
                return Err(format!(
                    "Function '{function_id}' is in an internal namespace and cannot back a skill resource"
                ));
            }
            let value = iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(cfg.state_timeout_ms),
                })
                .await
                .map_err(|e| format!("trigger {function_id}: {e}"))?;
            let (text, mime) = normalize_function_output(value);
            Ok(wrap_contents(uri, mime, &text))
        }
    }
}

// ---------- batched fetch (skills::fetch_skill / skill::fetch) ----------

/// Pure half of the fetch tool: validates the input shape, normalizes
/// to an ordered list of trimmed `iii://` URIs, and rejects anything
/// outside the `iii://` scheme. Split out so the validation branches
/// can be unit-tested without an iii engine.
pub fn validate_fetch_input(input: FetchSkillInput) -> Result<Vec<String>, String> {
    // `uris` wins when both are provided — matches the TS reference
    // impl and the handoff doc.
    let raw: Vec<String> = match (input.uris, input.uri) {
        (Some(v), _) if !v.is_empty() => v,
        (_, Some(s)) if !s.trim().is_empty() => vec![s],
        _ => return Err("Provide uri or a non-empty uris array".into()),
    };
    let list: Vec<String> = raw
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .collect();
    if list.is_empty() {
        return Err("Provide uri or a non-empty uris array".into());
    }
    for u in &list {
        if !u.starts_with(URI_PREFIX) {
            return Err(format!("Invalid URI (must start with iii://): {u}"));
        }
    }
    Ok(list)
}

/// Resolve every `iii://` URI in `input` through [`read`], wrap each
/// result as `# {uri}\n\n{text}`, and join sections with
/// `\n\n---\n\n`. Returns plain markdown — the MCP bridge's
/// `tool_text` passes through `Value::String` as `text/plain` content
/// without re-encoding it as JSON.
pub async fn fetch_skill(
    iii: &III,
    cfg: &SkillsConfig,
    input: FetchSkillInput,
) -> Result<String, String> {
    let list = validate_fetch_input(input)?;
    let mut sections = Vec::with_capacity(list.len());
    for uri in &list {
        let v = read(iii, cfg, uri).await?;
        let text = v["contents"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        sections.push(format!("# {uri}\n\n{}", text.trim_end()));
    }
    Ok(sections.join("\n\n---\n\n"))
}

// ---------- URI parsing ----------

#[derive(Debug, PartialEq, Eq)]
pub enum ParsedUri {
    /// `iii://skills` — the auto-rendered tree-of-skills index.
    Index,
    /// State-backed skill body. The payload is the full slashed path
    /// the body was registered under (1+ segments). The first segment
    /// is never `fn` — that prefix is reserved for [`Section`].
    Skill(String),
    /// Function trigger. The payload is the resolved iii function id
    /// built by joining the URI segments after `fn/` with `::`.
    /// e.g. `iii://fn/scope/echo` → `function_id == "scope::echo"`.
    Section { function_id: String },
}

/// Parse an `iii://...` resource URI into a [`ParsedUri`].
///
/// Branching is on the **first path segment**:
///
/// - Empty body → error.
/// - `skills` → [`ParsedUri::Index`].
/// - `fn` → [`ParsedUri::Section`]; remaining segments must satisfy
///   [`validate_id_segment`] and are joined with `::` to form the
///   function id. `iii://fn` alone (no segments after) is an error.
/// - Anything else → [`ParsedUri::Skill`] with the full slashed path
///   as the state key.
///
/// Empty path segments anywhere (`iii://a//b`, leading or trailing
/// `/`) are rejected so the parser stays a strict bijection with the
/// state key.
pub fn parse_uri(uri: &str) -> Result<ParsedUri, String> {
    let rest = uri
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| format!("Resource URI must start with iii://: {uri}"))?;
    if rest.is_empty() {
        return Err(format!("Empty resource id: {uri}"));
    }
    if rest == "skills" {
        return Ok(ParsedUri::Index);
    }

    let segments: Vec<&str> = rest.split('/').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(format!(
            "Resource URI may not contain empty segments (no leading, trailing, or doubled '/'): {uri}"
        ));
    }

    if segments[0] == FN_PREFIX {
        let fn_segments = &segments[1..];
        if fn_segments.is_empty() {
            return Err(format!(
                "Section URI 'iii://fn' is missing a function path: expected iii://fn/{{a}}/{{b}}/...: {uri}"
            ));
        }
        for seg in fn_segments {
            validate_id_segment(seg)
                .map_err(|e| format!("invalid section URI segment {seg:?}: {e}"))?;
        }
        Ok(ParsedUri::Section {
            function_id: fn_segments.join("::"),
        })
    } else {
        Ok(ParsedUri::Skill(rest.to_string()))
    }
}

/// Validate a single id segment. Used for both the per-segment check
/// in [`validate_id`] and the per-segment check inside section URIs
/// in [`parse_uri`].
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
/// The first segment must NOT equal [`FN_PREFIX`] (`"fn"`) — that
/// literal is reserved as the section-URI prefix at the top level so
/// `iii://fn/...` is unambiguously a function trigger. Other segments
/// can use `fn` freely (e.g. `docs/fn-reference` is a valid id).
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
    if segments[0] == FN_PREFIX {
        return Err(format!(
            "id may not have {FN_PREFIX:?} as its first segment (reserved as the iii://fn/ section-URI marker): {id:?}"
        ));
    }
    Ok(())
}

// ---------- markdown helpers ----------

async fn render_index(iii: &III, cfg: &SkillsConfig) -> String {
    let state_skills = match list_stored(iii, cfg).await {
        Ok(s) => s,
        Err(e) => {
            return format!("# Skills\n\n_Error reading skills index: {e}_\n");
        }
    };
    let mut out = String::from(
        "# Skills\n\nRead each skill's resource for orientation on when and why to call its functions. \
         Sub-skills are indented under their parent path so a top-level skill stays small \
         and the LLM can drill in only when it needs more detail.\n\n",
    );

    let state_ids: HashSet<String> = state_skills.iter().map(|s| s.id.clone()).collect();
    let fs_skills = non_colliding_fs_skills(cfg, &state_ids);

    if state_skills.is_empty() && fs_skills.is_empty() {
        out.push_str("_No skills are currently registered._\n");
        return out;
    }

    // `list_stored` returns entries sorted lexicographically by id, so a
    // single linear pass yields a correct tree: every nested entry
    // appears immediately after its parent (or its parent's last
    // descendant). Indent each entry by `2 * depth` spaces, where depth
    // is the number of '/' separators in the id.
    for s in &state_skills {
        let title = extract_title(&s.skill).unwrap_or(&s.id);
        let desc = extract_description(&s.skill).unwrap_or_default();
        push_index_bullet(&mut out, &s.id, title, &desc);
    }

    if !fs_skills.is_empty() {
        if !state_skills.is_empty() {
            out.push('\n');
        }
        out.push_str(
            "## Custom skills\n\nLoaded from `skills:` glob patterns in the worker config. \
             The file system is the source of truth for these entries.\n\n",
        );
        for fs in &fs_skills {
            let body = fs_source::read_body(&fs.abs_path).ok();
            let title = body
                .as_deref()
                .and_then(extract_title)
                .map(String::from)
                .unwrap_or_else(|| fs.id.clone());
            let desc = body
                .as_deref()
                .and_then(extract_description)
                .unwrap_or_default();
            push_index_bullet(&mut out, &fs.id, &title, &desc);
        }
    }

    out
}

fn push_index_bullet(out: &mut String, id: &str, title: &str, desc: &str) {
    let depth = id.matches('/').count();
    let indent = " ".repeat(depth * 2);
    let suffix = if desc.is_empty() {
        String::new()
    } else {
        format!(" — {desc}")
    };
    out.push_str(&format!(
        "{indent}- [`{id}`](iii://{id}) — {title}{suffix}\n"
    ));
}

pub fn extract_title(markdown: &str) -> Option<&str> {
    markdown.lines().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed.strip_prefix("# ").map(|s| s.trim())
    })
}

pub fn extract_description(markdown: &str) -> Option<String> {
    let mut buf = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
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
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(trimmed);
    }
    if buf.is_empty() {
        return None;
    }
    // Return the first paragraph verbatim. We deliberately do not cap the
    // length here: the LLM consuming the index needs the full opening
    // sentence to decide whether to drill into the skill, and the loop
    // already stops at the first blank line / heading so the buffer is
    // bounded by the author's paragraph length, not the full body.
    Some(buf)
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((byte_end, _)) => format!("{}...", &s[..byte_end]),
        None => s.to_string(),
    }
}

// ---------- output normalization for iii://{skill}/{function} ----------

pub fn normalize_function_output(v: Value) -> (String, &'static str) {
    if let Value::String(s) = &v {
        return (s.clone(), "text/markdown");
    }
    if let Some(content) = v.get("content").and_then(|c| c.as_str()) {
        return (content.to_string(), "text/markdown");
    }
    let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
    (pretty, "application/json")
}

fn wrap_contents(uri: &str, mime: &str, text: &str) -> Value {
    json!({
        "contents": [
            { "uri": uri, "mimeType": mime, "text": text }
        ]
    })
}

// ---------- state wrappers ----------

async fn read_skill(
    iii: &III,
    cfg: &SkillsConfig,
    id: &str,
) -> Result<Option<StoredSkill>, String> {
    let raw = state::state_get(iii, &cfg.scopes.skills, id, cfg.state_timeout_ms)
        .await
        .map_err(|e| format!("state::get: {e}"))?;
    if raw.is_null() {
        return Ok(None);
    }
    serde_json::from_value::<StoredSkill>(raw)
        .map(Some)
        .map_err(|e| format!("decode stored skill {id}: {e}"))
}

async fn list_stored(iii: &III, cfg: &SkillsConfig) -> Result<Vec<StoredSkill>, IIIError> {
    let raw = state::state_list(iii, &cfg.scopes.skills, cfg.state_timeout_ms).await?;
    let entries = state::extract_state_entries(raw);
    let mut out: Vec<StoredSkill> = entries
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

// ---------- fs source helpers ----------

/// Re-glob the configured `skills:` patterns and return entries whose
/// id is NOT shadowed by a state-backed skill. This is the single
/// place the collision policy is enforced — every read/list/index
/// path goes through it.
pub fn non_colliding_fs_skills(cfg: &SkillsConfig, state_ids: &HashSet<String>) -> Vec<FsSkill> {
    let base = cfg.glob_base_dir();
    let (fs, _skipped) = fs_source::expand_skill_globs(&base, &cfg.skills);
    fs.into_iter()
        .filter(|s| !state_ids.contains(&s.id))
        .collect()
}

/// Targeted lookup for the read path. Returns `None` if no glob
/// pattern matches `id` or if a glob expansion error swallowed it.
fn find_fs_skill(cfg: &SkillsConfig, id: &str) -> Option<FsSkill> {
    let base = cfg.glob_base_dir();
    let (fs, _skipped) = fs_source::expand_skill_globs(&base, &cfg.skills);
    fs.into_iter().find(|s| s.id == id)
}

/// Cheap metadata for `skills::list`. Bytes is the on-disk file size;
/// `registered_at` is the file's mtime as RFC 3339. Falls back to "0"
/// / "" when the metadata read fails so the listing entry still shows
/// up rather than being silently dropped.
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
    use super::*;

    // ── parse_uri: index ────────────────────────────────────────────────

    #[test]
    fn parse_index_uri() {
        assert_eq!(parse_uri("iii://skills").unwrap(), ParsedUri::Index);
    }

    // ── parse_uri: skill bodies (state-backed) ──────────────────────────

    #[test]
    fn parse_single_skill_uri() {
        assert_eq!(
            parse_uri("iii://brain").unwrap(),
            ParsedUri::Skill("brain".into())
        );
    }

    #[test]
    fn parse_two_segment_skill_uri() {
        assert_eq!(
            parse_uri("iii://parent/sub").unwrap(),
            ParsedUri::Skill("parent/sub".into())
        );
    }

    #[test]
    fn parse_three_segment_skill_uri() {
        assert_eq!(
            parse_uri("iii://a/b/c").unwrap(),
            ParsedUri::Skill("a/b/c".into())
        );
    }

    #[test]
    fn parse_deeply_nested_skill_uri() {
        assert_eq!(
            parse_uri("iii://a/b/c/d/e").unwrap(),
            ParsedUri::Skill("a/b/c/d/e".into())
        );
    }

    #[test]
    fn parse_skill_uri_allows_fn_at_non_first_segment() {
        // `fn` is reserved only at depth 0. Deeper occurrences are
        // ordinary path segments.
        assert_eq!(
            parse_uri("iii://docs/fn-reference").unwrap(),
            ParsedUri::Skill("docs/fn-reference".into())
        );
        assert_eq!(
            parse_uri("iii://a/fn/c").unwrap(),
            ParsedUri::Skill("a/fn/c".into())
        );
    }

    // ── parse_uri: section URIs (function triggers) ─────────────────────

    #[test]
    fn parse_section_uri_single_segment() {
        // `iii://fn/foo` triggers function `foo` (no scope).
        assert_eq!(
            parse_uri("iii://fn/foo").unwrap(),
            ParsedUri::Section {
                function_id: "foo".into(),
            }
        );
    }

    #[test]
    fn parse_section_uri_two_segments_join_with_double_colon() {
        assert_eq!(
            parse_uri("iii://fn/scope/echo").unwrap(),
            ParsedUri::Section {
                function_id: "scope::echo".into(),
            }
        );
    }

    #[test]
    fn parse_section_uri_three_segments() {
        assert_eq!(
            parse_uri("iii://fn/resend/email/send").unwrap(),
            ParsedUri::Section {
                function_id: "resend::email::send".into(),
            }
        );
    }

    #[test]
    fn parse_section_uri_arbitrary_depth() {
        assert_eq!(
            parse_uri("iii://fn/a/b/c/d").unwrap(),
            ParsedUri::Section {
                function_id: "a::b::c::d".into(),
            }
        );
    }

    // ── parse_uri: error cases ──────────────────────────────────────────

    #[test]
    fn rejects_missing_prefix() {
        assert!(parse_uri("brain").is_err());
        assert!(parse_uri("https://example.com").is_err());
    }

    #[test]
    fn rejects_empty_body() {
        assert!(parse_uri("iii://").is_err());
    }

    #[test]
    fn rejects_empty_segments() {
        // Leading slash, trailing slash, doubled slash all yield empty
        // segments and must error rather than collapse silently.
        assert!(parse_uri("iii:///fn").is_err());
        assert!(parse_uri("iii://skill/").is_err());
        assert!(parse_uri("iii://a//b").is_err());
        assert!(parse_uri("iii://fn/").is_err());
    }

    #[test]
    fn rejects_section_uri_with_no_function_path() {
        // `iii://fn` alone has nothing to call.
        let err = parse_uri("iii://fn").unwrap_err();
        assert!(err.contains("missing a function path"), "got: {err}");
    }

    #[test]
    fn rejects_section_uri_with_invalid_segment() {
        // Per-segment rules apply to the function path too — uppercase
        // ASCII would otherwise smuggle through as a function id.
        assert!(parse_uri("iii://fn/Bad-Case").is_err());
        assert!(parse_uri("iii://fn/a/b::c").is_err());
        assert!(parse_uri("iii://fn/a b").is_err());
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
    fn id_validation_allows_fn_at_non_first_segment() {
        // Reserved-word rule only applies at depth 0.
        assert!(validate_id("docs/fn-reference").is_ok());
        assert!(validate_id("a/fn").is_ok());
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
    fn id_validation_rejects_fn_as_first_segment() {
        let err = validate_id("fn").unwrap_err();
        assert!(err.contains("first segment"), "got: {err}");
        assert!(validate_id("fn/anything").is_err());
        assert!(validate_id("fn/a/b").is_err());
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
        // Build an id just over the total cap: many short segments.
        // Each "ab/" is 3 chars; ~342 of them get over 1024.
        let too_long: String = "ab/".repeat((ID_TOTAL_MAX_LEN / 3) + 5);
        let trimmed = too_long.trim_end_matches('/').to_string();
        assert!(trimmed.len() > ID_TOTAL_MAX_LEN);
        assert!(validate_id(&trimmed).is_err());
    }

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
    fn extract_description_keeps_long_first_paragraph() {
        // The 140-char cap that used to truncate this paragraph mangled
        // the index bullets surfaced over MCP. The first paragraph must
        // come back verbatim — the loop already stops at the first
        // blank line / heading, so length is bounded by the author.
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

    #[test]
    fn normalize_string_returns_markdown() {
        let (text, mime) = normalize_function_output(Value::String("hello".into()));
        assert_eq!(text, "hello");
        assert_eq!(mime, "text/markdown");
    }

    #[test]
    fn normalize_content_object_returns_markdown() {
        let (text, mime) = normalize_function_output(json!({ "content": "hi" }));
        assert_eq!(text, "hi");
        assert_eq!(mime, "text/markdown");
    }

    #[test]
    fn normalize_other_falls_back_to_json() {
        let (text, mime) = normalize_function_output(json!({ "x": 1 }));
        assert_eq!(mime, "application/json");
        assert!(text.contains("\"x\""));
    }

    #[test]
    fn truncate_chars_handles_multibyte() {
        let s = "áéíóú".repeat(50);
        let out = truncate_chars(&s, 5);
        assert!(out.starts_with("áéíóú"));
        assert!(out.ends_with("..."));
        assert_eq!(out.chars().count(), 5 + 3);
    }

    #[test]
    fn hard_floor_rejects_infra_namespaces() {
        assert!(is_always_hidden("engine::foo"));
        assert!(is_always_hidden("state::get"));
        assert!(is_always_hidden("stream::publish"));
        assert!(is_always_hidden("iii.on_foo"));
        assert!(is_always_hidden("iii::internal"));
        assert!(is_always_hidden("mcp::handler"));
        assert!(is_always_hidden("a2a::send"));
        assert!(is_always_hidden("skills::register"));
        assert!(is_always_hidden("prompts::register"));
    }

    #[test]
    fn hard_floor_allows_ordinary_namespaces() {
        assert!(!is_always_hidden("mem::observe"));
        assert!(!is_always_hidden("brain::summarize"));
        assert!(!is_always_hidden("my-worker::my-fn"));
    }

    #[test]
    fn list_templates_has_two_entries() {
        let v = list_templates();
        let templates = v["resourceTemplates"].as_array().unwrap();
        assert_eq!(templates.len(), 2);
    }

    // ── fetch input validation ─────────────────────────────────────────

    #[test]
    fn fetch_skill_rejects_no_uri() {
        let err = validate_fetch_input(FetchSkillInput::default()).unwrap_err();
        assert!(err.contains("Provide uri"), "got: {err}");
    }

    #[test]
    fn fetch_skill_rejects_blank_uri() {
        let err = validate_fetch_input(FetchSkillInput {
            uri: Some("   ".into()),
            uris: None,
        })
        .unwrap_err();
        assert!(err.contains("Provide uri"), "got: {err}");
    }

    #[test]
    fn fetch_skill_rejects_empty_uris_array() {
        let err = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec![]),
        })
        .unwrap_err();
        assert!(err.contains("Provide uri"), "got: {err}");
    }

    #[test]
    fn fetch_skill_rejects_non_iii_uri() {
        let err = validate_fetch_input(FetchSkillInput {
            uri: Some("https://example.com".into()),
            uris: None,
        })
        .unwrap_err();
        assert!(err.contains("iii://"), "got: {err}");
    }

    #[test]
    fn fetch_skill_rejects_non_iii_uri_in_array() {
        let err = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec!["iii://ok".into(), "ftp://nope".into()]),
        })
        .unwrap_err();
        assert!(err.contains("iii://"), "got: {err}");
    }

    #[test]
    fn fetch_skill_uris_takes_precedence_when_both_provided() {
        // `uris` wins; the single `uri` is ignored.
        let list = validate_fetch_input(FetchSkillInput {
            uri: Some("iii://from-uri".into()),
            uris: Some(vec!["iii://from-uris".into()]),
        })
        .unwrap();
        assert_eq!(list, vec!["iii://from-uris".to_string()]);
    }

    #[test]
    fn fetch_skill_trims_whitespace_around_uris() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec!["  iii://a  ".into(), "iii://b\n".into()]),
        })
        .unwrap();
        assert_eq!(list, vec!["iii://a".to_string(), "iii://b".to_string()]);
    }

    #[test]
    fn fetch_skill_drops_blank_entries_in_uris_array() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec!["iii://a".into(), "   ".into(), "iii://b".into()]),
        })
        .unwrap();
        assert_eq!(list, vec!["iii://a".to_string(), "iii://b".to_string()]);
    }

    #[test]
    fn fetch_skill_single_uri_preserved_after_trim() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: Some("  iii://only  ".into()),
            uris: None,
        })
        .unwrap();
        assert_eq!(list, vec!["iii://only".to_string()]);
    }

    #[test]
    fn skill_fetch_alias_namespace_not_in_hard_floor() {
        // Regression guard: if someone broadens ALWAYS_HIDDEN_PREFIXES
        // to also catch `skill::` (singular), the public alias would
        // disappear from `tools/list` and fail under the section
        // resolver's recursion guard. This pins the namespace as
        // user-callable.
        assert!(!is_always_hidden("skill::fetch"));
    }
}
