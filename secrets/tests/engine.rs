//! `secrets::*` over a real, isolated engine: the console stores a key for
//! `llm-router`, lists it (metadata only), the router resolves it and is
//! told about rotations through `secrets::changed`, and any other worker is
//! refused; the same for a key kept in the project's `.env` and referenced
//! as `env://NAME`, including an edit made to the file by hand. Run with
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
            dotenv: None,
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
        env_watch: Default::default(),
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

#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine"]
async fn env_variables_are_shared_written_and_followed() {
    let engine = Engine::start("secrets-env").await;
    let root = tempfile::tempdir().unwrap();
    let dotenv = root.path().join(".env");
    std::fs::write(&dotenv, "# Provider keys\n# OPENAI_API_KEY=\n").unwrap();
    let worker = Arc::new(connect(&engine.url, "secrets").await);
    let store = Arc::new(
        Store::new(
            StorePaths {
                data_dir: root.path().join("data/secrets"),
                key_file: None,
                key_dir: None,
                project_dir: None,
                dotenv: Some(dotenv.clone()),
            },
            EnvKey::default(),
        )
        .without_process_env(),
    );
    store.open().await.unwrap();
    let ctx = Arc::new(Ctx {
        iii: worker.clone(),
        store,
        callers: CallerDirectory::new(worker.clone()),
        subscribers: Subscribers::default(),
        detector: Detector {
            process_env: false,
            dotenv: Some(dotenv.clone()),
            shell: None,
            shell_timeout: Duration::from_secs(1),
        },
        env_watch: Default::default(),
    });
    secrets::register(&ctx);
    assert!(secrets::envwatch::follow(&ctx).await, "the OS watches .env");

    let console = connect(&engine.url, "ade").await;
    let router = connect(&engine.url, "llm-router").await;
    let intruder = connect(&engine.url, "harness").await;
    for id in ids::ALL_FUNCTIONS {
        wait_for(&console, id).await;
    }
    let (tx, mut changes) = mpsc::unbounded_channel::<Value>();
    router.register_function(
        "router-test::on-env-changed",
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
            function_id: "router-test::on-env-changed".into(),
            config: json!({"names":["env://OPENAI_API_KEY"]}),
            metadata: None,
            namespace: None,
            trigger_namespace: None,
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while ctx.subscribers.is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("subscriber bound");

    // The console pastes a key and keeps it in .env.
    let meta = invoke(
        &console,
        ids::SET,
        json!({"name":"OPENAI_API_KEY","value":VALUE,"store":"env","consumers":["llm-router"]}),
    )
    .await
    .unwrap();
    assert_eq!(meta["ref"], "env://OPENAI_API_KEY");
    assert_eq!(meta["store"], "env");
    assert_eq!(meta["location"], dotenv.display().to_string());
    assert_eq!(
        std::fs::read_to_string(&dotenv).unwrap(),
        format!("# Provider keys\nOPENAI_API_KEY={VALUE}\n")
    );
    let created = tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(created["ref"], "env://OPENAI_API_KEY");
    assert_eq!(created["action"], "created");

    let resolved = invoke(&router, ids::RESOLVE, json!({"ref":"env://OPENAI_API_KEY"}))
        .await
        .unwrap();
    assert_eq!(resolved, json!({"name":"OPENAI_API_KEY","value":VALUE}));
    let refused = invoke(
        &intruder,
        ids::RESOLVE,
        json!({"ref":"env://OPENAI_API_KEY"}),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(refused), "SECRET_FORBIDDEN");
    // The vault store is a different place: nothing is stored there.
    let vault = invoke(
        &router,
        ids::RESOLVE,
        json!({"ref":"secret://OPENAI_API_KEY"}),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(vault), "SECRET_NOT_FOUND");

    // An edit made by hand is noticed and served.
    std::fs::write(&dotenv, format!("OPENAI_API_KEY={ROTATED}\n")).unwrap();
    let rotated = tokio::time::timeout(Duration::from_secs(10), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rotated["action"], "rotated");
    assert!(!rotated.to_string().contains(ROTATED));
    let resolved = invoke(&router, ids::RESOLVE, json!({"ref":"env://OPENAI_API_KEY"}))
        .await
        .unwrap();
    assert_eq!(resolved["value"], ROTATED);
    let meta = invoke(
        &console,
        ids::GET,
        json!({"name":"OPENAI_API_KEY","store":"env"}),
    )
    .await
    .unwrap();
    assert_eq!(meta["last_resolved_by"], "llm-router");
    let listed = invoke(&console, ids::LIST, json!({})).await.unwrap();
    assert!(!listed.to_string().contains(ROTATED), "{listed}");

    // Without a .env: the variable is gone (announced), and the router is
    // told it is not set rather than handed anything.
    std::fs::remove_file(&dotenv).unwrap();
    let removed = tokio::time::timeout(Duration::from_secs(10), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(removed["action"], "deleted");
    let missing = invoke(&router, ids::RESOLVE, json!({"ref":"env://OPENAI_API_KEY"}))
        .await
        .unwrap_err();
    assert_eq!(remote_code(missing), "SECRET_NOT_FOUND");
    // Saving through the console again creates it, owner-only, with that line.
    invoke(
        &console,
        ids::SET,
        json!({"name":"OPENAI_API_KEY","value":VALUE,"store":"env"}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&dotenv).unwrap(),
        format!("OPENAI_API_KEY={VALUE}\n")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dotenv).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let recreated = tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recreated["action"], "created");
    let resolved = invoke(&router, ids::RESOLVE, json!({"ref":"env://OPENAI_API_KEY"}))
        .await
        .unwrap();
    assert_eq!(resolved["value"], VALUE);

    // The configuration points this namespace at another env file: the
    // store reads it, what differs is announced, and only it is followed.
    let staging = root.path().join(".env.staging");
    std::fs::write(&staging, format!("OPENAI_API_KEY={ROTATED}\n")).unwrap();
    assert!(
        ctx.store
            .reconfigure(StorePaths {
                data_dir: root.path().join("data/secrets"),
                key_file: None,
                key_dir: None,
                project_dir: None,
                dotenv: Some(staging.clone()),
            })
            .await
    );
    assert!(secrets::envwatch::follow(&ctx).await);
    for event in ctx.store.env_changes().await {
        ctx.changed(&event);
    }
    let switched = tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(switched["action"], "rotated");
    let resolved = invoke(&router, ids::RESOLVE, json!({"ref":"env://OPENAI_API_KEY"}))
        .await
        .unwrap();
    assert_eq!(resolved["value"], ROTATED);
    // The old file is no longer followed; the new one is.
    std::fs::write(&dotenv, "OPENAI_API_KEY=sk-ignored-old-file-value-9\n").unwrap();
    std::fs::write(&staging, format!("OPENAI_API_KEY={VALUE}\n")).unwrap();
    let edited = tokio::time::timeout(Duration::from_secs(10), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edited["action"], "rotated");
    let resolved = invoke(&router, ids::RESOLVE, json!({"ref":"env://OPENAI_API_KEY"}))
        .await
        .unwrap();
    assert_eq!(resolved["value"], VALUE);
    assert!(
        tokio::time::timeout(Duration::from_millis(800), changes.recv())
            .await
            .is_err(),
        "an edit to the old file is not announced"
    );

    ctx.env_watch.lock().unwrap().take();
    for client in [console, router, intruder] {
        client.shutdown_async().await;
    }
    worker.shutdown_async().await;
}
