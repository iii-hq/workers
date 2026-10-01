//! `resolve_source` against real git (a bare repository served over
//! `file://`) and in local-checkout (`dir`) mode.

use std::path::{Path, PathBuf};
use std::process::Command;

use ide::code::config::TemplatesConfig;
use ide::code::templates::resolve_source;

/// git with the developer's own config out of the way and a fixed identity.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "iii")
        .env("GIT_AUTHOR_EMAIL", "iii@test")
        .env("GIT_COMMITTER_NAME", "iii")
        .env("GIT_COMMITTER_EMAIL", "iii@test")
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A bare templates repo plus the work tree that pushes to it.
struct Remote {
    _tmp: tempfile::TempDir,
    bare: PathBuf,
    work: PathBuf,
}

impl Remote {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("templates.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(work.join("iii")).unwrap();
        git(
            tmp.path(),
            &["init", "-q", "--bare", "-b", "main", "templates.git"],
        );
        git(&work, &["init", "-q", "-b", "main"]);
        let remote = Self {
            _tmp: tmp,
            bare,
            work,
        };
        remote.commit("one");
        remote
    }

    fn url(&self) -> String {
        format!("file://{}", self.bare.display())
    }

    /// Commit `iii/template.yaml` tagged `tag`, push it, return the sha.
    fn commit(&self, tag: &str) -> String {
        std::fs::write(
            self.work.join("iii/template.yaml"),
            format!("# {tag}\ntemplates: []\n"),
        )
        .unwrap();
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "-m", tag]);
        git(&self.work, &["push", "-q", self.url().as_str(), "main"]);
        git(&self.work, &["rev-parse", "HEAD"])
    }

    fn cfg(&self, cache_dir: &Path) -> TemplatesConfig {
        TemplatesConfig {
            url: self.url(),
            cache_dir: cache_dir.display().to_string(),
            ..TemplatesConfig::default()
        }
    }
}

#[tokio::test]
async fn clones_then_reuses_the_cache_until_a_refresh() {
    let remote = Remote::new();
    let cache = tempfile::tempdir().unwrap();
    let cfg = remote.cfg(cache.path());
    let one = git(&remote.work, &["rev-parse", "HEAD"]);

    let first = resolve_source(&cfg, false).await.unwrap();
    assert_eq!(first.info.kind, "git");
    assert_eq!(first.info.location, remote.url());
    assert_eq!(first.info.git_ref.as_deref(), Some("main"));
    assert_eq!(first.info.revision.as_deref(), Some(one.as_str()));
    assert_eq!(first.info.warning, None);
    assert!(first.root.starts_with(cache.path()));
    assert!(first.root.join("template.yaml").is_file());

    let two = remote.commit("two");
    let cached = resolve_source(&cfg, false).await.unwrap();
    assert_eq!(
        cached.info.revision.as_deref(),
        Some(one.as_str()),
        "within refresh_secs the cache is reused without a fetch"
    );

    let refreshed = resolve_source(&cfg, true).await.unwrap();
    assert_eq!(refreshed.info.revision.as_deref(), Some(two.as_str()));
    let manifest = std::fs::read_to_string(refreshed.root.join("template.yaml")).unwrap();
    assert!(manifest.contains("two"), "{manifest}");

    let three = remote.commit("three");
    let expired = TemplatesConfig {
        refresh_secs: 0,
        ..cfg.clone()
    };
    let aged = resolve_source(&expired, false).await.unwrap();
    assert_eq!(
        aged.info.revision.as_deref(),
        Some(three.as_str()),
        "an expired cache refreshes on its own"
    );
}

#[tokio::test]
async fn unreachable_remote_serves_the_stale_cache_with_a_warning() {
    let remote = Remote::new();
    let cache = tempfile::tempdir().unwrap();
    let cfg = remote.cfg(cache.path());
    let cloned = resolve_source(&cfg, false).await.unwrap();
    std::fs::rename(&remote.bare, remote.bare.with_extension("gone")).unwrap();

    let stale = resolve_source(&cfg, true).await.unwrap();
    assert_eq!(stale.info.revision, cloned.info.revision);
    let warning = stale.info.warning.expect("a stale cache is flagged");
    assert!(warning.contains(&remote.url()), "{warning}");
    assert!(stale.root.join("template.yaml").is_file());
}

#[tokio::test]
async fn unreachable_remote_without_a_cache_is_c230() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = TemplatesConfig {
        url: format!("file://{}", tmp.path().join("nowhere.git").display()),
        cache_dir: tmp.path().join("cache").display().to_string(),
        ..TemplatesConfig::default()
    };
    let err = resolve_source(&cfg, false).await.unwrap_err();
    assert_eq!(err.code(), "C230");
    assert!(
        err.message().contains("code.templates.dir"),
        "{}",
        err.message()
    );
}

#[tokio::test]
async fn a_cache_without_its_own_head_is_recloned_not_resolved_upwards() {
    // The cache sits inside another repository. A clone dir that lost its
    // .git must not resolve to the enclosing repo, where a refresh would
    // fetch and `reset --hard`.
    let remote = Remote::new();
    let outer = tempfile::tempdir().unwrap();
    git(outer.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(outer.path().join("keep.txt"), "outer\n").unwrap();
    git(outer.path(), &["add", "-A"]);
    git(outer.path(), &["commit", "-q", "-m", "outer"]);
    let cfg = remote.cfg(&outer.path().join("data/templates"));

    let first = resolve_source(&cfg, false).await.unwrap();
    let clone_dir = first.root.parent().unwrap().to_path_buf();
    std::fs::remove_dir_all(clone_dir.join(".git")).unwrap();

    let again = resolve_source(&cfg, false).await.unwrap();
    assert_eq!(again.info.revision, first.info.revision);
    assert!(clone_dir.join(".git").is_dir(), "re-cloned");
    assert_eq!(
        std::fs::read_to_string(outer.path().join("keep.txt")).unwrap(),
        "outer\n"
    );
    assert_eq!(git(outer.path(), &["log", "--format=%s"]), "outer");
}

#[tokio::test]
async fn dir_mode_accepts_the_repo_root_or_its_iii_dir() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/templates");
    let iii = repo.join("iii");
    for dir in [&repo, &iii] {
        let cfg = TemplatesConfig {
            dir: Some(dir.display().to_string()),
            ..TemplatesConfig::default()
        };
        let source = resolve_source(&cfg, false).await.unwrap();
        assert_eq!(source.root, iii);
        assert_eq!(source.info.kind, "dir");
        assert_eq!(source.info.location, iii.display().to_string());
        assert_eq!(source.info.git_ref, None);
        assert_eq!(source.info.revision, None);
    }

    let empty = tempfile::tempdir().unwrap();
    let cfg = TemplatesConfig {
        dir: Some(empty.path().display().to_string()),
        ..TemplatesConfig::default()
    };
    assert_eq!(
        resolve_source(&cfg, false).await.unwrap_err().code(),
        "C230"
    );
}
