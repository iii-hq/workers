//! Copy-ignored provisioning: filter matrix (include / exclude /
//! always-excluded VCS dirs and worktree roots), the byte budget, the
//! macOS clone-flag fallback (via an injected cp shim), and the spawned
//! background wiring on create.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use support::{create_request, git, init_repo, make_env, test_config};
use worktree::config::ProvisionConfig;
use worktree::functions::create;
use worktree::provision::{copy_entry_with, copy_ignored, filter_entries, CopyMode};

fn pcfg(mutate: impl FnOnce(&mut ProvisionConfig)) -> ProvisionConfig {
    let mut cfg = ProvisionConfig {
        copy_ignored: true,
        ..Default::default()
    };
    mutate(&mut cfg);
    cfg
}

/// Repo with tracked + gitignored content.
fn ignored_fixture(tmp: &Path) -> std::path::PathBuf {
    let repo = tmp.join("repo");
    init_repo(&repo);
    std::fs::write(repo.join(".gitignore"), ".env\nnode_modules/\n*.log\n").unwrap();
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["commit", "-m", "ignore rules"]);
    std::fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
    std::fs::create_dir_all(repo.join("node_modules/pkg")).unwrap();
    std::fs::write(repo.join("node_modules/pkg/a.js"), "js\n").unwrap();
    std::fs::write(repo.join("debug.log"), "log\n").unwrap();
    repo
}

#[test]
fn filter_matrix_applies_include_exclude_and_always_excludes() {
    let repo = Path::new("/repo");
    let root = Path::new("/repo/.wts");
    let entries = vec![
        ".env".to_string(),
        "node_modules/".to_string(),
        "debug.log".to_string(),
        ".hg/store".to_string(),
        ".jj/state".to_string(),
        ".wts/wt_x/junk".to_string(),
        "vendor/.svn/props".to_string(),
    ];

    // Empty include keeps everything not always-excluded.
    let all = filter_entries(entries.clone(), &pcfg(|_| {}), repo, root, &[]);
    assert_eq!(all, vec![".env", "node_modules/", "debug.log"]);

    // Include narrows.
    let only_env = filter_entries(
        entries.clone(),
        &pcfg(|c| c.include = vec![".env".into()]),
        repo,
        root,
        &[],
    );
    assert_eq!(only_env, vec![".env"]);

    // Exclude subtracts from the included set.
    let no_logs = filter_entries(
        entries.clone(),
        &pcfg(|c| c.exclude = vec!["*.log".into()]),
        repo,
        root,
        &[],
    );
    assert_eq!(no_logs, vec![".env", "node_modules/"]);

    // Registered worktree paths are never copied.
    let skip_nested = filter_entries(
        entries,
        &pcfg(|_| {}),
        repo,
        root,
        &["/repo/node_modules".to_string()],
    );
    assert_eq!(skip_nested, vec![".env", "debug.log"]);
}

#[tokio::test]
async fn copy_ignored_replicates_into_the_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = ignored_fixture(tmp.path());
    let wt = tmp.path().join("wt");
    std::fs::create_dir_all(&wt).unwrap();

    let summary = copy_ignored(
        &repo,
        &wt,
        &pcfg(|_| {}),
        &[],
        &tmp.path().join("wts"),
        CopyMode::Plain,
        30_000,
    )
    .await
    .unwrap();

    assert!(summary.copied >= 3, "copied {}", summary.copied);
    assert!(!summary.over_budget);
    assert_eq!(
        std::fs::read_to_string(wt.join(".env")).unwrap(),
        "SECRET=1\n"
    );
    assert!(wt.join("node_modules/pkg/a.js").is_file());
    assert!(wt.join("debug.log").is_file());
    assert!(!wt.join(".git").exists());
}

#[tokio::test]
async fn copy_budget_stops_the_copy_with_a_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = ignored_fixture(tmp.path());
    let wt = tmp.path().join("wt");
    std::fs::create_dir_all(&wt).unwrap();

    let summary = copy_ignored(
        &repo,
        &wt,
        &pcfg(|c| c.max_copy_bytes = 1),
        &[],
        &tmp.path().join("wts"),
        CopyMode::Plain,
        30_000,
    )
    .await
    .unwrap();
    assert!(summary.over_budget);
    assert_eq!(summary.copied, 0);
    assert!(!wt.join(".env").exists());
}

#[test]
fn clone_flag_falls_back_to_a_plain_copy() {
    let tmp = tempfile::tempdir().unwrap();
    // A cp shim that refuses the macOS clone flag and otherwise delegates,
    // simulating a filesystem without clone support.
    let shim = tmp.path().join("cp-shim");
    std::fs::write(
        &shim,
        "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = \"-c\" ] && exit 1; done\nexec cp \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

    let src = tmp.path().join("src.txt");
    std::fs::write(&src, "payload\n").unwrap();
    let dst = tmp.path().join("dst.txt");

    let ok = copy_entry_with(
        shim.to_string_lossy().as_ref(),
        CopyMode::MacClone,
        &src,
        &dst,
    );
    assert!(ok, "fallback plain copy must succeed");
    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "payload\n");
}

#[tokio::test]
async fn create_spawns_the_background_provisioning() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = ignored_fixture(tmp.path());
    let mut cfg = test_config(tmp.path());
    cfg.provision.copy_ignored = true;
    let env = make_env(tmp.path(), cfg);

    let created = create::handle(&env.deps, create_request(&repo))
        .await
        .unwrap();

    // The response returns before the copy; poll for the background task
    // with a generous deadline (cold CI runners need well over a second).
    let env_file = Path::new(&created.path).join(".env");
    for _ in 0..600 {
        if env_file.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(env_file.exists(), "background provisioning never landed");
}
