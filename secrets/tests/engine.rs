//! `secrets::*` over a real, isolated engine: the console stores a key for
//! `llm-router`, lists it (metadata only), the router resolves it and is
//! told about rotations through `secrets::changed`, and any other worker is
//! refused. Run with
//! `III_ENGINE_BIN=/abs/path/to/iii cargo test --test engine -- --ignored`.
mod support;

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::RegisterFunction;
use secrets::access::CallerDirectory;
use secrets::api::ids;
use secrets::crypto::MasterKey;
use secrets::detect::Detector;
use secrets::events::Subscribers;
use secrets::keys::EnvKey;
use secrets::{Ctx, Store, StorePaths};
use serde_json::{json, Value};
use support::{connect, invoke, wait_for, Engine};
use tokio::sync::mpsc;

const VALUE: &str = "sk-ant-api03-engine-test-value-0000";
const ROTATED: &str = "sk-ant-api03-engine-test-rotated-1111";

fn remote_code(error: Error) -> String {
    match error {
        Error::Remote { code, .. } => code,
        other => panic!("expected a remote error, got {other}"),
    }
}

#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine"]
async fn console_stores_router_resolves_others_are_refused() {
    let engine = Engine::start("secrets").await;
    let root = tempfile::tempdir().unwrap();
    let worker = Arc::new(connect(&engine.url, "secrets").await);
    let store = Arc::new(Store::new(
        StorePaths {
            data_dir: root.path().join("data/secrets"),
            key_file: None,
            key_dir: None,
            project_dir: None,
        },
        EnvKey::from_value(Some(MasterKey::generate().unwrap().to_base64().to_string())),
    ));
    let ctx = Arc::new(Ctx {
        iii: worker.clone(),
        store,
        callers: CallerDirectory::new(worker.clone()),
        subscribers: Subscribers::default(),
        detector: Detector {
            process_env: false,
            dotenv: None,
            shell: None,
            shell_timeout: Duration::from_secs(1),
        },
    });
    secrets::register(&ctx);

    let console = connect(&engine.url, "ade").await;
    let router = connect(&engine.url, "llm-router").await;
    let intruder = connect(&engine.url, "harness").await;
    for id in ids::ALL_FUNCTIONS {
        wait_for(&console, id).await;
    }

    // The router follows changes to its key, as llm-router does.
    let (tx, mut changes) = mpsc::unbounded_channel::<Value>();
    router.register_function(
        "router-test::on-secret-changed",
        RegisterFunction::new_async(move |event: Value| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(event);
                Ok::<Value, Error>(json!({}))
            }
        }),
    );
    router
        .register_trigger(RegisterTriggerInput {
            trigger_type: ids::CHANGED_TRIGGER.into(),
            function_id: "router-test::on-secret-changed".into(),
            config: json!({"names":["secret://ANTHROPIC_API_KEY"]}),
            metadata: None,
            namespace: None,
            trigger_namespace: None,
        })
        .unwrap();
    // Registration is asynchronous; wait until the worker holds the binding.
    tokio::time::timeout(Duration::from_secs(10), async {
        while ctx.subscribers.is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("subscriber bound");

    let meta = invoke(
        &console,
        ids::SET,
        json!({"name":"ANTHROPIC_API_KEY","value":VALUE,"consumers":["llm-router"]}),
    )
    .await
    .unwrap();
    assert_eq!(meta["ref"], "secret://ANTHROPIC_API_KEY");
    assert!(meta.get("value").is_none());
    let created = tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(created["action"], "created");
    assert!(!created.to_string().contains(VALUE));

    let listed = invoke(&console, ids::LIST, json!({})).await.unwrap();
    assert_eq!(listed["secrets"][0]["name"], "ANTHROPIC_API_KEY");
    assert!(!listed.to_string().contains(VALUE), "{listed}");

    let resolved = invoke(
        &router,
        ids::RESOLVE,
        json!({"ref":"secret://ANTHROPIC_API_KEY"}),
    )
    .await
    .unwrap();
    assert_eq!(resolved, json!({"name":"ANTHROPIC_API_KEY","value":VALUE}));

    // A forged caller id is overwritten by the engine.
    let forged = invoke(
        &intruder,
        ids::RESOLVE,
        json!({"ref":"secret://ANTHROPIC_API_KEY","_caller_worker_id":"llm-router"}),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(forged), "SECRET_FORBIDDEN");
    let console_read = invoke(&console, ids::RESOLVE, json!({"ref":"ANTHROPIC_API_KEY"}))
        .await
        .unwrap_err();
    assert_eq!(remote_code(console_read), "SECRET_FORBIDDEN");
    let missing = invoke(&router, ids::RESOLVE, json!({"ref":"secret://NOPE"}))
        .await
        .unwrap_err();
    assert_eq!(remote_code(missing), "SECRET_NOT_FOUND");

    // Rotation reaches the subscriber and the next resolve.
    invoke(
        &console,
        ids::SET,
        json!({"name":"ANTHROPIC_API_KEY","value":ROTATED}),
    )
    .await
    .unwrap();
    let rotated = tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rotated["action"], "rotated");
    assert_ne!(rotated["fingerprint"], created["fingerprint"]);
    let resolved = invoke(
        &router,
        ids::RESOLVE,
        json!({"ref":"secret://ANTHROPIC_API_KEY"}),
    )
    .await
    .unwrap();
    assert_eq!(resolved["value"], ROTATED);
    let meta = invoke(&console, ids::GET, json!({"name":"ANTHROPIC_API_KEY"}))
        .await
        .unwrap();
    assert_eq!(meta["last_resolved_by"], "llm-router");

    // Revoking access stops the router.
    invoke(
        &console,
        ids::ACCESS,
        json!({"name":"ANTHROPIC_API_KEY","consumers":[]}),
    )
    .await
    .unwrap();
    let revoked = tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(revoked["action"], "access_changed");
    let refused = invoke(
        &router,
        ids::RESOLVE,
        json!({"ref":"secret://ANTHROPIC_API_KEY"}),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(refused), "SECRET_FORBIDDEN");

    for client in [console, router, intruder] {
        client.shutdown_async().await;
    }
    worker.shutdown_async().await;
}
