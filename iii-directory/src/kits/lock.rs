//! `kits.lock` — what kits are installed in this project, at which version,
//! and the content each installed file had when the kit wrote it.
//!
//! ```yaml
//! version: 1
//! kits:
//!   acme/kanban-team:
//!     requested: latest
//!     version: 1.3.0
//!     installed_at: 2026-10-08T14:02:11Z
//!     workers:
//!       kanban: ^1.4
//!     files:
//!       agents/product-manager.md:
//!         sha256: 4be1…
//!       agents/reviewer.md:
//!         sha256: 77c0…
//!         skipped: true
//!     ignored_versions: [1.4.0]
//! ```
//!
//! Each file's `sha256` is the kit's content for that path at `version` —
//! the base of a three-way merge on the next update, fetched back from the
//! registry (`GET /blobs/<sha>`), so no copies are kept locally. A file's
//! state is computed on demand: intact (local sha = base), edited, or
//! missing. `skipped: true` records that the user kept the file that was
//! already there (or that another kit took the path over); the kit never
//! writes it until that choice is reverted.

use std::collections::BTreeMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const LOCK_VERSION: u32 = 1;

const HEADER: &str = "# kits.lock — kits installed in this project by iii-directory.\n\
# Commit this file. Edit kits with `iii trigger directory::download-kit` or the\n\
# ADE (Directory → Kits), not by hand.\n";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KitsLock {
    pub version: u32,
    #[serde(default)]
    pub kits: BTreeMap<String, LockedKit>,
}

impl Default for KitsLock {
    fn default() -> Self {
        Self {
            version: LOCK_VERSION,
            kits: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LockedKit {
    /// What was asked for: a release tag, an exact version or a range.
    pub requested: String,
    /// The installed version.
    pub version: String,
    /// RFC 3339 time of the install or last update.
    pub installed_at: String,
    /// Worker ranges the installed version declares.
    #[serde(default)]
    pub workers: BTreeMap<String, String>,
    /// Install path → base content hash.
    #[serde(default)]
    pub files: BTreeMap<String, LockedFile>,
    /// Versions the user chose to ignore in update checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored_versions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LockedFile {
    pub sha256: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

impl KitsLock {
    /// Read the lock; a missing file is an empty lock.
    pub fn read(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    /// Read the lock, treating an unreadable or malformed one as empty
    /// (logged). For read paths that must never fail over the lock.
    pub fn read_lenient(path: &Path) -> Self {
        Self::read(path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "kits.lock unreadable; treating as empty");
            Self::default()
        })
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let lock: Self =
            serde_yaml::from_str(text).map_err(|e| format!("invalid kits.lock: {e}"))?;
        if lock.version != LOCK_VERSION {
            return Err(format!(
                "kits.lock version {} is not supported (expected {LOCK_VERSION})",
                lock.version
            ));
        }
        Ok(lock)
    }

    pub fn render(&self) -> Result<String, String> {
        let body = serde_yaml::to_string(self).map_err(|e| format!("encode kits.lock: {e}"))?;
        Ok(format!("{HEADER}{body}"))
    }

    /// Write atomically. An empty lock (no kits) removes the file.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        if self.kits.is_empty() {
            return match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("remove {}: {e}", path.display())),
            };
        }
        crate::sources::write_file_atomic(path, self.render()?.as_bytes())
    }

    /// Two-segment skill namespaces (`<handle>/<kit>`) of installed kits.
    pub fn namespaces(&self) -> Vec<String> {
        self.kits.keys().cloned().collect()
    }

    /// Which installed kit owns an install path (not skipped).
    pub fn owner_of(&self, path: &str) -> Option<(&str, &LockedKit)> {
        self.kits.iter().find_map(|(name, kit)| {
            kit.files
                .get(path)
                .filter(|f| !f.skipped)
                .map(|_| (name.as_str(), kit))
        })
    }

    /// Agent ids owned by kits (`agents/<id>.md`, not skipped) → kit name.
    pub fn owned_agents(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (name, kit) in &self.kits {
            for (path, file) in &kit.files {
                if file.skipped {
                    continue;
                }
                if let Some(id) = super::paths::agent_id_for_path(path) {
                    out.insert(id, name.clone());
                }
            }
        }
        out
    }
}

/// Lower-case hex sha256 of a file body.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Local state of an installed file relative to its lock entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Intact,
    Edited,
    Missing,
    /// The lock records the user's choice to keep someone else's file.
    Skipped,
}

/// sha256 of the file on disk, `None` when it does not exist.
pub fn local_sha(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| sha256_hex(&b))
}

pub fn file_state(locked: &LockedFile, local: Option<&str>) -> FileState {
    if locked.skipped {
        return FileState::Skipped;
    }
    match local {
        None => FileState::Missing,
        Some(sha) if sha == locked.sha256 => FileState::Intact,
        Some(_) => FileState::Edited,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> KitsLock {
        let mut files = BTreeMap::new();
        files.insert(
            "agents/product-manager.md".to_string(),
            LockedFile {
                sha256: "4be1".into(),
                skipped: false,
            },
        );
        files.insert(
            "agents/reviewer.md".to_string(),
            LockedFile {
                sha256: "77c0".into(),
                skipped: true,
            },
        );
        files.insert(
            "skills/acme/kanban-team/tickets/flow.md".to_string(),
            LockedFile {
                sha256: "9a3e".into(),
                skipped: false,
            },
        );
        let mut kits = BTreeMap::new();
        kits.insert(
            "acme/kanban-team".to_string(),
            LockedKit {
                requested: "latest".into(),
                version: "1.3.0".into(),
                installed_at: "2026-10-08T14:02:11Z".into(),
                workers: BTreeMap::from([("kanban".to_string(), "^1.4".to_string())]),
                files,
                ignored_versions: vec!["1.4.0".into()],
            },
        );
        KitsLock { version: 1, kits }
    }

    #[test]
    fn lock_round_trips_through_yaml_with_the_plan_shape() {
        let lock = sample();
        let text = lock.render().unwrap();
        assert!(text.starts_with("# kits.lock"));
        assert!(text.contains("acme/kanban-team:"));
        assert!(text.contains("skipped: true"));
        assert!(text.contains("ignored_versions:"));
        // Intact files carry no `skipped` key.
        assert_eq!(text.matches("skipped").count(), 1);
        assert_eq!(KitsLock::parse(&text).unwrap(), lock);
    }

    #[test]
    fn missing_lock_reads_as_empty_and_empty_lock_removes_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("kits.lock");
        assert_eq!(KitsLock::read(&path).unwrap(), KitsLock::default());
        sample().write(&path).unwrap();
        assert!(path.is_file());
        KitsLock::default().write(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn unsupported_version_is_an_error() {
        let err = KitsLock::parse("version: 9\nkits: {}\n").unwrap_err();
        assert!(err.contains("not supported"), "{err}");
    }

    #[test]
    fn owned_agents_skip_skipped_entries() {
        let lock = sample();
        let owned = lock.owned_agents();
        assert_eq!(
            owned.get("product-manager").map(String::as_str),
            Some("acme/kanban-team")
        );
        assert!(!owned.contains_key("reviewer"));
        assert!(lock.owner_of("agents/reviewer.md").is_none());
    }

    #[test]
    fn file_state_classifies_against_the_base() {
        let f = LockedFile {
            sha256: "abc".into(),
            skipped: false,
        };
        assert_eq!(file_state(&f, Some("abc")), FileState::Intact);
        assert_eq!(file_state(&f, Some("def")), FileState::Edited);
        assert_eq!(file_state(&f, None), FileState::Missing);
        let s = LockedFile {
            sha256: "abc".into(),
            skipped: true,
        };
        assert_eq!(file_state(&s, Some("zzz")), FileState::Skipped);
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
