//! Summariser scripting, lease setup, and clock control for the
//! compaction features.

use cucumber::{given, then};
use std::sync::atomic::Ordering;

use context_manager::core::lease::{default_lease_key, LEASE_SCOPE};
use context_manager::ports::LeaseRecord;
use context_manager::types::AgentMessage;

use crate::common::fakes::SummarizerScript;
use crate::common::world::ContextWorld;

fn parsed_history(world: &ContextWorld) -> Vec<AgentMessage> {
    world
        .messages
        .iter()
        .map(|m| serde_json::from_value(m.clone()).expect("history messages parse"))
        .collect()
}

/// The default lease key the worker derives for the current history.
pub fn history_lease_key(world: &ContextWorld) -> String {
    default_lease_key(&parsed_history(world))
}

// ---------------------------------------------------------------------------
// Summariser scripting
// ---------------------------------------------------------------------------

#[given(regex = r#"^the summariser returns "([^"]*)"$"#)]
async fn summariser_returns(world: &mut ContextWorld, summary: String) {
    // Gherkin quoted strings carry `\n` literally; unescape so the
    // scripted summary matches what JSON-parsing assertions expect.
    let summary = summary.replace("\\n", "\n");
    world
        .summarizer
        .set_script(SummarizerScript::Return(summary));
}

#[given("the summariser is unavailable")]
async fn summariser_unavailable(world: &mut ContextWorld) {
    world.summarizer.set_script(SummarizerScript::Unavailable);
}

#[given(regex = r#"^the summariser fails with "([^"]+)"$"#)]
async fn summariser_fails(world: &mut ContextWorld, reason: String) {
    world.summarizer.set_script(SummarizerScript::Fail(reason));
}

#[given("the summariser returns an empty summary")]
async fn summariser_empty(world: &mut ContextWorld) {
    world.summarizer.set_script(SummarizerScript::Empty);
}

#[then(regex = r#"^the summariser was invoked (\d+) times?$"#)]
async fn summariser_invoked(world: &mut ContextWorld, times: usize) {
    if world.soft_skipped {
        return;
    }
    let calls = world.summarizer.calls();
    assert_eq!(
        calls.len(),
        times,
        "expected {times} summariser call(s), got {}",
        calls.len()
    );
}

#[then("the summariser was never invoked")]
async fn summariser_never(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    let calls = world.summarizer.calls();
    assert!(
        calls.is_empty(),
        "expected no summariser calls, got {}",
        calls.len()
    );
}

#[then(regex = r#"^the summariser system prompt contains "([^"]+)"$"#)]
async fn summariser_system_contains(world: &mut ContextWorld, needle: String) {
    let calls = world.summarizer.calls();
    let last = calls.last().expect("the summariser was never invoked");
    assert!(
        last.system_prompt.contains(&needle),
        "summariser system prompt does not contain `{needle}`:\n{}",
        last.system_prompt
    );
}

#[then(regex = r#"^the summariser system prompt does not contain "([^"]+)"$"#)]
async fn summariser_system_not_contains(world: &mut ContextWorld, needle: String) {
    let calls = world.summarizer.calls();
    let last = calls.last().expect("the summariser was never invoked");
    assert!(
        !last.system_prompt.contains(&needle),
        "summariser system prompt unexpectedly contains `{needle}`"
    );
}

#[then(regex = r#"^the summariser user prompt contains "([^"]+)"$"#)]
async fn summariser_user_contains(world: &mut ContextWorld, needle: String) {
    let calls = world.summarizer.calls();
    let last = calls.last().expect("the summariser was never invoked");
    assert!(
        last.user_prompt.contains(&needle),
        "summariser user prompt does not contain `{needle}`:\n{}",
        last.user_prompt
    );
}

#[then(regex = r#"^the summariser user prompt does not contain "([^"]+)"$"#)]
async fn summariser_user_not_contains(world: &mut ContextWorld, needle: String) {
    let calls = world.summarizer.calls();
    let last = calls.last().expect("the summariser was never invoked");
    assert!(
        !last.user_prompt.contains(&needle),
        "summariser user prompt unexpectedly contains `{needle}`"
    );
}

#[then(regex = r#"^the summariser ran on model "([^"]+)"$"#)]
async fn summariser_model(world: &mut ContextWorld, model: String) {
    let calls = world.summarizer.calls();
    let last = calls.last().expect("the summariser was never invoked");
    assert_eq!(last.model, model);
}

// ---------------------------------------------------------------------------
// Leases and clock
// ---------------------------------------------------------------------------

#[given(regex = r#"^a foreign lease on key "([^"]+)" taken (\d+) ms ago$"#)]
async fn foreign_lease(world: &mut ContextWorld, key: String, age_ms: i64) {
    let now = context_manager::ports::Clock::now_ms(world.clock.as_ref());
    world.leases.put(
        LEASE_SCOPE,
        &key,
        LeaseRecord {
            nonce: "foreign-holder".into(),
            ts: now - age_ms,
        },
    );
}

#[given(regex = r#"^a foreign lease on the history's default key taken (\d+) ms ago$"#)]
async fn foreign_lease_default_key(world: &mut ContextWorld, age_ms: i64) {
    let key = history_lease_key(world);
    let now = context_manager::ports::Clock::now_ms(world.clock.as_ref());
    world.leases.put(
        LEASE_SCOPE,
        &key,
        LeaseRecord {
            nonce: "foreign-holder".into(),
            ts: now - age_ms,
        },
    );
}

#[given(regex = r#"^the clock advances by (\d+) ms$"#)]
async fn clock_advances(world: &mut ContextWorld, ms: i64) {
    world.clock.advance(ms);
}

#[given("the lease store is unavailable")]
async fn lease_store_unavailable(world: &mut ContextWorld) {
    world.leases.fail_all.store(true, Ordering::SeqCst);
}

#[then("no lease claim remains")]
async fn no_lease_remains(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    let keys = world.leases.keys();
    assert!(
        keys.is_empty(),
        "expected no lease claims, found keys {keys:?}"
    );
}

#[then(regex = r#"^the lease on key "([^"]+)" still belongs to "([^"]+)"$"#)]
async fn lease_still_belongs(world: &mut ContextWorld, key: String, nonce: String) {
    if world.soft_skipped {
        return;
    }
    let record = world
        .leases
        .peek(LEASE_SCOPE, &key)
        .unwrap_or_else(|| panic!("no lease claim on key {key}"));
    assert_eq!(
        record.nonce, nonce,
        "lease on {key} changed hands: now held by {}",
        record.nonce
    );
}

#[then("the lease on the history's default key still belongs to the foreign holder")]
async fn default_lease_still_foreign(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    let key = history_lease_key(world);
    let record = world
        .leases
        .peek(LEASE_SCOPE, &key)
        .unwrap_or_else(|| panic!("no lease claim on the default key"));
    assert_eq!(record.nonce, "foreign-holder");
}
