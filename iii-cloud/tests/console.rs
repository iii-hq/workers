//! `iii-cloud::config::import` and `iii-cloud::override::choose` against a
//! mock cloud: the local config folder copied by container key, secrets left
//! out, the draft then the apply at the version read, and the person's pick
//! per console override.

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use iii_cloud::console::{choose, import};
use iii_cloud::types::*;
use serde_json::{Value, json};
use support::repo::*;
use support::{Fake, Harness, harness};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MANIFEST: &str = "api/worker-compose.yaml";
const KEY_VALUE: &str = "sk-live-0123456789abcdefghijklmnop";
const AMQP_PASSWORD: &str = "amqp-pass-0001";

fn compose(directory: &str) -> String {
    format!(
        "engine:
  workers:
    configuration:
      adapter:
        name: fs
        config:
          directory: {directory}
containers:
  http:
    worker: package://http
  queue:
    worker: package://queue
  jobs:
    worker: path://./jobs
    config_name: jobs-config
"
    )
}

fn entry(id: &str, value: Value) -> String {
    serde_yaml_ng::to_string(&json!({"id": id, "name": id, "description": "seed", "value": value}))
        .unwrap()
}

/// The environment's config as `GET …/config` answers: workers by container
/// key, and the fields iii cloud sets (not editable).
fn view(version: i64, draft_changed: bool) -> Value {
    let fixed = |key: &str| json!({"key": key, "value": null, "draft": null, "source": "managed", "editable": false, "applies": "restart:web"});
    json!({"version": version, "draft_changed": draft_changed, "source_commit": "abc", "workers": [
        {"name": "http", "origin": "registry", "role": "http", "fields": [
            fixed("host"), fixed("port"),
            {"key": "cors.allowed_origins", "value": ["*"], "draft": ["*"], "source": "default", "editable": true, "applies": "live"}
        ]},
        {"name": "queue", "origin": "registry", "role": "queue", "fields": [fixed("adapter.config.amqp_url")]},
        {"name": "jobs", "origin": "path", "role": null, "fields": []}
    ]})
}

struct World {
    h: Harness,
    server: MockServer,
    checkout: PathBuf,
    root: tempfile::TempDir,
}

fn config_path() -> String {
    format!("/api/v1/teams/{TEAM}/environments/{ENV}/config")
}

/// A browser session on `test`, the person's folder with `api/config`.
async fn world(directory: &str) -> World {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let config = checkout.join("api/config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(root.path().join("worktree")).unwrap();
    std::fs::write(checkout.join(MANIFEST), compose(directory)).unwrap();
    let put = |name: &str, text: String| std::fs::write(config.join(name), text).unwrap();
    // Older packages: the entry is the bare container key.
    put(
        "http.yaml",
        entry(
            "http",
            json!({
                "host": "0.0.0.0", "port": 3111,
                "cors": {"allowed_origins": ["${CORS_ALLOWED_ORIGINS:*}"], "allowed_methods": ["GET", "POST"]},
                "api_key": KEY_VALUE,
                "auth": {"token": "${HTTP_TOKEN}", "issuer": "https://user:hunter2@login.example"},
                "middleware": [],
                "unset": null
            }),
        ),
    );
    // Compose's own name, `<namespace>-<key>`.
    put(
        "default-queue.yaml",
        entry(
            "default-queue",
            json!({"adapter": {"name": "rabbitmq", "config": {"amqp_url": format!("amqp://u:{AMQP_PASSWORD}@mq")}}, "max_retries": 3}),
        ),
    );
    // A container's own `config_name`.
    put(
        "jobs-config.yaml",
        entry("jobs-config", json!({"concurrency": 4})),
    );
    // The engine's own worker: no container iii cloud runs.
    put(
        "iii-observability.yaml",
        entry("iii-observability", json!({"level": "info"})),
    );
    put("broken.yaml", "value: [unclosed\n".into());
    put("big.yaml", format!("# {}\n", "x".repeat(1 << 20)));
    // Not what the engine's `fs` adapter loads.
    put("notes.json", "{\"id\": \"http\"}".into());
    put("http.bak.yaml", entry("http", json!({"port": 1})));
    std::fs::write(
        root.path().join("outside.yaml"),
        entry("http", json!({"port": 2})),
    )
    .unwrap();
    std::os::unix::fs::symlink(root.path().join("outside.yaml"), config.join("link.yaml")).unwrap();

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
        role: "developer".into(),
    }];
    h.ctx.set_connection(conn);
    let project: Project = serde_json::from_value(json!({
        "path": checkout, "repository": "ada/hello-iii",
        "checkout": {"branch": "main", "head": "h", "changed_files": [], "ahead": 0},
        "team": {"id": TEAM, "name": "Acme", "slug": "acme"}, "teams": [],
        "cloud_project": {"id": "p1", "name": "registry", "slug": "registry"},
        "environment": {"id": ENV, "name": "test", "production": false, "branch": "main", "manifest": MANIFEST, "url": null},
        "environments": [{"id": ENV, "name": "test", "production": false, "branch": "main"}],
        "importable": null, "github_access": true,
        "workspace": {"mode": "worktree", "path": root.path().join("worktree"), "branch": "iii-cloud/ready-test"}
    }))
    .unwrap();
    h.ctx.store.update(|s| s.project = Some(project));
    World {
        h,
        server,
        checkout,
        root,
    }
}

async fn mount_view(server: &MockServer, version: i64, draft_changed: bool) {
    Mock::given(method("GET"))
        .and(path(config_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(view(version, draft_changed)))
        .mount(server)
        .await;
}

fn req(dry_run: bool, expected_version: Option<i64>) -> ConfigImport {
    ConfigImport {
        environment_id: ENV.into(),
        dry_run,
        expected_version,
    }
}

fn keys(w: &ConfigImportWorker) -> Vec<&str> {
    w.fields.iter().map(|f| f.key.as_str()).collect()
}

#[tokio::test]
async fn the_preview_maps_each_entry_to_its_container_and_leaves_secrets_out() {
    let w = world("./config").await;
    mount_view(&w.server, 7, false).await;
    let out = import(&w.h.ctx, req(true, None)).await.unwrap();
    assert_eq!(
        (
            out.directory.as_str(),
            out.version,
            out.applied,
            out.draft_changed
        ),
        ("api/config", 7, false, false)
    );
    let by = |name: &str| out.workers.iter().find(|w| w.worker == name).unwrap();
    let http = by("http");
    assert_eq!(http.file, "api/config/http.yaml");
    // Nested keys dotted, lists whole, nulls left out; a `${NAME}` reference is no secret.
    assert_eq!(
        keys(http),
        [
            "auth.token",
            "cors.allowed_methods",
            "cors.allowed_origins",
            "middleware"
        ]
    );
    assert_eq!(http.fields[2].value, json!(["${CORS_ALLOWED_ORIGINS:*}"]));
    assert_eq!(http.left_out, ["api_key", "auth.issuer"]);
    assert_eq!(http.set_elsewhere, ["host", "port"]);
    let queue = by("queue");
    assert_eq!(
        (queue.file.as_str(), keys(queue)),
        (
            "api/config/default-queue.yaml",
            vec!["adapter.name", "max_retries"]
        )
    );
    assert_eq!(queue.set_elsewhere, ["adapter.config.amqp_url"]);
    assert_eq!(keys(by("jobs")), ["concurrency"]);
    let skipped: Vec<(&str, ConfigSkipReason)> = out
        .skipped
        .iter()
        .map(|s| (s.file.as_str(), s.reason))
        .collect();
    assert_eq!(
        skipped,
        [
            ("api/config/big.yaml", ConfigSkipReason::TooLarge),
            ("api/config/broken.yaml", ConfigSkipReason::Unreadable),
            (
                "api/config/iii-observability.yaml",
                ConfigSkipReason::NotAWorker
            ),
            ("api/config/link.yaml", ConfigSkipReason::NotAPlainFile),
        ]
    );
    let text = serde_json::to_string(&out).unwrap();
    for secret in [KEY_VALUE, AMQP_PASSWORD, "hunter2"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    // A dry run writes nothing.
    let writes = w.server.received_requests().await.unwrap();
    assert!(writes.iter().all(|r| r.method.as_str() == "GET"));
}

/// The planner's rule (`crates/plan/src/lib.rs:331-347`): `LOCAL_CONFIG_NOT_SET`
/// goes once any manifest container key has an entry in the console layer.
fn clears_local_config_not_set(manifest: &str, layer: &Value) -> bool {
    let m: Value = serde_yaml_ng::from_str(manifest).unwrap();
    m["containers"]
        .as_object()
        .unwrap()
        .keys()
        .any(|k| layer.get(k).is_some())
}

#[tokio::test]
async fn apply_sends_the_imported_fields_as_the_draft_then_applies_at_the_version_read() {
    let w = world("./config").await;
    mount_view(&w.server, 7, false).await;
    let draft = json!({"workers": {
        "http": {
            "auth.token": "${HTTP_TOKEN}",
            "cors.allowed_methods": ["GET", "POST"],
            "cors.allowed_origins": ["${CORS_ALLOWED_ORIGINS:*}"],
            "middleware": []
        },
        "queue": {"adapter.name": "rabbitmq", "max_retries": 3},
        "jobs": {"concurrency": 4}
    }});
    Mock::given(method("PATCH"))
        .and(path(format!("{}/draft", config_path())))
        .and(body_json(&draft))
        .respond_with(ResponseTemplate::new(200).set_body_json(view(7, true)))
        .expect(1)
        .mount(&w.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{}/apply", config_path())))
        .and(body_json(json!({"expected_version": 7})))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"number": 8, "status": "pending", "deployment_id": null, "applied_by": null, "applied_at": "t", "restarts": ["web"]}),
        ))
        .expect(1)
        .mount(&w.server)
        .await;
    let out = import(&w.h.ctx, req(false, Some(7))).await.unwrap();
    assert_eq!((out.applied, out.version), (true, 8));
    // No value comes back once applied.
    assert!(
        out.workers
            .iter()
            .flat_map(|w| &w.fields)
            .all(|f| f.value.is_null())
    );
    let seen = w.server.received_requests().await.unwrap();
    let writes: Vec<&str> = seen
        .iter()
        .filter(|r| r.method.as_str() != "GET")
        .map(|r| r.url.path())
        .collect();
    assert_eq!(
        writes,
        [
            format!("{}/draft", config_path()),
            format!("{}/apply", config_path())
        ]
    );
    // The draft is keyed by container key: the plan's rule then finds the config.
    let sent: Value = seen
        .iter()
        .find(|r| r.method.as_str() == "PATCH")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .unwrap();
    assert!(clears_local_config_not_set(
        &compose("./config"),
        &sent["workers"]
    ));
    // File stems would not be: `default-queue` and `jobs-config` are no container.
    assert!(!clears_local_config_not_set(
        &compose("./config"),
        &json!({"default-queue": {}, "jobs-config": {}, "iii-observability": {}})
    ));
    let body = String::from_utf8_lossy(
        &seen
            .iter()
            .find(|r| r.method.as_str() == "PATCH")
            .unwrap()
            .body,
    )
    .into_owned();
    for secret in [KEY_VALUE, AMQP_PASSWORD, "hunter2"] {
        assert!(!body.contains(secret), "{secret} sent: {body}");
    }
}

#[tokio::test]
async fn a_moved_version_a_conflict_a_pending_draft_and_a_missing_role_are_named() {
    // The version moved since the dry run: refused before any write.
    let w = world("./config").await;
    mount_view(&w.server, 8, false).await;
    let e = import(&w.h.ctx, req(false, Some(7))).await.err().unwrap();
    assert_eq!(e.code, "CONFIG_CHANGED");
    assert!(e.message.contains("test's config"), "{}", e.message);
    assert!(
        w.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
    // Applying without the dry run's version.
    let e = import(&w.h.ctx, req(false, None)).await.err().unwrap();
    assert_eq!(e.code, "INVALID_REQUEST");

    // Someone applied between the draft and the apply: 409.
    let w = world("./config").await;
    mount_view(&w.server, 7, false).await;
    Mock::given(method("PATCH"))
        .and(path(format!("{}/draft", config_path())))
        .respond_with(ResponseTemplate::new(200).set_body_json(view(7, true)))
        .mount(&w.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{}/apply", config_path())))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({"code": "CONFLICT", "message": "The config is at version 8, not 7: someone applied first."})))
        .mount(&w.server)
        .await;
    let e = import(&w.h.ctx, req(false, Some(7))).await.err().unwrap();
    assert_eq!((e.code.as_str(), e.status), ("CONFIG_CHANGED", Some(409)));
    assert_eq!(e.message, "Someone changed test's config; review again.");

    // Someone else's console edits wait unapplied: never applied with ours.
    let w = world("./config").await;
    mount_view(&w.server, 7, true).await;
    let e = import(&w.h.ctx, req(false, Some(7))).await.err().unwrap();
    assert_eq!(e.code, "CONFIG_DRAFT_PENDING");
    assert!(
        w.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );

    // No config.edit: 403.
    let w = world("./config").await;
    mount_view(&w.server, 7, false).await;
    Mock::given(method("PATCH"))
        .and(path(format!("{}/draft", config_path())))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"code": "FORBIDDEN", "message": "no"})),
        )
        .mount(&w.server)
        .await;
    let e = import(&w.h.ctx, req(false, Some(7))).await.err().unwrap();
    assert_eq!(e.code, "ROLE_REQUIRED");
}

#[tokio::test]
async fn a_config_folder_outside_the_repository_or_a_link_is_refused() {
    let w = world("../../outside").await;
    std::fs::create_dir_all(w.root.path().join("outside")).unwrap();
    mount_view(&w.server, 7, false).await;
    let e = import(&w.h.ctx, req(true, None)).await.err().unwrap();
    assert_eq!(e.code, "CONFIG_FOLDER_REFUSED");

    let w = world("./linked").await;
    std::os::unix::fs::symlink(w.checkout.join("api/config"), w.checkout.join("api/linked"))
        .unwrap();
    mount_view(&w.server, 7, false).await;
    let e = import(&w.h.ctx, req(true, None)).await.err().unwrap();
    assert_eq!(e.code, "CONFIG_FOLDER_REFUSED");

    let w = world("./nowhere").await;
    mount_view(&w.server, 7, false).await;
    let e = import(&w.h.ctx, req(true, None)).await.err().unwrap();
    assert_eq!(e.code, "CONFIG_FOLDER_MISSING");

    // Another environment selected meanwhile.
    let mut other = req(true, None);
    other.environment_id = "other".into();
    let e = import(&w.h.ctx, other).await.err().unwrap();
    assert_eq!(e.code, "ENVIRONMENT_CHANGED");
}

fn seed_check(h: &Harness, console_port: u16) {
    let (plan, _) = iii_cloud::check::plan_part(&json!({"can_deploy": true, "plan": {
        "overrides": [{"setting": "web", "console": {"serve": "web", "port": console_port, "access": "team"}, "file": {"serve": "web", "port": 3000, "access": "public"}}]
    }}));
    h.ctx.store.update(|s| {
        s.put_check(Check {
            id: format!("chk-{console_port}"),
            environment_id: ENV.into(),
            commit: "abc".into(),
            base: String::new(),
            checked_at: "t".into(),
            duration_ms: 0,
            plan,
            here: HerePart {
                problems: 0,
                iii_version: None,
                accepts_cloud_block: true,
            },
            problems: vec![],
            run: None,
            ready: false,
            report: String::new(),
            manifest: None,
            handoff_id: None,
            commits: vec![],
        })
    });
}

async fn choices(h: &Harness) -> std::collections::BTreeMap<String, OverrideChoice> {
    iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .override_choices
}

#[tokio::test]
async fn an_override_choice_is_kept_for_its_values_and_dropped_when_they_change() {
    let w = world("./config").await;
    seed_check(&w.h, 3000);
    let pick = |setting: &str, choice| OverrideChoose {
        environment_id: ENV.into(),
        setting: setting.into(),
        choice,
    };
    let out = choose(&w.h.ctx, pick("web", OverrideChoice::File))
        .await
        .unwrap();
    assert_eq!(
        out.override_choices,
        [("web".to_string(), OverrideChoice::File)].into()
    );
    assert_eq!(out.override_choices, choices(&w.h).await);
    // Persisted with the hashes of the pair it was made for.
    let state = std::fs::read_to_string(w.h.ctx.store.dir().join("state.json")).unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    let stored = &state["override_choices"][ENV]["web"];
    assert_eq!(stored["choice"], "file");
    assert_eq!(stored["console_hash"].as_str().map(str::len), Some(64));
    // A later pick replaces it.
    choose(&w.h.ctx, pick("web", OverrideChoice::Console))
        .await
        .unwrap();
    assert_eq!(
        choices(&w.h).await,
        [("web".to_string(), OverrideChoice::Console)].into()
    );
    // The console's value changed: the choice no longer counts.
    seed_check(&w.h, 8080);
    assert!(choices(&w.h).await.is_empty());
    let e = choose(&w.h.ctx, pick("build", OverrideChoice::File))
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, "NO_SUCH_OVERRIDE");
    let mut other = pick("web", OverrideChoice::File);
    other.environment_id = "other".into();
    assert_eq!(
        choose(&w.h.ctx, other).await.err().unwrap().code,
        "ENVIRONMENT_CHANGED"
    );
}

#[test]
fn credential_looking_values_are_told_apart_from_plain_ones() {
    use iii_cloud::console::credential_like;
    for secret in [
        "postgres://app:s3cret@db/app",
        "ghp_0123456789abcdefghijklmnopqrstuvwxyz",
        "AKIAIOSFODNN7EXAMPLE",
        "-----BEGIN PRIVATE KEY-----",
        "eyJhbGciOiJIUzI1NiJ9.e30.x",
        "q8Zr2vT0xY7mN4kP1sL9wB3cD6fG5hJ0",
    ] {
        assert!(credential_like(&json!(secret)), "{secret}");
    }
    for plain in [
        "0.0.0.0",
        "https://login.example/callback",
        "${DATABASE_URL}",
        "/var/lib/iii/data/0123456789abcdefghijklmnop",
        "4b1f6a9e-1d2c-4e3f-8a7b-6c5d4e3f2a1b",
        "info",
    ] {
        assert!(!credential_like(&json!(plain)), "{plain}");
    }
    assert!(credential_like(&json!(["ok", "redis://:pw@cache"])));
}
