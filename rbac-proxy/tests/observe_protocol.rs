//! iii 0.25's observe protocol against the interceptor, one vendored wire
//! fixture at a time (`tests/fixtures/protocol/observe/`, see its README).
//!
//! The proxy does not support observe yet, and the engine behind it gates none
//! of it for a connection without a session, so every observe message must be
//! refused or dropped, and the frames that keep flowing must lose what would
//! turn observe on: the `features` either side declares, the `observe`
//! descriptor of a registration, and the call-chain fields the SDK asserts.
//!
//! Every fixture file needs an entry in `expectation`, so one added upstream
//! fails here until someone decides what the proxy does with it.

use std::path::PathBuf;
use std::sync::Arc;

use rbac_proxy::config::WorkerConfig;
use rbac_proxy::engine_overrides::CatalogCache;
use rbac_proxy::interceptor::{DownstreamAction, Interceptor, UpstreamAction};
use rbac_proxy::rbac::{FunctionFilter, ProxySession, RbacConfig};
use serde_json::{json, Value};
use tokio::sync::mpsc;

/// What the proxy must do with one fixture.
enum Expect {
    /// Worker → engine, refused with this frame type and error code.
    RefusedDownstream {
        reply_type: &'static str,
        code: &'static str,
    },
    /// Worker → engine, dropped.
    DroppedDownstream,
    /// Worker → engine, forwarded as `rewrite` turns the fixture.
    ForwardedDownstream(fn(&mut Value)),
    /// Engine → worker, dropped.
    DroppedUpstream,
    /// Engine → worker, forwarded as `rewrite` turns the fixture.
    ForwardedUpstream(fn(&mut Value)),
}

fn unchanged(_: &mut Value) {}

fn without_features(v: &mut Value) {
    v.as_object_mut().unwrap().remove("features");
}

/// The register call as the engine gets it: no features, and the name pinned
/// behind the proxy's `rbac-proxy:` prefix.
fn registered_through_proxy(v: &mut Value) {
    v["data"].as_object_mut().unwrap().remove("features");
    let name = v["data"]["name"].as_str().unwrap();
    v["data"]["name"] = json!(format!("rbac-proxy:{name}"));
}

fn without_observe(v: &mut Value) {
    v.as_object_mut().unwrap().remove("observe");
}

fn opt_out_only(v: &mut Value) {
    v["observe"] = json!({ "allow": false });
}

fn expectation(file: &str) -> Expect {
    use Expect::*;
    match file {
        // Worker → engine.
        "observe.json" | "observe-minimal.json" | "observe-auto.json" => RefusedDownstream {
            reply_type: "observeresult",
            code: "UNSUPPORTED",
        },
        "unobserve.json"
        | "observerack.json"
        | "observerack-error.json"
        | "invocationsobserved.json"
        | "localinvocationstats.json" => DroppedDownstream,
        "invokefunction-owner-call.json" | "invokefunction-owner-call-namespace.json" => {
            RefusedDownstream {
                reply_type: "invocationresult",
                code: "UNSUPPORTED",
            }
        }
        "registerfunction.json" => ForwardedDownstream(unchanged),
        "registerfunction-owner.json" => ForwardedDownstream(without_observe),
        "registerfunction-not-observable.json" => ForwardedDownstream(opt_out_only),
        "invokefunction-legacy.json" => ForwardedDownstream(unchanged),
        "invokefunction-workers-register.json" => ForwardedDownstream(registered_through_proxy),
        "unregistertriggertype.json" => ForwardedDownstream(unchanged),

        // Engine → worker.
        "workerregistered.json" => ForwardedUpstream(without_features),
        "workerregistered-legacy.json" => ForwardedUpstream(unchanged),
        "observeresult.json"
        | "observeresult-error.json"
        | "observerstatechanged.json"
        | "observerstatechanged-reason.json"
        | "functionregistered.json"
        | "functionregistered-empty.json"
        | "observeradded.json"
        | "observeradded-legacy.json"
        | "observerremoved.json" => DroppedUpstream,
        // A trusted worker's observer may target a function this connection
        // hosts; the activation is an ordinary call to it.
        "invokefunction-activation.json" => ForwardedUpstream(unchanged),
        "invokefunction-caller.json" => ForwardedUpstream(unchanged),

        other => panic!(
            "no expectation for fixture {other}: decide what rbac-proxy does with it \
             and add it to `expectation`"
        ),
    }
}

fn fixtures() -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/protocol/observe");
    let mut out: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .expect("fixture directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "json").then_some(path)
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).expect("fixture reads");
            (name, serde_json::from_str(&text).expect("fixture is JSON"))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A connection that may call anything, in any namespace the fixtures use,
/// so what the proxy does to a frame is down to observe alone.
fn interceptor() -> Interceptor {
    let iii = Arc::new(iii_sdk::register_worker(
        "ws://127.0.0.1:1",
        iii_sdk::InitOptions::default(),
    ));
    let config = Arc::new(WorkerConfig {
        rbac: RbacConfig {
            expose_functions: vec![FunctionFilter::match_pattern("*")],
            ..RbacConfig::default()
        },
        ..WorkerConfig::default()
    });
    let session = Arc::new(ProxySession {
        namespaces: ["default", "orders", "acme"]
            .into_iter()
            .map(|ns| (ns.to_string(), vec!["*".to_string()]))
            .collect(),
        ..ProxySession::permissive("127.0.0.1".to_string())
    });
    let (tx, rx) = mpsc::channel::<String>(16);
    Box::leak(Box::new(rx));
    Interceptor::new(
        iii.clone(),
        config,
        Arc::new(CatalogCache::new(iii)),
        session,
        tx,
    )
}

#[tokio::test]
async fn every_observe_fixture_is_refused_dropped_or_defused() {
    let fixtures = fixtures();
    assert!(fixtures.len() >= 29, "fixtures missing: {}", fixtures.len());

    for (file, fixture) in fixtures {
        let text = fixture.to_string();
        // A fresh connection per fixture: the register call records state.
        let i = interceptor();
        match expectation(&file) {
            Expect::RefusedDownstream { reply_type, code } => {
                let action = i.handle_downstream(&text).await;
                let DownstreamAction::ReplyToClient(reply) = action else {
                    panic!("{file}: expected a refusal, got {action:?}");
                };
                let reply: Value = serde_json::from_str(&reply).unwrap();
                assert_eq!(reply["type"], reply_type, "{file}");
                assert_eq!(reply["error"]["code"], code, "{file}");
            }
            Expect::DroppedDownstream => {
                assert_eq!(
                    i.handle_downstream(&text).await,
                    DownstreamAction::Drop,
                    "{file}"
                );
            }
            Expect::ForwardedDownstream(rewrite) => {
                let action = i.handle_downstream(&text).await;
                let DownstreamAction::Forward(out) = action else {
                    panic!("{file}: expected a forward, got {action:?}");
                };
                let mut expected = fixture.clone();
                rewrite(&mut expected);
                assert_eq!(
                    serde_json::from_str::<Value>(&out).unwrap(),
                    expected,
                    "{file}"
                );
            }
            Expect::DroppedUpstream => {
                assert_eq!(
                    i.handle_upstream(&text).await,
                    UpstreamAction::Drop,
                    "{file}"
                );
            }
            Expect::ForwardedUpstream(rewrite) => {
                let action = i.handle_upstream(&text).await;
                let UpstreamAction::Forward(out) = action else {
                    panic!("{file}: expected a forward, got {action:?}");
                };
                let mut expected = fixture.clone();
                rewrite(&mut expected);
                assert_eq!(
                    serde_json::from_str::<Value>(&out).unwrap(),
                    expected,
                    "{file}"
                );
            }
        }
    }
}

/// The `caller` an SDK asserts on a call it makes is removed before the
/// engine sees it (the fixture carries one on the engine → worker side; the
/// same fields from a worker are the ones observers would trust).
#[tokio::test]
async fn caller_fields_from_a_worker_are_removed() {
    let (_, fixture) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "invokefunction-caller.json")
        .expect("invokefunction-caller.json");
    let action = interceptor().handle_downstream(&fixture.to_string()).await;
    let DownstreamAction::Forward(out) = action else {
        panic!("expected a forward, got {action:?}");
    };
    let out: Value = serde_json::from_str(&out).unwrap();
    assert!(out.get("caller").is_none());
    assert!(out.get("observe_depth").is_none());
    assert_eq!(out["data"], fixture["data"]);
}
