//! An unqualified `harness::observe` whose source resolves outside this
//! harness's namespace is authorized in the namespace it resolved to, against
//! a real engine (`III_ENGINE_BIN`, or `iii` on PATH; skipped without one).
//!
//! Its own binary: the harness reads its namespace from `III_NAMESPACE`, which
//! only a process with no other test can set.

#[path = "support/engine.rs"]
mod engine;

use engine::{isolated_engine, worker_options};

use std::sync::Arc;
use std::time::Duration;

use harness::config::WorkerConfig;
use harness::deps::Deps;
use harness::events::TurnEvents;
use harness::functions::observe::{OBSERVE_ID, invoke};
use harness::policy::CompiledPolicy;
use harness::types::turn::FunctionPolicy;
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use serde_json::{Value, json};
use tokio::sync::RwLock;

#[tokio::test]
async fn an_unqualified_source_is_authorized_in_the_namespace_it_resolved_to() {
    // SAFETY: the only test in this binary, set before any worker connects.
    unsafe { std::env::set_var("III_NAMESPACE", "project") };
    let Some(engine) = isolated_engine().await else {
        return;
    };
    // The source is hosted in `default` only, so the probe resolves it there.
    let source = register_worker(
        &engine.url,
        InitOptions {
            namespace: Some("default".into()),
            ..worker_options("source")
        },
    );
    source.register_function(
        "state::on-changed",
        RegisterFunction::new(|event: Value| Ok::<Value, iii_sdk::Error>(event)).private(),
    );
    source
        .wait_until_registered(Duration::from_secs(5))
        .await
        .expect("source registered");
    // The owner's turn record is absent: a root session.
    let state = register_worker(&engine.url, worker_options("state"));
    state.register_function(
        "state::get",
        RegisterFunction::new(|_: Value| Ok::<Value, iii_sdk::Error>(Value::Null)),
    );
    state
        .wait_until_registered(Duration::from_secs(5))
        .await
        .expect("state registered");
    let iii = Arc::new(register_worker(&engine.url, worker_options("harness")));
    let cfg = WorkerConfig {
        session_timeout_ms: 2_000,
        dispatch_timeout_ms: 2_000,
        ..WorkerConfig::default()
    };
    let deps = Deps::new(
        iii.clone(),
        Arc::new(RwLock::new(Arc::new(cfg))),
        harness::discovery::new_cell(),
        harness::skills::new_cell(),
        TurnEvents::register(&iii),
        harness::hooks::HookRegistry::register(&iii),
    );
    let client = deps.engine().await;

    for deny in ["default::state::on-changed", "default/state::on-changed"] {
        let policy = CompiledPolicy::from(Some(&FunctionPolicy {
            allow: vec!["*".into()],
            deny: vec![deny.into()],
            expose: Default::default(),
        }));
        let result = invoke(
            &deps,
            &client,
            &policy,
            OBSERVE_ID,
            None,
            &json!({ "function_id": "state::on-changed" }),
            "owner",
            false,
            None,
        )
        .await;
        assert!(result.is_error, "{deny}: {:?}", result.details);
        let error = result.details["error"].as_str().unwrap_or_default();
        assert!(error.contains("dispatch policy"), "{deny}: {error}");
    }
}
