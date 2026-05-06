//! Step defs for tests/features/notifications.feature.
//!
//! Each scenario registers a private "observer" function, subscribes it
//! to either `skills::on-change` or `prompts::on-change` via a trigger
//! instance, mutates the registry, and asserts the observer saw the
//! expected number of events within a bounded window.

use std::sync::{Arc, Mutex, OnceLock};

use cucumber::{given, then, when};
use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, TriggerRequest};
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const OBS_COUNT_SLOT: &str = "notifications_observed";

/// Shared counter that both the observer function (closure-captured) and
/// the assertion step read. We keep Arc<Observer>s in a process-wide Vec
/// so the observer function's closure can increment the counter while
/// the Then step pulls the snapshot out by index.
struct Observer {
    counter: Arc<Mutex<u64>>,
}

static OBSERVERS: OnceLock<Mutex<Vec<Arc<Observer>>>> = OnceLock::new();

fn observers() -> &'static Mutex<Vec<Arc<Observer>>> {
    OBSERVERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn new_observer(_world: &IiiSkillsWorld) -> Arc<Observer> {
    let o = Arc::new(Observer {
        counter: Arc::new(Mutex::new(0)),
    });
    observers().lock().unwrap().push(o.clone());
    o
}

async fn register_observer(
    world: &mut IiiSkillsWorld,
    trigger_type: &str,
    label: &str,
) -> Arc<Observer> {
    let obs = new_observer(world);
    let Some(iii) = world.iii.clone() else {
        return obs;
    };
    let fn_id = format!("bdd::obs-{label}-{}", world.unique_id);
    let counter = obs.counter.clone();
    iii.register_function(
        RegisterFunction::new_async(fn_id.clone(), move |_input: Value| {
            let counter = counter.clone();
            async move {
                *counter.lock().unwrap() += 1;
                Ok::<_, IIIError>(json!({}))
            }
        })
        .description("bdd: notifications observer"),
    );
    // Give the engine a beat to publish the function registration so
    // the trigger-type subscription has a valid target.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: fn_id.clone(),
        config: json!({}),
        metadata: None,
    }) {
        Ok(_trigger) => {
            // Leak the Trigger guard intentionally: we want the
            // subscription to live until the binary exits. The
            // `observer` handle on the world holds the counter.
        }
        Err(e) => {
            eprintln!("failed to register {trigger_type} observer: {e}");
        }
    }
    // Let the trigger-type handler stash the subscription before we
    // start mutating state.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    obs
}

async fn wait_for_count(obs: &Observer, want: u64, timeout_ms: u64) -> u64 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        let now = *obs.counter.lock().unwrap();
        if now >= want {
            return now;
        }
        if std::time::Instant::now() >= deadline {
            return now;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

// ── setup ───────────────────────────────────────────────────────────────

#[given("a subscriber to skills::on-change is registered")]
async fn subscribe_skills(world: &mut IiiSkillsWorld) {
    let obs = register_observer(world, "skills::on-change", "skills").await;
    world
        .stash
        .insert(OBS_COUNT_SLOT.into(), Value::Number(0.into()));
    let idx = observers().lock().unwrap().len() - 1;
    world
        .stash
        .insert("notifications_obs_idx".into(), json!(idx));
    drop(obs);
}

#[given("a subscriber to prompts::on-change is registered")]
async fn subscribe_prompts(world: &mut IiiSkillsWorld) {
    let obs = register_observer(world, "prompts::on-change", "prompts").await;
    world
        .stash
        .insert(OBS_COUNT_SLOT.into(), Value::Number(0.into()));
    let idx = observers().lock().unwrap().len() - 1;
    world
        .stash
        .insert("notifications_obs_idx".into(), json!(idx));
    drop(obs);
}

// ── mutation triggers ───────────────────────────────────────────────────

async fn trigger(world: &IiiSkillsWorld, function_id: &str, payload: Value) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let _ = iii
        .trigger(TriggerRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
}

#[when("I register a scoped skill with a short body")]
async fn mut_register_skill(world: &mut IiiSkillsWorld) {
    let id = world.scoped_id("nskill");
    world
        .stash
        .insert("notif_mut_id".into(), Value::String(id.clone()));
    trigger(
        world,
        "skills::register",
        json!({ "id": id, "skill": "# n\nok\n" }),
    )
    .await;
}

#[when("I unregister that skill")]
async fn mut_unregister_skill(world: &mut IiiSkillsWorld) {
    let id = world
        .stash
        .get("notif_mut_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    trigger(world, "skills::unregister", json!({ "id": id })).await;
}

#[when(regex = r#"^I register (\d+) scoped skills in quick succession$"#)]
async fn mut_register_many(world: &mut IiiSkillsWorld, n: usize) {
    for i in 0..n {
        let id = world.scoped_id(&format!("burst-{i}"));
        trigger(
            world,
            "skills::register",
            json!({ "id": id, "skill": format!("# burst-{i}\n") }),
        )
        .await;
    }
}

#[when("I register a scoped prompt")]
async fn mut_register_prompt(world: &mut IiiSkillsWorld) {
    let name = world.scoped_id("nprompt");
    world
        .stash
        .insert("notif_mut_name".into(), Value::String(name.clone()));
    trigger(
        world,
        "prompts::register",
        json!({
            "name": name,
            "description": "notifications probe",
            "function_id": "bdd::dummy-handler",
        }),
    )
    .await;
}

#[when("I unregister that prompt")]
async fn mut_unregister_prompt(world: &mut IiiSkillsWorld) {
    let name = world
        .stash
        .get("notif_mut_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    trigger(world, "prompts::unregister", json!({ "name": name })).await;
}

// ── assertions ──────────────────────────────────────────────────────────

#[then(regex = r#"^the subscriber observed at least (\d+) event within (\d+) ms$"#)]
async fn observed_at_least_single(world: &mut IiiSkillsWorld, want: u64, timeout_ms: u64) {
    observed_at_least(world, want, timeout_ms).await;
}

#[then(regex = r#"^the subscriber observed at least (\d+) events within (\d+) ms$"#)]
async fn observed_at_least(world: &mut IiiSkillsWorld, want: u64, timeout_ms: u64) {
    if world.iii.is_none() {
        return;
    }
    let idx = world
        .stash
        .get("notifications_obs_idx")
        .and_then(|v| v.as_u64())
        .expect("no observer index recorded") as usize;
    let obs = observers().lock().unwrap()[idx].clone();
    let got = wait_for_count(&obs, want, timeout_ms).await;
    assert!(
        got >= want,
        "observed {got} events, wanted >= {want} within {timeout_ms}ms"
    );
}
