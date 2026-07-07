//! Async trash removal: the worktree directory leaves its original path
//! immediately, appears under `.trash`, and a detached task deletes it; the
//! prune sweep clears stale entries; the busy probe refuses removal while a
//! process holds files open under the worktree.

mod support;

use std::path::Path;
use std::time::Duration;

use support::{create_request, git, init_repo, make_env, test_config, TestEnv};
use worktree::functions::{create, prune, remove};
use worktree::trash;
use worktree::types::now_ms;

async fn create_wt(env: &TestEnv, repo: &Path) -> create::Response {
    create::handle(&env.deps, create_request(repo))
        .await
        .unwrap()
}

async fn wait_gone(path: &Path) {
    for _ in 0..100 {
        if !path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("background delete never cleared {}", path.display());
}

#[tokio::test]
async fn remove_stages_into_trash_and_deletes_in_background() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let cfg = test_config(tmp.path());
    let root = cfg.expanded_worktree_root();
    let env = make_env(tmp.path(), cfg);
    let created = create_wt(&env, &repo).await;
    let wt_path = Path::new(&created.path).to_path_buf();

    let resp = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: created.worktree_id.clone(),
            force: false,
            delete_branch: false,
        },
    )
    .await
    .unwrap();
    assert!(resp.removed);

    // Gone from the worktree root immediately; the git admin entry is
    // pruned synchronously.
    assert!(!wt_path.exists(), "original path cleared synchronously");
    let porcelain = git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(!porcelain.contains(&created.worktree_id));

    // Any surviving staged entry belongs to this worktree and disappears
    // once the detached delete lands.
    let trash = trash::trash_dir(&root);
    if let Ok(entries) = std::fs::read_dir(&trash) {
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                name.starts_with(&created.worktree_id),
                "unexpected trash entry {name}"
            );
            wait_gone(&entry.path()).await;
        }
    }
}

#[tokio::test]
async fn prune_sweeps_stale_trash_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let cfg = test_config(tmp.path());
    let root = cfg.expanded_worktree_root();
    let env = make_env(tmp.path(), cfg);

    let trash = trash::trash_dir(&root);
    std::fs::create_dir_all(&trash).unwrap();
    let stale_secs = (now_ms() - trash::TRASH_MAX_AGE_MS - 60_000) / 1000;
    let stale = trash.join(format!("wt_stale000-{stale_secs}"));
    let fresh = trash.join(format!("wt_fresh000-{}", now_ms() / 1000));
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("f.txt"), "x").unwrap();
    std::fs::create_dir_all(&fresh).unwrap();

    prune::handle(
        &env.deps,
        prune::Request {
            repo_path: None,
            dry_run: false,
        },
    )
    .await
    .unwrap();

    assert!(!stale.exists(), "stale trash entry swept");
    assert!(
        fresh.exists(),
        "fresh trash entry kept for its detached delete"
    );
}

#[tokio::test]
async fn busy_worktree_refuses_removal_until_forced() {
    if which::which("lsof").is_err() {
        eprintln!("skipping: lsof not available");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let created = create_wt(&env, &repo).await;
    let wt_path = Path::new(&created.path).to_path_buf();

    // Hold a file open under the worktree so `lsof +D` sees this process.
    let held = std::fs::File::open(wt_path.join("README.md")).unwrap();
    let err = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: created.worktree_id.clone(),
            force: false,
            delete_branch: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W222");

    // force bypasses the probe (test_config opens gates.allow_force).
    let resp = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: created.worktree_id,
            force: true,
            delete_branch: false,
        },
    )
    .await
    .unwrap();
    assert!(resp.removed);
    drop(held);
    assert!(!wt_path.exists());
}
