// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Reads the operator's `rootfs.mode` from the project config file
//! (`config.yaml` by default).

pub(crate) const CONFIG_FILE: &str = "config.yaml";

/// Project config file this process reads and edits.
///
/// Resolution order:
/// 1. `III_CONFIG_PATH` — set by the engine on every process it spawns
///    (worker-ops daemon, sandbox daemon, `iii worker start` children) so
///    they target the engine's ACTUAL config file. Without it, an engine
///    started with `--config config.yml` (or from a different directory)
///    and this code silently operate on two different files.
/// 2. `./config.yaml` — the historical default for direct CLI use.
pub(crate) fn config_path() -> std::path::PathBuf {
    match std::env::var_os("III_CONFIG_PATH") {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => std::path::PathBuf::from(CONFIG_FILE),
    }
}

/// Operator-selected rootfs mode from config.yaml's top-level `rootfs.mode`
/// (`overlay` | `legacy` / `off`), or `None` when unset/unreadable.
///
/// The engine spawns `iii-worker` without forwarding arbitrary env, so
/// config.yaml — read from disk in the start process — is the reliable way for
/// an operator setting to reach the boot path. The `III_ROOTFS_MODE` env var
/// still takes precedence; see [`crate::cli::overlay::overlay_enabled`].
///
/// Note: only the MODE is configurable. The overlay upper is sized by the
/// embedded golden image (ext4 can't grow without `resize2fs`), so there is no
/// `upper_size_gib` knob — changing the size means regenerating the golden via
/// `vendor/regen-golden.sh`.
pub fn rootfs_mode() -> Option<String> {
    let content = std::fs::read_to_string(config_path()).ok()?;
    rootfs_mode_from(&content)
}

/// Pure extraction used by [`rootfs_mode`], exposed for unit tests so the YAML
/// parsing doesn't need filesystem I/O.
pub(crate) fn rootfs_mode_from(content: &str) -> Option<String> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(content).ok()?;
    yaml.get("rootfs")
        .and_then(|r| r.get("mode"))
        .and_then(|m| m.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rootfs_mode_from_reads_top_level_mode() {
        let yaml = "rootfs:\n  mode: legacy\n";
        assert_eq!(rootfs_mode_from(yaml).as_deref(), Some("legacy"));
        let yaml2 = "rootfs:\n  mode: overlay\n";
        assert_eq!(rootfs_mode_from(yaml2).as_deref(), Some("overlay"));
    }

    #[test]
    fn rootfs_mode_from_none_when_absent_or_malformed() {
        assert_eq!(rootfs_mode_from("workers: []\n"), None);
        assert_eq!(rootfs_mode_from("rootfs:\n  other: x\n"), None);
        assert_eq!(rootfs_mode_from("not: valid: yaml: :"), None);
        // Empty/whitespace value is treated as unset.
        assert_eq!(rootfs_mode_from("rootfs:\n  mode: '   '\n"), None);
    }

    /// Scoped env-var setter that restores the prior value on drop so a
    /// failing test can't leak state into siblings.
    struct EnvGuard {
        key: String,
        prior: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            // SAFETY: tests holding TEST_HOME_LOCK (lock_home) serialize all env mutation.
            unsafe { std::env::set_var(key, value) };
            Self {
                key: key.to_string(),
                prior,
            }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(v) => unsafe { std::env::set_var(&self.key, v) },
                None => unsafe { std::env::remove_var(&self.key) },
            }
        }
    }

    // ──────────────────────────────────────────────────────────────────
    // III_CONFIG_PATH — the engine exports it to spawned processes so this
    // module targets the engine's actual config file, not ./config.yaml.
    // These tests hold the crate-wide TEST_HOME_LOCK (`lock_home()`), which
    // serializes every test in this crate that touches env vars, HOME or cwd,
    // because the env override redirects every reader in the process.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn config_path_ignores_empty_env_override() {
        let _home = crate::cli::test_support::lock_home();
        let _g = EnvGuard::set("III_CONFIG_PATH", "");
        assert_eq!(config_path(), std::path::PathBuf::from("config.yaml"));
    }
}
