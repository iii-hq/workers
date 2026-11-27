//! `iii-cloud::values` and `iii-cloud::values::save` against a mock cloud: the
//! names a compose file reads, where values can come from, and the page's
//! save. No value from a file or the person ever comes back.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iii_cloud::types::*;
use iii_cloud::values::{save, values};
use serde_json::{Value, json};
use support::repo::*;
use support::{Fake, Harness, harness};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PROD: &str = "33333333-3333-4333-8333-333333333333";
const DEV: &str = "44444444-4444-4444-8444-444444444444";
const MANIFEST: &str = "api/worker-compose.yaml";
/// The secret strings in the person's env files: never in an answer.
const FILE_SECRET: &str = "gh-file-secret-value-0001";
const ROOT_SECRET: &str = "gh-root-secret-value-0002";
const PROD_SECRET: &str = "gh-prod-secret-value-0003";

const COMPOSE: &str = "containers:
  api:
    worker: path://./workers/api
    environment:
      GH_TOKEN: ${GH_TOKEN}
      APP_BASE_URL: ${APP_BASE_URL}
      DATABASE_URL: ${DATABASE_URL}
      SLACK_CHANNEL: ${SLACK_CHANNEL}
      OPTIONAL_FLAG: ${OPTIONAL_FLAG:-}
      RELEASE_SHA: ${RELEASE_SHA:-development}
      SET_ONE: ${SET_ONE}
      LOCAL_API: ${LOCAL_API}
      IIICLOUD_URL: ${IIICLOUD_URL}
      # GONE: ${GONE}
  web:
    worker: path://./workers/web
    environment:
      GH_TOKEN: ${GH_TOKEN}
";

struct World {
    h: Harness,
    server: MockServer,
    checkout: PathBuf,
    ws: PathBuf,
    _root: tempfile::TempDir,
}

async fn get(server: &MockServer, p: &str, status: u16, body: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1{p}")))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

fn vars(env: &str) -> String {
    format!("/teams/{TEAM}/environments/{env}/variables")
}

/// A browser session on `test` (not production) of `registry`, the person's
/// folder with its env files, and a worktree with the compose file.
async fn world(production: bool, role: &str) -> World {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let ws = root.path().join("worktree");
    for dir in [&checkout, &ws] {
        std::fs::create_dir_all(dir.join("api")).unwrap();
        std::fs::write(dir.join(MANIFEST), COMPOSE).unwrap();
    }
    // The worktree is a repository: its `.env` is written only where git says untracked.
    git(&ws, &["init", "-q", "-b", "main"]);
    let write = |rel: &str, text: &str| std::fs::write(checkout.join(rel), text).unwrap();
    write(
        "api/.env.test",
        &format!("GH_TOKEN={FILE_SECRET}\nSTILL=here\nDATABASE_URL=postgres://db.example/app\n"),
    );
    write(
        ".env.production",
        &format!("GH_TOKEN={PROD_SECRET}\nSLACK_CHANNEL=#ops-prod-room\n"),
    );
    write(
        ".env",
        &format!("GH_TOKEN={ROOT_SECRET}\nLOCAL_API=http://localhost:3000/api\n"),
    );
    write(".env.example", "GH_TOKEN=example-never-read\n");
    get(&server, &vars(ENV), 200, json!({"variables": [
        {"name": "SET_ONE", "type": "variable", "value": "on", "status": "set", "required_by": ["api"], "applies": null, "updated_at": "2026-11-20T00:00:00Z"},
        {"name": "SLACK_CHANNEL", "type": "variable", "value": null, "status": "missing", "required_by": ["api"], "applies": null, "updated_at": null},
        {"name": "GH_TOKEN", "type": "secret", "value": null, "status": "missing", "required_by": ["api"], "applies": null, "updated_at": null}
    ]}))
    .await;
    get(&server, &vars(PROD), 200, json!({"variables": [
        {"name": "SLACK_CHANNEL", "type": "secret", "value": null, "status": "set", "required_by": ["api"], "applies": null, "updated_at": null},
        {"name": "GH_TOKEN", "type": "secret", "value": null, "status": "missing", "required_by": ["api"], "applies": null, "updated_at": null}
    ]}))
    .await;
    get(
        &server,
        &vars(DEV),
        403,
        json!({"code": "FORBIDDEN", "message": "no"}),
    )
    .await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("IIICLOUD_SESSION", "iiia_tok"),
    );
    let mut conn = Connection::signed_out(&server.uri());
    conn.status = ConnStatus::Connected;
    conn.method = Some(Method::Browser);
    conn.teams = vec![TeamRole {
        id: TEAM.into(),
        name: "Acme".into(),
        slug: "acme".into(),
        role: role.into(),
    }];
    h.ctx.set_connection(conn);
    let (name, id) = match production {
        true => ("production", PROD),
        false => ("test", ENV),
    };
    let project: Project = serde_json::from_value(json!({
        "path": checkout, "repository": "ada/hello-iii",
        "checkout": {"branch": "main", "head": "h", "changed_files": [], "ahead": 0},
        "team": {"id": TEAM, "name": "Acme", "slug": "acme"}, "teams": [],
        "cloud_project": {"id": "p1", "name": "registry", "slug": "registry"},
        "environment": {"id": id, "name": name, "production": production, "branch": "main", "manifest": MANIFEST, "url": "https://test.example.app"},
        "environments": [
            {"id": ENV, "name": "test", "production": false, "branch": "main"},
            {"id": PROD, "name": "production", "production": true, "branch": "main"},
            {"id": DEV, "name": "dev", "production": false, "branch": "dev"}
        ],
        "importable": null, "github_access": true,
        "workspace": {"mode": "worktree", "path": ws, "branch": "iii-cloud/ready-test"}
    }))
    .unwrap();
    h.ctx.store.update(|s| s.project = Some(project));
    World {
        h,
        server,
        checkout,
        ws,
        _root: root,
    }
}

async fn read(w: &World) -> Values {
    values(&w.h.ctx, ValuesRequest::default()).await.unwrap()
}

fn item<'a>(v: &'a Values, name: &str) -> &'a ValueItem {
    v.items.iter().find(|i| i.name == name).unwrap()
}

fn file(file: &str, preselect: bool, local: bool) -> Suggestion {
    Suggestion::EnvFile {
        file: file.into(),
        preselect,
        local,
    }
}

#[tokio::test]
async fn the_answer_never_carries_a_value_from_the_persons_files() {
    let w = world(false, "developer").await;
    let v = read(&w).await;
    let text = serde_json::to_string(&v).unwrap();
    for secret in [
        FILE_SECRET,
        ROOT_SECRET,
        PROD_SECRET,
        "#ops-prod-room",
        "localhost:3000",
        "db.example",
        "example-never-read",
    ] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    // Nor anything the store keeps or an event carries.
    let stored = w.h.ctx.store.read(|s| serde_json::to_string(s).unwrap());
    assert!(!stored.contains(FILE_SECRET));
    let events = serde_json::to_string(&*w.h.fake.events.lock().unwrap()).unwrap();
    assert!(!events.contains(FILE_SECRET));
}

#[tokio::test]
async fn values_lists_the_files_names_with_their_state_and_where_values_can_come_from() {
    let w = world(false, "developer").await;
    std::fs::write(w.ws.join("api/.env"), "SET_ONE=on\n").unwrap();
    iii_cloud::values::skip(
        &w.h.ctx,
        ValuesSkip {
            environment_id: ENV.into(),
            names: vec!["APP_BASE_URL".into()],
            skip: true,
        },
    )
    .await
    .unwrap();
    let v = read(&w).await;
    assert_eq!(
        v.items
            .iter()
            .filter(|i| i.skipped)
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>(),
        vec!["APP_BASE_URL"]
    );
    // Whether the run here has it: its `.env`, the folder's files in the run's
    // order (`.env.test`, `.env.local`, `.env`, `.env.production`), or nothing.
    let here: Vec<(&str, Here, Option<&str>)> = v
        .items
        .iter()
        .map(|i| (i.name.as_str(), i.here, i.here_file.as_deref()))
        .collect();
    assert_eq!(
        here,
        vec![
            ("GH_TOKEN", Here::FromFile, Some("api/.env.test")),
            ("APP_BASE_URL", Here::Unset, None),
            ("DATABASE_URL", Here::Unset, None),
            ("SLACK_CHANNEL", Here::FromFile, Some(".env.production")),
            ("OPTIONAL_FLAG", Here::NotNeeded, None),
            ("RELEASE_SHA", Here::NotNeeded, None),
            ("SET_ONE", Here::Set, None),
            ("LOCAL_API", Here::FromFile, Some(".env")),
        ],
        "a database URL never comes from a file: the run makes its databases"
    );
    assert_eq!(
        (
            v.environment_id.as_str(),
            v.environment.as_str(),
            v.manifest.as_str()
        ),
        (ENV, "test", MANIFEST)
    );
    assert_eq!(
        (v.read_from, v.cloud, v.save),
        (ValuesFrom::Workspace, CloudRead::Read, SaveAllowed::Ok)
    );
    assert_eq!(
        v.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        vec![
            "GH_TOKEN",
            "APP_BASE_URL",
            "DATABASE_URL",
            "SLACK_CHANNEL",
            "OPTIONAL_FLAG",
            "RELEASE_SHA",
            "SET_ONE",
            "LOCAL_API",
        ],
        "IIICLOUD_* is iii cloud's own, and a comment is not read"
    );
    let gh = item(&v, "GH_TOKEN");
    assert_eq!(
        (gh.need, gh.kind, gh.in_cloud, gh.line),
        (Need::Required, ValueKind::Secret, InCloud::Unset, Some(5))
    );
    assert_eq!(gh.containers, vec!["api", "web"]);
    // `.env.<env>` first and preselected; `.env.production` only for production;
    // `.env` last; `.env.example` never.
    assert_eq!(
        gh.suggestions,
        vec![
            file("api/.env.test", true, false),
            file(".env.production", false, false),
            file(".env", true, false),
        ]
    );
    let url = item(&v, "APP_BASE_URL");
    assert_eq!(url.kind, ValueKind::Variable, "the environment's address");
    assert_eq!(
        url.suggestions,
        vec![Suggestion::EnvUrl {
            url: "https://test.example.app".into(),
            preselect: true
        }]
    );
    let db = item(&v, "DATABASE_URL");
    assert!(db.database);
    assert_eq!(
        db.suggestions,
        vec![file("api/.env.test", false, false)],
        "iii cloud makes the database: never preselected"
    );
    // Stored as a variable, unset here, set in production; dev can't be read.
    let slack = item(&v, "SLACK_CHANNEL");
    assert_eq!(
        (slack.kind, slack.in_cloud),
        (ValueKind::Variable, InCloud::Unset)
    );
    assert_eq!(
        slack.suggestions,
        vec![
            file(".env.production", false, false),
            Suggestion::Copy {
                environment_id: PROD.into(),
                environment: "production".into(),
                preselect: false
            },
        ]
    );
    let optional = item(&v, "OPTIONAL_FLAG");
    assert_eq!(
        (optional.need, optional.default.as_deref()),
        (Need::Optional, Some(""))
    );
    let release = item(&v, "RELEASE_SHA");
    assert_eq!(
        (release.need, release.default.as_deref()),
        (Need::HasDefault, Some("development"))
    );
    let set = item(&v, "SET_ONE");
    assert_eq!(
        (
            set.in_cloud,
            set.value.as_deref(),
            set.updated_at.as_deref()
        ),
        (InCloud::Set, Some("on"), Some("2026-11-20T00:00:00Z"))
    );
    // A value that points at this machine is not preselected from `.env`.
    assert_eq!(
        item(&v, "LOCAL_API").suggestions,
        vec![file(".env", false, true)]
    );
}

#[tokio::test]
async fn an_unreadable_cloud_is_unknown_and_an_ask_is_a_row() {
    let w = world(false, "developer").await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1{}", vars(ENV))))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"code": "INTERNAL"})))
        .with_priority(1)
        .mount(&w.server)
        .await;
    for name in ["GH_TOKEN"] {
        iii_cloud::secret::ask(
            &w.h.ctx,
            SecretAsk {
                handoff_id: None,
                name: name.into(),
                description: format!("why {name}"),
                mode: None,
                external: None,
            },
        )
        .await
        .unwrap();
    }
    let v = read(&w).await;
    assert_eq!(v.cloud, CloudRead::Unreadable);
    assert!(v.items.iter().all(|i| i.in_cloud == InCloud::Unknown));
    let asked = item(&v, "GH_TOKEN").asked.clone().unwrap();
    assert_eq!(asked.description, "why GH_TOKEN");
}

#[tokio::test]
async fn without_owner_in_production_or_with_an_api_key_save_sends_nothing() {
    let typed =
        json!([{"name": "GH_TOKEN", "from": "typed", "type": "secret", "value": "some-value"}]);
    let w = world(true, "developer").await;
    let v = read(&w).await;
    assert_eq!(v.save, SaveAllowed::Role);
    // Production's own file is preselected there.
    assert_eq!(
        item(&v, "GH_TOKEN").suggestions[0],
        file(".env.production", true, false)
    );
    let before = w.server.received_requests().await.unwrap().len();
    let err = save(
        &w.h.ctx,
        serde_json::from_value(json!({"environment_id": PROD, "items": typed})).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "ROLE_REQUIRED");
    assert_eq!(
        w.server.received_requests().await.unwrap().len(),
        before,
        "nothing reached iii cloud"
    );

    let w = world(true, "Owner").await;
    assert_eq!(read(&w).await.save, SaveAllowed::Ok);
    let mut conn = w.h.ctx.store.read(|s| s.connection.clone()).unwrap();
    conn.method = Some(Method::ApiKey);
    w.h.ctx.set_connection(conn);
    assert_eq!(read(&w).await.save, SaveAllowed::ApiKey);
    let before = w.server.received_requests().await.unwrap().len();
    let err = save(
        &w.h.ctx,
        serde_json::from_value(json!({"environment_id": PROD, "items": typed})).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "NEEDS_BROWSER_SIGN_IN");
    assert_eq!(w.server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn a_name_iii_cloud_refuses_is_flagged_and_fails_alone() {
    let w = world(false, "developer").await;
    std::fs::write(
        w.ws.join(MANIFEST),
        "containers:\n  api:\n    environment:\n      P: ${db_password}\n      GH_TOKEN: ${GH_TOKEN}\n",
    )
    .unwrap();
    std::fs::write(
        w.checkout.join("api/.env.test"),
        "db_password=lower-secret-0007\nGH_TOKEN=upper\n",
    )
    .unwrap();
    let v = read(&w).await;
    let low = item(&v, "db_password");
    assert!(
        low.invalid_name
            .as_deref()
            .is_some_and(|r| r.contains("A-Z"))
    );
    assert_eq!(low.suggestions, vec![file("api/.env.test", false, false)]);
    assert_eq!(item(&v, "GH_TOKEN").invalid_name, None);
    put_ok(&w.server).await;
    let out = save(
        &w.h.ctx,
        request(json!([
            {"name": "db_password", "from": "env_file", "type": "secret", "file": "api/.env.test"},
            {"name": "lower_copy", "from": "copy", "environment_id": PROD},
            {"name": "GH_TOKEN", "from": "env_file", "type": "secret", "file": "api/.env.test"}
        ])),
    )
    .await
    .unwrap();
    assert_eq!(
        codes(&out),
        vec![
            ("db_password", Some("INVALID_REQUEST")),
            ("lower_copy", Some("INVALID_REQUEST")),
            ("GH_TOKEN", None),
        ]
    );
    assert!(
        !serde_json::to_string(&out)
            .unwrap()
            .contains("lower-secret")
    );
    let puts = sent(&w.server, "PUT").await;
    assert_eq!(puts.len(), 1);
    assert!(puts[0].0.ends_with("/GH_TOKEN"));
    assert!(
        sent(&w.server, "POST").await.is_empty(),
        "no copy of nothing"
    );
}

#[tokio::test]
async fn an_env_file_that_is_a_link_is_never_read() {
    let w = world(false, "developer").await;
    let outside = w.checkout.parent().unwrap().join("outside.env");
    std::fs::write(&outside, "GH_TOKEN=outside-secret-0006\n").unwrap();
    std::os::unix::fs::symlink(&outside, w.checkout.join(".env.local")).unwrap();
    std::os::unix::fs::symlink(w.checkout.join(".env"), w.checkout.join("api/.env")).unwrap();
    let v = read(&w).await;
    assert!(
        !serde_json::to_string(&v).unwrap().contains(".env.local"),
        "a link is not listed"
    );
    assert_eq!(
        item(&v, "GH_TOKEN")
            .suggestions
            .iter()
            .filter(|s| matches!(s, Suggestion::EnvFile { file, .. } if file == ".env"))
            .count(),
        1,
        "the root `.env`, not the link to it in api/"
    );
    put_ok(&w.server).await;
    let out = save(
        &w.h.ctx,
        request(json!([
            {"name": "GH_TOKEN", "from": "env_file", "type": "secret", "file": ".env.local"},
            {"name": "LOCAL_API", "from": "env_file", "type": "secret", "file": "api/.env"}
        ])),
    )
    .await
    .unwrap();
    assert_eq!(
        codes(&out),
        vec![
            ("GH_TOKEN", Some("FILE_CHANGED")),
            ("LOCAL_API", Some("FILE_CHANGED")),
        ]
    );
    assert!(sent(&w.server, "PUT").await.is_empty());
}

#[tokio::test]
async fn variables_read_for_one_environment_never_land_in_another() {
    let w = world(false, "developer").await;
    let sel = iii_cloud::check::selected(&w.h.ctx).unwrap();
    // The person picks production while test's variables are still being read.
    w.h.ctx.store.update(|s| {
        let p = s.project.as_mut().unwrap();
        p.environment = Some(serde_json::from_value(json!({"id": PROD, "name": "production", "production": true, "branch": "main", "manifest": MANIFEST, "url": null})).unwrap());
        s.variables = vec![];
    });
    iii_cloud::status::refresh_variables(&w.h.ctx, &sel).await;
    assert!(w.h.ctx.store.read(|s| s.variables.is_empty()));
    let sel = iii_cloud::check::selected(&w.h.ctx).unwrap();
    iii_cloud::status::refresh_variables(&w.h.ctx, &sel).await;
    let slack = w.h.ctx.store.read(|s| {
        s.variables
            .iter()
            .find(|r| r.name == "SLACK_CHANNEL")
            .map(|r| r.status.clone())
    });
    assert_eq!(slack.as_deref(), Some("set"), "production's own list");
}

fn request(items: Value) -> ValuesSave {
    serde_json::from_value(json!({"environment_id": ENV, "items": items})).unwrap()
}

async fn put_ok(server: &MockServer) {
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server)
        .await;
}

async fn sent(server: &MockServer, verb: &str) -> Vec<(String, Value)> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == verb)
        .map(|r| {
            (
                r.url.path().to_string(),
                serde_json::from_slice(&r.body).unwrap_or(Value::Null),
            )
        })
        .collect()
}

#[tokio::test]
async fn save_writes_each_choice_once_and_answers_without_a_value() {
    let w = world(false, "developer").await;
    put_ok(&w.server).await;
    for (from, body) in [
        (
            PROD,
            json!({"copied": ["SESSION_SECRET"], "skipped": ["EXTRA"]}),
        ),
        (DEV, json!({"copied": ["DEV_ONLY"], "skipped": []})),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/v1{}/copy", vars(ENV))))
            .and(body_partial_json(json!({"from_environment_id": from})))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&w.server)
            .await;
    }
    let ask = iii_cloud::secret::ask(
        &w.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "GH_TOKEN".into(),
            description: "the app's token".into(),
            mode: None,
            external: None,
        },
    )
    .await
    .unwrap();
    let typed = "typed-secret-value-0004";
    let out = save(
        &w.h.ctx,
        request(json!([
            {"name": "GH_TOKEN", "from": "typed", "type": "secret", "value": typed},
            {"name": "SLACK_CHANNEL", "from": "env_file", "type": "variable", "file": ".env.production"},
            {"name": "APP_BASE_URL", "from": "env_url"},
            {"name": "OPTIONAL_FLAG", "from": "leave_empty"},
            {"name": "SESSION_SECRET", "from": "copy", "environment_id": PROD},
            {"name": "EXTRA", "from": "copy", "environment_id": PROD},
            {"name": "DEV_ONLY", "from": "copy", "environment_id": DEV},
            {"name": "STILL", "from": "env_file", "type": "secret", "file": "api/.env.test"},
            {"name": "LOCAL_API", "from": "env_file", "type": "secret", "file": "api/.env.test"},
            {"name": "SET_ONE", "from": "typed", "type": "variable", "value": "two\nlines"}
        ])),
    )
    .await
    .unwrap();
    let text = serde_json::to_string(&out).unwrap();
    assert!(
        !text.contains(typed) && !text.contains("#ops-prod-room"),
        "{text}"
    );
    let codes: Vec<(&str, bool, Option<&str>)> = out
        .results
        .iter()
        .map(|r| {
            (
                r.name.as_str(),
                r.ok,
                r.error.as_ref().map(|e| e.code.as_str()),
            )
        })
        .collect();
    assert_eq!(
        codes,
        vec![
            ("GH_TOKEN", true, None),
            ("SLACK_CHANNEL", true, None),
            ("APP_BASE_URL", true, None),
            ("OPTIONAL_FLAG", true, None),
            ("SESSION_SECRET", true, None),
            ("EXTRA", false, Some("ALREADY_SET")),
            ("DEV_ONLY", true, None),
            ("STILL", true, None),
            ("LOCAL_API", false, Some("FILE_CHANGED")),
            ("SET_ONE", true, None),
        ]
    );
    let base = format!("/api/v1{}", vars(ENV));
    assert_eq!(
        sent(&w.server, "PUT").await,
        vec![
            (
                format!("{base}/GH_TOKEN"),
                json!({"type": "secret", "value": typed})
            ),
            (
                format!("{base}/SLACK_CHANNEL"),
                json!({"type": "variable", "value": "#ops-prod-room"})
            ),
            (
                format!("{base}/APP_BASE_URL"),
                json!({"type": "variable", "value": "https://test.example.app"})
            ),
            (
                format!("{base}/OPTIONAL_FLAG"),
                json!({"type": "variable", "value": ""})
            ),
            (
                format!("{base}/STILL"),
                json!({"type": "secret", "value": "here"})
            ),
            (
                format!("{base}/SET_ONE"),
                json!({"type": "variable", "value": "two\nlines"})
            ),
        ]
    );
    let posts = sent(&w.server, "POST").await;
    assert_eq!(posts.len(), 2, "one copy per source");
    assert_eq!(
        posts[0].1,
        json!({"from_environment_id": PROD, "names": ["SESSION_SECRET", "EXTRA"]})
    );
    // The run here's `.env`: the names the file reads as `${NAME}`, not `${NAME:-}`.
    let env = std::fs::read_to_string(w.ws.join("api/.env")).unwrap();
    assert!(env.contains(&format!("GH_TOKEN={typed}")), "{env}");
    assert!(env.contains("APP_BASE_URL=https://test.example.app"));
    assert!(!env.contains("OPTIONAL_FLAG"), "{env}");
    assert!(
        !env.contains("SET_ONE"),
        "a value of two lines stays out of .env"
    );
    assert!(
        !w.checkout.join("api/.env").exists(),
        "never the person's folder"
    );
    // The agent's ask is answered, with an event.
    let answered =
        w.h.ctx
            .store
            .read(|s| s.secrets.iter().find(|r| r.id == ask.id).cloned())
            .unwrap();
    assert_eq!(answered.state, SecretState::Set);
    assert!(w.h.fake.kinds().contains(&"secret:true".to_string()));
    // Redaction knows every saved value.
    let known = w.h.ctx.known_secrets.lock().unwrap().clone();
    assert!(known.contains(&typed.to_string()) && known.contains(&"#ops-prod-room".to_string()));
    // Nothing kept, emitted or logged in the status.
    let stored = w.h.ctx.store.read(|s| serde_json::to_string(s).unwrap());
    let events = serde_json::to_string(&*w.h.fake.events.lock().unwrap()).unwrap();
    for text in [&stored, &events] {
        assert!(
            !text.contains(typed) && !text.contains("#ops-prod-room"),
            "{text}"
        );
    }
}

fn codes(out: &ValuesSaved) -> Vec<(&str, Option<&str>)> {
    out.results
        .iter()
        .map(|r| (r.name.as_str(), r.error.as_ref().map(|e| e.code.as_str())))
        .collect()
}

#[tokio::test]
async fn names_are_written_four_at_a_time_and_answered_in_the_requests_order() {
    let w = world(false, "developer").await;
    for (name, status) in [("ONE", 200), ("TWO", 422), ("THREE", 200)] {
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1{}/{name}", vars(ENV))))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_delay(std::time::Duration::from_millis(400))
                    .set_body_json(json!({"code": "REFUSED"})),
            )
            .mount(&w.server)
            .await;
    }
    let started = std::time::Instant::now();
    let out = save(
        &w.h.ctx,
        request(json!([
            {"name": "ONE", "from": "typed", "type": "variable", "value": "one"},
            {"name": "TWO", "from": "typed", "type": "variable", "value": "two"},
            {"name": "THREE", "from": "typed", "type": "variable", "value": "three"}
        ])),
    )
    .await
    .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1100),
        "three 400 ms writes ran one after the other"
    );
    assert_eq!(
        codes(&out),
        vec![("ONE", None), ("TWO", Some("NOT_SAVED")), ("THREE", None)],
        "the failing name in the middle is its own result; the others are written"
    );
    assert_eq!(sent(&w.server, "PUT").await.len(), 3);
}

#[tokio::test]
async fn a_refusal_of_the_session_leaves_the_later_chunks_untried() {
    let w = world(false, "developer").await;
    for (name, status) in [
        ("FIRST", 200),
        ("SECOND", 422),
        ("THIRD", 403),
        ("FOURTH", 200),
    ] {
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1{}/{name}", vars(ENV))))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"code": "REFUSED", "message": "the cloud's own text"})),
            )
            .mount(&w.server)
            .await;
    }
    let out = save(
        &w.h.ctx,
        request(json!([
            {"name": "FIRST", "from": "typed", "type": "variable", "value": "one"},
            {"name": "SECOND", "from": "typed", "type": "variable", "value": "two"},
            {"name": "THIRD", "from": "typed", "type": "variable", "value": "three"},
            {"name": "FOURTH", "from": "typed", "type": "variable", "value": "four"},
            {"name": "FIFTH", "from": "typed", "type": "variable", "value": "five"}
        ])),
    )
    .await
    .unwrap();
    assert_eq!(
        codes(&out),
        vec![
            ("FIRST", None),
            ("SECOND", Some("NOT_SAVED")),
            ("THIRD", Some("NOT_SAVED")),
            ("FOURTH", None),
            ("FIFTH", Some("NOT_TRIED")),
        ],
        "a 422 lets the rest run; a 403 stops the chunks after its own"
    );
    for r in &out.results[1..3] {
        assert!(!r.error.as_ref().unwrap().message.contains("own text"));
    }
    assert_eq!(sent(&w.server, "PUT").await.len(), 4, "FIFTH was not sent");

    // A copy refused with a 403 stops the puts after it.
    let w = world(false, "developer").await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"code": "FORBIDDEN"})))
        .mount(&w.server)
        .await;
    let out = save(
        &w.h.ctx,
        request(json!([
            {"name": "TYPED", "from": "typed", "type": "secret", "value": "some-value"},
            {"name": "COPIED", "from": "copy", "environment_id": PROD},
            {"name": "OTHER", "from": "copy", "environment_id": DEV}
        ])),
    )
    .await
    .unwrap();
    assert_eq!(
        codes(&out),
        vec![
            ("TYPED", Some("NOT_TRIED")),
            ("COPIED", Some("NOT_SAVED")),
            ("OTHER", Some("NOT_TRIED")),
        ]
    );
    assert_eq!(sent(&w.server, "POST").await.len(), 1);
    assert!(sent(&w.server, "PUT").await.is_empty());
}

#[tokio::test]
async fn save_is_refused_whole_for_a_key_another_environment_or_a_bad_item() {
    let w = world(false, "developer").await;
    let before = w.server.received_requests().await.unwrap().len();
    let err = save(
        &w.h.ctx,
        serde_json::from_value(json!({"environment_id": PROD, "items": []})).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "ENVIRONMENT_CHANGED");
    let secret = "never-in-a-message-0005";
    for (items, needle) in [
        (
            json!([{"name": "A", "from": "env_file", "type": "secret", "file": ".env.example"}]),
            "item 1 (A)",
        ),
        (
            json!([{"name": "A", "from": "typed", "type": "secret", "value": format!("{secret}\u{0}")}]),
            "item 1 (A)",
        ),
        (
            json!([{"name": "A", "from": "leave_empty"}, {"name": "A", "from": "leave_empty"}]),
            "item 2 (A)",
        ),
        (
            json!([{"name": "A", "from": "copy", "environment_id": ENV}]),
            "item 1 (A)",
        ),
    ] {
        let err = save(&w.h.ctx, request(items)).await.unwrap_err();
        assert_eq!(err.code, "INVALID_REQUEST");
        assert!(err.message.starts_with(needle), "{}", err.message);
        assert!(!err.message.contains(secret));
    }
    let mut conn = w.h.ctx.store.read(|s| s.connection.clone()).unwrap();
    conn.method = Some(Method::ApiKey);
    w.h.ctx.set_connection(conn);
    let err = save(
        &w.h.ctx,
        request(json!([{"name": "A", "from": "typed", "type": "secret", "value": secret}])),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "NEEDS_BROWSER_SIGN_IN");
    assert_eq!(
        w.server.received_requests().await.unwrap().len(),
        before,
        "nothing reached iii cloud"
    );
}

#[test]
fn the_env_parse_fixture_is_shared_with_the_page() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/env-parse.json");
    let cases: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for case in cases {
        let got: Vec<Value> = iii_cloud::local::parse_env(case["text"].as_str().unwrap())
            .into_iter()
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect();
        assert_eq!(Value::Array(got), case["expected"]);
    }
}

#[tokio::test]
async fn the_runs_env_is_written_only_where_git_says_it_is_untracked() {
    let w = world(false, "developer").await;
    put_ok(&w.server).await;
    let mut sel = iii_cloud::check::selected(&w.h.ctx).unwrap();
    let write = |sel: iii_cloud::check::Selected| {
        let ctx = w.h.ctx.clone();
        async move {
            iii_cloud::secret::write_value(
                &ctx,
                &sel,
                "GH_TOKEN",
                "secret",
                "v-1",
                Some(MANIFEST),
                true,
            )
            .await
            .local
        }
    };
    assert_eq!(write(sel.clone()).await, LocalOutcome::Set);
    // Tracked: left alone.
    git(&w.ws, &["add", "api/.env"]);
    assert_eq!(write(sel.clone()).await, LocalOutcome::Skipped);
    // No repository at all: unknown, so not written either.
    let plain = w.checkout.parent().unwrap().join("plain");
    std::fs::create_dir_all(plain.join("api")).unwrap();
    std::fs::write(plain.join(MANIFEST), COMPOSE).unwrap();
    sel.workspace = plain.clone();
    assert_eq!(write(sel).await, LocalOutcome::Skipped);
    assert!(!plain.join("api/.env").exists());
}

fn ask_of(name: &str, mode: Option<SecretMode>) -> SecretAsk {
    SecretAsk {
        handoff_id: None,
        name: name.into(),
        description: "why".into(),
        mode,
        external: None,
    }
}

#[tokio::test]
async fn a_secret_request_is_for_a_secret_the_compose_file_reads() {
    use iii_cloud::secret::ask;
    let w = world(false, "developer").await;
    let stored = || w.h.ctx.store.read(|s| s.secrets.len());
    // A worker's name is no variable name: refused, nothing stored.
    let e = ask(&w.h.ctx, ask_of("video-editor", None))
        .await
        .unwrap_err();
    assert_eq!(e.code, "NOT_A_SECRET");
    assert_eq!(
        e.message,
        "`iii-cloud::secret` asks the person for the value of one secret the compose file reads. `video-editor` is not a variable name. A decision goes to the person with `harness::ask`."
    );
    // A variable name the file does not read: refused too.
    let e = ask(&w.h.ctx, ask_of("NOT_IN_THE_FILE", None))
        .await
        .unwrap_err();
    assert_eq!(e.code, "NOT_READ_BY_FILE");
    assert_eq!(
        e.message,
        format!(
            "`NOT_IN_THE_FILE` is not read by {MANIFEST}. Add `${{NOT_IN_THE_FILE}}` where it is used first, or ask the person with `harness::ask`."
        )
    );
    assert_eq!(stored(), 0);
    // A note may name what the file no longer reads; a name the file reads is stored.
    ask(&w.h.ctx, ask_of("GONE_NOW", Some(SecretMode::NotNeeded)))
        .await
        .unwrap();
    let r = ask(&w.h.ctx, ask_of("GH_TOKEN", None)).await.unwrap();
    assert_eq!(r.state, SecretState::Waiting);
    // Requests stored before the refusal vanish from the status; the others stay.
    let env = r.environment_id.clone();
    w.h.ctx.store.update(|s| {
        let mut old = s.secrets[0].clone();
        old.id = "old".into();
        old.name = "rbac-auth".into();
        old.environment_id = env;
        s.secrets.push(old);
    });
    let st = iii_cloud::status::status(&w.h.ctx, StatusRequest::default())
        .await
        .unwrap();
    let mut names: Vec<_> = st.secrets.iter().map(|r| r.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["GH_TOKEN", "GONE_NOW"]);
}
