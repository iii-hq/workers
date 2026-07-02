//! Configuration gates: every mutating handler checks its gate FIRST, so
//! denials need no repository or record — the error must carry the exact
//! config key to flip.

mod support;

use support::{make_env, test_config, TestEnv};
use worktree::config::WorkerConfig;
use worktree::functions::{claim, land, prune, release, remove};

fn env_with(tmp: &std::path::Path, mutate: impl FnOnce(&mut WorkerConfig)) -> TestEnv {
    let mut cfg = test_config(tmp);
    // test_config opens the force gate for behavioral suites; this suite
    // starts from the shipped defaults.
    cfg.gates = Default::default();
    mutate(&mut cfg);
    make_env(tmp, cfg)
}

#[tokio::test]
async fn remove_denied_when_gate_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_with(tmp.path(), |c| c.gates.allow_remove = false);
    let err = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: "wt_whatever".into(),
            force: false,
            delete_branch: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W500");
    assert!(
        err.message.contains("gates.allow_remove"),
        "{}",
        err.message
    );
}

#[tokio::test]
async fn branch_delete_denied_when_gate_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_with(tmp.path(), |c| c.gates.allow_branch_delete = false);
    let err = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: "wt_whatever".into(),
            force: false,
            delete_branch: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W500");
    assert!(
        err.message.contains("gates.allow_branch_delete"),
        "{}",
        err.message
    );
}

#[tokio::test]
async fn default_force_gate_blocks_all_four_force_paths() {
    let tmp = tempfile::tempdir().unwrap();
    // Shipped defaults: allow_force = false.
    let env = env_with(tmp.path(), |_| {});

    let err = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: "wt_x".into(),
            force: true,
            delete_branch: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W501");
    assert!(err.message.contains("gates.allow_force"), "{}", err.message);

    let err = claim::handle(
        &env.deps,
        claim::Request {
            worktree_id: "wt_x".into(),
            session_id: "s_1".into(),
            force: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W501");

    let err = release::handle(
        &env.deps,
        release::Request {
            worktree_id: "wt_x".into(),
            session_id: "s_1".into(),
            force: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W501");

    let err = land::handle(
        &env.deps,
        land::Request {
            worktree_id: "wt_x".into(),
            target_branch: "main".into(),
            test_cmd: None,
            force_restart: true,
            keep: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W501");
    assert!(err.message.contains("gates.allow_force"), "{}", err.message);
}

#[tokio::test]
async fn land_denied_when_gate_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_with(tmp.path(), |c| c.gates.allow_land = false);
    let err = land::handle(
        &env.deps,
        land::Request {
            worktree_id: "wt_x".into(),
            target_branch: "main".into(),
            test_cmd: None,
            force_restart: false,
            keep: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W500");
    assert!(err.message.contains("gates.allow_land"), "{}", err.message);
}

#[tokio::test]
async fn prune_denied_when_gate_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_with(tmp.path(), |c| c.gates.allow_prune = false);
    let err = prune::handle(
        &env.deps,
        prune::Request {
            repo_path: None,
            dry_run: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W500");
    assert!(err.message.contains("gates.allow_prune"), "{}", err.message);
}

#[tokio::test]
async fn land_targets_glob_pins_the_target() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_with(tmp.path(), |c| {
        c.gates.land_targets = vec!["main".into(), "release/*".into()];
    });

    let err = land::handle(
        &env.deps,
        land::Request {
            worktree_id: "wt_x".into(),
            target_branch: "trunk".into(),
            test_cmd: None,
            force_restart: false,
            keep: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W502");
    assert!(
        err.message.contains("gates.land_targets"),
        "{}",
        err.message
    );

    // An allowed target passes the gate and fails later on the missing
    // record instead — proof the gate itself admitted it.
    let err = land::handle(
        &env.deps,
        land::Request {
            worktree_id: "wt_x".into(),
            target_branch: "release/1.2".into(),
            test_cmd: None,
            force_restart: false,
            keep: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W200");
}
