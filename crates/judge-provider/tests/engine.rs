//! `secrets::changed` over a real engine. Run the ignored case with
//! .github/scripts/judge-e2e.sh.
#[allow(dead_code)]
#[path = "../../../judge-typesafe/tests/support/mod.rs"]
mod support;

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use judge_provider::secrets::{register_secret_trigger, Fetch, SecretCache, SecretValue};
use serde_json::json;
use support::{connect, invoke, Engine};
use tokio::{sync::mpsc, time::timeout};

const ON_CHANGE: &str = "probe::on-secret-change";

/// The secrets worker's side of the type: hand each binding the engine delivers to the test.
struct Bindings(mpsc::UnboundedSender<TriggerConfig>);

#[async_trait::async_trait]
impl TriggerHandler for Bindings {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let _ = self.0.send(config);
        Ok(())
    }
    async fn unregister_trigger(&self, _: TriggerConfig) -> Result<(), Error> {
        Ok(())
    }
}

async fn wait_for_status(iii: &IIIClient, status: &str) {
    timeout(Duration::from_secs(15), async {
        loop {
            let listed = invoke(
                iii,
                "engine::registered-triggers::list",
                json!({"include_pending": true, "function_id": ON_CHANGE}),
            )
            .await
            .unwrap();
            if listed["registered_triggers"][0]["status"] == status {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the secrets::changed binding never became {status}"));
}

/// The binding is sent once at boot, before any secrets worker runs. The engine parks
/// it, hands it to the provider when `secrets::changed` registers, and parks it again
/// while that provider restarts, so every change still evicts the cached key.
#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine"]
async fn secrets_changed_reaches_a_binding_made_before_its_provider() {
    let engine = Engine::start("judge_provider_secrets_changed").await;
    let probe = connect(&engine.url, "probe").await;
    let fetches = Arc::new(AtomicU32::new(0));
    let counter = fetches.clone();
    let fetch: Fetch = Arc::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(SecretValue::new("sk-probe".into())) })
    });
    let cache = Arc::new(SecretCache::new(fetch, "probe"));
    register_secret_trigger(&probe, cache.clone(), ON_CHANGE);
    let key = Some("secret://PROBE_KEY");
    cache.configured_key(key).await.unwrap();

    for (start, fetched) in [("first start", 2), ("restart", 3)] {
        wait_for_status(&probe, "pending").await;
        let secrets = connect(&engine.url, "secrets").await;
        let (tx, mut bindings) = mpsc::unbounded_channel();
        secrets.register_trigger_type(RegisterTriggerType::new(
            "secrets::changed",
            "test provider",
            Bindings(tx),
        ));
        let binding = timeout(Duration::from_secs(15), bindings.recv())
            .await
            .unwrap_or_else(|_| panic!("{start}: the parked binding never reached the provider"))
            .unwrap();
        assert_eq!(binding.function_id, ON_CHANGE);
        let answer = invoke(
            &secrets,
            &binding.function_id,
            json!({"name": "PROBE_KEY", "ref": "secret://PROBE_KEY", "action": "rotated"}),
        )
        .await
        .unwrap();
        assert_eq!(answer, json!({"ok": true}), "{start}");
        cache.configured_key(key).await.unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), fetched, "{start}: evicted");
        secrets.shutdown_async().await;
    }
}
