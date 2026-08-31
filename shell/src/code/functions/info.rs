//! `coder::info` — report the jail contract: allowed roots, caps,
//! response budgets, default noise excludes (`default_exclude_globs`),
//! and non-accessible glob patterns. No I/O; pure read from the runtime
//! `PathResolver` and `CoderConfig`. Call this first when unsure where
//! coder may read or write, or when a path was rejected.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::path::PathResolver;

/// No arguments — `coder::info` is a pure discovery call.
// examples are wire-contract; goldens pin them.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(example = "example_info_input")]
pub struct InfoInput {
    /// Internal harness filesystem scope; omitted from published schema.
    /// Echoed back as `session_root` so a caller whose path was rejected
    /// learns the anchor its relative paths ACTUALLY resolve against.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_info_input() -> serde_json::Value {
    serde_json::json!({})
}

/// Effective access mode of the coder surface — the same deny-only policy
/// switch `shell::fs::*` runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    /// Paths are confined to `base_paths`; anything outside is rejected.
    Jailed,
    /// Operator opt-in (`fs.allow_unjailed: true`, empty `fs.host_roots`):
    /// absolute paths anywhere on the host are accepted, confined only by
    /// `fs.denylist_paths` and `non_accessible_globs`. `base_paths` only
    /// anchor relative wire paths.
    Unjailed,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InfoOutput {
    /// `jailed`: paths must stay inside `base_paths`. `unjailed`: absolute
    /// paths anywhere on the host are accepted (deny-only, like
    /// `shell::fs::*`) and `base_paths` only anchor relative paths.
    pub mode: AccessMode,

    /// Canonical absolute paths of the allowed roots, in configuration order.
    /// The primary root (index 0) is where relative wire paths resolve; an
    /// absolute path is accepted when it canonicalises inside ANY of these
    /// (in `unjailed` mode: anywhere on the host). In `jailed` mode paths
    /// outside every root are rejected — use `shell::fs::*` instead.
    pub base_paths: Vec<String>,

    /// Convenience duplicate of `base_paths[0]` — the primary allowed root.
    /// Relative paths resolve against this directory UNLESS `session_root`
    /// is set, which takes precedence.
    pub primary_root: String,

    /// The session's working directory when the caller runs under a
    /// harness-stamped filesystem scope; `null` otherwise. When set, THIS
    /// is what relative wire paths (and `coder::search` glob matching)
    /// anchor against — not `primary_root`. In `unjailed` mode it may sit
    /// outside every `base_paths` entry, so a relative path that looks
    /// wrong against the allowed roots can still be correct here. Check it
    /// first when a relative path was rejected.
    pub session_root: Option<String>,

    /// Glob patterns matched per root (root-relative). Files whose
    /// root-relative path matches are listable but not
    /// readable/writable/deletable/creatable; they return C211.
    pub non_accessible_globs: Vec<String>,

    /// Noise-exclusion globs (root-relative, same matching as
    /// `non_accessible_globs`): matching paths (node_modules, .git, …)
    /// are omitted from `coder::search` results and pruned from
    /// `coder::tree` descent — the directory surfaces as a childless
    /// `truncated` stub. Hide-only — no access protection. Pass
    /// `use_default_excludes: false` on those calls to look inside.
    pub default_exclude_globs: Vec<String>,

    /// Per-file IO ceiling for `coder::read-file`. Full reads of files
    /// larger than this are rejected with C218; windowed reads cap the
    /// returned window bytes instead, so larger files stay readable
    /// window by window. Also the ceiling for `coder::search` content
    /// scanning — larger files are silently skipped during search.
    pub max_read_bytes: u64,

    /// Maximum bytes that `coder::create-file` or `coder::update-file` will
    /// accept for a single file write. Larger writes are rejected with C218.
    pub max_write_bytes: u64,

    /// Default `max_depth` used by `coder::tree` when the caller omits it.
    pub tree_default_depth: u32,

    /// Maximum entries returned per folder node by `coder::tree`; folders
    /// that exceed this are flagged `truncated`.
    pub tree_per_folder_limit: u32,

    /// Default `page_size` used by `coder::list-folder` when the caller
    /// omits it.
    pub list_default_page_size: u32,

    /// Hard cap on `page_size` accepted by `coder::list-folder`.
    pub list_max_page_size: u32,

    /// Default `max_matches` used by `coder::search` when the caller omits
    /// it.
    pub search_default_max_matches: u32,

    /// Per-line byte cap in `coder::search`: matching considers at most
    /// this many bytes of each line, and matched/context lines are
    /// truncated to it.
    pub search_default_max_line_bytes: u32,

    /// Aggregate byte budget for one `coder::search` response, measured
    /// in payload bytes (paths + matched text + context lines). When the
    /// budget is hit the response sets `truncated: true` — refine the
    /// query or add `include_globs`.
    pub search_response_budget_bytes: u64,

    /// Aggregate budget across a single `paths[]` batch call to
    /// `coder::read-file`, measured in bytes of returned content (after
    /// UTF-8 sanitization — invalid bytes expand to U+FFFD before being
    /// counted, so the cap bounds what the caller actually receives).
    /// Entries are collected in request order; each entry may consume up
    /// to `min(remaining_budget, max_read_bytes)`. An entry reached with
    /// zero remaining budget receives a per-entry C218 naming this key,
    /// its value, and the bytes already consumed, with recovery guidance.
    /// Budget topology: batch reads are governed by this key; single-path
    /// full reads by `max_output_bytes`; windowed reads by `max_read_bytes`
    /// applied per returned window — `max_read_bytes` is also the per-file
    /// IO ceiling for all of them.
    pub batch_read_budget_bytes: u64,

    /// Context budget for single-path FULL reads in `coder::read-file`,
    /// in bytes of returned content. Full reads larger than this return
    /// C218 with the file's size/line count and window/stat recovery
    /// guidance; a per-call `max_output_bytes` override is available on
    /// `coder::read-file` (clamped to `max_read_bytes`).
    pub max_output_bytes: u64,

    /// Coder worker version (`CARGO_PKG_VERSION`).
    pub version: String,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: InfoInput,
) -> Result<InfoOutput, String> {
    // info canonicalizes the configured roots (fs), so keep it off the executor
    // for consistency with the other read handlers.
    tokio::task::spawn_blocking(move || Ok(inner(&resolver, &cfg, &req)))
        .await
        .map_err(|e| format!("info task join failed: {e}"))?
}

fn inner(resolver: &PathResolver, cfg: &CoderConfig, req: &InfoInput) -> InfoOutput {
    let base_paths: Vec<String> = resolver
        .roots()
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let primary_root = base_paths[0].clone();
    // Canonicalized through the same gate the path resolution uses, so a
    // scope this worker would reject is reported as absent rather than as a
    // usable anchor.
    let session_root = crate::fs::scope_anchor(req.fs_scope.as_ref())
        .and_then(|root| resolver.session_root(root))
        .map(|p| p.display().to_string());

    InfoOutput {
        mode: if resolver.unjailed() {
            AccessMode::Unjailed
        } else {
            AccessMode::Jailed
        },
        base_paths,
        primary_root,
        session_root,
        non_accessible_globs: cfg.non_accessible_globs.clone(),
        default_exclude_globs: cfg.default_exclude_globs.clone(),
        max_read_bytes: cfg.max_read_bytes,
        max_write_bytes: cfg.max_write_bytes,
        tree_default_depth: cfg.tree_default_depth,
        tree_per_folder_limit: cfg.tree_per_folder_limit,
        list_default_page_size: cfg.list_default_page_size,
        list_max_page_size: cfg.list_max_page_size,
        search_default_max_matches: cfg.search_default_max_matches,
        search_default_max_line_bytes: cfg.search_default_max_line_bytes,
        search_response_budget_bytes: cfg.search_response_budget_bytes,
        batch_read_budget_bytes: cfg.batch_read_budget_bytes,
        max_output_bytes: cfg.max_output_bytes,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn make_resolver_cfg(
        roots: Vec<PathBuf>,
        globs: Vec<&str>,
    ) -> (Arc<PathResolver>, Arc<CoderConfig>) {
        let cfg = Arc::new(CoderConfig {
            base_paths: roots,
            non_accessible_globs: globs.into_iter().map(String::from).collect(),
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (resolver, cfg)
    }

    /// `base_paths` in the output must be CANONICAL, not the raw configured
    /// form. On macOS /tmp is a symlink to /private/tmp — we verify by
    /// using a path form that differs from its canonical counterpart.
    #[test]
    fn canonical_not_raw() {
        let tmp = tempdir().unwrap();
        // Construct a non-canonical path form: append a "." segment.
        let non_canon = tmp.path().join(".");
        let (resolver, cfg) = make_resolver_cfg(vec![non_canon], vec![]);

        let out = inner(&resolver, &cfg, &InfoInput::default());

        let expected_canon = std::fs::canonicalize(tmp.path())
            .unwrap()
            .display()
            .to_string();
        assert_eq!(
            out.base_paths,
            vec![expected_canon.clone()],
            "base_paths must be canonical, not raw configured form"
        );
        assert_eq!(
            out.primary_root, expected_canon,
            "primary_root must equal base_paths[0]"
        );
    }

    /// All caps, globs, and version must be populated from config;
    /// primary_root must equal base_paths[0]; version must match
    /// CARGO_PKG_VERSION.
    #[test]
    fn field_completeness() {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string(), "**/*.pem".to_string()],
            default_exclude_globs: vec!["**/build/**".to_string()],
            max_read_bytes: 42,
            max_write_bytes: 43,
            tree_default_depth: 7,
            tree_per_folder_limit: 9,
            list_default_page_size: 11,
            list_max_page_size: 13,
            search_default_max_matches: 17,
            search_default_max_line_bytes: 19,
            search_response_budget_bytes: 29,
            batch_read_budget_bytes: 23,
            max_output_bytes: 31,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());

        let out = inner(&resolver, &cfg, &InfoInput::default());

        // primary_root == base_paths[0]
        assert!(!out.base_paths.is_empty());
        assert_eq!(out.primary_root, out.base_paths[0]);
        assert_eq!(out.mode, AccessMode::Jailed);

        // globs
        assert_eq!(
            out.non_accessible_globs,
            vec!["**/.env".to_string(), "**/*.pem".to_string()]
        );
        assert_eq!(out.default_exclude_globs, vec!["**/build/**".to_string()]);

        // caps
        assert_eq!(out.max_read_bytes, 42);
        assert_eq!(out.max_write_bytes, 43);
        assert_eq!(out.tree_default_depth, 7);
        assert_eq!(out.tree_per_folder_limit, 9);
        assert_eq!(out.list_default_page_size, 11);
        assert_eq!(out.list_max_page_size, 13);
        assert_eq!(out.search_default_max_matches, 17);
        assert_eq!(out.search_default_max_line_bytes, 19);
        assert_eq!(out.search_response_budget_bytes, 29);
        assert_eq!(out.batch_read_budget_bytes, 23);
        assert_eq!(out.max_output_bytes, 31);

        // version
        assert_eq!(out.version, env!("CARGO_PKG_VERSION"));
    }

    /// The harness-stamped session scope is what relative paths anchor
    /// against, and in unjailed mode it can sit outside every allowed root.
    /// Reporting only `primary_root` sends a caller whose relative path was
    /// rejected hunting in the wrong tree — the whole point of this call.
    #[test]
    fn reports_session_root_when_scoped() {
        let roots = tempdir().unwrap();
        let scope = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![roots.path().to_path_buf()],
            unjailed: true,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        let req = InfoInput {
            fs_scope: Some(crate::fs::FsScope {
                root: scope.path().display().to_string(),
                grants: vec![],
                boundary: crate::fs::FsBoundary::ConfiguredRoots,
            }),
        };

        let out = inner(&resolver, &cfg, &req);

        let expected = std::fs::canonicalize(scope.path())
            .unwrap()
            .display()
            .to_string();
        assert_eq!(out.session_root.as_deref(), Some(expected.as_str()));
        assert_ne!(
            out.session_root.as_deref(),
            Some(out.primary_root.as_str()),
            "the scope this test pins must differ from the configured anchor"
        );

        // Unscoped callers keep the old contract: null, anchor is primary_root.
        let unscoped = inner(&resolver, &cfg, &InfoInput::default());
        assert_eq!(unscoped.session_root, None);
    }
}
