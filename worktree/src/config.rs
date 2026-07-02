//! Worker configuration (Path B: the `configuration` worker is the
//! authoritative store; no committed config.yaml). Defaults live in
//! `WorkerConfig::default()` and every field hot-reloads: tuning knobs are
//! read from the live snapshot per call, and the structural half
//! (`boot_signature`) re-binds the prune cron trigger.

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Root directory managed worktrees are created under. A leading `~/`
    /// expands to the home directory. Must be inside the shell worker's
    /// `fs.host_roots` for the land test gate to work.
    #[serde(default = "default_worktree_root")]
    pub worktree_root: String,
    /// Prefix for auto-minted branches (`<prefix><worktree_id>`).
    #[serde(default = "default_branch_prefix")]
    pub branch_prefix: String,
    /// Default branch naming: `id` uses the worktree id; `codename` mints a
    /// deterministic friendly `<adjective>-<noun>-<4hex>` name from it.
    #[serde(default)]
    pub branch_naming: BranchNaming,
    /// Six-field cron schedule for the automatic prune sweep.
    #[serde(default = "default_prune_schedule")]
    pub prune_schedule: String,
    /// Idle hours before an unclaimed, clean worktree becomes prunable.
    #[serde(default = "default_prune_expire_hours")]
    pub prune_expire_hours: u64,
    /// Engine queue name land jobs are enqueued to.
    #[serde(default = "default_land_queue")]
    pub land_queue: String,
    /// Rebase/merge retries when the target branch moves during a land.
    #[serde(default = "default_max_land_retries")]
    pub max_land_retries: u32,
    /// Per-git-subprocess timeout, milliseconds.
    #[serde(default = "default_git_timeout_ms")]
    pub git_timeout_ms: u64,
    /// Land test gate timeout, milliseconds.
    #[serde(default = "default_test_timeout_ms")]
    pub test_timeout_ms: u64,
    /// Operation gates: deny-by-config for destructive surfaces. Every gate
    /// hot-reloads; denials name the exact key to flip.
    #[serde(default)]
    pub gates: GatesConfig,
    /// Best-effort provisioning of gitignored files into fresh worktrees.
    #[serde(default)]
    pub provision: ProvisionConfig,
}

/// Copy-ignored provisioning: replicate gitignored files (.env, caches)
/// from the source repository into each new worktree, in the background,
/// after `worktree::create` returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProvisionConfig {
    /// Enable the background copy of gitignored files on create.
    #[serde(default)]
    pub copy_ignored: bool,
    /// Globs selecting which ignored paths to copy (empty = all ignored).
    #[serde(default)]
    pub include: Vec<String>,
    /// Globs excluding ignored paths from the copy.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Total copy budget in bytes; entries beyond it are skipped with a
    /// warning.
    #[serde(default = "default_max_copy_bytes")]
    pub max_copy_bytes: u64,
}

fn default_max_copy_bytes() -> u64 {
    2 * 1024 * 1024 * 1024
}

impl Default for ProvisionConfig {
    fn default() -> Self {
        Self {
            copy_ignored: false,
            include: Vec::new(),
            exclude: Vec::new(),
            max_copy_bytes: default_max_copy_bytes(),
        }
    }
}

/// How default branch names are minted at create time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BranchNaming {
    /// `<branch_prefix><worktree_id>`.
    #[default]
    Id,
    /// `<branch_prefix><adjective>-<noun>-<4hex>`, deterministic per id.
    Codename,
}

/// Deny-by-config gates over the mutating surface. Checked FIRST by every
/// mutating handler; a denial returns a `W5xx` error naming the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GatesConfig {
    /// Allow `worktree::remove`.
    #[serde(default = "default_true")]
    pub allow_remove: bool,
    /// Allow the force paths: remove force, claim force takeover, release
    /// force, and land force_restart. Disabled out of the box.
    #[serde(default)]
    pub allow_force: bool,
    /// Allow branch deletion (remove delete_branch and the land finalize
    /// branch cleanup).
    #[serde(default = "default_true")]
    pub allow_branch_delete: bool,
    /// Allow `worktree::land`.
    #[serde(default = "default_true")]
    pub allow_land: bool,
    /// Allow `worktree::prune` (including the cron sweep).
    #[serde(default = "default_true")]
    pub allow_prune: bool,
    /// Branches `worktree::land` may target. Glob list (`*` matches any
    /// characters); pin to `["main"]` to allow only mainline lands.
    #[serde(default = "default_land_targets")]
    pub land_targets: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_land_targets() -> Vec<String> {
    vec!["*".to_string()]
}

impl Default for GatesConfig {
    fn default() -> Self {
        Self {
            allow_remove: true,
            allow_force: false,
            allow_branch_delete: true,
            allow_land: true,
            allow_prune: true,
            land_targets: default_land_targets(),
        }
    }
}

impl GatesConfig {
    /// True when `target` matches any of the `land_targets` globs.
    pub fn land_target_allowed(&self, target: &str) -> bool {
        self.land_targets.iter().any(|g| glob_match(g, target))
    }
}

/// Minimal glob: `*` matches any run of characters (including `/`); every
/// other character matches literally.
pub fn glob_match(pattern: &str, value: &str) -> bool {
    fn inner(p: &[u8], v: &[u8]) -> bool {
        match (p.first(), v.first()) {
            (None, None) => true,
            (Some(b'*'), _) => inner(&p[1..], v) || (!v.is_empty() && inner(p, &v[1..])),
            (Some(pc), Some(vc)) if pc == vc => inner(&p[1..], &v[1..]),
            _ => false,
        }
    }
    inner(pattern.as_bytes(), value.as_bytes())
}

fn default_worktree_root() -> String {
    "~/.iii/worktrees".to_string()
}

fn default_branch_prefix() -> String {
    "iii/".to_string()
}

fn default_prune_schedule() -> String {
    "0 0 * * * *".to_string()
}

fn default_prune_expire_hours() -> u64 {
    72
}

fn default_land_queue() -> String {
    "worktree-land".to_string()
}

fn default_max_land_retries() -> u32 {
    3
}

fn default_git_timeout_ms() -> u64 {
    60_000
}

fn default_test_timeout_ms() -> u64 {
    600_000
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worktree_root: default_worktree_root(),
            branch_prefix: default_branch_prefix(),
            branch_naming: BranchNaming::default(),
            prune_schedule: default_prune_schedule(),
            prune_expire_hours: default_prune_expire_hours(),
            land_queue: default_land_queue(),
            max_land_retries: default_max_land_retries(),
            git_timeout_ms: default_git_timeout_ms(),
            test_timeout_ms: default_test_timeout_ms(),
            gates: GatesConfig::default(),
            provision: ProvisionConfig::default(),
        }
    }
}

/// Comparison key for the structural half of the config. A changed
/// signature re-binds the prune cron trigger; everything else is a
/// tuning-only snapshot swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootSignature {
    pub prune_schedule: String,
}

impl WorkerConfig {
    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            prune_schedule: self.prune_schedule.clone(),
        }
    }

    /// `worktree_root` with a leading `~/` expanded.
    pub fn expanded_worktree_root(&self) -> PathBuf {
        expand_home(&self.worktree_root)
    }

    /// Schema registered with the `configuration` worker; shipped defaults
    /// attached as the top-level `example`.
    pub fn json_schema() -> Value {
        let mut schema =
            serde_json::to_value(schemars::schema_for!(WorkerConfig)).unwrap_or_default();
        schema["example"] = WorkerConfig::default().to_json();
        schema
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Parse a value already env-expanded by the configuration worker.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid worktree config: {e}"))
    }

    /// Parse the optional `--config` seed file: env-expand `${VAR}` /
    /// `${VAR:default}` against the process env, then parse YAML.
    pub fn from_yaml(contents: &str) -> Result<Self> {
        let expanded = expand_env(contents);
        let cfg: WorkerConfig = serde_yaml::from_str(&expanded)?;
        Ok(cfg)
    }

    pub fn from_file(path: &str) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_yaml(&contents)
    }
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Expand `${VAR}` and `${VAR:default}` against the live process env.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let inner = &after[..end];
                let (name, default) = match inner.split_once(':') {
                    Some((n, d)) => (n, Some(d)),
                    None => (inner, None),
                };
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => out.push_str(default.unwrap_or("")),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg, WorkerConfig::default());
        assert_eq!(cfg.land_queue, "worktree-land");
        assert_eq!(cfg.prune_expire_hours, 72);
    }

    #[test]
    fn gates_default_open_except_force() {
        let gates = WorkerConfig::default().gates;
        assert!(gates.allow_remove);
        assert!(
            !gates.allow_force,
            "force paths are disabled out of the box"
        );
        assert!(gates.allow_branch_delete);
        assert!(gates.allow_land);
        assert!(gates.allow_prune);
        assert_eq!(gates.land_targets, vec!["*".to_string()]);
        assert!(gates.land_target_allowed("anything"));
    }

    #[test]
    fn gates_yaml_overrides() {
        let cfg = WorkerConfig::from_yaml(
            "gates:\n\
             \x20 allow_remove: false\n\
             \x20 allow_force: true\n\
             \x20 land_targets: [\"main\", \"release/*\"]\n",
        )
        .unwrap();
        assert!(!cfg.gates.allow_remove);
        assert!(cfg.gates.allow_force);
        assert!(cfg.gates.allow_land, "unset keys keep their defaults");
        assert!(cfg.gates.land_target_allowed("main"));
        assert!(cfg.gates.land_target_allowed("release/1.2"));
        assert!(!cfg.gates.land_target_allowed("trunk"));
    }

    #[test]
    fn glob_match_star_and_literals() {
        assert!(glob_match("*", "anything/at/all"));
        assert!(glob_match("main", "main"));
        assert!(!glob_match("main", "main2"));
        assert!(glob_match("release/*", "release/1.2"));
        assert!(!glob_match("release/*", "release"));
        assert!(glob_match("*-wip", "feature-wip"));
        assert!(!glob_match("", "x"));
        assert!(glob_match("", ""));
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let cfg = WorkerConfig::from_yaml(
            "worktree_root: /tmp/wts\n\
             branch_prefix: agent/\n\
             prune_schedule: \"0 30 * * * *\"\n\
             prune_expire_hours: 4\n\
             land_queue: my-queue\n\
             max_land_retries: 1\n\
             git_timeout_ms: 1000\n\
             test_timeout_ms: 2000\n",
        )
        .unwrap();
        assert_eq!(cfg.worktree_root, "/tmp/wts");
        assert_eq!(cfg.branch_prefix, "agent/");
        assert_eq!(cfg.prune_schedule, "0 30 * * * *");
        assert_eq!(cfg.prune_expire_hours, 4);
        assert_eq!(cfg.land_queue, "my-queue");
        assert_eq!(cfg.max_land_retries, 1);
        assert_eq!(cfg.git_timeout_ms, 1000);
        assert_eq!(cfg.test_timeout_ms, 2000);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(WorkerConfig::from_yaml("bogus: 1").is_err());
    }

    #[test]
    fn json_round_trip_and_schema_example() {
        let cfg = WorkerConfig::default();
        let back = WorkerConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(back, cfg);
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["example"], cfg.to_json());
        assert!(schema["properties"].is_object());
    }

    #[test]
    fn boot_signature_tracks_structural_fields_only() {
        let base = WorkerConfig::default();
        let mut tuned = base.clone();
        tuned.git_timeout_ms = 5;
        tuned.land_queue = "other".into();
        assert_eq!(base.boot_signature(), tuned.boot_signature());
        let mut structural = base.clone();
        structural.prune_schedule = "0 15 * * * *".into();
        assert_ne!(base.boot_signature(), structural.boot_signature());
    }

    #[test]
    fn env_expansion_supports_defaults() {
        std::env::set_var("WT_TEST_ROOT", "/data/w");
        assert_eq!(expand_env("root: ${WT_TEST_ROOT}"), "root: /data/w");
        assert_eq!(
            expand_env("root: ${WT_TEST_MISSING:/fallback}"),
            "root: /fallback"
        );
        std::env::remove_var("WT_TEST_ROOT");
    }

    #[test]
    fn expand_home_handles_tilde() {
        let cfg = WorkerConfig::default();
        let root = cfg.expanded_worktree_root();
        assert!(!root.to_string_lossy().starts_with("~"));
    }
}
