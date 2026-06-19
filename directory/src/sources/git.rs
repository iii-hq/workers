//! GitHub repo source: shells out to `git clone --depth 1 --quiet`,
//! then copies a single subfolder into `<skills_folder>/<namespace>/`.
//!
//! The skill name doubles as the destination namespace inside
//! `skills_folder`, so two parallel downloads of different skills from
//! the same repo never collide on disk.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use super::{validate_relative_path, write_file_atomic, DownloadResult};

/// Validate the user-supplied skill name. The name is used both as the
/// subfolder inside the cloned repo (`skills/<skill>/`) and as the
/// destination namespace in `skills_folder`. Same character class as
/// the worker name regex, plus `.` so popular naming conventions like
/// `frontend-design.v2` work — no path separators.
pub fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("skill name must be non-empty".into());
    }
    if name.contains('/') || name.contains('\\') {
        return Err(format!("skill name may not contain '/' or '\\': {name:?}"));
    }
    if name.contains("..") {
        return Err(format!("skill name may not contain '..': {name:?}"));
    }
    for c in name.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
        if !ok {
            return Err(format!(
                "skill name may only contain alphanumerics, '-', '_', '.': {name:?}"
            ));
        }
    }
    if name.starts_with('.') {
        return Err(format!("skill name may not start with '.': {name:?}"));
    }
    Ok(())
}

/// Validate that a repo URL is safe to pass to `git clone`. Rejects
/// empty strings, argument injection (`-`-prefixed), transport tricks
/// (`::`, `ext::`), insecure `file:` scheme, and anything that isn't
/// `https://`, `ssh://`, or `git@`-style SCP notation.
pub fn validate_repo_url(repo: &str) -> Result<(), String> {
    let repo = repo.trim();
    if repo.is_empty() {
        return Err("repo URL must be non-empty".into());
    }
    if repo.starts_with('-') {
        return Err(format!(
            "repo URL may not start with '-' (argument injection): {repo:?}"
        ));
    }
    if repo.contains("::") {
        return Err(format!(
            "repo URL may not contain '::' (transport trick): {repo:?}"
        ));
    }
    if repo.starts_with("file:") {
        return Err(format!("repo URL may not use 'file:' scheme: {repo:?}"));
    }
    // Only allow https://, ssh://, or git@ (SCP notation).
    let allowed =
        repo.starts_with("https://") || repo.starts_with("ssh://") || repo.starts_with("git@");
    if !allowed {
        return Err(format!(
            "repo URL must start with https://, ssh://, or git@ — got: {repo:?}"
        ));
    }
    Ok(())
}

/// Run `git clone --depth 1 --branch <branch> --quiet <repo> <tmpdir>`
/// then copy `<tmpdir>/skills/<skill>/**` into
/// `<skills_folder>/<skill>/`.
///
/// `timeout_ms` caps the entire operation (clone + copy). The tempdir
/// is cleaned up on every exit path because [`tempfile::TempDir`] does
/// it on drop.
pub async fn download(
    repo: &str,
    skill: &str,
    branch: &str,
    skills_folder: &Path,
    timeout_ms: u64,
) -> Result<DownloadResult, String> {
    validate_skill_name(skill)?;
    validate_repo_url(repo)?;
    if branch.trim().is_empty() {
        return Err("branch must be non-empty".into());
    }

    let tmp = tempfile::tempdir().map_err(|e| format!("create tempdir: {e}"))?;
    let clone_dir = tmp.path().to_path_buf();

    run_git_clone(repo, branch, &clone_dir, timeout_ms).await?;

    // The cloned tree may already exist if a previous run left
    // something behind; in practice tempdir is fresh so this is a
    // safety net.
    let source_dir = clone_dir.join("skills").join(skill);
    if !source_dir.exists() {
        return Err(format!(
            "repo {repo:?} has no folder skills/{skill}/ (cloned to {})",
            clone_dir.display()
        ));
    }
    if !source_dir.is_dir() {
        return Err(format!("skills/{skill} in {repo:?} is not a directory"));
    }

    let dest_root = skills_folder.join(skill);
    std::fs::create_dir_all(&dest_root)
        .map_err(|e| format!("create_dir_all {}: {e}", dest_root.display()))?;

    let mut result = DownloadResult::new(skill);
    copy_recursive(&source_dir, &dest_root, &mut result, &PathBuf::new())?;

    Ok(result)
}

async fn run_git_clone(
    repo: &str,
    branch: &str,
    dest: &Path,
    timeout_ms: u64,
) -> Result<(), String> {
    let dest_str = dest
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 tempdir path: {}", dest.display()))?;
    let fut = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .args([
            "-c",
            "protocol.ext.allow=never",
            "clone",
            "--depth",
            "1",
            "--branch",
            branch,
            "--quiet",
            repo,
            dest_str,
        ])
        .output();
    let output = timeout(Duration::from_millis(timeout_ms), fut)
        .await
        .map_err(|_| format!("git clone timed out after {timeout_ms} ms"))?
        .map_err(|e| format!("spawn git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git clone --branch {branch} exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }
    Ok(())
}

/// Recursively copy every file under `src` into the matching path
/// under `dest_root`. `rel` is the path relative to the original
/// `source_dir` of the calling [`download`]; we use it to record what
/// was written into the [`DownloadResult`].
///
/// Markdown files under `prompts/` segments are recorded in
/// `prompts_written` (by file stem) so the caller can fire
/// `prompts::on-change` selectively. All other files are recorded as
/// `skills_written` (by relative path under the namespace).
fn copy_recursive(
    src: &Path,
    dest_root: &Path,
    result: &mut DownloadResult,
    rel: &Path,
) -> Result<(), String> {
    let entries = std::fs::read_dir(src).map_err(|e| format!("read_dir {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read_dir entry: {e}"))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("file_type {}: {e}", entry.path().display()))?;
        let name = entry.file_name();
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                tracing::warn!(path = %entry.path().display(), "skipping non-UTF-8 entry");
                continue;
            }
        };
        // Defensive: refuse anything that would escape the destination.
        if name_str.contains('/') || name_str.contains('\\') || name_str.starts_with('.') {
            // `.` and `..` are filtered above; this also drops
            // `.git`, `.DS_Store`, etc.
            if name_str.starts_with('.') {
                continue;
            }
        }
        let next_rel = rel.join(&name_str);
        let dest = dest_root.join(&next_rel);
        if file_type.is_dir() {
            copy_recursive(&entry.path(), dest_root, result, &next_rel)?;
        } else if file_type.is_file() {
            let bytes = std::fs::read(entry.path())
                .map_err(|e| format!("read {}: {e}", entry.path().display()))?;
            write_file_atomic(&dest, &bytes)?;
            // Re-validate the relative path for sanity, even though
            // we built it ourselves.
            let rel_str = next_rel.to_string_lossy().replace('\\', "/");
            // ignore validation errors (path is already constructed)
            let _ = validate_relative_path(&rel_str);

            if is_prompt_relpath(&next_rel) && next_rel.extension().is_some_and(|e| e == "md") {
                let stem = next_rel
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                if !stem.is_empty() {
                    result.prompts_written.push(stem);
                }
            } else {
                result.skills_written.push(rel_str);
            }
        }
    }
    Ok(())
}

fn is_prompt_relpath(rel: &Path) -> bool {
    rel.components().any(|c| c.as_os_str() == "prompts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_skill_name_accepts_simple() {
        assert!(validate_skill_name("frontend-design").is_ok());
        assert!(validate_skill_name("foo_bar").is_ok());
        assert!(validate_skill_name("v2").is_ok());
        assert!(validate_skill_name("a.b.c").is_ok());
    }

    #[test]
    fn validate_skill_name_rejects_traversal_and_separators() {
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("..").is_err());
        assert!(validate_skill_name("a/b").is_err());
        assert!(validate_skill_name("a\\b").is_err());
        assert!(validate_skill_name(".hidden").is_err());
    }

    #[test]
    fn is_prompt_relpath_detects_segment() {
        assert!(is_prompt_relpath(Path::new("prompts/foo.md")));
        assert!(is_prompt_relpath(Path::new("a/prompts/b.md")));
        assert!(!is_prompt_relpath(Path::new("foo/bar.md")));
        assert!(!is_prompt_relpath(Path::new("promptsx/foo.md")));
    }

    // ── validate_repo_url ─────────────────────────────────────────────

    #[test]
    fn repo_url_accepts_https() {
        assert!(validate_repo_url("https://github.com/x/y").is_ok());
    }

    #[test]
    fn repo_url_accepts_git_at_scp() {
        assert!(validate_repo_url("git@github.com:x/y").is_ok());
    }

    #[test]
    fn repo_url_accepts_ssh() {
        assert!(validate_repo_url("ssh://git@github.com/x/y").is_ok());
    }

    #[test]
    fn repo_url_rejects_ext_transport() {
        let err = validate_repo_url("ext::sh -c 'x'").unwrap_err();
        assert!(err.contains("::"), "got: {err}");
    }

    #[test]
    fn repo_url_rejects_file_scheme() {
        let err = validate_repo_url("file:///etc").unwrap_err();
        assert!(err.contains("file:"), "got: {err}");
    }

    #[test]
    fn repo_url_rejects_arg_injection() {
        let err = validate_repo_url("--upload-pack=evil").unwrap_err();
        assert!(err.contains("'-'"), "got: {err}");
    }

    #[test]
    fn repo_url_rejects_empty() {
        assert!(validate_repo_url("").is_err());
        assert!(validate_repo_url("   ").is_err());
    }

    #[test]
    fn repo_url_rejects_http_insecure() {
        let err = validate_repo_url("http://insecure.com/repo").unwrap_err();
        assert!(err.contains("https://"), "got: {err}");
    }

    #[test]
    fn repo_url_rejects_double_colon() {
        assert!(validate_repo_url("git::https://x.com/y").is_err());
    }
}
