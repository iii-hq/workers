use std::fs;
use std::path::{Path, PathBuf};

/// Current branch of the repo at `repo_root`, read straight from `.git/HEAD`
/// rather than a `git` subprocess, so the TUI can refresh it on the poll
/// cadence for pennies. Returns the branch name, `@<short-hash>` for a
/// detached HEAD, or `None` when `repo_root` isn't a git checkout (or its
/// HEAD is unreadable/garbled) — callers just omit the display.
pub fn current_branch(repo_root: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir(repo_root)?.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
        return (!branch.is_empty()).then(|| branch.to_string());
    }
    // Symbolic ref outside refs/heads (rare; e.g. mid-bisect states): show the
    // ref path itself rather than pretending it's a branch.
    if let Some(other) = head.strip_prefix("ref: ") {
        return (!other.is_empty()).then(|| other.to_string());
    }
    // Detached HEAD: a bare commit hash.
    if head.len() >= 40 && head.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(format!("@{}", &head[..8]));
    }
    None
}

/// Resolve the actual git dir for `repo_root`. `.git` is a directory in a
/// normal checkout, but a `gitdir: <path>` pointer *file* in linked worktrees
/// and submodules — the very setups where telling instances apart matters.
fn git_dir(repo_root: &Path) -> Option<PathBuf> {
    let dot_git = repo_root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let pointer = fs::read_to_string(&dot_git).ok()?;
    let target = pointer.trim().strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    let path = PathBuf::from(target);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(repo_root.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn branch_from_normal_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".git/HEAD"), "ref: refs/heads/main\n");
        assert_eq!(current_branch(tmp.path()).as_deref(), Some("main"));
    }

    #[test]
    fn branch_with_slashes() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join(".git/HEAD"),
            "ref: refs/heads/feat/workers-dev-improvement\n",
        );
        assert_eq!(
            current_branch(tmp.path()).as_deref(),
            Some("feat/workers-dev-improvement")
        );
    }

    #[test]
    fn worktree_gitdir_pointer_absolute() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("main-clone/.git/worktrees/wt");
        write(&real.join("HEAD"), "ref: refs/heads/feature-x\n");
        let wt = tmp.path().join("wt");
        write(&wt.join(".git"), &format!("gitdir: {}\n", real.display()));
        assert_eq!(current_branch(&wt).as_deref(), Some("feature-x"));
    }

    #[test]
    fn worktree_gitdir_pointer_relative() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join("real/HEAD"),
            "ref: refs/heads/relative-branch\n",
        );
        let wt = tmp.path().join("wt");
        write(&wt.join(".git"), "gitdir: ../real\n");
        assert_eq!(current_branch(&wt).as_deref(), Some("relative-branch"));
    }

    #[test]
    fn detached_head_shows_short_hash() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join(".git/HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        );
        assert_eq!(current_branch(tmp.path()).as_deref(), Some("@01234567"));
    }

    #[test]
    fn non_repo_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(current_branch(tmp.path()), None);
    }

    #[test]
    fn garbage_head_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".git/HEAD"), "not a ref at all");
        assert_eq!(current_branch(tmp.path()), None);
    }

    #[test]
    fn empty_gitdir_pointer_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".git"), "gitdir: \n");
        assert_eq!(current_branch(tmp.path()), None);
    }
}
