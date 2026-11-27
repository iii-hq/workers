//! Cloud projects and environments, as a person with a browser session sees them:
//! several projects for one repository, choosing one, environments on their own
//! branch, creating a project or an environment, and console deploys.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iii_cloud::create::{create_environment, create_project, environment_error};
use iii_cloud::project::{resolve, use_project};
use iii_cloud::types::*;
use serde_json::{Value, json};
use support::repo::*;
use support::{Fake, Harness, harness};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const P1: &str = "aaaaaaa1-0000-4000-8000-000000000001";
const P2: &str = "aaaaaaa2-0000-4000-8000-000000000002";
const P3: &str = "aaaaaaa3-0000-4000-8000-000000000003";
const E1: &str = "bbbbbbb1-0000-4000-8000-000000000001";
const E1_DEV: &str = "bbbbbbb2-0000-4000-8000-000000000002";
const E2: &str = "bbbbbbb3-0000-4000-8000-000000000003";
const E3: &str = "bbbbbbb4-0000-4000-8000-000000000004";
const DEV_BRANCH: &str = "iii-cloud/ready-production";

fn env(id: &str, name: &str, production: bool, branch: &str) -> Value {
    json!({"id": id, "project_id": "p", "name": name, "production": production, "branch": branch, "manifest": "worker-compose.yaml", "url": null, "settings": {}})
}

fn project(id: &str, name: &str, created: &str) -> Value {
    json!({"id": id, "name": name, "endpoint_slug": name, "created_at": created, "repository": {"id": 77, "full_name": "ada/hello-iii", "default_branch": "main"}})
}

async fn get(server: &MockServer, p: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1{p}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// A GET that wins over the ones mounted before it.
async fn get_first(server: &MockServer, p: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1{p}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .with_priority(1)
        .mount(server)
        .await;
}

/// A browser session, and a cloud with two projects for the repository: `registry`
/// (production and a `dev` env on its own branch) and a newer `registry-3`.
async fn cloud(server: &MockServer, p1_deployed: bool) -> Harness {
    get(server, "/me", json!({"user": {"id": "u", "email": "ada@example.test"}, "teams": [{"id": TEAM, "name": "Acme", "slug": "acme", "personal": false, "role": "owner"}]})).await;
    get(server, &format!("/teams/{TEAM}/projects"), json!({"projects": [project(P1, "registry", "2026-11-01T00:00:00Z"), project(P2, "registry-3", "2026-11-20T00:00:00Z")]})).await;
    get(server, &format!("/teams/{TEAM}/projects/{P1}/environments"), json!({"environments": [env(E1, "production", true, "main"), env(E1_DEV, "dev", false, DEV_BRANCH)]})).await;
    get(
        server,
        &format!("/teams/{TEAM}/projects/{P2}/environments"),
        json!({"environments": [env(E2, "production", true, "main")]}),
    )
    .await;
    for (id, name, production, branch) in [
        (E1, "production", true, "main"),
        (E1_DEV, "dev", false, DEV_BRANCH),
        (E2, "production", true, "main"),
    ] {
        get(
            server,
            &format!("/teams/{TEAM}/environments/{id}"),
            env(id, name, production, branch),
        )
        .await;
    }
    let deployments = if p1_deployed {
        vec![
            json!({"id": "d1", "number": 1, "environment_id": E1, "type": "manual", "status": "ready", "status_detail": null, "commit": {"sha": "abc1234", "branch": "main"}, "stages": [], "error": null, "created_at": "2026-11-25T10:00:00Z"}),
        ]
    } else {
        vec![]
    };
    get(
        server,
        &format!("/teams/{TEAM}/environments/{E1}/deployments"),
        json!({"deployments": deployments, "next_cursor": null}),
    )
    .await;
    for id in [E1_DEV, E2] {
        get(
            server,
            &format!("/teams/{TEAM}/environments/{id}/deployments"),
            json!({"deployments": [], "next_cursor": null}),
        )
        .await;
    }
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("IIICLOUD_SESSION", "iiia_tok"),
    );
    let mut conn = Connection::signed_out(&server.uri());
    conn.status = ConnStatus::Connected;
    conn.method = Some(Method::Browser);
    h.ctx.set_connection(conn);
    h
}

fn use_req(
    path: &std::path::Path,
    cloud_project_id: Option<&str>,
    environment_id: Option<&str>,
) -> UseRequest {
    UseRequest {
        path: path.to_string_lossy().into(),
        environment_id: environment_id.map(str::to_owned),
        mode: WorkspaceMode::Worktree,
        cloud_project_id: cloud_project_id.map(str::to_owned),
        from: None,
        source: None,
    }
}

fn resolve_req(path: &std::path::Path) -> ResolveRequest {
    ResolveRequest {
        path: path.to_string_lossy().into(),
        team_id: None,
        cloud_project_id: None,
    }
}

#[tokio::test]
async fn every_cloud_project_of_the_repository_is_listed_and_the_most_recent_deployment_wins() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    let p = resolve(&h.ctx, resolve_req(&r.checkout)).await.unwrap();
    assert_eq!(p.default_branch.as_deref(), Some("main"));
    let ids: Vec<&str> = p.cloud_projects.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec![P1, P2]);
    let first = &p.cloud_projects[0];
    assert_eq!(
        (
            first.name.as_str(),
            first.team.id.as_str(),
            first.team.name.as_str()
        ),
        ("registry", TEAM, "Acme")
    );
    let dev = first.environments.iter().find(|e| e.name == "dev").unwrap();
    assert_eq!(
        (dev.branch.as_str(), dev.manifest.as_deref(), dev.production),
        (DEV_BRANCH, Some("worker-compose.yaml"), false)
    );
    // `registry` has the newest deployment, though `registry-3` is the newer project.
    assert_eq!(p.cloud_project.as_ref().unwrap().id, P1);
    assert_eq!(
        first.last_deployment_at.as_deref(),
        Some("2026-11-25T10:00:00Z")
    );
}

#[tokio::test]
async fn without_deployments_the_newest_project_is_taken_and_none_is_silent() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, false).await;
    let p = resolve(&h.ctx, resolve_req(&r.checkout)).await.unwrap();
    assert_eq!(p.cloud_project.as_ref().unwrap().id, P2, "the newest");
    assert_eq!(
        p.cloud_projects.len(),
        2,
        "the others are listed, not hidden"
    );
    // The person can choose.
    let mut req = resolve_req(&r.checkout);
    req.cloud_project_id = Some(P1.into());
    assert_eq!(
        resolve(&h.ctx, req)
            .await
            .unwrap()
            .cloud_project
            .unwrap()
            .id,
        P1
    );
    // An id no team lists: gone.
    let mut req = resolve_req(&r.checkout);
    req.cloud_project_id = Some("nope".into());
    assert_eq!(
        resolve(&h.ctx, req).await.unwrap_err().code,
        "PROJECT_DELETED"
    );
}

#[tokio::test]
async fn use_takes_a_project_an_environment_or_both_and_checks_they_belong_together() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    // A project alone: its default environment (production).
    let p = use_project(&h.ctx, use_req(&r.checkout, Some(P2), None))
        .await
        .unwrap();
    assert_eq!(
        (p.cloud_project.unwrap().id, p.environment.unwrap().id),
        (P2.to_string(), E2.to_string())
    );
    // An environment alone: the project it belongs to.
    let p = use_project(&h.ctx, use_req(&r.checkout, None, Some(E1_DEV)))
        .await
        .unwrap();
    assert_eq!(
        (p.cloud_project.unwrap().id, p.environment.unwrap().id),
        (P1.to_string(), E1_DEV.to_string())
    );
    // Both, together.
    let p = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    assert_eq!(p.environment.unwrap().id, E1);
    // An environment of another project: refused.
    let err = use_project(&h.ctx, use_req(&r.checkout, Some(P2), Some(E1_DEV)))
        .await
        .unwrap_err();
    assert_eq!(err.code, "INVALID_REQUEST");
    assert!(err.message.contains("does not belong"));
    // An unknown environment.
    let err = use_project(
        &h.ctx,
        use_req(
            &r.checkout,
            None,
            Some("99999999-9999-4999-8999-999999999999"),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "INVALID_REQUEST");
}

#[tokio::test]
async fn an_environment_on_its_own_branch_gets_a_fix_branch_never_the_environments() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    // The branch is not on origin yet: the fix branch starts from the default branch.
    let p = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1_DEV)))
        .await
        .unwrap();
    let ws = p.workspace.unwrap();
    assert_eq!(
        ws.branch, "iii-cloud/ready-dev",
        "the agent never works on the environment's branch"
    );
    assert_eq!(
        git(std::path::Path::new(&ws.path), &["rev-parse", "HEAD"]),
        git(&r.origin, &["rev-parse", "main"])
    );
    assert_eq!(
        git(
            std::path::Path::new(&ws.path),
            &["rev-parse", "--abbrev-ref", "HEAD"]
        ),
        "iii-cloud/ready-dev"
    );
    assert_eq!(p.default_branch.as_deref(), Some("main"));
    // A default-branch env still works from a pull request branch.
    let prod = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(
        prod.branch, "iii-cloud/ready-production-2",
        "a name that does not clash with the env branch"
    );
}

#[tokio::test]
async fn an_older_worktree_on_the_environments_branch_moves_to_the_fix_branch_with_its_work() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    // An older worker made the direct worktree on the environment's branch, and
    // the agent committed there and left a change.
    let old = r.checkout.parent().unwrap().join("hello-iii-iii-cloud");
    git(
        &r.checkout,
        &[
            "worktree",
            "add",
            "-b",
            DEV_BRANCH,
            old.to_str().unwrap(),
            "origin/main",
        ],
    );
    std::fs::write(old.join("fix.txt"), "fix").unwrap();
    git(&old, &["add", "."]);
    git(&old, &["commit", "-m", "agent fix"]);
    let fix = git(&old, &["rev-parse", "HEAD"]);
    std::fs::write(old.join("wip.txt"), "wip").unwrap();
    let ws = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1_DEV)))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(
        std::path::PathBuf::from(&ws.path).canonicalize().unwrap(),
        old.canonicalize().unwrap(),
        "the same worktree"
    );
    assert_eq!(ws.branch, "iii-cloud/ready-dev");
    assert_eq!(git(&old, &["rev-parse", "--abbrev-ref", "HEAD"]), ws.branch);
    assert_eq!(
        git(&old, &["rev-parse", "HEAD"]),
        fix,
        "its commit came along"
    );
    assert!(old.join("wip.txt").exists(), "and its uncommitted change");
    assert_eq!(
        git(&r.checkout, &["rev-parse", DEV_BRANCH]),
        fix,
        "the environment's branch keeps it too"
    );
    // Not an earlier try: nothing marks its commit as old.
    assert!(h.ctx.store.read(|s| {
        !s.worktree_started
            .contains_key(&format!("{}\n{}", ws.path, ws.branch))
    }));
    // Used again, it stays on the fix branch.
    let again = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1_DEV)))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!((again.path, again.branch), (ws.path, ws.branch));
}

#[tokio::test]
async fn a_worktree_the_person_made_on_the_environments_branch_is_left_alone() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    let mine = r.checkout.parent().unwrap().join("my-dev");
    git(
        &r.checkout,
        &[
            "worktree",
            "add",
            "-b",
            DEV_BRANCH,
            mine.to_str().unwrap(),
            "origin/main",
        ],
    );
    let ws = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1_DEV)))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(ws.branch, "iii-cloud/ready-dev");
    assert_ne!(
        std::path::PathBuf::from(&ws.path).canonicalize().unwrap(),
        mine.canonicalize().unwrap(),
        "a worktree of its own beside the person's"
    );
    assert_eq!(
        git(&mine, &["rev-parse", "--abbrev-ref", "HEAD"]),
        DEV_BRANCH,
        "the person's worktree stays on its branch"
    );
}

#[tokio::test]
async fn an_environment_branch_that_exists_on_origin_is_checked_out_from_there() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    // An earlier push left the env's branch on origin, one commit ahead of main.
    let t = tempfile::tempdir().unwrap();
    git(t.path(), &["clone", "-q", r.origin.to_str().unwrap(), "c"]);
    let c = t.path().join("c");
    git(&c, &["checkout", "-b", DEV_BRANCH]);
    std::fs::write(c.join("dev.txt"), "d").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "dev work"]);
    git(&c, &["push", "-q", "origin", DEV_BRANCH]);
    let ws = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1_DEV)))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(
        git(std::path::Path::new(&ws.path), &["rev-parse", "HEAD"]),
        git(&r.origin, &["rev-parse", DEV_BRANCH])
    );
    assert!(std::path::Path::new(&ws.path).join("dev.txt").exists());
}

fn select(h: &Harness) -> Project {
    h.ctx.store.read(|s| s.project.clone()).unwrap()
}

#[tokio::test]
async fn a_new_project_is_imported_from_a_choosable_manifest_then_selected() {
    let server = MockServer::start().await;
    let r = repo();
    let h2 = cloud(&server, true).await;
    get(&server, &format!("/teams/{TEAM}/github/repositories"), json!({"repositories": [{"id": 77, "full_name": "ada/hello-iii", "default_branch": "main", "private": true, "manifests": [{"path": "worker-compose.yaml", "lock": true, "workers": [], "choosable": true, "issue": null, "suggested_environment": "production"}, {"path": "plural-compose.yaml", "lock": false, "workers": [], "choosable": false, "issue": "PLURAL_FILENAME", "suggested_environment": null}]}]})).await;
    get_first(&server, &format!("/teams/{TEAM}/projects"), json!({"projects": [project(P1, "registry", "2026-11-01T00:00:00Z"), project(P3, "hello-iii", "2026-11-26T00:00:00Z")]})).await;
    get(
        &server,
        &format!("/teams/{TEAM}/projects/{P3}/environments"),
        json!({"environments": [env(E3, "production", true, "main")]}),
    )
    .await;
    get(
        &server,
        &format!("/teams/{TEAM}/environments/{E3}"),
        env(E3, "production", true, "main"),
    )
    .await;
    get(
        &server,
        &format!("/teams/{TEAM}/environments/{E3}/deployments"),
        json!({"deployments": [], "next_cursor": null}),
    )
    .await;
    Mock::given(method("POST")).and(path(format!("/api/v1/teams/{TEAM}/projects"))).respond_with(ResponseTemplate::new(201).set_body_json(json!({"project": project(P3, "hello-iii", "2026-11-26T00:00:00Z"), "environment": env(E3, "production", true, "main")}))).expect(1).mount(&server).await;

    let req = ProjectCreateRequest {
        path: r.checkout.to_string_lossy().into(),
        team_id: TEAM.into(),
        manifest: "worker-compose.yaml".into(),
        environment_name: "production".into(),
        production: None,
    };
    // A manifest that is not choosable is refused before anything is created.
    let bad = ProjectCreateRequest {
        manifest: "plural-compose.yaml".into(),
        ..req.clone()
    };
    assert_eq!(
        create_project(&h2.ctx, bad).await.unwrap_err().code,
        "MANIFEST_NOT_FOUND"
    );
    let missing = ProjectCreateRequest {
        manifest: "nope.yaml".into(),
        ..req.clone()
    };
    assert_eq!(
        create_project(&h2.ctx, missing).await.unwrap_err().code,
        "MANIFEST_NOT_FOUND"
    );
    assert_eq!(
        create_project(
            &h2.ctx,
            ProjectCreateRequest {
                environment_name: "Bad Name".into(),
                ..req.clone()
            }
        )
        .await
        .unwrap_err()
        .code,
        "INVALID_REQUEST"
    );
    // The real one.
    let out = create_project(&h2.ctx, req).await.unwrap();
    assert_eq!(
        (out.project_id.as_deref(), out.environment_id.as_deref()),
        (Some(P3), Some(E3)),
        "the new ids, explicitly"
    );
    let p = out.project.clone();
    assert!(out.cloud_projects.iter().any(|c| c.id == P3));
    assert_eq!(
        (
            p.cloud_project.as_ref().unwrap().id.as_str(),
            p.environment.as_ref().unwrap().id.as_str()
        ),
        (P3, E3)
    );
    assert!(
        p.cloud_projects.iter().any(|c| c.id == P3),
        "the answer carries the fresh cloud_projects[]"
    );
    assert!(p.workspace.is_some(), "selected with project::use");
    assert_eq!(select(&h2).environment.unwrap().id, E3);
    let posts: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|q| q.method.as_str() == "POST")
        .collect();
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(
        body,
        json!({"repository_id": 77, "manifest": "worker-compose.yaml", "environment_name": "production", "production": true})
    );
    assert!(
        posts[0]
            .headers
            .get("idempotency-key")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("iii-cloud:project:")
    );
    // The cloud is asked about this repository only: for the whole list it makes a trip to GitHub per
    // repository (25 s for a team of 71), in the folder's first read and again in the create.
    let asked: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/github/repositories"))
        .collect();
    assert!(asked.len() >= 2, "{asked:?}");
    assert!(
        asked
            .iter()
            .all(|r| r.url.query() == Some("full_name=ada/hello-iii")),
        "{asked:?}"
    );
}

#[tokio::test]
async fn a_new_environment_defaults_to_the_default_branch_the_selected_manifest_and_a_copy_of_production()
 {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    let new_env = "ccccccc1-0000-4000-8000-000000000009";
    get_first(&server, &format!("/teams/{TEAM}/projects/{P1}/environments"), json!({"environments": [env(E1, "production", true, "main"), env(E1_DEV, "dev", false, DEV_BRANCH), env(new_env, "staging", false, "main")]})).await;
    get(
        &server,
        &format!("/teams/{TEAM}/environments/{new_env}"),
        env(new_env, "staging", false, "main"),
    )
    .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/api/v1/teams/{TEAM}/projects/{P1}/environments"
        )))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(env(new_env, "staging", false, "main")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let req = EnvironmentCreateRequest {
        cloud_project_id: P1.into(),
        name: "staging".into(),
        branch: None,
        manifest: None,
        copy_from: None,
        production: None,
        start_empty: None,
    };
    let out = create_environment(&h.ctx, req).await.unwrap();
    assert_eq!(
        (out.project_id.as_deref(), out.environment_id.as_deref()),
        (Some(P1), Some(new_env))
    );
    assert!(
        out.cloud_projects
            .iter()
            .any(|c| c.id == P1 && c.environments.iter().any(|e| e.id == new_env))
    );
    assert_eq!(
        out.project.environment.unwrap().id,
        new_env,
        "selected with project::use"
    );
    let post = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|q| q.method.as_str() == "POST")
        .unwrap();
    let body: Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(
        body,
        json!({"name": "staging", "manifest": "worker-compose.yaml", "branch": "main", "production": false, "copy_from": E1})
    );
    assert_eq!(
        post.headers
            .get("idempotency-key")
            .unwrap()
            .to_str()
            .unwrap(),
        format!("iii-cloud:env:{P1}:staging:0")
    );
    // Bad input never reaches the cloud.
    let bad = EnvironmentCreateRequest {
        cloud_project_id: P1.into(),
        name: "Not Ok".into(),
        branch: None,
        manifest: None,
        copy_from: None,
        production: None,
        start_empty: None,
    };
    assert_eq!(
        create_environment(&h.ctx, bad).await.unwrap_err().code,
        "INVALID_REQUEST"
    );
    let unknown = EnvironmentCreateRequest {
        cloud_project_id: "nope".into(),
        name: "x".into(),
        branch: None,
        manifest: None,
        copy_from: None,
        production: None,
        start_empty: None,
    };
    assert_eq!(
        create_environment(&h.ctx, unknown).await.unwrap_err().code,
        "INVALID_REQUEST"
    );
}

#[test]
fn the_clouds_refusals_of_a_create_are_clear_codes() {
    use iii_cloud::create::create_error;
    let body = |code: &str, message: &str| json!({"code": code, "message": message});
    assert_eq!(
        environment_error(409, &body("CONFLICT", "The name is taken.")).code,
        "ENV_NAME_TAKEN"
    );
    assert_eq!(
        create_error(409, &body("CONFLICT", "taken"), "PROJECT_NAME_TAKEN").code,
        "PROJECT_NAME_TAKEN"
    );
    assert_eq!(
        environment_error(
            422,
            &body("VALIDATION_FAILED", "branch: not a valid git branch name")
        )
        .code,
        "INVALID_BRANCH"
    );
    assert_eq!(
        environment_error(
            422,
            &body("VALIDATION_FAILED", "branch: not found in the repository")
        )
        .code,
        "BRANCH_NOT_FOUND"
    );
    assert_eq!(
        environment_error(
            422,
            &body(
                "VALIDATION_FAILED",
                "manifest: the file is not in the repository"
            )
        )
        .code,
        "MANIFEST_NOT_FOUND"
    );
    assert_eq!(
        environment_error(403, &body("FORBIDDEN", "Only Owners and Admins can.")).code,
        "TEAM_FORBIDDEN"
    );
    assert_eq!(
        environment_error(
            403,
            &body(
                "LIMIT_EXCEEDED",
                "The environment limit of the plan is reached."
            )
        )
        .code,
        "PLAN_LIMIT"
    );
    assert_eq!(
        environment_error(402, &body("PAYMENT_REQUIRED", "Upgrade.")).code,
        "PLAN_LIMIT"
    );
    // Anything else keeps the cloud's own error.
    let other = environment_error(500, &body("INTERNAL", "down"));
    assert_eq!(
        (other.code.as_str(), other.status),
        ("CLOUD_ERROR", Some(500))
    );
    assert_eq!(
        environment_error(422, &body("VALIDATION_FAILED", "name: too long")).code,
        "CLOUD_ERROR"
    );
}

#[tokio::test]
async fn a_branch_the_repository_does_not_have_is_refused_before_anything_is_created() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path_regex(r"/environments$"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    let req = EnvironmentCreateRequest {
        cloud_project_id: P1.into(),
        name: "qa".into(),
        branch: Some("no-such-branch".into()),
        manifest: None,
        copy_from: None,
        production: None,
        start_empty: None,
    };
    assert_eq!(
        create_environment(&h.ctx, req).await.unwrap_err().code,
        "BRANCH_NOT_FOUND"
    );
}

#[tokio::test]
async fn refresh_reads_the_environments_again_and_lists_console_deploys_as_such() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1_DEV)))
        .await
        .unwrap();
    // A `qa` env made later in the console, and a deploy started there that failed.
    server.reset().await;
    let h2_get = |p: String, body: Value| {
        let server = &server;
        async move { get(server, &p, body).await }
    };
    h2_get("/me".into(), json!({"user": {"id": "u", "email": "e"}, "teams": [{"id": TEAM, "name": "Acme", "slug": "acme", "personal": false, "role": "owner"}]})).await;
    h2_get(
        format!("/teams/{TEAM}/projects"),
        json!({"projects": [project(P1, "registry", "2026-11-01T00:00:00Z")]}),
    )
    .await;
    let envs = json!({"environments": [env(E1, "production", true, "main"), env(E1_DEV, "dev", false, DEV_BRANCH), env("ddddddd1-0000-4000-8000-000000000001", "qa", false, "qa")]});
    h2_get(format!("/teams/{TEAM}/projects/{P1}/environments"), envs).await;
    h2_get(
        format!("/teams/{TEAM}/environments/{E1_DEV}"),
        env(E1_DEV, "dev", false, DEV_BRANCH),
    )
    .await;
    let failed = json!({"id": "44444444-4444-4444-8444-444444444444", "number": 44, "environment_id": E1_DEV, "type": "manual", "status": "failed", "status_detail": null, "commit": {"sha": "86c49e1c", "branch": DEV_BRANCH},
        "stages": [{"name": "workers", "status": "failed", "detail": null, "started_at": null, "finished_at": null}],
        "error": {"stage": "workers", "worker": "api", "code": "RELEASE_FAILED", "hint": "The release command failed.", "action": null}, "created_at": "2026-11-25T23:00:00Z"});
    h2_get(
        format!("/teams/{TEAM}/environments/{E1_DEV}/deployments"),
        json!({"deployments": [failed], "next_cursor": null}),
    )
    .await;
    for other in [
        E1.to_string(),
        "ddddddd1-0000-4000-8000-000000000001".to_string(),
    ] {
        h2_get(
            format!("/teams/{TEAM}/environments/{other}/deployments"),
            json!({"deployments": [], "next_cursor": null}),
        )
        .await;
    }
    Mock::given(method("GET")).and(path_regex(r"/logs$")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"lines": [{"message": "> api@ db:migrate"}, {"message": "ELIFECYCLE Command failed with exit code 1."}], "next_cursor": null}))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path_regex(r"/health$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "healthy", "engine": "ready", "workers": []})),
        )
        .mount(&server)
        .await;
    let status = iii_cloud::status::status(
        &h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap();
    let names: Vec<&str> = status
        .project
        .as_ref()
        .unwrap()
        .environments
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["production", "dev", "qa"],
        "an environment made later shows"
    );
    // Each environment's branch head on origin: the page sees a branch that moved.
    let heads: Vec<(String, Option<String>)> = status.project.as_ref().unwrap().cloud_projects[0]
        .environments
        .iter()
        .map(|e| (e.name.clone(), e.head.clone()))
        .collect();
    assert_eq!(
        heads[0],
        (
            "production".into(),
            Some(git(&r.origin, &["rev-parse", "main"]))
        )
    );
    assert_eq!(heads[2], ("qa".into(), None), "origin has no such branch");
    assert!(
        status
            .project
            .unwrap()
            .cloud_projects
            .iter()
            .all(|c| c.id != P2),
        "cloud_projects is read fresh too"
    );
    let d = status
        .deploys
        .iter()
        .find(|d| d.number == 44)
        .expect("the console deploy shows");
    assert!(d.from_console, "this worker did not create it");
    assert_eq!(d.outcome, Outcome::Failed);
    assert_eq!(d.failure.as_ref().unwrap().code, "RELEASE_FAILED");
}

#[tokio::test]
async fn a_retry_after_a_refusal_is_a_new_idempotency_key() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    let new_env = "ccccccc1-0000-4000-8000-000000000009";
    get_first(&server, &format!("/teams/{TEAM}/projects/{P1}/environments"), json!({"environments": [env(E1, "production", true, "main"), env(E1_DEV, "dev", false, DEV_BRANCH), env(new_env, "staging", false, "main")]})).await;
    get(
        &server,
        &format!("/teams/{TEAM}/environments/{new_env}"),
        env(new_env, "staging", false, "main"),
    )
    .await;
    let post_path = format!("/api/v1/teams/{TEAM}/projects/{P1}/environments");
    // The first try is refused (the name was taken a moment ago); the second is accepted.
    Mock::given(method("POST"))
        .and(path(post_path.clone()))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(json!({"code": "CONFLICT", "message": "taken"})),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(post_path))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(env(new_env, "staging", false, "main")),
        )
        .mount(&server)
        .await;
    let req = || EnvironmentCreateRequest {
        cloud_project_id: P1.into(),
        name: "staging".into(),
        branch: None,
        manifest: None,
        copy_from: None,
        production: None,
        start_empty: None,
    };
    assert_eq!(
        create_environment(&h.ctx, req()).await.unwrap_err().code,
        "ENV_NAME_TAKEN"
    );
    create_environment(&h.ctx, req()).await.unwrap();
    let keys: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|q| q.method.as_str() == "POST")
        .map(|q| {
            q.headers
                .get("idempotency-key")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        keys,
        vec![
            format!("iii-cloud:env:{P1}:staging:0"),
            format!("iii-cloud:env:{P1}:staging:1")
        ]
    );
}

#[tokio::test]
async fn a_repository_the_cloud_sends_without_an_id_is_an_error_not_zero() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    get_first(
        &server,
        &format!("/teams/{TEAM}/github/repositories"),
        json!({"repositories": [{"full_name": "ada/hello-iii", "manifests": [{"path": "worker-compose.yaml", "choosable": true}]}]}),
    )
    .await;
    let req = ProjectCreateRequest {
        path: r.checkout.to_string_lossy().into(),
        team_id: TEAM.into(),
        manifest: "worker-compose.yaml".into(),
        environment_name: "production".into(),
        production: None,
    };
    resolve(&h.ctx, resolve_req(&r.checkout)).await.ok();
    let err = create_project(&h.ctx, req).await.unwrap_err();
    assert_eq!(err.code, "CLOUD_ERROR");
    assert!(err.message.contains("without an id"));
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|q| q.method.as_str() != "POST"),
        "nothing was created"
    );
}

fn with(mut v: Value, key: &str, val: Value) -> Value {
    v[key] = val;
    v
}

fn project_of(id: &str, name: &str, repo: &str, removed: bool) -> Value {
    let mut p = json!({"id": id, "name": name, "endpoint_slug": name, "created_at": "2026-11-01T00:00:00Z", "repository": {"id": 5, "full_name": repo, "default_branch": "main"}});
    if removed {
        p["delete_after"] = json!("2026-12-02T00:00:00Z");
    }
    p
}

#[tokio::test]
async fn removed_projects_and_environments_are_dropped_and_other_repositories_are_listed_apart() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    let other = "ddddddd1-0000-4000-8000-0000000000aa";
    let old = "ddddddd1-0000-4000-8000-0000000000bb";
    let removed = Arc::new(AtomicBool::new(false));
    let flag = removed.clone();
    let live = json!({"projects": [
        project(P1, "registry", "2026-11-01T00:00:00Z"),
        project_of(old, "registry-old", "ada/hello-iii", true),
        project_of(other, "workers", "iii-hq/workers", false),
        project_of("ddddddd1-0000-4000-8000-0000000000cc", "gone-workers", "iii-hq/gone", true),
    ]});
    let gone = json!({"projects": [project_of(P1, "registry", "ada/hello-iii", true)]});
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/projects")))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(if flag.load(Ordering::SeqCst) {
                gone.clone()
            } else {
                live.clone()
            })
        })
        .with_priority(1)
        .mount(&server)
        .await;
    get_first(&server, &format!("/teams/{TEAM}/projects/{P1}/environments"), json!({"environments": [
        env(E1, "production", true, "main"),
        with(env(E1_DEV, "dev", false, DEV_BRANCH), "status", json!("deleting")),
        with(env("ccccccc1-0000-4000-8000-0000000000dd", "old", false, "main"), "delete_after", json!("2026-12-02T00:00:00Z")),
    ]})).await;
    let p = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    assert_eq!(
        p.cloud_projects
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        vec![P1]
    );
    assert_eq!(
        p.cloud_projects[0]
            .environments
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        vec!["production"]
    );
    assert_eq!(
        p.other_cloud_projects,
        vec![OtherCloudProject {
            id: other.into(),
            name: "workers".into(),
            team: TeamName {
                id: TEAM.into(),
                name: "Acme".into()
            },
            repository: "iii-hq/workers".into(),
        }]
    );
    // A project of another repository can't be used with this folder.
    let err = use_project(&h.ctx, use_req(&r.checkout, Some(other), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "REPOSITORY_MISMATCH");
    assert!(err.message.contains("workers deploys iii-hq/workers"));
    // Nothing is wrong yet.
    let status = iii_cloud::status::status(
        &h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap();
    assert!(status.problems.is_empty());
    assert_eq!(status.project.unwrap().other_cloud_projects.len(), 1);
    // The cloud removes the selected environment, then the project.
    get_first(
        &server,
        &format!("/teams/{TEAM}/environments/{E1}"),
        with(
            env(E1, "production", true, "main"),
            "delete_after",
            json!("2026-12-02T00:00:00Z"),
        ),
    )
    .await;
    removed.store(true, Ordering::SeqCst);
    let status = iii_cloud::status::status(
        &h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap();
    let codes: Vec<_> = status.problems.iter().map(|p| p.code.as_str()).collect();
    assert_eq!(codes, vec!["PROJECT_DELETED", "ENVIRONMENT_DELETED"]);
    assert!(
        status.problems[0]
            .message
            .contains("removed in iii cloud; choose another")
    );
}

#[tokio::test]
async fn an_importable_repository_lists_each_compose_files_workers_and_cloud_section() {
    let server = MockServer::start().await;
    let r = repo();
    std::fs::write(
        r.checkout.join("worker-compose-prod.yaml"),
        "containers:\n  api:\n    worker: path://./api\n  http:\n    worker: package://http\ncloud: {}\n",
    )
    .unwrap();
    let h = cloud(&server, false).await;
    get_first(
        &server,
        &format!("/teams/{TEAM}/projects"),
        json!({"projects": []}),
    )
    .await;
    get(&server, &format!("/teams/{TEAM}/github/repositories"), json!({"repositories": [{"id": 77, "full_name": "ada/hello-iii", "default_branch": "main", "private": true, "manifests": [
        {"path": "worker-compose.yaml", "lock": true, "workers": ["stale"], "choosable": true, "issue": null, "suggested_environment": "production"},
        {"path": "worker-compose-prod.yaml", "lock": false, "workers": [], "choosable": true, "issue": null, "suggested_environment": null},
        {"path": "api/worker-compose-next.yaml", "lock": false, "workers": ["api", "http", "cron"], "choosable": true, "issue": null, "suggested_environment": null}]}]})).await;
    let p = resolve(&h.ctx, resolve_req(&r.checkout)).await.unwrap();
    let rows: Vec<(String, Option<u32>, bool)> = p
        .importable
        .unwrap()
        .manifests
        .into_iter()
        .map(|m| (m.path, m.workers, m.has_cloud_block))
        .collect();
    assert_eq!(
        rows,
        vec![
            // The files in this checkout, as they are here.
            ("worker-compose.yaml".into(), Some(0), false),
            ("worker-compose-prod.yaml".into(), Some(2), true),
            // Not in this checkout: the cloud's list, of the default branch.
            ("api/worker-compose-next.yaml".into(), Some(3), false),
        ]
    );
}

#[tokio::test]
async fn a_new_environment_that_starts_empty_copies_nothing() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    let new_env = "ccccccc1-0000-4000-8000-000000000009";
    get_first(&server, &format!("/teams/{TEAM}/projects/{P1}/environments"), json!({"environments": [env(E1, "production", true, "main"), env(E1_DEV, "dev", false, DEV_BRANCH), env(new_env, "staging", false, "main")]})).await;
    get(
        &server,
        &format!("/teams/{TEAM}/environments/{new_env}"),
        env(new_env, "staging", false, "main"),
    )
    .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/api/v1/teams/{TEAM}/projects/{P1}/environments"
        )))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(env(new_env, "staging", false, "main")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let req = EnvironmentCreateRequest {
        cloud_project_id: P1.into(),
        name: "staging".into(),
        branch: None,
        manifest: None,
        copy_from: None,
        production: None,
        start_empty: Some(true),
    };
    // Not both: an empty start copies from nobody.
    let both = EnvironmentCreateRequest {
        copy_from: Some(E1.into()),
        ..req.clone()
    };
    assert_eq!(
        create_environment(&h.ctx, both).await.unwrap_err().code,
        "INVALID_REQUEST"
    );
    create_environment(&h.ctx, req).await.unwrap();
    let post = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|q| q.method.as_str() == "POST")
        .unwrap();
    let body: Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(
        body,
        json!({"name": "staging", "manifest": "worker-compose.yaml", "branch": "main", "production": false}),
        "no copy_from"
    );
}

#[tokio::test]
async fn a_folder_used_before_is_offered_again_and_switched_to_with_use_alone() {
    let server = MockServer::start().await;
    let (r1, r2) = (repo(), repo());
    let h = cloud(&server, true).await;
    let recent = || async {
        iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .recent_projects
    };
    use_project(&h.ctx, use_req(&r1.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    assert!(recent().await.is_empty(), "the current one is not listed");
    let second = use_project(&h.ctx, use_req(&r2.checkout, Some(P1), Some(E1_DEV)))
        .await
        .unwrap();
    let first = recent().await;
    assert_eq!(first.len(), 1);
    let e = &first[0];
    assert_eq!(
        (
            std::path::PathBuf::from(&e.path).canonicalize().unwrap(),
            e.repository.as_str(),
            e.cloud_project_id.as_str(),
            e.cloud_project_name.as_str(),
            e.team_name.as_deref(),
            e.environment_id.as_deref(),
            e.environment_name.as_deref(),
            e.mode,
        ),
        (
            r1.checkout.canonicalize().unwrap(),
            "ada/hello-iii",
            P1,
            "registry",
            Some("Acme"),
            Some(E1),
            Some("production"),
            WorkspaceMode::Worktree,
        )
    );
    // Back to it from the entry alone: `use` resolves the other folder itself.
    let back = use_project(
        &h.ctx,
        UseRequest {
            path: e.path.clone(),
            environment_id: e.environment_id.clone(),
            mode: e.mode,
            cloud_project_id: Some(e.cloud_project_id.clone()),
            from: None,
            source: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(back.path, e.path);
    assert_eq!(back.environment.unwrap().id, E1);
    let listed: Vec<String> = recent().await.into_iter().map(|r| r.path).collect();
    assert_eq!(
        listed,
        vec![second.path.clone()],
        "the current one left the list"
    );
    // A folder that is gone is not offered.
    std::fs::remove_dir_all(&r2.checkout).unwrap();
    assert!(recent().await.is_empty());
}

#[tokio::test]
async fn refresh_says_when_the_project_in_use_last_deployed_even_if_this_environment_never_did() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    // dev never deployed; production of the same project did.
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1_DEV)))
        .await
        .unwrap();
    let project = iii_cloud::status::status(
        &h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap()
    .project
    .unwrap();
    let at = |id: &str| {
        project
            .cloud_projects
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.last_deployment_at.clone())
    };
    assert_eq!(at(P1).as_deref(), Some("2026-11-25T10:00:00Z"));
    assert_eq!(
        project
            .cloud_project
            .as_ref()
            .and_then(|c| c.last_deployment_at.as_deref()),
        Some("2026-11-25T10:00:00Z")
    );
    assert_eq!(at(P2), None, "only the project in use is read");
}

#[tokio::test]
async fn an_environment_the_project_no_longer_lists_falls_back_to_its_default_and_says_so() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    let gone = "99999999-9999-4999-8999-999999999999";
    let p = use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(gone)))
        .await
        .unwrap();
    assert_eq!(p.environment.as_ref().unwrap().id, E1);
    assert_eq!(p.fell_back_from.as_deref(), Some(gone));
    assert_eq!(select(&h).fell_back_from, None, "only in the answer");
    // The person's team is kept with the entry, for reading it from another project.
    assert_eq!(
        h.ctx
            .store
            .read(|s| s.recent_projects[0].team_id.clone())
            .as_deref(),
        Some(TEAM)
    );
}

#[tokio::test]
async fn a_recent_project_iii_cloud_no_longer_lists_is_dropped_only_when_its_team_is_still_yours() {
    let server = MockServer::start().await;
    let r = repo();
    let h = cloud(&server, true).await;
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    let gone = "aaaaaaa9-0000-4000-8000-000000000009";
    let remember = |team: Option<&str>| {
        h.ctx.store.update(|s| {
            s.remember_project(RecentProject {
                path: r.checkout.to_string_lossy().into(),
                repository: "ada/hello-iii".into(),
                cloud_project_id: gone.into(),
                cloud_project_name: "billing".into(),
                team_id: team.map(str::to_owned),
                team_name: None,
                environment_id: None,
                environment_name: None,
                mode: WorkspaceMode::Worktree,
                used_at: "t".into(),
            })
        })
    };
    let kept = || {
        h.ctx
            .store
            .read(|s| s.recent_projects.iter().any(|e| e.cloud_project_id == gone))
    };
    let before = select(&h);
    let try_it = || async {
        let e = use_project(&h.ctx, use_req(&r.checkout, Some(gone), None))
            .await
            .unwrap_err();
        (e.code, e.message)
    };
    // Out of the team that owned it: not gone for everyone, so it stays.
    remember(Some("dddddddd-0000-4000-8000-000000000001"));
    assert_eq!(
        try_it().await,
        (
            "NO_ACCESS".into(),
            "You're no longer in the team that owns billing.".into()
        )
    );
    assert!(kept());
    // No team known: said plainly, and kept.
    remember(None);
    assert_eq!(
        try_it().await,
        (
            "PROJECT_DELETED".into(),
            "billing no longer exists in iii cloud.".into()
        )
    );
    assert!(kept());
    // Still in its team: gone, and dropped.
    remember(Some(TEAM));
    assert_eq!(try_it().await.0, "PROJECT_DELETED");
    assert!(!kept());
    assert_eq!(select(&h), before, "the selection stays");
}

/// The importable-repository answers `create_project` reads, and the new project P3.
async fn importable(server: &MockServer) {
    get(server, &format!("/teams/{TEAM}/github/repositories"), json!({"repositories": [{"id": 77, "full_name": "ada/hello-iii", "default_branch": "main", "private": true, "manifests": [{"path": "worker-compose.yaml", "lock": true, "workers": [], "choosable": true, "issue": null, "suggested_environment": "production"}]}]})).await;
    get_first(server, &format!("/teams/{TEAM}/projects"), json!({"projects": [project(P1, "registry", "2026-11-01T00:00:00Z"), project(P3, "hello-iii", "2026-11-26T00:00:00Z")]})).await;
    get(
        server,
        &format!("/teams/{TEAM}/projects/{P3}/environments"),
        json!({"environments": [env(E3, "production", true, "main")]}),
    )
    .await;
    get(
        server,
        &format!("/teams/{TEAM}/environments/{E3}/deployments"),
        json!({"deployments": [], "next_cursor": null}),
    )
    .await;
    Mock::given(method("POST")).and(path(format!("/api/v1/teams/{TEAM}/projects"))).respond_with(ResponseTemplate::new(201).set_body_json(json!({"project": project(P3, "hello-iii", "2026-11-26T00:00:00Z"), "environment": env(E3, "production", true, "main")}))).expect(1).mount(server).await;
}

#[tokio::test]
async fn a_new_project_while_another_is_selected_selects_nothing_and_is_the_candidate() {
    let server = MockServer::start().await;
    let (r1, r2) = (repo(), repo());
    let h = cloud(&server, true).await;
    use_project(&h.ctx, use_req(&r1.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    importable(&server).await;
    let before = select(&h);
    let out = create_project(
        &h.ctx,
        ProjectCreateRequest {
            path: r2.checkout.to_string_lossy().into(),
            team_id: TEAM.into(),
            manifest: "worker-compose.yaml".into(),
            environment_name: "production".into(),
            production: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (out.project_id.as_deref(), out.environment_id.as_deref()),
        (Some(P3), Some(E3))
    );
    assert_eq!(out.project.cloud_project.as_ref().unwrap().id, P3);
    assert_eq!(out.project.workspace, None, "not selected");
    assert_eq!(select(&h), before);
    let candidate = h.ctx.store.read(|s| s.candidate.clone()).unwrap();
    assert_eq!(candidate, out.project);
    assert_eq!(
        std::path::PathBuf::from(&candidate.path),
        r2.checkout.canonicalize().unwrap()
    );
}

#[tokio::test]
async fn a_new_environment_of_the_candidates_project_is_made_there_and_not_selected() {
    let server = MockServer::start().await;
    let (r1, r2) = (repo(), repo());
    let h = cloud(&server, true).await;
    use_project(&h.ctx, use_req(&r1.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    // The selected folder's list is older than P2; the folder being added lists it.
    h.ctx.store.update(|s| {
        s.project
            .as_mut()
            .unwrap()
            .cloud_projects
            .retain(|c| c.id == P1)
    });
    resolve(&h.ctx, resolve_req(&r2.checkout)).await.unwrap();
    let before = select(&h);
    let new_env = "ccccccc2-0000-4000-8000-000000000010";
    get_first(
        &server,
        &format!("/teams/{TEAM}/projects/{P2}/environments"),
        json!({"environments": [env(E2, "production", true, "main"), env(new_env, "staging", false, "main")]}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/api/v1/teams/{TEAM}/projects/{P2}/environments"
        )))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(env(new_env, "staging", false, "main")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let out = create_environment(
        &h.ctx,
        EnvironmentCreateRequest {
            cloud_project_id: P2.into(),
            name: "staging".into(),
            branch: None,
            manifest: None,
            copy_from: None,
            production: None,
            start_empty: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (out.project_id.as_deref(), out.environment_id.as_deref()),
        (Some(P2), Some(new_env))
    );
    assert_eq!(
        std::path::PathBuf::from(&out.project.path),
        r2.checkout.canonicalize().unwrap()
    );
    assert!(out.project.environments.iter().any(|e| e.id == new_env));
    assert_eq!(select(&h), before, "nothing is selected");
    assert_eq!(h.ctx.store.read(|s| s.candidate.clone()), Some(out.project));
    let post = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|q| q.method.as_str() == "POST")
        .unwrap();
    let body: Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(
        body,
        json!({"name": "staging", "manifest": "worker-compose.yaml", "branch": "main", "production": false, "copy_from": E2})
    );
}

const DA: &str = "dddddddd-0000-4000-8000-00000000000a";

/// `E1`'s deployment `DA`, whose status flips to ready when `ready` is set.
async fn deployment_a(server: &MockServer, ready: Arc<AtomicBool>) {
    struct Flip(Arc<AtomicBool>);
    impl wiremock::Respond for Flip {
        fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
            let status = if self.0.load(Ordering::SeqCst) {
                "ready"
            } else {
                "in_progress"
            };
            ResponseTemplate::new(200).set_body_json(json!({"id": DA, "number": 3, "environment_id": E1, "type": "manual", "status": status, "status_detail": null,
                "commit": {"sha": "abc1234", "branch": "main", "message": "m"}, "stages": [], "error": null, "created_at": "2026-11-26T10:00:00Z", "finished_at": null}))
        }
    }
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/teams/{TEAM}/environments/{E1}/deployments/{DA}"
        )))
        .respond_with(Flip(ready))
        .mount(server)
        .await;
}

async fn outcome_of_a(ctx: &iii_cloud::ctx::Ctx, want: Outcome) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let got = ctx
            .store
            .read(|s| s.deploys.iter().find(|d| d.id == DA).map(|d| d.outcome));
        if got == Some(want) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{got:?}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// A deploy keeps moving when the person switches to another project: it is
/// listed with its names, followed to ready, and then leaves the list.
#[tokio::test]
async fn a_running_deploy_of_another_project_stays_listed_and_followed_until_ready() {
    let server = MockServer::start().await;
    let h = cloud(&server, false).await;
    let r = repo();
    let ready = Arc::new(AtomicBool::new(false));
    deployment_a(&server, ready.clone()).await;
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    let d = iii_cloud::watch::watch(
        &h.ctx,
        WatchRequest {
            handoff_id: None,
            deployment_id: Some(DA.into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(d.outcome, Outcome::Running);
    // The person switches to another project.
    use_project(&h.ctx, use_req(&r.checkout, Some(P2), Some(E2)))
        .await
        .unwrap();
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert!(
        status.deploys.iter().all(|d| d.id != DA),
        "the selection's own only"
    );
    let listed: Vec<_> = status
        .running_deploys
        .iter()
        .map(|r| {
            (
                r.deploy.id.as_str(),
                r.cloud_project_id.as_deref(),
                r.project_name.as_deref(),
                r.environment_name.as_deref(),
                r.repository.as_deref(),
                r.path.as_deref(),
            )
        })
        .collect();
    let path = r.checkout.to_string_lossy().into_owned();
    assert_eq!(
        listed,
        vec![(
            DA,
            Some(P1),
            Some("registry"),
            Some("production"),
            Some("ada/hello-iii"),
            Some(path.as_str())
        )]
    );
    // Still followed: it reaches ready, and leaves the list.
    ready.store(true, Ordering::SeqCst);
    outcome_of_a(&h.ctx, Outcome::Ready).await;
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert!(status.running_deploys.is_empty());
    assert!(
        h.ctx.store.read(|s| s.deploys.iter().any(|d| d.id == DA)),
        "kept"
    );
}

/// After a restart with nothing selected, the poller resumes a known running
/// deploy with its own team and environment.
#[tokio::test]
async fn a_restart_with_nothing_selected_resumes_a_running_deploy() {
    let server = MockServer::start().await;
    let h = cloud(&server, false).await;
    let r = repo();
    let ready = Arc::new(AtomicBool::new(false));
    deployment_a(&server, ready.clone()).await;
    use_project(&h.ctx, use_req(&r.checkout, Some(P1), Some(E1)))
        .await
        .unwrap();
    // Recorded (with where it came from) but not followed by this worker.
    let running = iii_cloud::watch::to_deploy(
        &server.uri(),
        (Some("acme"), Some("registry"), "production"),
        &json!({"id": DA, "number": 3, "environment_id": E1, "type": "manual", "status": "in_progress", "commit": {"sha": "abc1234"}, "stages": [], "created_at": "2026-11-26T10:00:00Z"}),
        None,
        false,
    )
    .unwrap();
    iii_cloud::watch::record(&h.ctx, &running);
    h.ctx.store.update(|s| s.project = None);
    // The restarted worker: the same state on disk, a new context.
    let cfg = iii_cloud::config::Config {
        cloud_url: server.uri(),
        data_dir: h.dir.path().join("data").to_string_lossy().into_owned(),
        ..iii_cloud::config::Config::default()
    };
    let store = Arc::new(iii_cloud::store::Store::open(&cfg.data_path()));
    let again = iii_cloud::ctx::Ctx::with_tick(
        Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
        store,
        h.fake.clone(),
        Some(std::time::Duration::from_millis(10)),
    );
    ready.store(true, Ordering::SeqCst);
    iii_cloud::watch::poll_once(&again).await;
    outcome_of_a(&again, Outcome::Ready).await;
}
