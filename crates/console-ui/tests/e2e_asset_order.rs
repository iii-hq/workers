//! `ConsoleUi::register` against a live engine: the order a console sees a
//! worker's asset observers in.
//!
//! A stand-in console hosts `console::ui::asset` with an added hook that
//! records the order its hooks run in (the order the real console commits
//! assets in) and fetches each asset through its observer. Every case runs
//! in a namespace of its own. Needs an engine at `III_ENGINE_WS_URL`
//! (default `ws://127.0.0.1:49134`); without one the tests skip, or fail when
//! `III_E2E_REQUIRE` is set.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_console_ui::ConsoleUi;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    CallRequest, IIIClient, InitOptions, ObserveRejection, Observer, RegisterFunction,
    register_worker,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";
/// Fresh workers per case: the SDK replays observers created before the
/// connection is up in an order of its own, so one run proves little.
const RUNS: usize = 3;

fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

fn client(namespace: &str, name: &str) -> Arc<IIIClient> {
    Arc::new(register_worker(
        &ws_url(),
        InitOptions {
            namespace: Some(namespace.to_string()),
            metadata: Some(WorkerMetadata {
                name: name.to_string(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ))
}

/// Waits until `iii` reaches the engine; false after about 5 s.
async fn connected(iii: &IIIClient) -> bool {
    for _ in 0..20 {
        let probe = iii
            .call(
                CallRequest {
                    function_id: "engine::workers::list".to_string(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(800),
                }
                .namespace("default"),
            )
            .await;
        if probe.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

/// True when an engine answers; otherwise skips, or panics under
/// `III_E2E_REQUIRE`. `case` keeps concurrent probes apart.
async fn engine_or_skip(case: &str) -> bool {
    let probe = client(&format!("cui-{case}-probe-{}", std::process::id()), "probe");
    let up = connected(&probe).await;
    probe.shutdown_async().await;
    if up {
        return true;
    }
    if std::env::var("III_E2E_REQUIRE").is_ok() {
        panic!(
            "e2e requires a running iii engine at {} - set III_ENGINE_WS_URL",
            ws_url()
        );
    }
    eprintln!(
        "[skip] no iii engine reachable at {} - skipping (set III_E2E_REQUIRE=1 to fail instead)",
        ws_url()
    );
    false
}

/// A stand-in for the console's asset function. Other workers' assets (the
/// engine's own console UI among them) may reach it too; only the paths under
/// `prefix/` count.
struct Console {
    iii: Arc<IIIClient>,
    /// Asset paths, in the order the added hook ran for them.
    hooks: Arc<Mutex<Vec<String>>>,
    prefix: String,
    /// `(args, content)` for every asset fetched through its observer.
    fetched: mpsc::UnboundedReceiver<(Value, Value)>,
}

fn console(namespace: &str, prefix: &str) -> Console {
    let iii = client(namespace, "console");
    let hooks = Arc::new(Mutex::new(Vec::new()));
    let (tx, fetched) = mpsc::unbounded_channel();
    let (order, fetcher) = (hooks.clone(), iii.clone());
    iii.on_observer_added_for("console::ui::asset", move |obs: Observer| {
        let (order, tx, iii) = (order.clone(), tx.clone(), fetcher.clone());
        async move {
            let args: Value = obs.args()?;
            let Some(path) = args["path"].as_str().map(str::to_string) else {
                return Err(ObserveRejection::new("INVALID_ARGS", "no path"));
            };
            order.lock().unwrap().push(path.clone());
            tokio::spawn(async move {
                if obs.accepted().await.is_err() {
                    return;
                }
                let content = iii
                    .call(CallRequest::observer(&obs, json!({ "path": path })))
                    .await
                    .expect("the content function answers through the observer");
                let _ = tx.send((args, content));
            });
            Ok(())
        }
    });
    iii.register_function(
        "console::ui::asset",
        RegisterFunction::new(|input: Value| Ok::<_, Error>(input))
            .args_format_value(json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["script", "style", "module"] },
                    "path": { "type": "string", "maxLength": 512 }
                },
                "required": ["kind", "path"],
                "additionalProperties": false
            }))
            .private(),
    );
    Console {
        iii,
        hooks,
        prefix: format!("{prefix}/"),
        fetched,
    }
}

/// Two scripts, two modules and a style under `prefix/`, a script first.
fn ui(prefix: &str) -> ConsoleUi {
    ConsoleUi::new(prefix)
        .script(format!("{prefix}/page.js"), "export const page = 1")
        .style(format!("{prefix}/styles.css"), "a { color: red }")
        .module(format!("{prefix}/heavy.js"), "export const heavy = 1")
        .script(format!("{prefix}/other.js"), "export const other = 1")
        .module(format!("{prefix}/diagram.js"), "export const diagram = 1")
}

/// Waits for all five assets, then checks that every module's hook ran
/// before every script's and that each asset served its own content.
async fn assert_modules_before_scripts(console: &mut Console, case: &str) {
    let mut fetched = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        while fetched.len() < 5 {
            let (args, content) = console.fetched.recv().await.expect("fetch");
            if args["path"]
                .as_str()
                .is_some_and(|path| path.starts_with(&console.prefix))
            {
                fetched.push((args, content));
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{case}: five assets fetched through their observers"));

    let hooks: Vec<String> = console
        .hooks
        .lock()
        .unwrap()
        .iter()
        .filter(|path| path.starts_with(&console.prefix))
        .cloned()
        .collect();
    let at = |name: &str| {
        let path = format!("{}{name}", console.prefix);
        hooks
            .iter()
            .position(|p| *p == path)
            .unwrap_or_else(|| panic!("{case}: no hook ran for {path} (hooks: {hooks:?})"))
    };
    let last_module = at("heavy.js").max(at("diagram.js"));
    let first_script = at("page.js").min(at("other.js"));
    assert!(
        last_module < first_script,
        "{case}: a script reached the console before a module: {hooks:?}"
    );

    for (args, content) in fetched {
        let path = args["path"].as_str().unwrap();
        let name = path
            .trim_start_matches(&console.prefix)
            .trim_end_matches(".js")
            .trim_end_matches(".css");
        let body = content["content"].as_str().unwrap();
        match args["kind"].as_str().unwrap() {
            "style" => assert!(
                content["content_type"]
                    .as_str()
                    .unwrap()
                    .starts_with("text/css")
            ),
            kind @ ("script" | "module") => {
                assert_eq!(
                    body,
                    format!("export const {name} = 1"),
                    "{case}: {kind} {path}"
                );
                assert!(
                    content["content_type"]
                        .as_str()
                        .unwrap()
                        .starts_with("text/javascript")
                );
            }
            other => panic!("{case}: unexpected kind {other}"),
        }
    }
}

/// The usual boot: `register` right after `register_worker`, before the
/// connection is up, and the console registers afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scripts_follow_modules_when_registered_before_the_connection_and_the_console() {
    if !engine_or_skip("boot").await {
        return;
    }
    for run in 0..RUNS {
        let namespace = format!("cui-boot-{}-{run}", std::process::id());
        let prefix = format!("boot{run}");
        let worker = client(&namespace, &prefix);
        ui(&prefix).register(&worker);
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut console = console(&namespace, &prefix);
        assert_modules_before_scripts(&mut console, &format!("boot run {run}")).await;
        worker.shutdown_async().await;
        console.iii.shutdown_async().await;
    }
}

/// The console is up and the worker connected before `register`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scripts_follow_modules_when_the_console_is_up() {
    if !engine_or_skip("live").await {
        return;
    }
    for run in 0..RUNS {
        let namespace = format!("cui-live-{}-{run}", std::process::id());
        let prefix = format!("live{run}");
        let mut console = console(&namespace, &prefix);
        let worker = client(&namespace, &prefix);
        assert!(connected(&worker).await, "the worker connects");
        assert!(connected(&console.iii).await, "the console connects");
        ui(&prefix).register(&worker);
        assert_modules_before_scripts(&mut console, &format!("live run {run}")).await;
        worker.shutdown_async().await;
        console.iii.shutdown_async().await;
    }
}
