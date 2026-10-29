//! `secrets::on-changed` over a real engine. Run the ignored case with
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
use iii_sdk::{CallRequest, ObserverHandle, ObserverState, RegisterFunction};
use judge_provider::secrets::{observe_secret_changes, Fetch, SecretCache, SecretValue};
use serde_json::{json, Value};
use support::{connect, Engine};
use tokio::time::timeout;

const ON_CHANGE: &str = "probe::on-secret-change";
const ON_CHANGED_FN: &str = "secrets::on-changed";

async fn wait_for_state(handle: &ObserverHandle, state: ObserverState) {
    timeout(Duration::from_secs(15), async {
        while handle.state() != state {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the secrets::on-changed observer never became {state:?}"));
}

/// The observer is created once at boot, before any secrets worker runs. It stays
/// `pending`, becomes `active` when `secrets::on-changed` registers, and goes back to
/// `pending` while that provider restarts, so every change still evicts the cached key.
#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine"]
async fn secrets_changed_reaches_an_observer_made_before_its_provider() {
    let engine = Engine::start("judge_provider_secrets_changed").await;
    let probe = connect(&engine.url, "probe").await;
    let fetches = Arc::new(AtomicU32::new(0));
    let counter = fetches.clone();
    let fetch: Fetch = Arc::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(SecretValue::new("sk-probe".into())) })
    });
    let cache = Arc::new(SecretCache::new(fetch, "probe"));
    let observer = observe_secret_changes(&probe, cache.clone(), ON_CHANGE);
    let key = Some("secret://PROBE_KEY");
    cache.configured_key(key).await.unwrap();

    for (start, fetched) in [("first start", 2), ("restart", 3)] {
        wait_for_state(&observer, ObserverState::Pending).await;
        // The secrets worker's side: a private source whose handler returns the event.
        let secrets = connect(&engine.url, "secrets").await;
        secrets.register_function(
            ON_CHANGED_FN,
            RegisterFunction::new(|event: Value| Ok::<_, Error>(event)).private(),
        );
        timeout(Duration::from_secs(15), observer.ready())
            .await
            .unwrap_or_else(|_| panic!("{start}: the pending observer never became active"))
            .unwrap_or_else(|error| panic!("{start}: the observer failed: {error}"));
        secrets
            .call(CallRequest::new(
                ON_CHANGED_FN,
                json!({"name": "PROBE_KEY", "ref": "secret://PROBE_KEY", "action": "rotated"}),
            ))
            .await
            .unwrap();
        // Delivery does not wait for the target: poll until the eviction lands, which
        // costs exactly one fetch.
        timeout(Duration::from_secs(15), async {
            loop {
                cache.configured_key(key).await.unwrap();
                if fetches.load(Ordering::SeqCst) == fetched {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{start}: the change never evicted the cached key"));
        secrets.shutdown_async().await;
    }
}
