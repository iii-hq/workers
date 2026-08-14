//! `retry_hook_bindings` registration semantics on a live engine: each hook
//! binds at most once per startup, attempts continue until success, and
//! harness readiness gates only the completion condition — a gate that starts
//! after the harness still binds (the readiness signal says the harness is
//! active, not that THIS worker registered its hook).
//!
//! Self-skips when no `iii` engine binary is on PATH or `III_ENGINE_BIN`.

use std::time::{Duration, Instant};

use approval_gate::configuration::{
    bind_filesystem_access_watch_hook, bind_hook, retry_hook_bindings,
};
use approval_gate::testkit::{engine_bin, spawn_engine};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterTriggerType};
use serde_json::json;

/// No-op handler standing in for the harness's hook trigger types — a test
/// registers these to simulate "the harness is active".
struct NullHandler;

#[async_trait::async_trait]
impl TriggerHandler for NullHandler {
    async fn register_trigger(&self, _config: TriggerConfig) -> Result<(), Error> {
        Ok(())
    }
    async fn unregister_trigger(&self, _config: TriggerConfig) -> Result<(), Error> {
        Ok(())
    }
}

fn register_harness_hook_types(iii: &IIIClient) {
    for hook_type in ["harness::hook::pre-trigger", "harness::hook::post-trigger"] {
        let _ = iii.register_trigger_type(RegisterTriggerType::new(
            hook_type,
            "test double for the harness hook trigger types",
            NullHandler,
        ));
    }
}

/// Engine-side instance counts for the two hook types (0 on any error).
async fn hook_instance_counts(iii: &IIIClient) -> (u64, u64) {
    let mut counts = (0, 0);
    for (index, hook_type) in ["harness::hook::pre-trigger", "harness::hook::post-trigger"]
        .iter()
        .enumerate()
    {
        if let Ok(response) = iii
            .trigger(TriggerRequest {
                function_id: "engine::triggers::info".to_string(),
                payload: json!({ "id": hook_type }),
                action: None,
                timeout_ms: None,
            })
            .await
        {
            if let Some(count) = response.get("instance_count").and_then(serde_json::Value::as_u64)
            {
                if index == 0 {
                    counts.0 = count;
                } else {
                    counts.1 = count;
                }
            }
        }
    }
    counts
}

/// Poll until both hook types report the expected instance count.
async fn wait_for_counts(iii: &IIIClient, expected: (u64, u64)) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if hook_instance_counts(iii).await == expected {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The gate starts AFTER a previous gate left instances behind: the hook
/// types exist and `engine::triggers::info` already reports count > 0 on the
/// first iteration. Readiness must not suppress this worker's own bind — the
/// old loop broke immediately on the foreign count, leaving the gate
/// detached. And each hook registers exactly once (no re-binding storm once
/// bound).
#[tokio::test(flavor = "multi_thread")]
async fn leftover_instances_do_not_suppress_this_workers_own_registration() {
    if engine_bin().is_none() {
        eprintln!("skipping: no iii engine");
        return;
    }
    let Some(engine) = spawn_engine().await else {
        eprintln!("skipping: failed to spawn engine");
        return;
    };
    let iii = register_worker(&engine.url, InitOptions::default());
    register_harness_hook_types(&iii);

    // Simulate a previous gate's instances that the engine has not yet
    // garbage-collected: both types already report count 1.
    assert!(bind_hook(&iii), "seed pre-trigger instance");
    assert!(
        bind_filesystem_access_watch_hook(&iii),
        "seed post-trigger instance"
    );
    assert_eq!(hook_instance_counts(&iii).await, (1, 1), "seeded");

    retry_hook_bindings(iii.clone());

    // The loop must register its OWN instances on top of the leftovers
    // (count 2), not conclude "ready" from the foreign count and skip.
    assert!(
        wait_for_counts(&iii, (2, 2)).await,
        "this worker must bind its own hooks even though the harness reports ready"
    );
    // Once bound, the loop must not re-register: counts stay stable across
    // several retry intervals instead of stacking duplicates.
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    assert_eq!(
        hook_instance_counts(&iii).await,
        (2, 2),
        "bindings must not be re-registered after success"
    );
}

/// The gate starts BEFORE the harness: binds fail while the hook types are
/// absent, and are retried once the harness comes up. The gate must not give
/// up after the first failure.
#[tokio::test(flavor = "multi_thread")]
async fn failed_registration_is_retried_until_the_harness_is_ready() {
    if engine_bin().is_none() {
        eprintln!("skipping: no iii engine");
        return;
    }
    let Some(engine) = spawn_engine().await else {
        eprintln!("skipping: failed to spawn engine");
        return;
    };
    let iii = register_worker(&engine.url, InitOptions::default());

    // Hook types absent: every bind attempt fails.
    retry_hook_bindings(iii.clone());
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    assert_eq!(
        hook_instance_counts(&iii).await,
        (0, 0),
        "no binding may register while the harness hook types are absent"
    );

    // The harness comes up; the retry loop must pick the binds up.
    register_harness_hook_types(&iii);
    assert!(
        wait_for_counts(&iii, (1, 1)).await,
        "failed registrations must be retried once the harness is ready"
    );
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    assert_eq!(
        hook_instance_counts(&iii).await,
        (1, 1),
        "bindings must not be re-registered after success"
    );
}
