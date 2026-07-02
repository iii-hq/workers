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

fn stable_hash(value: &str) -> u64 {
    // DefaultHasher::new() uses fixed keys, so results are stable across
    // runs and hosts.
    let mut hasher = std::hash::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Advisory dev-server port for a worktree: `10000 + hash(id) % 10000`.
/// Deterministic per id, collision-improbable across a handful of parallel
/// worktrees; nothing reserves the port.
pub fn dev_port(worktree_id: &str) -> u16 {
    10_000 + (stable_hash(worktree_id) % 10_000) as u16
}

const CODENAME_ADJECTIVES: [&str; 48] = [
    "amber", "bold", "brisk", "calm", "cedar", "civic", "clear", "cobalt", "copper", "coral",
    "crisp", "dapper", "deft", "dusky", "eager", "early", "ember", "fabled", "fleet", "gentle",
    "gilded", "hardy", "hazel", "keen", "limber", "lively", "lucid", "mellow", "misty", "nimble",
    "noble", "opal", "placid", "plucky", "prime", "quiet", "rapid", "rustic", "sable", "sage",
    "solid", "sunny", "swift", "tidal", "umber", "vivid", "wry", "zesty",
];

const CODENAME_NOUNS: [&str; 48] = [
    "anchor", "aspen", "badger", "basin", "beacon", "birch", "bison", "bluff", "brook", "canyon",
    "cliff", "comet", "crane", "creek", "delta", "dune", "falcon", "fern", "fjord", "gale",
    "glade", "grove", "harbor", "heron", "inlet", "jetty", "knoll", "lagoon", "larch", "lark",
    "ledge", "lynx", "marsh", "mesa", "otter", "pine", "plateau", "prairie", "quarry", "raven",
    "reef", "ridge", "shoal", "spruce", "summit", "tarn", "thicket", "wren",
];

/// Deterministic two-word friendly branch name derived from the worktree
/// id: `<prefix><adjective>-<noun>-<4hex>`, where the hex tail is the id's
/// own hash prefix (the collision guard).
pub fn codename_branch(branch_prefix: &str, worktree_id: &str) -> String {
    let h = stable_hash(worktree_id);
    let adjective = CODENAME_ADJECTIVES[(h % CODENAME_ADJECTIVES.len() as u64) as usize];
    let noun = CODENAME_NOUNS[((h >> 8) % CODENAME_NOUNS.len() as u64) as usize];
    let hex = worktree_id
        .strip_prefix("wt_")
        .unwrap_or(worktree_id)
        .chars()
        .take(4)
        .collect::<String>();
    format!("{branch_prefix}{adjective}-{noun}-{hex}")
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

    #[test]
    fn dev_port_is_deterministic_and_in_range() {
        let a = dev_port("wt_abc12345");
        let b = dev_port("wt_abc12345");
        assert_eq!(a, b);
        for id in ["wt_abc12345", "wt_00000000", "wt_ffffffff", "wt_1f2e3d4c"] {
            let port = dev_port(id);
            assert!((10_000..20_000).contains(&port), "{id} -> {port}");
        }
        assert_ne!(dev_port("wt_abc12345"), dev_port("wt_abc12346"));
    }

    #[test]
    fn codename_is_deterministic_with_id_hash_suffix() {
        let a = codename_branch("iii/", "wt_abc12345");
        let b = codename_branch("iii/", "wt_abc12345");
        assert_eq!(a, b);
        assert!(a.starts_with("iii/"));
        // <prefix><adjective>-<noun>-<4hex of the id>
        let tail = a.strip_prefix("iii/").unwrap();
        let parts: Vec<&str> = tail.split('-').collect();
        assert_eq!(parts.len(), 3, "{a}");
        assert_eq!(parts[2], "abc1", "collision suffix comes from the id");
        assert_ne!(
            codename_branch("iii/", "wt_abc12345"),
            codename_branch("iii/", "wt_deadbeef"),
        );
    }
}
