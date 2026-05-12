//! Step defs for `tests/features/download_repo.feature`.
//!
//! Each scenario spins up a local fixture git repo in a tempdir
//! (`git init` + `git add` + `git commit`) and points
//! `skills::download repo=…` at the resulting `file:///` URL. This
//! exercises the production code path against a real `git clone
//! --depth 1` without touching the network.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_OK: &str = "download_last_ok";
const LAST_ERR: &str = "download_last_err";

// One tempdir per test binary process, so re-runs of `cargo test --test bdd`
// don't leak gigabytes. Each scenario builds a fresh `local-<n>` repo
// inside it.
struct RepoSlot {
    root: PathBuf,
    counter: Mutex<usize>,
}

impl RepoSlot {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("repo slot tempdir");
        Self {
            root: tmp.keep(),
            counter: Mutex::new(0),
        }
    }
    fn next_repo(&self) -> PathBuf {
        let mut g = self.counter.lock().unwrap();
        *g += 1;
        let path = self.root.join(format!("local-{}", *g));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

fn repo_slot() -> &'static RepoSlot {
    use std::sync::OnceLock;
    static SLOT: OnceLock<RepoSlot> = OnceLock::new();
    SLOT.get_or_init(RepoSlot::new)
}

fn run_git(repo_dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo_dir)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    assert!(
        status.success(),
        "git {args:?} in {} exited with {}",
        repo_dir.display(),
        status
    );
}

fn init_repo() -> PathBuf {
    let path = repo_slot().next_repo();
    run_git(&path, &["init", "--quiet", "--initial-branch=main"]);
    run_git(&path, &["config", "user.email", "test@example.com"]);
    run_git(&path, &["config", "user.name", "BDD Test"]);
    // Don't sign commits; CI may not have keys configured.
    run_git(&path, &["config", "commit.gpgsign", "false"]);
    path
}

fn commit_all(repo_dir: &std::path::Path, message: &str) {
    run_git(repo_dir, &["add", "-A"]);
    run_git(repo_dir, &["commit", "--quiet", "-m", message]);
}

fn write_to_repo(repo_dir: &std::path::Path, rel: &str, body: &str) {
    let path = repo_dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, body).unwrap();
}

fn current_repo_path(world: &IiiSkillsWorld) -> Option<PathBuf> {
    world
        .stash
        .get("download_repo_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
}

#[given(regex = r#"^a local git repo with a folder skills/([^/]+)/ containing:$"#)]
async fn local_repo_with_folder(
    world: &mut IiiSkillsWorld,
    skill: String,
    step: &cucumber::gherkin::Step,
) {
    let repo = init_repo();
    let table = step.table.as_ref().expect("expected a step data table");
    // First row is the header (path, body); skip it.
    for row in table.rows.iter().skip(1) {
        let rel_in_skill = row.first().cloned().unwrap_or_default();
        let body = row.get(1).cloned().unwrap_or_default();
        let body = body.replace("\\n", "\n");
        let rel = format!("skills/{}/{}", skill, rel_in_skill);
        write_to_repo(&repo, &rel, &body);
    }
    commit_all(&repo, "seed");
    world.stash.insert(
        "download_repo_path".into(),
        Value::String(repo.to_string_lossy().into_owned()),
    );
}

#[when(regex = r#"^I update the local repo file "([^"]+)" to body "([^"]+)"$"#)]
async fn update_local_repo_file(world: &mut IiiSkillsWorld, rel: String, body: String) {
    let repo = current_repo_path(world).expect("no repo registered");
    let body = body.replace("\\n", "\n");
    write_to_repo(&repo, &rel, &body);
    commit_all(&repo, "update");
}

fn repo_url(world: &IiiSkillsWorld) -> String {
    let path = current_repo_path(world).expect("no repo registered");
    format!("file://{}", path.display())
}

#[when(regex = r#"^I trigger skills::download with repo=local skill="([^"]+)"$"#)]
async fn trigger_download_repo(world: &mut IiiSkillsWorld, skill: String) {
    world.stash.remove(LAST_OK);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let url = repo_url(world);
    match iii
        .trigger(TriggerRequest {
            function_id: "skills::download".to_string(),
            payload: json!({ "repo": url, "skill": skill }),
            action: None,
            timeout_ms: Some(60_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(LAST_OK.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_ERR.into(), Value::String(e.to_string()));
        }
    }
}

#[when(
    regex = r#"^I trigger skills::download with both repo="([^"]+)" and worker="([^"]+)" and tag="([^"]+)"$"#
)]
async fn trigger_download_conflicting(
    world: &mut IiiSkillsWorld,
    repo: String,
    worker: String,
    tag: String,
) {
    world.stash.remove(LAST_OK);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: "skills::download".to_string(),
            payload: json!({
                "repo": repo,
                "worker": worker,
                "tag": tag,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(LAST_OK.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_ERR.into(), Value::String(e.to_string()));
        }
    }
}

// ── shared assertions used by both download features ──────────────

#[then("the skills::download call succeeds")]
fn download_succeeds(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_OK),
        "expected download to succeed; got error: {:?}",
        world.stash.get(LAST_ERR)
    );
}

#[then(regex = r#"^the skills::download call fails with a message mentioning "([^"]+)"$"#)]
fn download_fails(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let err = world
        .stash
        .get(LAST_ERR)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(
        !err.is_empty(),
        "expected error; got: {:?}",
        world.stash.get(LAST_OK)
    );
    assert!(
        err.contains(&needle),
        "expected error to mention {needle:?}; got: {err:?}"
    );
}

#[then(regex = r#"^the file "([^"]+)" exists in skills_folder$"#)]
fn file_exists(world: &mut IiiSkillsWorld, rel: String) {
    if world.iii.is_none() {
        return;
    }
    let folder = world.skills_folder.clone().unwrap();
    let path = folder.join(&rel);
    assert!(path.exists(), "expected {} to exist", path.display());
}

#[then(regex = r#"^the file "([^"]+)" in skills_folder contains "([^"]+)"$"#)]
fn file_contains(world: &mut IiiSkillsWorld, rel: String, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let folder = world.skills_folder.clone().unwrap();
    let path = folder.join(&rel);
    let body =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(
        body.contains(&needle),
        "{} should contain {needle:?}; got: {body:?}",
        path.display()
    );
}

#[then(regex = r#"^the download response namespace is "([^"]+)"$"#)]
fn response_namespace(world: &mut IiiSkillsWorld, ns: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_OK).expect("no download recorded");
    assert_eq!(v["namespace"].as_str().unwrap_or(""), ns);
}

#[then(regex = r#"^the download skills_written count is (\d+)$"#)]
fn response_skills_count(world: &mut IiiSkillsWorld, n: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_OK).expect("no download recorded");
    let arr = v["skills_written"]
        .as_array()
        .expect("missing .skills_written array");
    assert_eq!(arr.len(), n, "skills_written: {arr:?}");
}

#[then(regex = r#"^the download prompts_written count is (\d+)$"#)]
fn response_prompts_count(world: &mut IiiSkillsWorld, n: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_OK).expect("no download recorded");
    let arr = v["prompts_written"]
        .as_array()
        .expect("missing .prompts_written array");
    assert_eq!(arr.len(), n, "prompts_written: {arr:?}");
}
