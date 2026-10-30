//! The observe protocol against the interceptor, one vendored wire fixture at
//! a time (`tests/fixtures/protocol/observe/`, see its README).
//!
//! The engine behind the proxy gates none of observe for a connection without
//! a session, so the proxy gates every frame itself: an `observe` needs access
//! to its source and target; the frames a host exchanges about the observers
//! of its functions pass for the functions the connection registered; a call
//! through an observer passes for an observer the engine announced to the
//! connection as the host of its function; and the `caller` a worker asserts
//! passes only when it names a call the engine routed to the connection.
//!
//! Each fixture runs on a fresh connection that first does what a real one
//! would have done before sending it (`Case::setup`): register the functions
//! the frame names, receive the call it reports on, or host the observer it
//! calls through. Every fixture file needs an entry in `case`, so one added
//! upstream fails here until someone decides what the proxy does with it.

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
    /// Worker → engine, answered by the proxy with a frame equal to this
    /// fixture.
    RepliedDownstream(&'static str),
    /// Worker → engine, forwarded as `rewrite` turns the fixture.
    ForwardedDownstream(fn(&mut Value)),
    /// Engine → worker, forwarded as `rewrite` turns the fixture.
    ForwardedUpstream(fn(&mut Value)),
}

/// What the connection does before the fixture arrives.
enum Setup {
    /// Register these functions (bare ids, in the connection's namespace).
    Register(&'static [&'static str]),
    /// Receive this engine frame.
    Upstream(Value),
    /// Receive this fixture from the engine.
    UpstreamFixture(&'static str),
}

struct Case {
    setup: Vec<Setup>,
    expect: Expect,
}

fn unchanged(_: &mut Value) {}

/// The register call as the engine gets it: the features the proxy enforces,
/// and the name behind the proxy's `rbac-proxy:` prefix.
fn registered_through_proxy(v: &mut Value) {
    v["data"]["features"] = json!(["observe.v1", "observe.v2", "local-dispatch.v1"]);
    let name = v["data"]["name"].as_str().unwrap();
    v["data"]["name"] = json!(format!("rbac-proxy:{name}"));
}

/// A call routed to this connection: what a `caller` is checked against.
fn routed(invocation_id: &str, function_id: &str, caller: Value) -> Setup {
    let mut frame = json!({
        "type": "invokefunction",
        "invocation_id": invocation_id,
        "function_id": function_id,
        "data": {}
    });
    if !caller.is_null() {
        frame["caller"] = caller;
    }
    Setup::Upstream(frame)
}

/// The engine announcing `observer_id` to this connection as an observer of
/// `function_id`, which the connection registered.
fn hosting(function_id: &'static str, observer_id: &str, target: &str) -> Vec<Setup> {
    vec![
        Setup::Register(Box::leak(Box::new([function_id]))),
        Setup::Upstream(json!({
            "type": "functionregistered",
            "id": function_id,
            "namespace": "default",
            "revision": 1,
            "local_dispatch": true,
            "observers": [{
                "id": observer_id,
                "args": {},
                "filters": {},
                "target": target,
                "capture": ["input", "output", "caller"],
                "on": "success"
            }]
        })),
    ]
}

fn forward_down(setup: Vec<Setup>) -> Case {
    Case {
        setup,
        expect: Expect::ForwardedDownstream(unchanged),
    }
}

fn forward_up() -> Case {
    Case {
        setup: vec![],
        expect: Expect::ForwardedUpstream(unchanged),
    }
}

fn case(file: &str) -> Case {
    use Expect::*;
    match file {
        // ----- worker → engine ------------------------------------------
        "observe.json" | "observe-minimal.json" | "observe-on-error.json" | "unobserve.json" => {
            forward_down(vec![])
        }
        "observerack.json" | "observerack-error.json" => {
            forward_down(vec![Setup::Register(&["cron::on-tick"])])
        }
        "localinvocationstats.json" => forward_down(vec![Setup::Register(&[
            "database::on-row-changed",
            "database::query",
        ])]),
        // Local calls made inside `checkout::confirm`, which the engine
        // routed here: their `caller` names that call.
        "invocationsobserved.json" => forward_down(vec![
            Setup::Register(&[
                "checkout::confirm",
                "payments::charge",
                "orders::archive",
                "orders::create",
            ]),
            routed(
                "5e7a9c1b-2d4f-4b6a-8c0e-1f3a5c7e9b20",
                "checkout::confirm",
                Value::Null,
            ),
        ]),
        // Made inside the routed `database::execute` call of
        // invokefunction-caller.json: the caller is that call, with its
        // caller as `parent`.
        "invocationsobserved-event-source.json" => forward_down(vec![
            Setup::Register(&["database::execute", "database::on-row-changed"]),
            Setup::UpstreamFixture("invokefunction-caller.json"),
        ]),
        // The host's answers to routed calls: `observed` passes through, the
        // engine checks every id.
        "invocationresult-observed.json" | "invocationresult-observed-error.json" => {
            forward_down(vec![])
        }
        "registerfunction.json"
        | "registerfunction-private.json"
        | "registerfunction-not-observable.json"
        | "registerfunction-v1-owner.json"
        | "registerfunction-v1-not-observable.json" => forward_down(vec![]),
        "invokefunction-legacy.json" => forward_down(vec![]),
        "invokefunction-workers-register.json" => Case {
            setup: vec![],
            expect: ForwardedDownstream(registered_through_proxy),
        },
        // A call through an observer this connection hosts goes out as sent.
        // `caller` names `http::on-request`'s call, which the engine routed
        // here.
        "invokefunction-observer-call.json" => {
            let mut setup = hosting(
                "http::on-request",
                "b6c4e2a0-9f1d-4b3a-8e7c-6d5f4e3a2b1c",
                "users::get",
            );
            setup.push(routed(
                "9b1d3f5a-7c9e-4b1d-8f3a-5c7e9b1d3f5a",
                "http::on-request",
                Value::Null,
            ));
            forward_down(setup)
        }
        "invokefunction-observer-call-void.json" => forward_down(hosting(
            "inventory::on-stock-changed",
            "868c8f79-e34f-4390-8ca6-7904b54ba84a",
            "storefront::sync-stock",
        )),
        // An observer of a function this connection does not host: refused
        // as the engine refuses it.
        "invokefunction-observer-call-namespace.json" => Case {
            setup: vec![],
            expect: RepliedDownstream("invocationresult-not-function-owner.json"),
        },

        // ----- engine → worker ------------------------------------------
        "workerregistered.json" | "workerregistered-v1.json" | "workerregistered-legacy.json" => {
            forward_up()
        }
        "registrationrejected-unsupported.json"
        | "registrationrejected-invalid-args-format.json" => forward_up(),
        "functionregistered.json"
        | "functionregistered-empty.json"
        | "observeradded.json"
        | "observerremoved.json"
        | "observeresult.json"
        | "observeresult-error.json"
        | "observerstatechanged.json"
        | "observerstatechanged-reason.json" => forward_up(),
        "invokefunction-activation.json"
        | "invokefunction-activation-engine-source.json"
        | "invokefunction-observer-call-forwarded.json"
        | "invokefunction-caller.json" => forward_up(),
        "invocationresult.json" | "invocationresult-not-function-owner.json" => forward_up(),

        other => panic!(
            "no expectation for fixture {other}: decide what rbac-proxy does with it \
             and add it to `case`"
        ),
    }
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/protocol/observe")
}

fn fixtures() -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = std::fs::read_dir(fixture_dir())
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

fn fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(fixture_dir().join(name)).expect("fixture reads");
    serde_json::from_str(&text).expect("fixture is JSON")
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

async fn run_setup(i: &Interceptor, file: &str, setup: Vec<Setup>) {
    for step in setup {
        match step {
            Setup::Register(ids) => {
                for id in ids {
                    let frame = json!({ "type": "registerfunction", "id": id });
                    let action = i.handle_downstream(&frame.to_string()).await;
                    assert!(
                        matches!(action, DownstreamAction::Forward(_)),
                        "{file}: registering {id} in setup: {action:?}"
                    );
                }
            }
            Setup::Upstream(frame) => {
                let action = i.handle_upstream(&frame.to_string()).await;
                assert!(
                    matches!(action, UpstreamAction::Forward(_)),
                    "{file}: setup frame {frame}: {action:?}"
                );
            }
            Setup::UpstreamFixture(name) => {
                let action = i.handle_upstream(&fixture(name).to_string()).await;
                assert!(
                    matches!(action, UpstreamAction::Forward(_)),
                    "{file}: setup fixture {name}: {action:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn every_observe_fixture_is_gated() {
    let fixtures = fixtures();
    assert!(fixtures.len() >= 40, "fixtures missing: {}", fixtures.len());

    for (file, fixture_value) in fixtures {
        let text = fixture_value.to_string();
        // A fresh connection per fixture: the frames record state.
        let i = interceptor();
        let Case { setup, expect } = case(&file);
        run_setup(&i, &file, setup).await;
        match expect {
            Expect::RepliedDownstream(reply_fixture) => {
                let action = i.handle_downstream(&text).await;
                let DownstreamAction::ReplyToClient(reply) = action else {
                    panic!("{file}: expected a reply, got {action:?}");
                };
                assert_eq!(
                    serde_json::from_str::<Value>(&reply).unwrap(),
                    fixture(reply_fixture),
                    "{file}"
                );
            }
            Expect::ForwardedDownstream(rewrite) => {
                let action = i.handle_downstream(&text).await;
                let DownstreamAction::Forward(out) = action else {
                    panic!("{file}: expected a forward, got {action:?}");
                };
                let mut expected = fixture_value.clone();
                rewrite(&mut expected);
                assert_eq!(
                    serde_json::from_str::<Value>(&out).unwrap(),
                    expected,
                    "{file}"
                );
            }
            Expect::ForwardedUpstream(rewrite) => {
                let action = i.handle_upstream(&text).await;
                let UpstreamAction::Forward(out) = action else {
                    panic!("{file}: expected a forward, got {action:?}");
                };
                let mut expected = fixture_value.clone();
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

/// Without the context that vouches for them, the host frames are dropped:
/// acks, reports and stats of functions the connection did not register,
/// and fire-and-forget calls through observers it does not host.
#[tokio::test]
async fn host_frames_without_their_context_are_dropped() {
    for name in [
        "observerack.json",
        "observerack-error.json",
        "invocationsobserved.json",
        "invocationsobserved-event-source.json",
        "localinvocationstats.json",
        "invokefunction-observer-call-void.json",
    ] {
        assert_eq!(
            interceptor()
                .handle_downstream(&fixture(name).to_string())
                .await,
            DownstreamAction::Drop,
            "{name}"
        );
    }
}

/// A report item for a call the engine did not route here keeps no
/// `caller`: a worker cannot place its calls in another worker's call chain.
#[tokio::test]
async fn an_unvouched_caller_is_removed_from_a_report() {
    let i = interceptor();
    run_setup(
        &i,
        "invocationsobserved.json",
        vec![Setup::Register(&[
            "payments::charge",
            "orders::archive",
            "orders::create",
        ])],
    )
    .await;
    let report = fixture("invocationsobserved.json");
    let DownstreamAction::Forward(out) = i.handle_downstream(&report.to_string()).await else {
        panic!("expected a forward");
    };
    let out: Value = serde_json::from_str(&out).unwrap();
    let items = out["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|item| item.get("caller").is_none()));
    assert_eq!(items[2]["metadata"], report["items"][2]["metadata"]);
}

/// The `caller` an SDK asserts on a call it makes is removed before the
/// engine sees it when it names no call routed to the connection (the
/// fixture carries one on the engine → worker side; the same fields from a
/// worker are the ones observers would trust).
#[tokio::test]
async fn caller_fields_from_a_worker_are_removed() {
    let fixture = fixture("invokefunction-caller.json");
    let action = interceptor().handle_downstream(&fixture.to_string()).await;
    let DownstreamAction::Forward(out) = action else {
        panic!("expected a forward, got {action:?}");
    };
    let out: Value = serde_json::from_str(&out).unwrap();
    assert!(out.get("caller").is_none());
    assert_eq!(out["data"], fixture["data"]);
}
