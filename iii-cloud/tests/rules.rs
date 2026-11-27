//! The pure rules of the worker, with no network or process.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use iii_cloud::api::{refuses_secret, valid_path};
use iii_cloud::check::*;
use iii_cloud::local::*;
use iii_cloud::project::{default_environment, parse_remote};
use iii_cloud::run::*;
use iii_cloud::secret::{set_env_line, valid_name};
use iii_cloud::types::*;
use iii_cloud::watch::{health_lines, no_route_in, outcome_of, release_row};
use serde_json::{Value, json};

fn plan_fixture() -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/plan.json"
        ))
        .unwrap(),
    )
    .unwrap()
}

fn compose_fixture() -> Value {
    serde_yaml_ng::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/compose.yaml"
        ))
        .unwrap(),
    )
    .unwrap()
}

// ------------------------------------------------------------ plan -> problems

#[test]
fn issues_become_problems_with_the_iii_code_and_the_owner_table() {
    let problems = plan_problems(&plan_fixture()["plan"]);
    let first = &problems[0];
    assert_eq!(first.code, "MISSING_WORKER_DIRECTORY");
    assert_eq!(first.cloud_code.as_deref(), Some("INVALID_COMPOSE_FILE"));
    assert_eq!(first.title, "A worker directory is missing");
    assert_eq!(
        first.detail.as_deref(),
        Some(
            "The folder workers/api is not in the repository. Version it or use package://. (at containers.api.worker)"
        )
    );
    assert_eq!(
        (first.file.as_deref(), first.line),
        (Some("worker-compose.yaml"), Some(12))
    );
    assert_eq!(first.excerpt.as_ref().unwrap().start_line, 10);
    assert_eq!(first.owner, Owner::Agent, "fix_in_repo is the agent's");
    assert_eq!(first.source, Source::CloudPlan);
    assert_eq!(first.fix.as_deref(), Some("Version the folder."));
    // deploy_issues follow issues.
    assert_eq!(problems.last().unwrap().code, "LOCAL_CONFIG_NOT_SET");
    // Ids are stable across checks.
    assert_eq!(first.id, plan_problems(&plan_fixture()["plan"])[0].id);
    assert_eq!(first.id.len(), 40);
}

#[test]
fn the_owner_table_is_pinned() {
    // (code, kind, severity): the rule, one row per case of spec §4 and plan-codes.md.
    let rows: Vec<(&str, Option<&str>, Severity)> = vec![
        ("LOCK_MISSING", Some("fix_in_repo"), Severity::Block),
        ("INVALID_COMPOSE_FILE", Some("fix_in_repo"), Severity::Block),
        (
            "MISSING_WORKER_DIRECTORY",
            Some("fix_in_repo"),
            Severity::Block,
        ),
        ("RUNTIME_UNSUPPORTED", Some("fix_in_repo"), Severity::Block),
        ("LITERAL_CREDENTIAL", Some("fix_in_repo"), Severity::Block),
        ("SECRET_MISSING", Some("resolve_here"), Severity::Block),
        ("MULTIPLE_HTTP_GROUPS", Some("fix_in_repo"), Severity::Block),
        (
            "WEB_CONTAINER_UNKNOWN",
            Some("fix_in_repo"),
            Severity::Block,
        ),
        ("WEB_API_WITHOUT_HTTP", None, Severity::Warn),
        ("LOCAL_CONFIG_NOT_SET", Some("fix_in_repo"), Severity::Warn),
        ("ENV_FILE_MISSING", None, Severity::Warn),
        ("EMPTY_DEFAULT_VARIABLE", None, Severity::Warn),
        ("SQLITE_DEFAULT_REPLACED", None, Severity::Warn),
        ("QUEUE_SEMANTICS_CHANGED", None, Severity::Warn),
        ("ENGINE_SECTION_IGNORED", None, Severity::Info),
        ("SOME_RESOLVE_CODE", Some("resolve_here"), Severity::Block),
        ("SOME_BLOCK_WITHOUT_KIND", None, Severity::Block),
    ];
    let table: Vec<Value> = rows
        .iter()
        .map(|(code, kind, severity)| json!({"code": code, "kind": kind, "severity": severity, "owner": owner_for(code, *kind, *severity)}))
        .collect();
    let mut text = serde_json::to_string_pretty(&table).unwrap();
    text.push('\n');
    support::check_golden("owners.json", &text).unwrap();
}

#[test]
fn the_plan_digest_ignores_the_commit_and_sees_a_changed_resource() {
    let plan = plan_fixture()["plan"].clone();
    let digest = plan_digest(&plan);
    // Pinned against the recorded plan.
    assert_eq!(
        digest, "185d5cc64be6e9ac4b6b8ee54cccbc564c134f5a023130d4e43aa5498bc1d0f3",
        "digest drifted: {digest}"
    );
    let mut other_commit = plan.clone();
    other_commit["source"]["commit"] = json!("ffffffffffffffffffffffffffffffffffffffff");
    other_commit["source"]["branch"] = json!("elsewhere");
    other_commit["environment"] = json!("staging");
    assert_eq!(plan_digest(&other_commit), digest);
    let mut changed = plan.clone();
    changed["resources"][0]["action"] = json!("reuse");
    assert_ne!(plan_digest(&changed), digest);
    assert_eq!(differing_keys(&plan, &changed), vec!["resources"]);
    // Key order does not matter.
    let reordered: Value = serde_json::from_str(&serde_json::to_string(&plan).unwrap().replace(
        "{\"kind\":\"database\"",
        "{\"alias\":\"primary\",\"kind\":\"database\"",
    ))
    .unwrap();
    assert_eq!(plan_digest(&reordered), digest);
}

#[test]
fn the_plan_part_counts_confirmations_and_stops_at_blocking() {
    let (part, problems) = plan_part(&plan_fixture());
    assert_eq!(part.status, PlanStatus::Blocked);
    assert_eq!((part.blocking, part.warnings), (4, 2));
    assert!(part.at_least);
    assert_eq!(problems.len(), 7);
    assert_eq!(
        part.confirmations.resources.len(),
        1,
        "only a create needs confirming"
    );
    assert_eq!(
        part.confirmations.resources[0].tier.as_deref(),
        Some("shared")
    );
    assert_eq!(part.confirmations.overrides[0].setting, "healthcheck");
    assert_eq!(part.fingerprint.as_deref(), Some("fp-abc"));
    assert!(part.web_candidates.is_some());
}

fn plan_with(status: PlanStatus) -> PlanPart {
    PlanPart {
        status,
        fingerprint: None,
        digest: None,
        blocking: 0,
        warnings: 0,
        at_least: false,
        confirmations: Confirmations {
            resources: vec![],
            overrides: vec![],
        },
        web_candidates: None,
        error: None,
        summary: None,
    }
}

fn passed_run(commit: &str) -> Run {
    Run {
        id: "r".into(),
        check_id: "c".into(),
        commit: commit.into(),
        engine: "0.27.0".into(),
        status: RunStatus::Passed,
        started_at: "t".into(),
        finished_at: None,
        stages: vec![],
        workers: WorkerCount { ready: 3, total: 3 },
        routes: RouteCount {
            count: 14,
            prefix: Some("/api".into()),
        },
        healthcheck: Some(HealthResult {
            path: "/api/health".into(),
            status: Some(200),
        }),
        not_covered: default_not_covered(),
        log_tail: vec![],
        error: None,
        reused: false,
        waiting_on: None,
    }
}

fn here_block() -> Problem {
    iii_cloud::local::here(
        "HEALTHCHECK_MISSING",
        "No healthcheck".into(),
        None,
        Owner::Agent,
        None,
    )
}

#[test]
fn ready_needs_a_clean_plan_no_blocking_check_here_and_a_passed_run_of_this_commit() {
    let clean = plan_with(PlanStatus::Clean);
    let run = passed_run("abc");
    assert!(is_ready(&clean, &[], Some(&run), "abc"));
    assert!(!is_ready(&clean, &[], None, "abc"));
    assert!(!is_ready(&clean, &[], Some(&run), "other"));
    assert!(!is_ready(
        &plan_with(PlanStatus::Blocked),
        &[],
        Some(&run),
        "abc"
    ));
    assert!(!is_ready(
        &plan_with(PlanStatus::NotPushed),
        &[],
        Some(&run),
        "abc"
    ));
    assert!(!is_ready(&clean, &[here_block()], Some(&run), "abc"));
    let mut failed = passed_run("abc");
    failed.status = RunStatus::Failed;
    assert!(!is_ready(&clean, &[], Some(&failed), "abc"));
}

#[test]
fn the_report_is_the_text_for_the_pull_request() {
    let text = report(
        "3c4d2e0aaaa",
        &plan_with(PlanStatus::Clean),
        Some(&passed_run("3c4d2e0aaaa")),
    );
    assert_eq!(
        text,
        "## iii cloud readiness\nCloud plan for 3c4d2e0: no blocking issue.\nRan here like the cloud: 3 of 3 workers, 14 routes under /api, healthcheck 200.\nNot covered: the cloud's build container, production's secret values, its machine size and real traffic.\n"
    );
    let blocked = PlanPart {
        blocking: 2,
        at_least: true,
        ..plan_with(PlanStatus::Blocked)
    };
    assert!(report("abc1234", &blocked, None).contains("2 blocking issues at least."));
}

// -------------------------------------------------------------- project

#[test]
fn remote_urls_parse_in_every_github_form() {
    for url in [
        "https://github.com/ada/hello-iii.git",
        "https://github.com/ada/hello-iii",
        "git@github.com:ada/hello-iii.git",
        "ssh://git@github.com/ada/hello-iii.git",
        "https://github.com/ada/hello-iii/",
    ] {
        assert_eq!(parse_remote(url).as_deref(), Some("ada/hello-iii"), "{url}");
    }
    for url in [
        "https://gitlab.com/ada/x.git",
        "/srv/git/x.git",
        "file:///tmp/x.git",
        "https://github.com/ada",
        "",
    ] {
        assert_eq!(parse_remote(url), None, "{url}");
    }
}

#[test]
fn the_default_environment_is_production_else_the_default_branch() {
    let env = |name: &str, production: bool, branch: &str| EnvironmentInfo {
        id: name.into(),
        name: name.into(),
        production,
        branch: branch.into(),
        manifest: None,
        url: None,
    };
    let list = vec![
        env("staging", false, "develop"),
        env("prod", true, "release"),
    ];
    assert_eq!(default_environment(&list, "main").unwrap().name, "prod");
    let list = vec![env("a", false, "x"), env("b", false, "main")];
    assert_eq!(default_environment(&list, "main").unwrap().name, "b");
}

// -------------------------------------------------------------- run here

#[test]
fn the_run_copy_moves_only_ports_and_databases() {
    let mut next = 40_000;
    let mut alloc = || {
        next += 1;
        next
    };
    let dbs = vec![
        Db {
            container: "database".into(),
            alias: "primary".into(),
            url: "postgres://postgres:pw@127.0.0.1:5001/app".into(),
        },
        Db {
            container: "database".into(),
            alias: "analytics".into(),
            url: "postgres://postgres:pw2@127.0.0.1:5002/app".into(),
        },
    ];
    let copy = rewrite_run_copy(
        &compose_fixture(),
        41_000,
        &[("http".into(), 42_000)],
        &dbs,
        &mut alloc,
    )
    .unwrap();
    let mut text = serde_json::to_string_pretty(&copy).unwrap();
    text.push('\n');
    support::check_golden("run-copy.json", &text).unwrap();
    // The harness's ports are gone from the copy.
    let flat = serde_json::to_string(&copy).unwrap();
    for port in FORBIDDEN_PORTS {
        assert!(
            !flat.contains(&format!(":{port}")) && !flat.contains(&format!("\"port\":{port}")),
            "{port}"
        );
    }
    // The cloud block is untouched.
    assert_eq!(copy["cloud"], compose_fixture()["cloud"]);
    // Free ports never are the harness's.
    for _ in 0..50 {
        assert!(!FORBIDDEN_PORTS.contains(&free_port()));
    }
}

#[test]
fn the_install_table_follows_the_cloud() {
    let dir = tempfile::tempdir().unwrap();
    let touch = |f: &str| std::fs::write(dir.path().join(f), "").unwrap();
    assert_eq!(lock_for("node@24", dir.path()), None);
    touch("pnpm-lock.yaml");
    assert_eq!(lock_for("node@24", dir.path()), Some(Lock::Pnpm));
    assert_eq!(
        lock_for("bun@1", dir.path()),
        None,
        "a bun project with only a pnpm lock installs nothing"
    );
    touch("bun.lockb");
    assert_eq!(lock_for("bun", dir.path()), Some(Lock::Bun));
    let table: Vec<(&str, &str)> = [
        (Lock::Npm, "npm ci"),
        (Lock::Pnpm, "pnpm install --frozen-lockfile"),
        (Lock::Yarn, "yarn install --frozen-lockfile"),
        (Lock::Bun, "bun install --frozen-lockfile"),
        (Lock::Uv, "uv sync --frozen --inexact"),
        (Lock::Go, "go mod download"),
        (Lock::Cargo, "cargo fetch --locked"),
    ]
    .iter()
    .map(|(l, c)| (*c, install_command(*l).0))
    .collect();
    for (want, got) in table {
        assert_eq!(want, got);
    }
    assert!(
        install_command(Lock::Pip)
            .0
            .contains(".venv/bin/pip install -r requirements.txt")
    );
    assert_eq!(
        install_command(Lock::Uv).1,
        &[("UV_PROJECT_ENVIRONMENT", ".venv")]
    );
}

#[test]
fn the_healthcheck_path_is_asked_as_the_cloud_asks_it() {
    // iii-cloud `crates/platform/src/health.rs`: the replica gets the path as set,
    // whatever `web.api.prefix` and `strip` say.
    assert_eq!(healthcheck_path("/api/health"), "/api/health");
    assert_eq!(healthcheck_path("health"), "/health");
}

#[test]
fn routes_must_sit_under_a_kept_prefix() {
    let routes = vec![
        "GET /api/users".to_string(),
        "GET /health".to_string(),
        "POST api".to_string(),
    ];
    assert_eq!(
        routes_outside_prefix(&routes, Some("/api"), false),
        vec!["GET /health".to_string()]
    );
    assert!(
        routes_outside_prefix(&routes, Some("/api"), true).is_empty(),
        "a stripped prefix puts every route under it"
    );
    assert!(routes_outside_prefix(&routes, None, false).is_empty());
}

#[test]
fn durations_variables_and_redaction() {
    assert_eq!(parse_duration("15m").unwrap().as_secs(), 900);
    assert_eq!(parse_duration("30s").unwrap().as_secs(), 30);
    assert_eq!(parse_duration("500ms").unwrap().as_millis(), 500);
    assert!(parse_duration("soon").is_none());
    let env = vec![("A".to_string(), "1".to_string())];
    assert_eq!(interpolate("${A}-${B:-x}-${C}-${D:y}", &env), "1-x--y");
    let secrets = vec!["hunter2hunter2".to_string(), "ab".to_string()];
    assert_eq!(
        redact("pw=hunter2hunter2 short", &secrets),
        "pw=[redacted] short",
        "values under 8 characters stay"
    );
}

#[test]
fn the_release_row_is_derived_from_the_workers_stage() {
    let stage = |status: &str| {
        vec![DeployStage {
            name: "workers".into(),
            status: status.into(),
            detail: None,
            started_at: None,
            finished_at: None,
        }]
    };
    assert!(release_row(false, &stage("running"), None).is_none());
    assert_eq!(
        release_row(true, &stage("pending"), None).unwrap().status,
        ReleaseRowStatus::Pending
    );
    assert_eq!(
        release_row(true, &stage("running"), None).unwrap().status,
        ReleaseRowStatus::Running
    );
    assert_eq!(
        release_row(true, &stage("passed"), None).unwrap().status,
        ReleaseRowStatus::Passed
    );
    let err = DeployError {
        stage: "workers".into(),
        worker: None,
        code: "RELEASE_FAILED".into(),
        hint: "x".into(),
        action: None,
    };
    assert_eq!(
        release_row(true, &stage("failed"), Some(&err))
            .unwrap()
            .status,
        ReleaseRowStatus::Failed
    );
    assert_eq!(outcome_of("in_progress"), Outcome::Running);
    assert_eq!(outcome_of("replaced"), Outcome::Replaced);
}

#[test]
fn a_no_route_line_is_a_deployment_event_and_health_is_evidence() {
    let events = vec![
        json!({"type": "log", "log": {"message": "Build ok"}}),
        json!({"type": "log", "log": {"message": "The group web runs http, but no function of this release serves a route. Routes are declared with iii.observe"}}),
    ];
    assert!(
        no_route_in(&events)
            .unwrap()
            .starts_with("The group web runs http")
    );
    assert!(no_route_in(&events[..1]).is_none());
    let lines = health_lines(
        &json!({"status": "degraded", "engine": "ready", "workers": [{"group": "web", "worker": "api", "ready": false, "restarts": 6, "crashloop": true}, {"group": "web", "worker": "http", "ready": true, "restarts": 0, "crashloop": false}]}),
    );
    assert_eq!(
        lines,
        vec![
            "health: degraded (engine ready)",
            "health: web/api not ready, 6 restarts, crashloop"
        ]
    );
}

// ------------------------------------------------------------- secrets

#[test]
fn secret_names_and_env_editing() {
    assert!(valid_name("SESSION_SECRET") && valid_name("_X1"));
    assert!(
        !valid_name("session")
            && !valid_name("III_TOKEN")
            && !valid_name("1A")
            && !valid_name("A-B")
            && !valid_name("")
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("sub/.env");
    set_env_line(&file, "A", "1").unwrap();
    set_env_line(&file, "B", "two words").unwrap();
    set_env_line(&file, "A", "3").unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "A=3\nB=two words\n",
        "replaced in place, appended otherwise"
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::write(&file, "# keep\nexport C=old\nD=1").unwrap();
    set_env_line(&file, "C", "new").unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# keep\nC=new\nD=1\n"
    );
    assert_eq!(
        set_env_line(&file, "E", "a\nb").unwrap_err().code,
        "INVALID_REQUEST"
    );
    assert!(!std::fs::read_to_string(&file).unwrap().contains("E="));
}

#[test]
fn a_value_in_env_reads_back_exactly_as_it_was_given() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".env");
    std::fs::write(&file, "P=old\nKEEP=1\nP=older\n").unwrap();
    let values = [
        ("P", "p@ss #1"),
        ("Q", "\"x\""),
        ("R", " a "),
        ("S", "it's \"both\""),
        ("T", "#not-a-comment"),
    ];
    for (name, value) in values {
        set_env_line(&file, name, value).unwrap();
    }
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        parse_env(&text),
        vec![
            ("P".to_string(), "p@ss #1".to_string()),
            ("KEEP".to_string(), "1".to_string()),
            ("Q".to_string(), "\"x\"".to_string()),
            ("R".to_string(), " a ".to_string()),
            ("S".to_string(), "it's \"both\"".to_string()),
            ("T".to_string(), "#not-a-comment".to_string()),
        ],
        "{text}"
    );
    assert_eq!(
        text.lines().filter(|l| l.starts_with("P=")).count(),
        1,
        "a later line would win: {text}"
    );
    // No way to write it that reads back the same: nothing is written.
    let err = set_env_line(&file, "U", " it's \\n").unwrap_err();
    assert_eq!(err.code, "LOCAL_NOT_SAVED");
    assert!(!err.message.contains("it's"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

#[test]
fn api_refuses_secret_values_and_temporary_credentials() {
    use iii_cloud::types::HttpMethod::*;
    let secret = json!({"type": "secret", "value": "x"});
    assert!(refuses_secret(
        Put,
        "/teams/t/environments/e/variables/NAME",
        Some(&secret)
    ));
    assert!(refuses_secret(
        Post,
        "/teams/t/environments/e/variables/NAME",
        Some(&secret)
    ));
    assert!(!refuses_secret(
        Put,
        "/teams/t/environments/e/variables/NAME",
        Some(&json!({"type": "variable", "value": "x"}))
    ));
    assert!(!refuses_secret(
        Get,
        "/teams/t/environments/e/variables",
        None
    ));
    assert!(refuses_secret(
        Get,
        "/teams/t/environments/e/resources/r/temporary-credential",
        None
    ));
    assert!(refuses_secret(
        Post,
        "/teams/t/environments/e/resources/r/temporary-credential",
        Some(&json!({}))
    ));
    assert!(valid_path("/teams/t/projects"));
    for bad in [
        "teams",
        "//evil",
        "/a/../b",
        "https://evil/x",
        "/a//b",
        "/a?x=1",
        "/a\\b",
    ] {
        assert!(!valid_path(bad), "{bad}");
    }
}

// -------------------------------------------------------- the checks here

fn write(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_checks_here_find_what_the_cloud_would_trip_on() {
    let ws = tempfile::tempdir().unwrap();
    let dir = ws.path();
    write(
        dir,
        "worker-compose.yaml",
        "containers:\n  http:\n    worker: package://http\n  api:\n    worker: path://./workers/api\n    environment:\n      TOKEN: ${TOKEN}\n      DB: ${DATABASE_PRIMARY_URL}\n      OK: ${OK:-fine}\n  database:\n    worker: package://database\n",
    );
    write(
        dir,
        "workers/api/package.json",
        r#"{"dependencies": {"iii-sdk": "^0.25.3"}}"#,
    );
    write(dir, "workers/api/iii.worker.yaml", "language: typescript\n");
    write(dir, "bun.lock", "");
    write(
        dir,
        ".github/workflows/ci.yml",
        "jobs:\n  x:\n    steps:\n      - run: pnpm db:migrate\n",
    );
    let manifest = Manifest::load(dir, None).unwrap();
    let matrix = json!({"engine": "0.27.0", "minimum_sdk": "0.26"});
    let settings = json!({});
    let problems = local_checks(&LocalInput {
        workspace: dir,
        manifest: Some(&manifest),
        filled: &[],
        settings: &settings,
        matrix: Some(&matrix),
        local_iii: Some("0.26.0"),
        plan_status: PlanStatus::Unavailable,
        plan_workers: None,
    });
    let by = |code: &str| {
        problems.iter().find(|p| p.code == code).unwrap_or_else(|| {
            panic!(
                "{code} missing in {:?}",
                problems.iter().map(|p| &p.code).collect::<Vec<_>>()
            )
        })
    };
    assert_eq!(
        by("LOCAL_ENGINE_VERSION").title,
        "iii 0.26.0 here, the cloud runs 0.27.0"
    );
    assert_eq!(by("LOCAL_ENGINE_VERSION").owner, Owner::Person);
    assert_eq!(by("SDK_TOO_OLD").title, "iii-sdk 0.25.3 is older than 0.26");
    assert_eq!(by("SDK_TOO_OLD").owner, Owner::Agent);
    assert_eq!(
        by("RUNTIME_LOCK_MISSING").title,
        "Runs with node, but the repository has only a bun.lock lock"
    );
    assert_eq!(
        by("RELEASE_IN_CI").detail.as_deref(),
        Some(".github/workflows/ci.yml runs db:migrate; it moves to cloud.release.")
    );
    assert_eq!(by("HEALTHCHECK_MISSING").owner, Owner::Agent);
    assert_eq!(
        by("LOCK_MISSING").detail.as_deref(),
        Some("worker-compose.lock")
    );
    let unset: Vec<_> = problems
        .iter()
        .filter(|p| p.code == "LOCAL_VARIABLE_UNSET")
        .map(|p| p.title.as_str())
        .collect();
    assert_eq!(
        unset,
        vec!["TOKEN has no value here"],
        "bindings and defaults are not asked for"
    );
    assert!(
        problems
            .iter()
            .all(|p| p.severity == Severity::Block && p.source == Source::CheckHere)
    );

    // With everything in place, nothing is found.
    write(
        dir,
        "workers/api/package.json",
        r#"{"dependencies": {"iii-sdk": "^0.26.1"}}"#,
    );
    write(
        dir,
        "workers/api/package-lock.json",
        r#"{"packages": {"node_modules/iii-sdk": {"version": "0.26.2"}}}"#,
    );
    write(dir, "package-lock.json", "{}");
    write(dir, "worker-compose.lock", "");
    write(dir, ".env", "TOKEN=abc\n");
    write(dir, ".github/workflows/ci.yml", "jobs: {}\n");
    write(
        dir,
        "worker-compose.yaml",
        &format!(
            "{}cloud:\n  healthcheck:\n    path: /health\n",
            std::fs::read_to_string(dir.join("worker-compose.yaml")).unwrap()
        ),
    );
    let manifest = Manifest::load(dir, None).unwrap();
    let problems = local_checks(&LocalInput {
        workspace: dir,
        manifest: Some(&manifest),
        filled: &[],
        settings: &settings,
        matrix: Some(&matrix),
        local_iii: Some("0.27.0"),
        plan_status: PlanStatus::Unavailable,
        plan_workers: None,
    });
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn versions_parse_and_compare() {
    assert_eq!(parse_version("0.27.0\n").as_deref(), Some("0.27.0"));
    assert_eq!(parse_version("iii 0.26.0 (abc)").as_deref(), Some("0.26.0"));
    assert!(
        accepts_cloud_block(Some("0.27.0"))
            && accepts_cloud_block(Some("0.27.1"))
            && !accepts_cloud_block(Some("0.26.9"))
            && !accepts_cloud_block(None)
    );
    assert!(version_tuple("0.26") < version_tuple("0.26.1"));
    assert!(version_tuple("0.25.9") < version_tuple("0.26"));
}

// ------------------------------------------------- review regressions

#[test]
fn every_env_value_of_three_characters_or_more_is_redacted() {
    let secrets = vec!["s3cret".to_string(), "ab".to_string()];
    assert_eq!(redact("pw=s3cret ab", &secrets), "pw=[redacted] ab");
}

#[test]
fn a_compose_file_of_the_wrong_shape_is_an_error_not_a_panic() {
    let mut next = 50_000;
    let mut alloc = || {
        next += 1;
        next
    };
    for text in [
        "engine: nope\n",
        "engine:\n  workers: [a]\n",
        "containers: [a, b]\n",
        "containers:\n  api: just-a-string\n",
        "containers:\n  api:\n    worker: path://./x\n    config_override: text\n",
        "- a list\n",
    ] {
        let yaml: Value = serde_yaml_ng::from_str(text).unwrap();
        assert!(
            rewrite_run_copy(&yaml, 1, &[], &[], &mut alloc).is_err(),
            "{text}"
        );
    }
}

#[test]
fn the_stream_worker_moves_even_without_a_port_in_the_file() {
    let mut next = 50_000;
    let mut alloc = || {
        next += 1;
        next
    };
    let yaml: Value = serde_yaml_ng::from_str("containers: {}\n").unwrap();
    let copy = rewrite_run_copy(&yaml, 41_000, &[], &[], &mut alloc).unwrap();
    {
        let name = "iii-stream";
        let port = copy["engine"]["workers"][name]["port"].as_u64().unwrap();
        assert!(
            port > 50_000 && !FORBIDDEN_PORTS.contains(&(port as u16)),
            "{name} {port}"
        );
        assert_eq!(copy["engine"]["workers"][name]["host"], "127.0.0.1");
    }
    // iii-http-functions takes no host or port (the engine refuses them): never added.
    assert!(
        copy["engine"]["workers"]
            .get("iii-http-functions")
            .is_none()
    );
}

#[test]
fn percent_encoded_dots_slashes_and_backslashes_are_refused() {
    for bad in [
        "/a/%2e%2e/b",
        "/a/%2E%2E/b",
        "/a/%2f/b",
        "/a%5Cb",
        "/a/b%00",
    ] {
        assert!(!valid_path(bad), "{bad}");
    }
    assert!(valid_path("/teams/t/projects"));
}

#[test]
fn an_env_file_that_cannot_be_read_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".env");
    std::fs::write(&file, [0xff, 0xfe, b'A', b'=', b'1', b'\n']).unwrap();
    assert!(set_env_line(&file, "B", "2").is_err());
    assert_eq!(
        std::fs::read(&file).unwrap(),
        vec![0xff, 0xfe, b'A', b'=', b'1', b'\n']
    );
}

#[test]
fn a_manifest_path_from_the_cloud_stays_inside_the_workspace() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("worker-compose.yaml"), "containers: {}\n").unwrap();
    assert!(Manifest::load(ws.path(), Some("worker-compose.yaml")).is_some());
    for bad in ["../x.yaml", "/etc/passwd", "a/../../x", "a\\b.yaml"] {
        assert!(Manifest::load(ws.path(), Some(bad)).is_none(), "{bad}");
    }
}

#[test]
fn a_state_file_that_does_not_parse_is_kept_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("state.json"), "{ not json, merges: [").unwrap();
    let store = iii_cloud::store::Store::open(dir.path());
    assert!(store.read(|s| s.merges.is_empty()));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("state.json.bad")).unwrap(),
        "{ not json, merges: ["
    );
    // Nothing was written over it before an update.
    assert!(!dir.path().join("state.json").exists());
    assert!(!iii_cloud::watch::is_terminal("replaced"));
    assert!(
        iii_cloud::watch::is_terminal("ready")
            && iii_cloud::watch::is_terminal("failed")
            && iii_cloud::watch::is_terminal("canceled")
    );
}

#[test]
fn problems_of_one_code_without_a_file_have_their_own_ids() {
    let ws = tempfile::tempdir().unwrap();
    let dir = ws.path();
    write(
        dir,
        "worker-compose.yaml",
        "containers:\n  api:\n    worker: path://./workers/api\n    environment:\n      A: ${ONE}\n      B: ${TWO}\n",
    );
    let manifest = Manifest::load(dir, None).unwrap();
    let settings = json!({});
    let problems = local_checks(&LocalInput {
        workspace: dir,
        manifest: Some(&manifest),
        filled: &[],
        settings: &settings,
        matrix: None,
        local_iii: None,
        plan_status: PlanStatus::Clean,
        plan_workers: None,
    });
    let unset: Vec<&Problem> = problems
        .iter()
        .filter(|p| p.code == "LOCAL_VARIABLE_UNSET")
        .collect();
    assert_eq!(unset.len(), 2);
    assert_ne!(unset[0].id, unset[1].id);
}

#[test]
fn a_secret_is_redacted_before_a_long_line_is_cut() {
    let secret = "s3cret-value-1234".to_string();
    let line = format!("{}{secret}", "x".repeat(395));
    let redacted = redact(&line, std::slice::from_ref(&secret));
    assert!(!redacted.contains("s3cret"));
    let cut_line: String = redacted.chars().take(400).collect();
    assert!(!cut_line.contains("s3cret-v"), "{cut_line}");
}

fn real_plan() -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/real-plan.json"
        ))
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn a_real_plan_gives_readable_problems_with_a_fix_each() {
    // The registry repo's plan from production: three blocking issues, none with a cloud `fix`.
    let problems = plan_problems(&real_plan()["plan"]);
    assert_eq!(problems.len(), 3);
    let lock = &problems[0];
    assert_eq!(lock.code, "LOCK_MISSING");
    assert_eq!(lock.title, "The lock file is missing");
    assert_eq!(
        lock.detail.as_deref(),
        Some("api/worker-compose-prod.lock is not in the repository.")
    );
    assert!(
        lock.fix
            .as_deref()
            .unwrap()
            .contains("iii compose build -f api/worker-compose-prod.yaml"),
        "{:?}",
        lock.fix
    );
    assert_eq!(lock.owner, Owner::Agent);
    let http = &problems[1];
    assert_eq!(http.code, "MISSING_WORKER_DIRECTORY");
    assert_eq!(http.cloud_code.as_deref(), Some("INVALID_COMPOSE_FILE"));
    assert_eq!(http.title, "A worker directory is missing");
    assert_eq!(
        http.detail.as_deref(),
        Some(
            "container 'http': worker directory api/workers/http does not exist (at containers.http.worker)"
        )
    );
    assert_eq!(
        http.fix.as_deref(),
        Some(
            "Point containers.http.worker at an existing directory, or add api/workers/http; paths are relative to api/."
        )
    );
    assert_eq!(
        http.owner,
        Owner::Agent,
        "fix_in_repo is the agent's, as the cloud says"
    );
    assert_eq!(
        (http.file.as_deref(), http.line),
        (Some("api/worker-compose-prod.yaml"), Some(21))
    );
    assert!(
        http.excerpt
            .as_ref()
            .unwrap()
            .lines
            .iter()
            .any(|l| l.contains("path://./workers/http"))
    );
    assert_ne!(problems[1].id, problems[2].id);
    for p in &problems {
        assert!(
            p.fix.as_deref().is_some_and(|f| !f.is_empty()),
            "{} has no fix",
            p.code
        );
        assert!(p.title != p.code && p.title.contains(' '), "{}", p.title);
        assert!(
            !p.detail.as_deref().unwrap_or("").contains(" · "),
            "{:?}",
            p.detail
        );
    }
}

#[test]
fn every_code_the_worker_can_emit_has_a_title_and_a_one_line_fix() {
    let known = iii_cloud::problems::known_codes();
    assert!(
        known.len() >= 60,
        "the skill's tables were not parsed: {}",
        known.len()
    );
    for (code, fix) in &known {
        assert!(
            !fix.trim().is_empty(),
            "{code} has an empty fix in skills/plan-codes.md"
        );
        let title = iii_cloud::problems::title_for(code);
        assert!(
            title != *code && title.contains(' ') && !title.contains('_'),
            "{code}: {title}"
        );
    }
    // The checks here, and the codes the worker adds, are all in the tables.
    for code in [
        "LOCAL_ENGINE_VERSION",
        "SDK_TOO_OLD",
        "RUNTIME_LOCK_MISSING",
        "RELEASE_IN_CI",
        "HEALTHCHECK_MISSING",
        "LOCAL_VARIABLE_UNSET",
        "LOCK_MISSING",
        "BASE_NOT_IN_HISTORY",
        "UNCOMMITTED_CHANGES",
    ] {
        assert!(
            iii_cloud::problems::fix_for(code, true).is_some_and(|f| !f.is_empty()),
            "{code}"
        );
    }
    // And every problem a check here makes carries one.
    let ws = tempfile::tempdir().unwrap();
    write(
        ws.path(),
        "worker-compose.yaml",
        "containers:\n  api:\n    worker: path://./workers/api\n    environment:\n      A: ${ONE}\n",
    );
    let manifest = Manifest::load(ws.path(), None).unwrap();
    let matrix = json!({"engine": "0.27.0", "minimum_sdk": "0.26"});
    let settings = json!({});
    let problems = local_checks(&LocalInput {
        workspace: ws.path(),
        manifest: Some(&manifest),
        filled: &[],
        settings: &settings,
        matrix: Some(&matrix),
        local_iii: Some("0.26.0"),
        plan_status: PlanStatus::Unavailable,
        plan_workers: None,
    });
    assert!(problems.len() >= 3);
    assert!(
        problems
            .iter()
            .all(|p| p.fix.as_deref().is_some_and(|f| !f.is_empty())),
        "{problems:?}"
    );
}

// ----------------------------------------------------- the database is the cloud's

fn local(dir: &Path, compose: &str, env: Option<&str>) -> Vec<Problem> {
    write(dir, "worker-compose.yaml", compose);
    if let Some(env) = env {
        write(dir, ".env", env);
    }
    let manifest = Manifest::load(dir, None).unwrap();
    let settings = json!({});
    local_checks(&LocalInput {
        workspace: dir,
        manifest: Some(&manifest),
        filled: &[],
        settings: &settings,
        matrix: None,
        local_iii: None,
        plan_status: PlanStatus::Clean,
        plan_workers: None,
    })
}

#[test]
fn database_shaped_names() {
    for yes in [
        "DATABASE_URL",
        "APP_DATABASE_URL",
        "DB_URL",
        "POSTGRES_URL",
        "POSTGRES_PRISMA_URL",
        "PGURL",
        "DATABASE_DIRECT_URL",
        "database_url",
    ] {
        assert!(database_shaped(yes), "{yes}");
    }
    for no in [
        "SESSION_SECRET",
        "API_URL",
        "REDIS_URL",
        "DATABASE_NAME",
        "TOKEN",
    ] {
        assert!(!database_shaped(no), "{no}");
    }
}

#[test]
fn a_database_url_with_an_empty_default_is_not_managed_either() {
    let ws = tempfile::tempdir().unwrap();
    let problems = local(
        ws.path(),
        "containers:\n  api:\n    worker: path://./workers/api\n    environment:\n      DATABASE_URL: ${DATABASE_URL:-}\n      DB_URL: ${DB_URL:-postgres://localhost/app}\n",
        None,
    );
    let db: Vec<&str> = problems
        .iter()
        .filter(|p| p.code == "DATABASE_NOT_MANAGED")
        .map(|p| p.title.as_str())
        .collect();
    assert_eq!(
        db,
        vec!["DATABASE_URL is not a managed database"],
        "a real default is a value, an empty one is not"
    );
    assert!(problems.iter().all(|p| p.code != "LOCAL_VARIABLE_UNSET"));
}

#[test]
fn a_database_url_read_from_the_environment_is_for_the_agent_not_a_person_to_provide() {
    // The registry's case: `DATABASE_URL: ${DATABASE_URL}` and no database worker.
    let ws = tempfile::tempdir().unwrap();
    let problems = local(
        ws.path(),
        "containers:\n  api:\n    worker: path://./workers/api\n    environment:\n      DATABASE_URL: ${DATABASE_URL}\n      TOKEN: ${TOKEN}\n",
        None,
    );
    let db = problems
        .iter()
        .find(|p| p.code == "DATABASE_NOT_MANAGED")
        .expect("flagged");
    assert_eq!((db.owner, db.severity), (Owner::Agent, Severity::Block));
    assert_eq!(
        db.detail.as_deref(),
        Some("api reads DATABASE_URL from the environment; iii cloud can create this database.")
    );
    let fix = db.fix.as_deref().unwrap();
    assert!(
        fix.contains("package://database")
            && fix.contains("worker-compose.yaml")
            && fix.contains("DATABASE_URL: ${DATABASE_PRIMARY_URL}")
            && fix.contains("on api")
            && fix.contains("pgvector"),
        "{fix}"
    );
    assert!(
        !problems
            .iter()
            .any(|p| p.code == "LOCAL_VARIABLE_UNSET" && p.title.contains("DATABASE_URL")),
        "never the person's"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.code == "LOCAL_VARIABLE_UNSET" && p.title.contains("TOKEN")),
        "a real variable still is"
    );
    // A value in the local .env does not make it managed.
    let problems = local(
        ws.path(),
        "containers:\n  api:\n    worker: path://./workers/api\n    environment:\n      DATABASE_URL: ${DATABASE_URL}\n",
        Some("DATABASE_URL=postgres://x\n"),
    );
    assert!(problems.iter().any(|p| p.code == "DATABASE_NOT_MANAGED"));
}

#[test]
fn a_binding_from_a_declared_database_worker_is_fine_and_another_name_gets_the_binding_in_its_fix()
{
    let ws = tempfile::tempdir().unwrap();
    let ok = "containers:\n  database:\n    worker: package://database\n    version: latest\n  api:\n    worker: path://./workers/api\n    environment:\n      DATABASE_URL: ${DATABASE_PRIMARY_URL}\n";
    assert!(
        local(ws.path(), ok, None)
            .iter()
            .all(|p| p.code != "DATABASE_NOT_MANAGED" && p.code != "LOCAL_VARIABLE_UNSET")
    );
    let wrong = ok.replace("${DATABASE_PRIMARY_URL}", "${DATABASE_URL}");
    let problems = local(ws.path(), &wrong, None);
    let db = problems
        .iter()
        .find(|p| p.code == "DATABASE_NOT_MANAGED")
        .unwrap();
    assert!(
        db.fix
            .as_deref()
            .unwrap()
            .contains("${DATABASE_PRIMARY_URL}")
            && db.fix.as_deref().unwrap().contains("already declared")
    );
}

#[test]
fn the_local_postgres_has_pgvector_like_the_cloud_and_every_default_agrees() {
    assert_eq!(
        iii_cloud::config::Config::default().postgres_image,
        "pgvector/pgvector:pg17"
    );
    let root = env!("CARGO_MANIFEST_DIR");
    for file in ["config.yaml", "iii.worker.yaml"] {
        let text = std::fs::read_to_string(format!("{root}/{file}")).unwrap();
        assert!(
            text.contains("postgres_image: pgvector/pgvector:pg17"),
            "{file}"
        );
    }
}

#[test]
fn only_package_http_is_the_http_worker_and_a_path_one_named_http_is_a_problem() {
    let ws = tempfile::tempdir().unwrap();
    let dir = ws.path();
    write(
        dir,
        "worker-compose.yaml",
        "containers:\n  http:\n    worker: path://./workers/http\n  front:\n    worker: path://./workers/front\n  api:\n    worker: path://./workers/api\n  pkg:\n    worker: package://http\n",
    );
    write(
        dir,
        "workers/front/iii.worker.yaml",
        "name: http\nlanguage: rust\n",
    );
    write(
        dir,
        "workers/api/iii.worker.yaml",
        "name: api\nlanguage: rust\n",
    );
    let manifest = Manifest::load(dir, None).unwrap();
    assert_eq!(manifest.http_containers(), vec!["pkg"], "the cloud's rule");
    let mut fake = manifest.fake_http_containers();
    fake.sort();
    assert_eq!(fake, vec!["front", "http"]);
    let problems = iii_cloud::local::local_checks(&iii_cloud::local::LocalInput {
        workspace: dir,
        manifest: Some(&manifest),
        filled: &[],
        settings: &json!({}),
        matrix: None,
        local_iii: None,
        plan_status: PlanStatus::Clean,
        plan_workers: None,
    });
    let p: Vec<_> = problems
        .iter()
        .filter(|p| p.code == "HTTP_WORKER_NOT_PACKAGE")
        .collect();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].owner, Owner::Agent);
    assert!(p[0].detail.as_deref().unwrap().contains("package://http"));
    // The run copy moves only the package to its own port; path workers are plain code.
    let mut next = 60_000;
    let mut alloc = || {
        next += 1;
        next
    };
    let ports: Vec<(String, u16)> = manifest
        .http_containers()
        .iter()
        .map(|n| (n.clone(), alloc()))
        .collect();
    let copy = rewrite_run_copy(&manifest.yaml, 41_000, &ports, &[], &mut alloc).unwrap();
    assert_eq!(
        copy["containers"]["pkg"]["config_override"]["host"],
        "127.0.0.1"
    );
    assert!(copy["containers"]["http"]["config_override"].is_null());
    assert!(copy["containers"]["front"]["config_override"].is_null());
}

#[test]
fn every_variable_reference_is_found_with_its_default() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "worker-compose.yaml",
        "containers:\n  api:\n    environment:\n      a: ${PLAIN}\n      b: ${EMPTY:-}\n      c: ${EMPTY2-}\n      d: ${WITH:-x}\n      e: ${PLAIN}\n      f: ${1BAD}\n      g: ${A.B}\n      h: ${COLON:}\n",
    );
    let manifest = Manifest::load(dir.path(), None).unwrap();
    let (uses, unsupported) = manifest.variable_uses();
    let found: Vec<(&str, Option<&str>)> = uses
        .iter()
        .map(|u| (u.name.as_str(), u.default.as_deref()))
        .collect();
    assert_eq!(
        found,
        vec![
            ("PLAIN", None),
            ("EMPTY", Some("")),
            ("WITH", Some("x")),
            ("PLAIN", None),
        ],
        "`${{A-}}` and `${{A:}}` outside config_override are text to iii cloud"
    );
    assert_eq!(
        unsupported
            .iter()
            .map(|u| u.written.as_str())
            .collect::<Vec<_>>(),
        vec!["${EMPTY2-}"]
    );
}

#[test]
fn the_config_the_runs_compose_wrote_is_removed_and_what_was_there_stays() {
    let ws = tempfile::tempdir().unwrap();
    let config = ws.path().join("api/config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("http.yaml"), "tracked").unwrap();
    let before = file_names(&config);
    for new in [
        "iii-cloud-run-database.yaml",
        "iii-cloud-run-secrets.yaml",
        "iii-cloud-run-state.yaml",
        "iii-stream.yaml",
    ] {
        std::fs::write(config.join(new), "written by the run").unwrap();
    }
    std::fs::write(config.join("notes.txt"), "not yaml: left alone").unwrap();
    remove_new_files(&config, &before);
    let mut left = file_names(&config);
    left.sort();
    assert_eq!(left, vec!["http.yaml", "notes.txt"]);
    // A config dir the run made itself goes too, once empty.
    let fresh = ws.path().join("web/config");
    std::fs::create_dir_all(&fresh).unwrap();
    std::fs::write(fresh.join("iii-stream.yaml"), "x").unwrap();
    remove_new_files(&fresh, &[]);
    assert!(!fresh.exists());
}

#[test]
fn a_failed_command_that_printed_no_error_is_silent() {
    use iii_cloud::watch::{failure_source, failure_title, silent_exit};
    let lines = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // drizzle-kit's case: the command ran, the tool printed nothing, the shell says the exit status.
    assert_eq!(
        silent_exit(&lines(&[
            "> api@ db:migrate",
            "> drizzle-kit migrate",
            "ELIFECYCLE  Command exited 1"
        ])),
        Some(1)
    );
    assert_eq!(
        silent_exit(&lines(&[
            "running migrations",
            "process exited with code 2"
        ])),
        Some(2)
    );
    // An error text anywhere means it did say why.
    assert_eq!(
        silent_exit(&lines(&[
            "error: permission denied for schema public",
            "exit status 1"
        ])),
        None
    );
    assert_eq!(
        silent_exit(&lines(&["Cannot find package 'express'", "exited 1"])),
        None
    );
    // No exit status, nothing to say.
    assert_eq!(silent_exit(&lines(&["all good"])), None);
    assert_eq!(silent_exit(&[]), None);
    // Titles and sources per code.
    assert_eq!(
        failure_title("RELEASE_FAILED", Some("pnpm --filter api db:migrate")),
        "The release command failed: `pnpm --filter api db:migrate`"
    );
    assert_eq!(
        failure_title("RELEASE_FAILED", None),
        "The release command failed"
    );
    assert_eq!(failure_title("BUILD_FAILED", None), "The build failed");
    assert_eq!(failure_source("RELEASE_FAILED", "workers"), Some("release"));
    assert_eq!(failure_source("BUILD_FAILED", "build"), Some("build"));
    assert_eq!(
        failure_source("WORKERS_NOT_READY", "workers"),
        Some("worker")
    );
    assert_eq!(failure_source("X", "validation"), None);
}

#[test]
fn passwords_in_urls_are_hidden() {
    use iii_cloud::run::redact_urls;
    assert_eq!(
        redact_urls("connect postgres://app_login:s3cr3tpw@127.0.0.1:5432/app failed"),
        "connect postgres://app_login:[redacted]@127.0.0.1:5432/app failed"
    );
    assert_eq!(
        redact_urls("see https://example.test/a/b and http://u@host/x"),
        "see https://example.test/a/b and http://u@host/x"
    );
    assert_eq!(
        redact_urls("a postgresql://u:p@h/db b postgres://x:y@z/d"),
        "a postgresql://u:[redacted]@h/db b postgres://x:[redacted]@z/d"
    );
}

#[test]
fn the_web_container_fix_says_where_the_setting_lives() {
    let plan = |source: &str, role: &str| {
        json!({"settings": {"web": {"serve": "container", "container": "http", "container_source": source}},
        "issues": [{"code": "WEB_CONTAINER_NOT_SERVABLE", "severity": "block", "context": {"container": "http", "role": role}}]})
    };
    let fix = |p: Value| iii_cloud::check::plan_problems(&p)[0].fix.clone().unwrap();
    // Stored in the environment (console or an earlier manifest): change it there.
    let stored = fix(plan("console", "code"));
    assert!(stored.contains("Change Web in the environment's settings"));
    assert!(stored.contains("does not clear it") && !stored.contains("package already serves"));
    // From the file: the file is the place.
    assert!(fix(plan("file", "code")).contains("Remove `cloud.web` from the compose file"));
    // The http package: say it already serves the routes.
    assert!(
        fix(plan("console", "http"))
            .contains("The http package already serves your routes: set Web to none.")
    );
}

#[test]
fn a_value_problem_about_a_database_url_is_the_agents_with_the_binding_fix() {
    let plan = json!({"issues": [
        {"code": "EMPTY_DEFAULT_VARIABLE", "severity": "warn", "kind": "resolve_here", "context": {"variable": "DATABASE_URL"}, "message": "DATABASE_URL defaults to empty."},
        {"code": "SECRET_MISSING", "severity": "block", "kind": "resolve_here", "message": "POSTGRES_URL has no value."},
        {"code": "SECRET_MISSING", "severity": "block", "kind": "resolve_here", "context": {"variable": "SESSION_SECRET"}, "message": "SESSION_SECRET has no value."}
    ]});
    let problems = iii_cloud::check::plan_problems(&plan);
    let owners: Vec<(&str, Owner)> = problems
        .iter()
        .map(|p| (p.code.as_str(), p.owner))
        .collect();
    assert_eq!(
        owners,
        vec![
            ("EMPTY_DEFAULT_VARIABLE", Owner::Agent),
            ("SECRET_MISSING", Owner::Agent),
            ("SECRET_MISSING", Owner::Person),
        ]
    );
    assert_eq!(
        problems[0].fix.as_deref(),
        Some(
            "Point DATABASE_URL at the managed database's binding (`${DATABASE_<ALIAS>_URL}`) in the app file; iii cloud makes the database."
        )
    );
    assert!(
        problems[1]
            .fix
            .as_deref()
            .unwrap()
            .starts_with("Point POSTGRES_URL at")
    );
    assert!(
        !problems[2]
            .fix
            .as_deref()
            .unwrap_or("")
            .contains("managed database")
    );
    // Here, a database URL is never LOCAL_VARIABLE_UNSET: DATABASE_NOT_MANAGED, the agent's, takes it.
    let ws = tempfile::tempdir().unwrap();
    let here = local(
        ws.path(),
        "containers:\n  api:\n    worker: path://./workers/api\n    environment:\n      DATABASE_URL: ${DATABASE_URL}\n",
        None,
    );
    assert!(here.iter().all(|p| p.code != "LOCAL_VARIABLE_UNSET"));
    assert!(
        here.iter()
            .any(|p| p.code == "DATABASE_NOT_MANAGED" && p.owner == Owner::Agent)
    );
}

#[test]
fn a_skipped_name_turns_an_empty_default_to_info_and_keeps_a_missing_secret_blocking() {
    let answer = json!({"can_deploy": false, "plan": {"issues": [
        {"code": "EMPTY_DEFAULT_VARIABLE", "severity": "warn", "kind": "resolve_here", "context": {"variable": "OPT"}, "message": "OPT defaults to empty."},
        {"code": "SECRET_MISSING", "severity": "block", "kind": "resolve_here", "context": {"variable": "TOKEN"}, "message": "TOKEN has no value."},
        {"code": "EMPTY_DEFAULT_VARIABLE", "severity": "warn", "kind": "resolve_here", "context": {"variable": "OTHER"}, "message": "OTHER defaults to empty."}
    ]}});
    let (mut part, mut problems) = iii_cloud::check::plan_part(&answer);
    assert_eq!(part.warnings, 2);
    let skipped = ["OPT", "TOKEN"].map(String::from).into_iter().collect();
    iii_cloud::check::mark_skipped(
        &answer,
        &mut part,
        &mut problems,
        &skipped,
        &Default::default(),
    );
    let got: Vec<(&str, Severity, bool)> = problems
        .iter()
        .map(|p| (p.code.as_str(), p.severity, p.skipped))
        .collect();
    assert_eq!(
        got,
        vec![
            ("EMPTY_DEFAULT_VARIABLE", Severity::Info, true),
            ("SECRET_MISSING", Severity::Block, true),
            ("EMPTY_DEFAULT_VARIABLE", Severity::Warn, false),
        ]
    );
    assert_eq!(part.warnings, 1, "a skipped one never counts");
    assert_eq!(part.blocking, 1, "iii cloud still blocks");
    assert_eq!(
        problems[1].detail.as_deref(),
        Some("iii cloud needs a value to deploy, even an empty one.")
    );
    let wire = serde_json::to_value(&problems[2]).unwrap();
    assert!(wire.get("skipped").is_none(), "absent when false");
}

#[test]
fn a_running_deploy_is_never_pruned_by_newer_ones() {
    let deploy = |id: &str, status: &str, at: &str| {
        iii_cloud::watch::to_deploy(
            "http://cloud.test",
            (None, None, "production"),
            &json!({"id": id, "number": 1, "environment_id": "e", "type": "push", "status": status, "commit": {"sha": "abc"}, "stages": [], "created_at": at}),
            None,
            false,
        )
        .unwrap()
    };
    let mut s = iii_cloud::store::State::default();
    s.put_deploy(deploy("old-running", "in_progress", "2026-01-01T00:00:00Z"));
    s.put_deploy(deploy("old-ready", "ready", "2026-01-01T00:00:01Z"));
    for i in 0..25 {
        s.put_deploy(deploy(
            &format!("new-{i:02}"),
            "ready",
            &format!("2026-11-01T00:00:{i:02}Z"),
        ));
    }
    assert!(s.deploys.iter().any(|d| d.id == "old-running"));
    assert!(s.deploys.iter().all(|d| d.id != "old-ready"));
    assert_eq!(s.deploys.len(), 21);
}

// ------------------------------------------------------------ how an environment deploys

/// The cases `ui/src/__tests__/logic.test.tsx` reads too: one rule, two languages.
#[test]
fn an_environment_deploys_by_pull_request_only_in_production() {
    use iii_cloud::project::{Way, deploy_way};
    let cases: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/deploy-way.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(cases.len() >= 11);
    for c in cases {
        let e = &c["env"];
        let env = EnvironmentInfo {
            id: "e".into(),
            name: e["name"].as_str().unwrap().into(),
            production: e["production"].as_bool().unwrap(),
            branch: e["branch"].as_str().unwrap().into(),
            manifest: None,
            url: None,
        };
        let others: Vec<String> = c["others"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o.as_str().unwrap().to_string())
            .collect();
        let want = match c["way"].as_str().unwrap() {
            "pull_request" => Way::PullRequest,
            _ => Way::Assisted {
                own: c["own"].as_str().map(str::to_owned),
                taken: c["taken"].as_bool().unwrap(),
            },
        };
        assert_eq!(
            deploy_way(c["default"].as_str(), &env, &others),
            want,
            "{}",
            c["case"]
        );
    }
}

#[test]
fn an_environment_whose_production_flag_cannot_be_read_counts_as_production() {
    let read = |v: Value| iii_cloud::project::env_info(&v).unwrap().production;
    assert!(read(json!({"id": "e", "name": "n", "branch": "b"})));
    assert!(read(
        json!({"id": "e", "name": "n", "branch": "b", "production": "yes"})
    ));
    assert!(!read(
        json!({"id": "e", "name": "n", "branch": "b", "production": false})
    ));
}
