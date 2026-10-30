//! `ConsoleUi::register` against a live engine: the order a console sees a
//! worker's asset observers in, the content each one serves, and the
//! observer each content fetch reaches.
//!
//! A stand-in console hosts `console::ui::asset` with an added hook that
//! records the order its hooks run in (the order the real console commits
//! assets in). Once an observer is active, the console fetches its asset with
//! a call of `console::ui::asset` for that observer alone (metadata
//! `{ for_observer }`). The match hook is the ADE console's
//! (`ade/src/ui_assets.rs`, `for_observer::own_calls_only`): a call for one
//! observer reaches it alone, and a call that names no observer reaches none.
//! The deliver hook calls the observer's target, the worker's content
//! function, with `{ path }` and answers with what the target returns. The
//! stand-in supersedes nothing, so two instances of a worker keep two active
//! observers of each path. Every case runs in a namespace of its own. Needs
//! an engine at `III_ENGINE_WS_URL` (default `ws://127.0.0.1:49134`); without
//! one the tests skip, or fail when `III_E2E_REQUIRE` is set.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{client, connected, engine_or_skip};
use iii_console_ui::ConsoleUi;
use iii_sdk::errors::Error;
use iii_sdk::{Call, CallRequest, IIIClient, Match, ObserveRejection, Observer, RegisterFunction};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// Fresh workers per case: the SDK replays observers created before the
/// connection is up in an order of its own, so one run proves little.
const RUNS: usize = 3;

const ASSET_FN: &str = "console::ui::asset";

/// One asset fetched through its observer.
struct Fetched {
    observer_id: String,
    args: Value,
    /// What the fetch answered with, or why it failed.
    content: Result<Value, String>,
}

/// One run of the asset function's deliver hook: the observer the call was
/// for, and the observers the match hook gave it.
struct Delivery {
    for_observer: Option<String>,
    observers: Vec<String>,
}

/// A stand-in for the console's asset function. Other workers' assets (the
/// engine's own console UI among them) may reach it too; only the paths under
/// `prefix/` count.
struct Console {
    iii: Arc<IIIClient>,
    /// Asset paths, in the order the added hook ran for them.
    hooks: Arc<Mutex<Vec<String>>>,
    /// Every observer the removed hook ran for.
    removed: Arc<Mutex<Vec<String>>>,
    /// Every run of the deliver hook.
    deliveries: Arc<Mutex<Vec<Delivery>>>,
    prefix: String,
    /// Every asset fetched through its observer.
    fetched: mpsc::UnboundedReceiver<Fetched>,
}

/// The observer a call is for: `metadata.for_observer`.
fn for_observer(call: &Call) -> Option<&str> {
    call.metadata
        .as_ref()
        .and_then(|metadata| metadata["for_observer"].as_str())
}

fn console(namespace: &str, prefix: &str) -> Console {
    let iii = client(namespace, "console");
    let hooks = Arc::new(Mutex::new(Vec::new()));
    let removed = Arc::new(Mutex::new(Vec::new()));
    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let (tx, fetched) = mpsc::unbounded_channel();
    let (order, fetcher) = (hooks.clone(), iii.clone());
    iii.on_observer_added_for(ASSET_FN, move |obs: Observer| {
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
                let fetch = CallRequest::new(ASSET_FN, json!({ "path": path }))
                    .metadata(json!({ "for_observer": obs.id() }));
                let content = iii.call(fetch).await.map_err(|error| error.to_string());
                let _ = tx.send(Fetched {
                    observer_id: obs.id().to_string(),
                    args,
                    content,
                });
            });
            Ok(())
        }
    });
    let gone = removed.clone();
    iii.on_observer_removed_for(ASSET_FN, move |obs: Observer| {
        let gone = gone.clone();
        async move {
            gone.lock().unwrap().push(obs.id().to_string());
        }
    });
    // A content fetch is a call for one asset observer and reaches it alone;
    // a call that names no observer reaches none.
    iii.on_observer_match_for(
        ASSET_FN,
        Match::predicate(|obs, call| for_observer(call) == Some(obs.id())),
    );
    // The target receives `{ path }`; the caller gets the target's answer.
    let (record, deliverer) = (deliveries.clone(), iii.clone());
    iii.on_observer_deliver_for(ASSET_FN, move |call: Call, observers: Vec<Observer>| {
        let (record, iii) = (record.clone(), deliverer.clone());
        async move {
            record.lock().unwrap().push(Delivery {
                for_observer: for_observer(&call).map(str::to_string),
                observers: observers.iter().map(|obs| obs.id().to_string()).collect(),
            });
            let [obs] = observers.as_slice() else {
                return Err(Error::Handler(format!(
                    "{} observers serve {}",
                    observers.len(),
                    call.input["path"]
                )));
            };
            if !obs.matches(&call) {
                return Err(Error::Handler(
                    "the observer's filters refuse the call".into(),
                ));
            }
            iii.call(CallRequest::observer(obs, call.input.clone()))
                .await
        }
    });
    iii.register_function(
        ASSET_FN,
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
        removed,
        deliveries,
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

/// Waits for `count` more assets under the console's prefix, fetched through
/// their observers.
async fn fetched(console: &mut Console, count: usize, case: &str) -> Vec<Fetched> {
    let mut fetched = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        while fetched.len() < count {
            let asset = console.fetched.recv().await.expect("fetch");
            if asset.args["path"]
                .as_str()
                .is_some_and(|path| path.starts_with(&console.prefix))
            {
                fetched.push(asset);
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{case}: {count} assets fetched through their observers"));
    fetched
}

/// Checks that every module's hook ran before every script's.
fn assert_modules_before_scripts(console: &Console, case: &str) {
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
}

/// Checks that each fetch reached its own observer alone, and that the
/// observer got no other call.
fn assert_each_fetch_reached_its_observer_alone(
    console: &Console,
    fetched: &[Fetched],
    case: &str,
) {
    let deliveries = console.deliveries.lock().unwrap();
    for asset in fetched {
        let id = asset.observer_id.as_str();
        let reached: Vec<&Delivery> = deliveries
            .iter()
            .filter(|delivery| delivery.observers.iter().any(|obs| obs == id))
            .collect();
        assert_eq!(
            reached.len(),
            1,
            "{case}: observer {id} of {} got one call, its own fetch",
            asset.args["path"]
        );
        assert_eq!(reached[0].for_observer.as_deref(), Some(id), "{case}");
        assert_eq!(
            reached[0].observers,
            [id],
            "{case}: the fetch reached {id} alone"
        );
    }
}

/// Checks that each asset served its own content.
fn assert_content(console: &Console, fetched: &[Fetched], case: &str) {
    for Fetched { args, content, .. } in fetched {
        let path = args["path"].as_str().unwrap();
        let content = content
            .as_ref()
            .unwrap_or_else(|error| panic!("{case}: the fetch of {path} failed: {error}"));
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

/// Waits for all five assets of one worker, then checks that every module's
/// hook ran before every script's, that each fetch reached its own observer
/// alone, once, and that each asset served its own content.
async fn assert_one_worker(console: &mut Console, case: &str) {
    let fetched = fetched(console, 5, case).await;
    assert_modules_before_scripts(console, case);
    assert_each_fetch_reached_its_observer_alone(console, &fetched, case);
    assert_content(console, &fetched, case);
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
        assert_one_worker(&mut console, &format!("boot run {run}")).await;
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
        assert_one_worker(&mut console, &format!("live run {run}")).await;
        worker.shutdown_async().await;
        console.iii.shutdown_async().await;
    }
}

/// Two instances of one worker, each under a worker name of its own, observe
/// the same five assets, so each path has two active observers when the
/// second instance's are fetched. A fetch for one observer reaches it alone:
/// the other observer of its path gets nothing. A call of the function that
/// names no observer reaches neither.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fetch_reaches_its_observer_alone_among_the_observers_of_its_path() {
    if !engine_or_skip("twins").await {
        return;
    }
    let case = "twins";
    let namespace = format!("cui-twins-{}", std::process::id());
    let mut console = console(&namespace, case);
    let first = client(&namespace, "twins-first");
    let second = client(&namespace, "twins-second");
    for iii in [&console.iii, &first, &second] {
        assert!(connected(iii).await, "the worker connects");
    }

    ui(case).register(&first);
    let mut assets = fetched(&mut console, 5, "twins, first instance").await;
    let first_ids: Vec<String> = assets.iter().map(|a| a.observer_id.clone()).collect();
    ui(case).register(&second);
    assets.extend(fetched(&mut console, 5, "twins, second instance").await);

    let removed = console.removed.lock().unwrap().clone();
    assert!(
        first_ids.iter().all(|id| !removed.contains(id)),
        "{case}: the first instance's observers stay active while the second's are fetched \
         (removed: {removed:?})"
    );
    let mut by_path: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for asset in &assets {
        by_path
            .entry(asset.args["path"].as_str().unwrap())
            .or_default()
            .push(asset.observer_id.as_str());
    }
    assert_eq!(by_path.len(), 5, "{case}: five paths: {by_path:?}");
    for (path, ids) in &by_path {
        assert!(
            ids.len() == 2 && ids[0] != ids[1],
            "{case}: two observers of {path}: {ids:?}"
        );
    }
    assert_each_fetch_reached_its_observer_alone(&console, &assets, case);
    assert_content(&console, &assets, case);

    // A call of the function that names no observer.
    let page = format!("{case}/page.js");
    let unnamed = console
        .iii
        .call(CallRequest::new(ASSET_FN, json!({ "path": page })))
        .await;
    assert!(
        unnamed.is_err(),
        "{case}: no observer serves a call that names none: {unnamed:?}"
    );
    {
        let deliveries = console.deliveries.lock().unwrap();
        let unnamed: Vec<&Vec<String>> = deliveries
            .iter()
            .filter(|delivery| delivery.for_observer.is_none())
            .map(|delivery| &delivery.observers)
            .collect();
        assert_eq!(
            unnamed,
            [&Vec::<String>::new()],
            "{case}: the call that names no observer reaches none"
        );
    }

    second.shutdown_async().await;
    first.shutdown_async().await;
    console.iii.shutdown_async().await;
}
