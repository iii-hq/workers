//! Temp git repositories: a bare `origin`, a clone whose remote reads as a
//! GitHub URL (an `insteadOf` rule sends git to the bare repository), and a
//! context connected with an API key against a mock cloud.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use iii_cloud::connect::connect;
use iii_cloud::types::ConnectRequest;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{Fake, Harness, harness};

pub const KEY: &str =
    "iii_0123456789abcdef_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub const TEAM: &str = "11111111-1111-4111-8111-111111111111";
pub const ENV: &str = "22222222-2222-4222-8222-222222222222";

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.test")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.test")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub struct Repo {
    pub root: tempfile::TempDir,
    pub origin: PathBuf,
    pub checkout: PathBuf,
}

/// `origin.git` with one commit on `main`, and `work/hello-iii` cloned from it.
pub fn repo() -> Repo {
    let root = tempfile::tempdir().unwrap();
    let origin = root.path().join("origin.git");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "--bare", "-b", "main"]);
    let seed = root.path().join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-b", "main"]);
    std::fs::write(seed.join("worker-compose.yaml"), "containers: {}\n").unwrap();
    std::fs::write(seed.join("README.md"), "hello\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-m", "first"]);
    git(&seed, &["push", origin.to_str().unwrap(), "main"]);
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["clone", origin.to_str().unwrap(), "hello-iii"]);
    let checkout = work.join("hello-iii");
    git(
        &checkout,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/ada/hello-iii.git",
        ],
    );
    git(
        &checkout,
        &[
            "config",
            &format!("url.{}.insteadOf", origin.display()),
            "https://github.com/ada/hello-iii.git",
        ],
    );
    git(
        &checkout,
        &["branch", "--set-upstream-to=origin/main", "main"],
    );
    // The person's repository has an identity, as a real one does: the worker's own merges and commits in
    // its worktrees use it, and a runner has no global one.
    for (key, value) in [("user.name", "Ada"), ("user.email", "ada@example.test")] {
        git(&checkout, &["config", key, value]);
    }
    Repo {
        root,
        origin,
        checkout,
    }
}

/// A key-connected context whose cloud answers the one environment.
pub async fn connected(server: &MockServer, manifest: &str) -> Harness {
    Mock::given(method("GET"))
        .and(path("/api/v1/keys/self"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"team_id": TEAM, "environment_id": ENV, "project_id": "p", "permissions": ["read", "deploy"]})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "project_id": "p", "name": "production", "production": true, "branch": "main", "manifest": manifest, "url": null, "settings": {}})))
        .mount(server)
        .await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("MY_KEY", KEY),
    );
    connect(
        &h.ctx,
        ConnectRequest::ApiKey {
            secret: "MY_KEY".into(),
            team_id: None,
            environment_id: None,
        },
    )
    .await
    .unwrap();
    h
}
