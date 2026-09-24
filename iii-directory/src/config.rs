//! Worker runtime config.
//!
//! The skills worker is a filesystem-backed reader plus a
//! `directory::skills::download` function that pulls markdown into the
//! configured `skills_folder`. There is no state-backed registry, no
//! glob arrays, no scopes — everything lives on disk under one root.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Shared, hot-reloadable config handle. Handlers snapshot the current
/// value per call (`handle.load_full()`, lock-free) so a
/// `configuration:updated` reload that `store`s a new value takes effect
/// on the next invocation without re-registering any function.
pub type SharedConfig = Arc<ArcSwap<SkillsConfig>>;

/// Default base URL for the workers registry. Overrideable via
/// `registry_url:` in the config so self-hosted deployments can repoint.
pub const DEFAULT_REGISTRY_URL: &str = "https://api.workers.iii.dev";

/// Project-relative destination for downloaded skills.
pub const DEFAULT_SKILLS_FOLDER: &str = "skills";

/// Project-relative destination for local skill overrides.
pub const DEFAULT_LOCAL_SKILLS_FOLDER: &str = "skills/iii";

/// Project-relative destination for reusable agent profiles.
pub const DEFAULT_AGENTS_FOLDER: &str = "agents";

/// User-global root for reusable agent profiles, shared by every project on
/// the machine. Profiles here are edited in place; the worker never creates
/// the directory itself.
pub const DEFAULT_GLOBAL_AGENTS_FOLDER: &str = "~/.iii/agents";

/// Project-relative root for agent skills. Read-only; never written by this worker.
pub const DEFAULT_AGENTS_SKILLS_FOLDER: &str = ".agents/skills";

/// User-global root for agent skills (the home-directory side of the
/// `~/.agents/skills` convention). Read-only; never written by this worker.
pub const DEFAULT_GLOBAL_AGENTS_SKILLS_FOLDER: &str = "~/.agents/skills";

fn default_skills_folder() -> String {
    iii_worker_paths::default_path(DEFAULT_SKILLS_FOLDER)
}

fn default_local_skills_folder() -> String {
    iii_worker_paths::default_path(DEFAULT_LOCAL_SKILLS_FOLDER)
}

fn default_agents_folder() -> String {
    iii_worker_paths::default_path(DEFAULT_AGENTS_FOLDER)
}

fn default_global_agents_folder() -> String {
    // Kept `~`-prefixed (not pre-resolved): resolve_path expands it at use
    // time, so the default follows the user's home, never the compose dir.
    DEFAULT_GLOBAL_AGENTS_FOLDER.to_string()
}

fn default_agents_skills_folder() -> String {
    iii_worker_paths::default_path(DEFAULT_AGENTS_SKILLS_FOLDER)
}

fn default_global_agents_skills_folder() -> String {
    // Kept `~`-prefixed (not pre-resolved): resolve_path expands it at use
    // time, so the default follows the user's home, never the compose dir.
    DEFAULT_GLOBAL_AGENTS_SKILLS_FOLDER.to_string()
}

fn default_registry_url() -> String {
    DEFAULT_REGISTRY_URL.to_string()
}

fn default_download_timeout_ms() -> u64 {
    60_000
}

fn default_registry_cache_ttl_ms() -> u64 {
    60_000
}

fn default_filter_unregistered() -> bool {
    true
}

fn default_auto_download() -> bool {
    true
}

/// The pinned MiniLM search bundle lives here unless configured otherwise:
/// embedding files at the root, reranker files under `reranker/`.
pub fn default_function_search_model_path() -> Option<String> {
    Some(format!(
        "~/.cache/iii/all-MiniLM-L6-v2-{}",
        crate::functions::search_semantic::MINILM_REVISION
    ))
}

fn default_function_search_model_download() -> bool {
    true
}

fn default_function_search_judge_timeout_ms() -> u64 {
    3000
}

fn default_function_search_judge_min_relevance() -> f64 {
    0.5
}

/// Measured on 22 English capabilities over a 16-document shortlist: every
/// search kept a correct function at 0.05-0.1 with JEV and SemIf, while 0.15+
/// dropped SemIf's runner-up answers.
fn default_function_search_judge_choice_min_probability() -> f64 {
    0.1
}

fn default_function_search_judge_side_lane_min_relevance() -> f64 {
    0.3
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FunctionSearchMode {
    Lexical,
    Hybrid,
    // The default: rank through the `judge` worker, falling back to Hybrid
    // (then Lexical) whenever `judge::evaluate` is not registered. A doc
    // comment here would turn the schema into a `oneOf`; keep it a comment.
    #[default]
    Judge,
}

/// How `judge` asks about a capability's shortlist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FunctionSearchJudgeQuestion {
    // One Noul per shortlisted document, admitted by the min_relevance
    // settings. A doc comment would turn the schema into a oneOf.
    Noul,
    // The default: one Choice per capability over its shortlist; the
    // documents compete, the best is always kept, the others need
    // function_search_judge_choice_min_probability. A judge that advertises
    // a context window under 4096 tokens (laya) gets a tournament instead.
    #[default]
    Choice,
    // Choice without the Hybrid shortlist: the whole function catalog plays
    // rounds of Choice questions over groups of at most 16 (sorted by id),
    // each group's winner goes on, and the last 16 get the final Choice.
    // Measured for laya: 21/22 searches against 15-17 with the shortlist.
    Tournament,
}

/// `hybrid`, and the Hybrid fallback `judge` uses whenever the judge worker
/// is unavailable, need a local semantic model; without a complete bundle at
/// `function_search_model_path` they silently run BM25-only, which is easy to
/// mistake for the model being active. Say so once, loudly. `judge` with the
/// path set to `null` chose a BM25 fallback on purpose and stays quiet.
/// `model_ready` is "the path is set and the bundle verifies".
pub fn warn_if_search_mode_lacks_model(
    mode: FunctionSearchMode,
    model_configured: bool,
    model_ready: bool,
) {
    let needs_model = match mode {
        FunctionSearchMode::Lexical => false,
        FunctionSearchMode::Hybrid => true,
        FunctionSearchMode::Judge => model_configured,
    };
    if needs_model && !model_ready {
        tracing::warn!(
            ?mode,
            "function_search_mode needs a local semantic model but no complete bundle is \
             available at function_search_model_path (unset, missing, or failed \
             verification/download); Hybrid ranking in directory::search_functions runs \
             BM25-only until the bundle is in place and iii-directory restarts"
        );
    }
}

fn deserialize_model_path<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if value.as_deref().is_some_and(|path| path.trim().is_empty()) {
        return Err(serde::de::Error::custom(
            "function_search_model_path must not be empty",
        ));
    }
    Ok(value)
}

fn deserialize_judge_timeout_ms<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u64::deserialize(deserializer)?;
    if !(1..=30_000).contains(&value) {
        return Err(serde::de::Error::custom(
            "function_search_judge_timeout_ms must be between 1 and 30000",
        ));
    }
    Ok(value)
}

fn deserialize_judge_min_relevance<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_unit_interval(deserializer, "function_search_judge_min_relevance")
}

fn deserialize_judge_side_lane_min_relevance<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_unit_interval(
        deserializer,
        "function_search_judge_side_lane_min_relevance",
    )
}

fn deserialize_judge_choice_min_probability<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_unit_interval(deserializer, "function_search_judge_choice_min_probability")
}

fn deserialize_unit_interval<'de, D>(deserializer: D, field: &str) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(serde::de::Error::custom(format!(
            "{field} must be finite and between 0 and 1"
        )));
    }
    Ok(value)
}

#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct SkillsConfig {
    /// Folder that backs every read (`directory::skills::list`,
    /// `directory::skills::get`, `directory::system-prompts::*`) and every
    /// write from `directory::skills::download`. Supports three forms:
    ///
    /// - Absolute path — used as-is.
    /// - `~`-prefixed — expands leading `~` via `dirs::home_dir()`.
    /// - Relative — resolved against `III_COMPOSE_DIR` when available.
    #[serde(default = "default_skills_folder")]
    pub skills_folder: String,

    /// Folder for local (project-scoped) skill overrides. A namespace
    /// directory present under this root shadows the same namespace in
    /// the global `skills_folder` entirely (whole-namespace override).
    /// Supports the same three resolution forms as `skills_folder`.
    #[serde(default = "default_local_skills_folder")]
    pub local_skills_folder: String,

    /// Read-write root for reusable agent profile Markdown files. Supports
    /// the same three resolution forms as `skills_folder`.
    #[serde(default = "default_agents_folder")]
    pub agents_folder: String,

    /// USER-GLOBAL root for reusable agent profiles (`~/.iii/agents`),
    /// shared by every project on the machine. Same direct `<id>.md` scan as
    /// `agents_folder`; an id present under `agents_folder` shadows the same
    /// id here. Profiles resolved here are updated and deleted IN PLACE
    /// (this is iii's own directory, unlike the external-tooling skills
    /// roots); `create` still writes `agents_folder` only. A missing
    /// directory is silently treated as empty and never materialized.
    /// Supports the same three resolution forms as `skills_folder`.
    #[serde(default = "default_global_agents_folder")]
    pub global_agents_folder: String,

    /// Read-only root for system-installed agent skills (the
    /// `~/.agents/skills` convention: one directory per skill, each
    /// containing a `SKILL.md`). Scanned shallowly — only
    /// `<skill>/SKILL.md`, never the skill's `reference/`/`scripts/`
    /// payload. A missing directory is silently treated as empty; the
    /// worker never creates or writes under this root. Namespaces here
    /// are shadowed by the same namespace under `skills_folder` or
    /// `local_skills_folder`. Supports the same three resolution forms
    /// as `skills_folder`.
    #[serde(default = "default_agents_skills_folder")]
    pub agents_skills_folder: String,

    /// Read-only root for USER-GLOBAL agent skills — the home-directory side
    /// of the `~/.agents/skills` convention, shared by every project on the
    /// machine. Same shallow `<skill>/SKILL.md` scan and read-only contract
    /// as `agents_skills_folder`; a namespace present there (or under
    /// `skills_folder` / `local_skills_folder`) shadows the same namespace
    /// here. Supports the same three resolution forms as `skills_folder`.
    #[serde(default = "default_global_agents_skills_folder")]
    pub global_agents_skills_folder: String,

    /// Workers registry base URL — used by `directory::skills::download`
    /// and the `directory::registry::*` proxies when a `worker=` source
    /// is specified. Stored without a trailing slash.
    #[serde(default = "default_registry_url")]
    pub registry_url: String,

    /// Timeout for a single download operation (HTTP request OR `git clone`)
    /// in milliseconds. Also used as the request timeout for
    /// `directory::registry::*` HTTP calls.
    #[serde(default = "default_download_timeout_ms")]
    pub download_timeout_ms: u64,

    /// TTL (in milliseconds) for cached
    /// `directory::registry::workers::list` and
    /// `directory::registry::workers::info` responses. Repeat lookups
    /// within this window skip the HTTP call. Set to 0 to disable
    /// caching.
    #[serde(default = "default_registry_cache_ttl_ms")]
    pub registry_cache_ttl_ms: u64,

    /// When `true` (default), read functions hide skills whose top
    /// namespace segment doesn't match a registered (installed) worker
    /// name. Orphan namespaces are hidden. When `false`, all scanned
    /// skills are returned regardless of installed workers.
    #[serde(default = "default_filter_unregistered")]
    pub filter_unregistered: bool,

    /// When `true` (default), the worker subscribes to `worker` trigger
    /// events and runs a boot-time reconcile to auto-download skills
    /// for installed workers that are missing from the global skills
    /// folder.
    #[serde(default = "default_auto_download")]
    pub auto_download: bool,

    /// Bind the `directory::pre-generate` hook so the conditional search
    /// hint can be injected into agent generations. Off by default: the
    /// harness identity prompt already teaches `directory::search_functions`
    /// as the default discovery path, so the per-generation hint is
    /// redundant. Turning it on binds the hook (hot, no restart) for
    /// deployments running a custom identity prompt without that doctrine.
    #[serde(default = "default_inject_hint")]
    pub inject_hint: bool,

    /// Only inject the search hint when the session's surface spans at
    /// least this many distinct non-engine workers. Narrower surfaces
    /// resolve faster through normal discovery than through a standing
    /// hint; `0` hints on every surface.
    #[serde(default = "default_hint_min_workers")]
    pub hint_min_workers: usize,

    /// Also search the private workers registry (every listed worker) on
    /// every `directory::search_functions` call and return matching
    /// NOT-installed workers as an `installable` section alongside the
    /// installed results. Fail-open: a registry error just omits the
    /// section.
    #[serde(default = "default_registry_search")]
    pub registry_search: bool,

    /// Installed-function search lane. Judge (the default) ranks through the
    /// `judge` worker when `judge::evaluate` is registered and answers; when
    /// the judge is missing, has no provider or API key, fails, or misses the
    /// deadline, it behaves as Hybrid (and, except on a deadline, skips the
    /// judge for 30 s). Hybrid fuses BM25 with the local MiniLM model and
    /// needs the bundle at `function_search_model_path` (downloaded on first
    /// run by default), serving BM25 until it is ready. Lexical is BM25 only.
    /// Mode changes apply without restart.
    #[serde(default)]
    pub function_search_mode: FunctionSearchMode,

    /// Total judge deadline per public search call, in milliseconds (1..=30000),
    /// shared by every judge call of the search. The registry lookup runs in
    /// parallel with the installed-function judge call, and the registry judge
    /// call gets what is left after it; local Hybrid fallback runs outside the
    /// budget. Hot-reloadable.
    #[serde(
        default = "default_function_search_judge_timeout_ms",
        deserialize_with = "deserialize_judge_timeout_ms"
    )]
    #[schemars(range(min = 1, max = 30000))]
    pub function_search_judge_timeout_ms: u64,

    /// Minimum judge relevance, finite and between 0 and 1 inclusive.
    /// A valid empty result stays empty; errors try Hybrid, then lexical.
    /// The default is an initial calibration value. Hot-reloadable.
    #[serde(
        default = "default_function_search_judge_min_relevance",
        deserialize_with = "deserialize_judge_min_relevance"
    )]
    #[schemars(range(min = 0, max = 1))]
    pub function_search_judge_min_relevance: f64,

    /// Minimum judge relevance for the side lanes (installed skills and
    /// registered triggers), finite and between 0 and 1 inclusive. Those
    /// documents score lower than functions for the same capability, so the
    /// default sits below `function_search_judge_min_relevance`. Hot-reloadable.
    #[serde(
        default = "default_function_search_judge_side_lane_min_relevance",
        deserialize_with = "deserialize_judge_side_lane_min_relevance"
    )]
    #[schemars(range(min = 0, max = 1))]
    pub function_search_judge_side_lane_min_relevance: f64,

    /// `choice` (the default) asks one multiple-choice question per capability
    /// over its shortlist: 16× fewer questions, and documents compete instead
    /// of each being approved on its own. `noul` asks one yes/no question per
    /// shortlisted document (up to 16 per capability) and admits them by the
    /// min_relevance settings. Local judges (SemIf, laya) need `choice` to fit
    /// the deadline. Hot-reloadable.
    #[serde(default)]
    pub function_search_judge_question: FunctionSearchJudgeQuestion,

    /// With `choice`, the best document of each capability is always kept and
    /// every other one needs at least this probability. Finite, between 0 and
    /// 1. Applies to functions, skills and triggers. Hot-reloadable.
    #[serde(
        default = "default_function_search_judge_choice_min_probability",
        deserialize_with = "deserialize_judge_choice_min_probability"
    )]
    #[schemars(range(min = 0, max = 1))]
    pub function_search_judge_choice_min_probability: f64,

    /// Local semantic model directory: the pinned MiniLM bundle (embedding
    /// files at the root, reranker files under `reranker/`). Defaults to `~/.cache/iii/all-MiniLM-L6-v2-<revision>`;
    /// `null` disables the local Hybrid lane, including the judge's Hybrid fallback.
    /// Changing it requires a restart.
    #[serde(
        default = "default_function_search_model_path",
        deserialize_with = "deserialize_model_path"
    )]
    pub function_search_model_path: Option<String>,

    /// When Hybrid or Judge mode is configured and the bundle at
    /// `function_search_model_path` is missing or incomplete at boot, download
    /// the pinned files from Hugging Face once, verifying every file by byte
    /// length and SHA-256 before use. Set `false` for air-gapped stacks; the
    /// worker then stays BM25-only until the bundle is provisioned by hand.
    /// Read once at boot: changing it requires a restart.
    #[serde(default = "default_function_search_model_download")]
    pub function_search_model_download: bool,
}

fn default_inject_hint() -> bool {
    false
}

fn default_hint_min_workers() -> usize {
    2
}

fn default_registry_search() -> bool {
    true
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_folder: default_skills_folder(),
            local_skills_folder: default_local_skills_folder(),
            agents_folder: default_agents_folder(),
            global_agents_folder: default_global_agents_folder(),
            agents_skills_folder: default_agents_skills_folder(),
            global_agents_skills_folder: default_global_agents_skills_folder(),
            registry_url: default_registry_url(),
            download_timeout_ms: default_download_timeout_ms(),
            registry_cache_ttl_ms: default_registry_cache_ttl_ms(),
            filter_unregistered: default_filter_unregistered(),
            auto_download: default_auto_download(),
            inject_hint: default_inject_hint(),
            hint_min_workers: default_hint_min_workers(),
            registry_search: default_registry_search(),
            function_search_mode: FunctionSearchMode::default(),
            function_search_judge_timeout_ms: default_function_search_judge_timeout_ms(),
            function_search_judge_min_relevance: default_function_search_judge_min_relevance(),
            function_search_judge_side_lane_min_relevance:
                default_function_search_judge_side_lane_min_relevance(),
            function_search_judge_question: FunctionSearchJudgeQuestion::default(),
            function_search_judge_choice_min_probability:
                default_function_search_judge_choice_min_probability(),
            function_search_model_path: default_function_search_model_path(),
            function_search_model_download: default_function_search_model_download(),
        }
    }
}

/// Resolve a path string supporting three forms:
///
/// - `~`-prefixed: expand leading `~` via the process home directory.
///   Leave the path unchanged if no home directory is available.
/// - Absolute: returned as-is.
/// - Relative: resolved against `III_COMPOSE_DIR`, or the process current
///   working directory when the worker runs outside Compose.
impl SkillsConfig {
    /// Absolute path to the configured global skills folder.
    pub fn resolved_skills_folder(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.skills_folder)
    }

    /// Absolute path to the configured local skills folder.
    pub fn local_skills_folder(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.local_skills_folder)
    }

    /// Absolute path to the configured read-write agent profile folder.
    pub fn resolved_agents_folder(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.agents_folder)
    }

    /// Every agent-profile root, precedence order: the read-write project
    /// root first, the read-only user-global (`~/.iii/agents`) root second —
    /// an id served by the project root shadows the same id in the global
    /// one. Deduplicated so pointing both settings at one directory never
    /// double-scans it.
    pub fn resolved_agents_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![
            self.resolved_agents_folder(),
            iii_worker_paths::resolve_path(&self.global_agents_folder),
        ];
        roots.dedup();
        roots
    }

    /// Absolute path to the configured (read-only) agents skills folder.
    pub fn resolved_agents_skills_folder(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.agents_skills_folder)
    }

    /// Every read-only agents skills root, precedence order: the project
    /// root first, the user-global (`~/.agents/skills`) root second — a
    /// namespace served by the project root shadows the same namespace in
    /// the global one. Deduplicated so pointing both settings at one
    /// directory never double-scans it.
    pub fn resolved_agents_skills_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![
            self.resolved_agents_skills_folder(),
            iii_worker_paths::resolve_path(&self.global_agents_skills_folder),
        ];
        roots.dedup();
        roots
    }

    /// Registry base URL with any trailing slash trimmed so callers can
    /// build URLs as `format!("{base}/w/{worker}/skills")`.
    pub fn registry_base(&self) -> &str {
        self.registry_url.trim_end_matches('/')
    }

    pub fn resolved_function_search_model_path(&self) -> Option<PathBuf> {
        self.function_search_model_path
            .as_deref()
            .map(iii_worker_paths::resolve_path)
    }

    /// Restart-requiring fields. A `configuration:updated` reload that
    /// changes any of these is refused (logged "restart required"):
    /// `skills_folder` / `local_skills_folder` / `agents_folder` /
    /// `agents_skills_folder`, `function_search_model_path` and `function_search_model_download` are baked into
    /// running tasks (and the fs-watch root set), and `auto_download` wires
    /// the `worker`-trigger subscription + boot reconcile at startup — none
    /// can be re-wired safely in place.
    pub fn topology(&self) -> Topology {
        Topology {
            skills_folder: self.skills_folder.clone(),
            local_skills_folder: self.local_skills_folder.clone(),
            agents_folder: self.agents_folder.clone(),
            agents_skills_folder: self.agents_skills_folder.clone(),
            auto_download: self.auto_download,
            function_search_model_path: self.resolved_function_search_model_path(),
            function_search_model_download: self.function_search_model_download,
        }
    }

    /// JSON Schema registered with the `configuration` worker so the
    /// console can render an editor for the `iii-directory` entry.
    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(SkillsConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("SkillsConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), SkillsConfig::default().to_json());
        }
        schema
    }

    /// Parse a YAML seed file. Used only to build `initial_value` on first
    /// `configuration::register`; the configuration worker env-expands
    /// `${VAR}` on read, so no local expansion is done here.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| format!("yaml parse: {e}"))
    }

    /// Read and parse a YAML seed file from disk.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse the authoritative value returned by `configuration::get`.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    /// Serialize for `initial_value` on register.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("SkillsConfig serializes")
    }

    /// Wrap into a shared, hot-reloadable handle (see [`SharedConfig`]).
    pub fn into_shared(self) -> SharedConfig {
        Arc::new(ArcSwap::from_pointee(self))
    }
}

/// Restart-requiring subset of [`SkillsConfig`] (see [`SkillsConfig::topology`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topology {
    pub skills_folder: String,
    pub local_skills_folder: String,
    pub agents_folder: String,
    pub agents_skills_folder: String,
    pub auto_download: bool,
    pub function_search_model_path: Option<PathBuf>,
    pub function_search_model_download: bool,
}

pub fn load_config(path: &str) -> Result<SkillsConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: SkillsConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_is_the_default_for_yaml_json_and_default() {
        for cfg in [
            SkillsConfig::default(),
            SkillsConfig::from_yaml("{}").unwrap(),
            SkillsConfig::from_json(&serde_json::json!({})).unwrap(),
        ] {
            let value = cfg.to_json();
            assert_eq!(value["function_search_mode"], "judge");
            assert_eq!(value["function_search_judge_timeout_ms"], 3000);
            assert_eq!(value["function_search_judge_min_relevance"], 0.5);
            assert_eq!(value["function_search_judge_side_lane_min_relevance"], 0.3);
            assert!(value.get("function_search_jev_api_key").is_none());
            assert!(value.get("function_search_jev_model").is_none());
        }
    }

    #[test]
    fn removed_jev_keys_are_ignored_but_the_jev_mode_value_is_rejected() {
        let stale = serde_json::json!({
            "function_search_jev_api_key": "old-secret",
            "function_search_jev_model": "jev-1.13.0",
            "function_search_jev_timeout_ms": 9000,
        });
        let cfg = SkillsConfig::from_json(&stale).unwrap();
        assert_eq!(cfg.function_search_judge_timeout_ms, 3000);
        assert!(!format!("{cfg:?}").contains("old-secret"));
        let error = SkillsConfig::from_yaml("function_search_mode: jev\n").unwrap_err();
        assert!(error.contains("jev"), "{error}");
    }

    #[test]
    fn judge_accepts_yaml_and_json_without_a_local_model() {
        let yaml = "function_search_mode: judge\nfunction_search_model_path: null\n";
        let json = serde_json::json!({
            "function_search_mode": "judge",
            "function_search_model_path": null,
        });
        for cfg in [
            SkillsConfig::from_yaml(yaml).unwrap(),
            SkillsConfig::from_json(&json).unwrap(),
        ] {
            assert_eq!(cfg.to_json()["function_search_mode"], "judge");
            assert!(cfg.function_search_model_path.is_none());
        }
    }

    #[test]
    fn judge_options_accept_inclusive_bounds_and_roundtrip() {
        for (timeout, relevance) in [(1, 0.0), (30_000, 1.0), (4500, 0.725)] {
            let value = serde_json::json!({
                "function_search_judge_timeout_ms": timeout,
                "function_search_judge_min_relevance": relevance,
            });
            for cfg in [
                SkillsConfig::from_json(&value).unwrap(),
                SkillsConfig::from_yaml(&serde_yaml::to_string(&value).unwrap()).unwrap(),
            ] {
                let serialized = cfg.to_json();
                assert_eq!(serialized["function_search_judge_timeout_ms"], timeout);
                assert_eq!(serialized["function_search_judge_min_relevance"], relevance);
                assert_eq!(
                    SkillsConfig::from_json(&serialized).unwrap().to_json(),
                    serialized
                );
            }
        }
    }

    #[test]
    fn judge_invalid_options_are_rejected_by_yaml_and_json_deserializers() {
        for (field, invalid) in [
            (
                "function_search_judge_timeout_ms",
                serde_json::json!([0, -1, 30001, 1.5, null, "3000"]),
            ),
            (
                "function_search_judge_min_relevance",
                serde_json::json!([-0.01, 1.01, null, "NaN", "Infinity", "0.5"]),
            ),
            (
                "function_search_judge_side_lane_min_relevance",
                serde_json::json!([-0.01, 1.01, null, "NaN", "Infinity", "0.3"]),
            ),
            (
                "function_search_judge_choice_min_probability",
                serde_json::json!([-0.01, 1.01, null, "NaN", "0.1"]),
            ),
            (
                "function_search_judge_question",
                serde_json::json!(["yes_no", "Choice", 1, null]),
            ),
        ] {
            for invalid in invalid.as_array().unwrap() {
                // Validate even when Judge is not the selected mode.
                let value = serde_json::json!({ field: invalid });
                let yaml = serde_yaml::to_string(&value).unwrap();
                assert!(
                    serde_json::from_value::<SkillsConfig>(value.clone()).is_err(),
                    "accepted JSON {value}"
                );
                assert!(
                    serde_yaml::from_str::<SkillsConfig>(&yaml).is_err(),
                    "accepted YAML {yaml}"
                );
                assert!(SkillsConfig::from_json(&value).is_err());
                assert!(SkillsConfig::from_yaml(&yaml).is_err());
            }
        }
        for invalid in [".nan", ".inf", "-.inf"] {
            let yaml = format!("function_search_judge_min_relevance: {invalid}\n");
            assert!(SkillsConfig::from_yaml(&yaml)
                .unwrap_err()
                .contains("function_search_judge_min_relevance"));
        }
    }

    #[test]
    fn judge_question_defaults_to_choice_with_its_threshold() {
        let defaults = SkillsConfig::default().to_json();
        assert_eq!(defaults["function_search_judge_question"], "choice");
        assert_eq!(
            defaults["function_search_judge_choice_min_probability"],
            0.1
        );
        let noul = SkillsConfig::from_json(&serde_json::json!({
            "function_search_judge_question": "noul",
            "function_search_judge_choice_min_probability": 0.05
        }))
        .unwrap();
        assert_eq!(
            noul.function_search_judge_question,
            FunctionSearchJudgeQuestion::Noul
        );
        assert_eq!(noul.function_search_judge_choice_min_probability, 0.05);
        let schema = SkillsConfig::json_schema();
        assert_eq!(
            schema["definitions"]["FunctionSearchJudgeQuestion"]["enum"],
            serde_json::json!(["noul", "choice", "tournament"])
        );
    }

    #[test]
    fn judge_schema_exposes_options_without_credentials() {
        let schema = SkillsConfig::json_schema();
        let props = schema["properties"].as_object().unwrap();
        assert_eq!(
            schema["definitions"]["FunctionSearchMode"]["enum"],
            serde_json::json!(["lexical", "hybrid", "judge"])
        );
        assert_eq!(props["function_search_mode"]["default"], "judge");
        assert_eq!(props["function_search_judge_timeout_ms"]["type"], "integer");
        assert_eq!(props["function_search_judge_timeout_ms"]["default"], 3000);
        assert_eq!(props["function_search_judge_timeout_ms"]["minimum"], 1.0);
        assert_eq!(
            props["function_search_judge_timeout_ms"]["maximum"],
            30000.0
        );
        for (field, default) in [
            ("function_search_judge_min_relevance", 0.5),
            ("function_search_judge_side_lane_min_relevance", 0.3),
        ] {
            assert_eq!(props[field]["type"], "number", "{field}");
            assert_eq!(props[field]["default"], default, "{field}");
            assert_eq!(props[field]["minimum"], 0.0, "{field}");
            assert_eq!(props[field]["maximum"], 1.0, "{field}");
        }
        assert_eq!(schema["example"], SkillsConfig::default().to_json());
        assert!(!props.keys().any(|key| key.contains("jev")), "{props:?}");
    }

    #[test]
    fn judge_mode_and_options_do_not_change_restart_topology() {
        let base = SkillsConfig::default();
        let tuned = SkillsConfig::from_json(&serde_json::json!({
            "function_search_mode": "hybrid",
            "function_search_judge_timeout_ms": 4500,
            "function_search_judge_min_relevance": 0.725,
            "function_search_judge_side_lane_min_relevance": 0.2,
        }))
        .unwrap();
        assert_eq!(base.topology(), tuned.topology());
        let shared = base.into_shared();
        shared.store(Arc::new(tuned.clone()));
        assert_eq!(shared.load_full().to_json(), tuned.to_json());
    }

    #[test]
    fn model_backed_modes_warn_when_the_local_model_is_missing() {
        #[derive(Clone)]
        struct Capture(Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        for (name, configured, ready, should_warn) in [
            ("lexical", true, false, false),
            ("judge", true, false, true),
            ("judge", true, true, false),
            // `null` path: judge chose a BM25 fallback on purpose.
            ("judge", false, false, false),
            ("hybrid", true, false, true),
            ("hybrid", false, false, true),
            ("hybrid", true, true, false),
        ] {
            let output = Capture(Arc::new(std::sync::Mutex::new(Vec::new())));
            let writer = output.clone();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::WARN)
                .with_writer(move || writer.clone())
                .finish();
            let mode = serde_json::from_value(serde_json::json!(name)).unwrap();
            tracing::subscriber::with_default(subscriber, || {
                warn_if_search_mode_lacks_model(mode, configured, ready)
            });
            let message = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
            assert_eq!(
                message.contains("needs a local semantic model"),
                should_warn,
                "{name}, configured={configured}, model_ready={ready}: {message}"
            );
        }
    }

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: SkillsConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.function_search_mode, FunctionSearchMode::Judge);
        assert_eq!(
            cfg.function_search_model_path.as_deref(),
            Some("~/.cache/iii/all-MiniLM-L6-v2-c9745ed1d9f207416be6d2e6f8de32d1f16199bf")
        );
        assert!(cfg.function_search_model_download);
        let disabled = SkillsConfig::from_yaml("function_search_model_path: null\n").unwrap();
        assert_eq!(disabled.function_search_model_path, None);
        assert_eq!(cfg.skills_folder, default_skills_folder());
        assert_eq!(cfg.local_skills_folder, default_local_skills_folder());
        assert_eq!(cfg.agents_folder, default_agents_folder());
        assert_eq!(cfg.agents_skills_folder, default_agents_skills_folder());
        assert_eq!(cfg.registry_url, DEFAULT_REGISTRY_URL);
        assert_eq!(cfg.download_timeout_ms, 60_000);
        assert_eq!(cfg.registry_cache_ttl_ms, 60_000);
        assert!(cfg.filter_unregistered);
        assert!(cfg.auto_download);
    }

    #[test]
    fn function_search_modes_parse_and_invalid_values_fail() {
        for (name, expected) in [
            ("lexical", FunctionSearchMode::Lexical),
            ("hybrid", FunctionSearchMode::Hybrid),
            ("judge", FunctionSearchMode::Judge),
        ] {
            let cfg = SkillsConfig::from_yaml(&format!("function_search_mode: {name}\n")).unwrap();
            assert_eq!(cfg.function_search_mode, expected);
        }
        assert!(SkillsConfig::from_yaml("function_search_mode: remote\n").is_err());
        assert!(SkillsConfig::from_yaml("function_search_model_path: ''\n").is_err());
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: SkillsConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = SkillsConfig::default();
        assert_eq!(from_empty.skills_folder, from_default.skills_folder);
        assert_eq!(
            from_empty.local_skills_folder,
            from_default.local_skills_folder
        );
        assert_eq!(from_empty.agents_folder, from_default.agents_folder);
        assert_eq!(
            from_empty.agents_skills_folder,
            from_default.agents_skills_folder
        );
        assert_eq!(from_empty.registry_url, from_default.registry_url);
        assert_eq!(
            from_empty.download_timeout_ms,
            from_default.download_timeout_ms
        );
        assert_eq!(
            from_empty.registry_cache_ttl_ms,
            from_default.registry_cache_ttl_ms
        );
        assert_eq!(
            from_empty.filter_unregistered,
            from_default.filter_unregistered
        );
        assert_eq!(from_empty.auto_download, from_default.auto_download);
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = "\
skills_folder: ./my-skills
local_skills_folder: ./local-skills
agents_folder: ./profiles
agents_skills_folder: ./agents-skills
registry_url: https://example.com/registry/
download_timeout_ms: 30000
registry_cache_ttl_ms: 5000
filter_unregistered: false
auto_download: false
";
        let cfg: SkillsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.skills_folder, "./my-skills");
        assert_eq!(cfg.local_skills_folder, "./local-skills");
        assert_eq!(cfg.agents_folder, "./profiles");
        assert_eq!(cfg.agents_skills_folder, "./agents-skills");
        assert_eq!(cfg.registry_url, "https://example.com/registry/");
        assert_eq!(cfg.download_timeout_ms, 30_000);
        assert_eq!(cfg.registry_cache_ttl_ms, 5_000);
        assert_eq!(cfg.registry_base(), "https://example.com/registry");
        assert!(!cfg.filter_unregistered);
        assert!(!cfg.auto_download);
    }

    #[test]
    fn malformed_yaml_errors() {
        let err = load_config("/no/such/path/for/skills.yaml");
        assert!(err.is_err());
    }

    #[test]
    fn resolved_skills_folder_absolute_passes_through() {
        let cfg = SkillsConfig {
            skills_folder: "/tmp/foo".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(cfg.resolved_skills_folder(), PathBuf::from("/tmp/foo"));
    }

    #[test]
    fn resolved_skills_folder_relative_resolves_against_project() {
        let cfg = SkillsConfig {
            skills_folder: "./bar".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(
            cfg.resolved_skills_folder(),
            iii_worker_paths::project_path("bar")
        );
    }

    #[test]
    fn resolved_skills_folder_tilde_expands_home() {
        let cfg = SkillsConfig {
            skills_folder: "~/.iii/skills".into(),
            ..SkillsConfig::default()
        };
        // dirs::home_dir() must return Some on CI and dev machines.
        // If it doesn't, the warning fallback is exercised instead.
        if let Some(home) = dirs::home_dir() {
            assert_eq!(cfg.resolved_skills_folder(), home.join(".iii/skills"),);
        }
    }

    #[test]
    fn resolved_agents_skills_folder_tilde_expands_home() {
        let cfg = SkillsConfig {
            agents_skills_folder: "~/.agents/skills".into(),
            ..SkillsConfig::default()
        };
        if let Some(home) = dirs::home_dir() {
            assert_eq!(
                cfg.resolved_agents_skills_folder(),
                home.join(".agents/skills")
            );
        }
    }

    /// The default global agents root follows the user's HOME (the
    /// `~/.agents/skills` convention), never the compose/project dir, and
    /// the roots list keeps project-before-global precedence — deduped when
    /// both settings point at one directory.
    #[test]
    fn agents_skills_roots_default_project_then_home() {
        let cfg = SkillsConfig::default();
        let roots = cfg.resolved_agents_skills_roots();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0], cfg.resolved_agents_skills_folder());
        if let Some(home) = dirs::home_dir() {
            assert_eq!(roots[1], home.join(".agents/skills"));
        }

        let aligned = SkillsConfig {
            agents_skills_folder: "~/.agents/skills".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(aligned.resolved_agents_skills_roots().len(), 1);
    }

    /// The default global agent-profile root follows the user's HOME
    /// (`~/.iii/agents`), never the compose dir, and the roots list keeps
    /// project-before-global precedence — deduped when both settings point
    /// at one directory.
    #[test]
    fn agents_roots_default_project_then_home() {
        let cfg = SkillsConfig::default();
        let roots = cfg.resolved_agents_roots();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0], cfg.resolved_agents_folder());
        if let Some(home) = dirs::home_dir() {
            assert_eq!(roots[1], home.join(".iii/agents"));
        }

        let aligned = SkillsConfig {
            agents_folder: "~/.iii/agents".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(aligned.resolved_agents_roots().len(), 1);
    }

    #[test]
    fn resolved_agents_folder_relative_resolves_against_project() {
        let cfg = SkillsConfig {
            agents_folder: "./profiles".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(
            cfg.resolved_agents_folder(),
            iii_worker_paths::project_path("profiles")
        );
    }

    #[test]
    fn local_skills_folder_relative_resolves_against_project() {
        let cfg = SkillsConfig {
            local_skills_folder: "./.iii/skills".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(
            cfg.local_skills_folder(),
            iii_worker_paths::project_path(".iii/skills")
        );
    }

    #[test]
    fn registry_base_trims_trailing_slash() {
        let cfg = SkillsConfig {
            registry_url: "https://api.example/".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(cfg.registry_base(), "https://api.example");
    }

    #[test]
    fn json_schema_is_object_with_known_properties() {
        let schema = SkillsConfig::json_schema();
        let props = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap();
        for field in [
            "skills_folder",
            "local_skills_folder",
            "agents_folder",
            "agents_skills_folder",
            "registry_url",
            "download_timeout_ms",
            "registry_cache_ttl_ms",
            "filter_unregistered",
            "auto_download",
            "function_search_mode",
            "function_search_model_path",
            "function_search_model_download",
        ] {
            assert!(props.contains_key(field), "schema missing {field}");
        }
        assert!(schema.get("example").is_some());
    }

    #[test]
    fn to_json_from_json_roundtrip() {
        let cfg = SkillsConfig {
            skills_folder: "./my-skills".into(),
            registry_url: "https://example.com/registry".into(),
            download_timeout_ms: 1234,
            registry_cache_ttl_ms: 5678,
            filter_unregistered: false,
            auto_download: false,
            ..SkillsConfig::default()
        };
        let back = SkillsConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(back.skills_folder, cfg.skills_folder);
        assert_eq!(back.registry_url, cfg.registry_url);
        assert_eq!(back.download_timeout_ms, cfg.download_timeout_ms);
        assert_eq!(back.registry_cache_ttl_ms, cfg.registry_cache_ttl_ms);
        assert_eq!(back.filter_unregistered, cfg.filter_unregistered);
        assert_eq!(back.auto_download, cfg.auto_download);
    }

    #[test]
    fn from_yaml_matches_from_json_for_seed_shape() {
        let yaml = "skills_folder: ./s\nregistry_url: https://r\ndownload_timeout_ms: 10\n";
        let from_yaml = SkillsConfig::from_yaml(yaml).unwrap();
        let from_json = SkillsConfig::from_json(&from_yaml.to_json()).unwrap();
        assert_eq!(from_yaml.skills_folder, from_json.skills_folder);
        assert_eq!(from_yaml.registry_url, from_json.registry_url);
        assert_eq!(from_yaml.download_timeout_ms, from_json.download_timeout_ms);
    }

    #[test]
    fn topology_equal_when_only_tunables_differ() {
        let base = SkillsConfig::default();
        let tuned = SkillsConfig {
            registry_url: "https://other".into(),
            download_timeout_ms: 1,
            registry_cache_ttl_ms: 2,
            filter_unregistered: !base.filter_unregistered,
            function_search_mode: FunctionSearchMode::Hybrid,
            ..base.clone()
        };
        assert_eq!(base.topology(), tuned.topology());
    }

    #[test]
    fn topology_differs_when_a_topology_field_changes() {
        let base = SkillsConfig::default();
        let folder = SkillsConfig {
            skills_folder: "/other".into(),
            ..base.clone()
        };
        let local = SkillsConfig {
            local_skills_folder: "/other-local".into(),
            ..base.clone()
        };
        let agents = SkillsConfig {
            agents_skills_folder: "/other-agents".into(),
            ..base.clone()
        };
        let profiles = SkillsConfig {
            agents_folder: "/other-profiles".into(),
            ..base.clone()
        };
        let auto = SkillsConfig {
            auto_download: !base.auto_download,
            ..base.clone()
        };
        let model = SkillsConfig {
            function_search_model_path: Some("/models/minilm".into()),
            ..base.clone()
        };
        let download = SkillsConfig {
            function_search_model_download: !base.function_search_model_download,
            ..base.clone()
        };
        assert_ne!(base.topology(), download.topology());
        assert_ne!(base.topology(), folder.topology());
        assert_ne!(base.topology(), local.topology());
        assert_ne!(base.topology(), agents.topology());
        assert_ne!(base.topology(), profiles.topology());
        assert_ne!(base.topology(), auto.topology());
        assert_ne!(base.topology(), model.topology());
    }
}
