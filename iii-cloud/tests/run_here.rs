//! The run here on a tiny app: one `path://` node worker on iii-sdk 0.27 and
//! `package://http`. Ignored: it needs the private iii 0.27.0 (`III_ENGINE_BIN`),
//! node and npm, and the network for `npm ci` and the http package.
//!
//!   III_ENGINE_BIN=~/tmp/iii-cloud-worker/bin/iii cargo test --locked -- --ignored run_here

mod support;

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_cloud::config::Config;
use iii_cloud::run::start;
use iii_cloud::types::*;
use serde_json::json;
use support::repo::*;
use wiremock::MockServer;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if entry.file_name() == "node_modules" {
            continue;
        }
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

fn listeners() -> Vec<String> {
    let out = Command::new("ss").args(["-ltn"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(":49134 ") || l.contains(":3111 "))
        .map(str::to_string)
        .collect()
}

fn containers() -> String {
    let out = Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "needs III_ENGINE_BIN, node, npm and the network"]
async fn run_here_passes_all_seven_stages_on_the_fixture_app() {
    let iii_bin =
        std::env::var("III_ENGINE_BIN").expect("III_ENGINE_BIN is the private iii 0.27.0");
    let before = listeners();
    let server = MockServer::start().await;
    let h = connected(&server, "worker-compose.yaml").await;
    *h.ctx.cfg.write().await = Arc::new(Config {
        cloud_url: server.uri(),
        iii_bin,
        data_dir: h.dir.path().join("data").to_string_lossy().into_owned(),
        ..Config::default()
    });
    // The workspace: the fixture, committed.
    let ws = h.dir.path().join("app");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run-app"),
        &ws,
    );
    git(&ws, &["init", "-b", "main"]);
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "app"]);
    let head = git(&ws, &["rev-parse", "HEAD"]);
    // A value of the run's .env that the release prints: it must not reach the run's record.
    let leaky = "leaky-value-0123456789";
    // The run's own files are excluded in a real workspace; so is this one.
    std::fs::write(ws.join(".git/info/exclude"), ".env\n").unwrap();
    std::fs::write(ws.join(".env"), format!("LEAKY={leaky}\n")).unwrap();
    let env = EnvironmentInfo {
        id: ENV.into(),
        name: "production".into(),
        production: true,
        branch: "main".into(),
        manifest: Some("worker-compose.yaml".into()),
        url: None,
    };
    h.ctx.store.update(|s| {
        s.project = Some(Project {
            path: ws.to_string_lossy().into(),
            repository: "ada/app".into(),
            checkout: Checkout { branch: Some("main".into()), head: head.clone(), changed_files: vec![], ahead: 0, not_served: None, branches: vec![] },
            team: None, teams: vec![], cloud_project: None, environment: Some(env), environments: vec![], importable: None, github_access: None, cloud_projects: vec![], other_cloud_projects: vec![], default_branch: None,
            workspace: Some(Workspace { mode: WorkspaceMode::Checkout, path: ws.to_string_lossy().into(), branch: "main".into(), from_head: None, not_brought: vec![], source: None, not_brought_reasons: Default::default(), source_sha: None }),
            fell_back_from: None,
        });
        s.put_check(Check {
            id: "chk".into(), environment_id: ENV.into(), commit: head, base: String::new(), checked_at: "t".into(), duration_ms: 0,
            plan: serde_json::from_value(json!({"status": "clean", "fingerprint": null, "digest": null, "blocking": 0, "warnings": 0, "at_least": false,
                "confirmations": {"resources": [], "overrides": []}, "web_candidates": null, "error": null})).unwrap(),
            here: HerePart { problems: 0, iii_version: Some("0.27.0".into()), accepts_cloud_block: true },
            problems: vec![], run: None, ready: false, report: String::new(), manifest: None, handoff_id: None, commits: vec![],
        });
    });
    start(&h.ctx, "chk".into()).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(600);
    let run = loop {
        let run = h
            .ctx
            .store
            .read(|s| s.check.as_ref().and_then(|c| c.run.clone()))
            .unwrap();
        if run.status != RunStatus::Running && run.finished_at.is_some() {
            break run;
        }
        assert!(Instant::now() < deadline, "the run never ended: {run:?}");
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    assert_eq!(
        run.status,
        RunStatus::Passed,
        "{:#?}\n{:?}",
        run.error,
        run.log_tail
    );
    let stages: Vec<_> = run.stages.iter().map(|s| (s.name, s.status)).collect();
    assert!(
        stages.iter().all(|(_, s)| *s == StageStatus::Passed),
        "{stages:?}"
    );
    assert_eq!(run.stages.len(), 7);
    assert_eq!((run.workers.ready, run.workers.total), (2, 2));
    assert!(run.routes.count >= 2 && run.routes.prefix.as_deref() == Some("/api"));
    assert_eq!(run.healthcheck.as_ref().unwrap().status, Some(200));
    assert_eq!(run.engine, "0.27.0");
    let everything = serde_json::to_string(&run).unwrap();
    assert!(
        !everything.contains(leaky),
        "a .env value reached the run's record"
    );
    assert!(
        run.log_tail
            .iter()
            .any(|l| l.contains("released [redacted]")),
        "{:?}",
        run.log_tail
    );
    assert!(h.ctx.store.read(|s| s.check.as_ref().unwrap().ready));
    // Teardown: no copy, no container, no listener of the harness's.
    assert!(
        !ws.join("worker-compose.iii-cloud-run.yaml").exists()
            && !ws.join("worker-compose.iii-cloud-run.lock").exists()
    );
    assert!(!ws.join(".iii/compose/iii-cloud-run").exists());
    assert!(!containers().contains("iii-cloud-run-"));
    assert_eq!(listeners(), before, "no change on 49134 or 3111");
    assert!(h.ctx.runs.lock().unwrap().is_empty());
}

/// The default local Postgres image carries pgvector, and the run gives it the
/// cloud's role model: the app is a non-superuser login whose sessions run as the
/// NOLOGIN owner of the database, `vector` comes from template1, and PUBLIC cannot connect.
#[tokio::test]
#[ignore = "needs docker and the network to pull the image"]
async fn run_here_default_postgres_image_has_pgvector_and_the_clouds_roles() {
    let image = Config::default().postgres_image;
    let name = format!("iii-cloud-run-pg-vectest-{}", std::process::id());
    let docker = |args: &[&str]| Command::new("docker").args(args).output().unwrap();
    let started = docker(&[
        "run",
        "-d",
        "--rm",
        "--name",
        &name,
        "-e",
        "POSTGRES_PASSWORD=pw",
        &image,
    ]);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let result: Result<String, String> = async {
        let mut ready = false;
        for _ in 0..60 {
            if docker(&[
                "exec",
                &name,
                "pg_isready",
                "-h",
                "127.0.0.1",
                "-U",
                "postgres",
                "-d",
                "postgres",
            ])
            .status
            .success()
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        if !ready {
            return Err("postgres never became ready".into());
        }
        iii_cloud::run::setup_cloud_roles(&name, "app_login", "loginpw").await?;
        // As the app: its login, with the password of the binding.
        let as_app = |sql: &str| {
            let out = Command::new("docker")
                .args([
                    "exec",
                    "-e",
                    "PGPASSWORD=loginpw",
                    &name,
                    "psql",
                    "-h",
                    "127.0.0.1",
                    "-U",
                    "app_login",
                    "-d",
                    "app",
                    "-qAt",
                    "-c",
                    sql,
                ])
                .output()
                .unwrap();
            (
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).to_string()
                    + &String::from_utf8_lossy(&out.stderr),
            )
        };
        let mut report = String::new();
        let check = |label: &str, ok: bool, text: &str, want: bool, report: &mut String| {
            if ok != want {
                report.push_str(&format!(
                    "{label}: expected {} but {text}\n",
                    if want { "ok" } else { "an error" }
                ));
            }
        };
        let (ok, text) = as_app("SELECT current_user || ',' || session_user");
        check(
            "sessions run as the owner role",
            ok && text.trim() == "app,app_login",
            &text,
            true,
            &mut report,
        );
        let (ok, text) = as_app("SELECT rolsuper FROM pg_roles WHERE rolname = session_user");
        check(
            "the login is not a superuser",
            ok && text.trim() == "f",
            &text,
            true,
            &mut report,
        );
        let (ok, text) = as_app("CREATE TABLE t (id int); INSERT INTO t VALUES (1)");
        check("the owner can migrate", ok, &text, true, &mut report);
        let (ok, text) = as_app("SELECT '[1,2,3]'::vector");
        check(
            "vector is in the database (template1)",
            ok && text.contains("[1,2,3]"),
            &text,
            true,
            &mut report,
        );
        // A drizzle-style migration, as the app's login: its own schema, tables, an
        // index, an alter and a vector column all work because the login acts as the owner.
        let (ok, text) = as_app(
            "CREATE SCHEMA IF NOT EXISTS drizzle; \
             CREATE TABLE IF NOT EXISTS drizzle.__drizzle_migrations (id SERIAL PRIMARY KEY, hash text NOT NULL, created_at bigint); \
             CREATE EXTENSION IF NOT EXISTS vector; \
             CREATE TABLE public.docs (id serial PRIMARY KEY, embedding vector(3)); \
             CREATE INDEX docs_idx ON public.docs (id); \
             ALTER TABLE public.docs ADD COLUMN title text; \
             INSERT INTO drizzle.__drizzle_migrations (hash, created_at) VALUES ('h', 1)",
        );
        check(
            "a drizzle-style migration runs as the app login",
            ok,
            &text,
            true,
            &mut report,
        );
        let (ok, text) = as_app("CREATE ROLE nope");
        check("no CREATE ROLE", ok, &text, false, &mut report);
        let (ok, text) = as_app("CREATE EXTENSION file_fdw");
        check(
            "no non-trusted CREATE EXTENSION",
            ok,
            &text,
            false,
            &mut report,
        );
        // Not PUBLIC: a role with no membership cannot connect to the database.
        let _ = docker(&[
            "exec",
            &name,
            "psql",
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
            "-d",
            "postgres",
            "-c",
            "CREATE ROLE outsider LOGIN PASSWORD 'o'",
        ]);
        let out = Command::new("docker")
            .args([
                "exec",
                "-e",
                "PGPASSWORD=o",
                &name,
                "psql",
                "-h",
                "127.0.0.1",
                "-U",
                "outsider",
                "-d",
                "app",
                "-qAt",
                "-c",
                "SELECT 1",
            ])
            .output()
            .unwrap();
        check(
            "PUBLIC cannot connect",
            out.status.success(),
            &String::from_utf8_lossy(&out.stderr),
            false,
            &mut report,
        );
        if report.is_empty() {
            Ok(String::new())
        } else {
            Err(report)
        }
    }
    .await;
    docker(&["rm", "-f", &name]);
    assert!(result.is_ok(), "{}", result.unwrap_err());
}
