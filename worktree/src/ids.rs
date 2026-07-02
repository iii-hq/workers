//! Id minting and path derivation. The worktree id doubles as the git admin
//! directory name (`.git/worktrees/<wt_id>`), so it stays stable across
//! moves and repairs.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// 12 hex chars (48 bits) of a v4 UUID: long enough that minted ids do not
/// collide in practice, so a state `put` can never silently overwrite an
/// unrelated record (which an 8-hex, 32-bit id risks at a few tens of
/// thousands of worktrees).
fn short_hex() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

pub fn mint_worktree_id() -> String {
    format!("wt_{}", short_hex())
}

pub fn mint_job_id() -> String {
    format!("job_{}", short_hex())
}

/// Filesystem-safe, collision-resistant slug for one repository: the
/// directory name sanitized plus a stable hash of the canonical repo key.
pub fn repo_slug(repo_key: &str) -> String {
    let name = Path::new(repo_key)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("repo");
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    // DefaultHasher::new() uses fixed keys, so the slug is stable across runs.
    let mut hasher = std::hash::DefaultHasher::new();
    repo_key.hash(&mut hasher);
    format!("{sanitized}-{:06x}", hasher.finish() & 0xff_ffff)
}

/// `<worktree_root>/<repo_slug>/<wt_id>`.
pub fn worktree_dir(root: &Path, repo_key: &str, worktree_id: &str) -> PathBuf {
    root.join(repo_slug(repo_key)).join(worktree_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_ids_have_prefix_and_length() {
        let id = mint_worktree_id();
        assert!(id.starts_with("wt_"));
        assert_eq!(id.len(), 15);
        assert_ne!(mint_worktree_id(), mint_worktree_id());
    }

    #[test]
    fn repo_slug_is_stable_and_sanitized() {
        let a = repo_slug("/home/u/My Repo/.git");
        let b = repo_slug("/home/u/My Repo/.git");
        assert_eq!(a, b);
        assert!(a.starts_with("my-repo-"));
        let other = repo_slug("/home/u/other/.git");
        assert_ne!(a, other);
    }

    #[test]
    fn worktree_dir_nests_slug_and_id() {
        let dir = worktree_dir(Path::new("/root"), "/home/u/proj/.git", "wt_abc12345");
        let s = dir.to_string_lossy();
        assert!(s.starts_with("/root/proj-"));
        assert!(s.ends_with("/wt_abc12345"));
    }
}
