//! Two instances of one worker against a live engine, each under a worker
//! name of its own in the same namespace. The first hosts the content
//! function and `<worker>::configuration-id`; the engine refuses the second
//! instance's registrations while the first is connected
//! (`FUNCTION_NAMESPACE_CONFLICT`), and the second registers both as a
//! standby: it registers them again every second and takes them over when the
//! first leaves. Needs an engine at `III_ENGINE_WS_URL` (default
//! `ws://127.0.0.1:49134`); without one the test skips, or fails when
//! `III_E2E_REQUIRE` is set.

mod common;

use std::time::Duration;

use common::{client, connected, engine_or_skip};
use iii_console_ui::{ConsoleUi, register_configuration_identity};
use iii_sdk::{CallRequest, IIIClient};
use serde_json::{Value, json};

const WORKER: &str = "standby";
const PAGE: &str = "standby/page.js";

/// What `<worker>::ui-content` serves for the page, if it answers.
async fn page(caller: &IIIClient) -> Option<String> {
    let mut request = CallRequest::new(format!("{WORKER}::ui-content"), json!({ "path": PAGE }));
    request.timeout_ms = Some(1000);
    let answer = caller.call(request).await.ok()?;
    answer["content"].as_str().map(str::to_string)
}

/// The entry id `<worker>::configuration-id` answers with, if it answers.
async fn identity(caller: &IIIClient) -> Option<String> {
    let mut request = CallRequest::new(format!("{WORKER}::configuration-id"), json!({}));
    request.timeout_ms = Some(1000);
    let answer: Value = caller.call(request).await.ok()?;
    answer["id"].as_str().map(str::to_string)
}

/// Waits up to 15 s until both functions answer with what `instance`
/// registered.
async fn served_by(caller: &IIIClient, instance: u8) {
    let content = format!("export const instance = {instance}");
    let entry = format!("entry-{instance}");
    for _ in 0..60 {
        if page(caller).await.as_deref() == Some(content.as_str())
            && identity(caller).await.as_deref() == Some(entry.as_str())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "instance {instance} does not serve: ui-content {:?}, configuration-id {:?}",
        page(caller).await,
        identity(caller).await
    );
}

/// Registers the worker's console UI and configuration identity as
/// instance `n` would.
fn register(iii: &std::sync::Arc<IIIClient>, n: u8) {
    ConsoleUi::new(WORKER)
        .script(PAGE, format!("export const instance = {n}"))
        .register(iii);
    let entry: &'static str = if n == 1 { "entry-1" } else { "entry-2" };
    register_configuration_identity(iii, WORKER, entry);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_instance_takes_over_the_console_functions_when_the_first_leaves() {
    if !engine_or_skip("standby").await {
        return;
    }
    let namespace = format!("cui-standby-{}", std::process::id());
    let first = client(&namespace, "standby-first");
    let second = client(&namespace, "standby-second");
    let caller = client(&namespace, "standby-caller");
    for iii in [&first, &second, &caller] {
        assert!(connected(iii).await, "the worker connects");
    }

    register(&first, 1);
    served_by(&caller, 1).await;

    register(&second, 2);
    // Past a few of the standby's registration attempts (one a second).
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert_eq!(
        page(&caller).await.as_deref(),
        Some("export const instance = 1"),
        "the first instance hosts the content function while it is connected"
    );
    assert_eq!(identity(&caller).await.as_deref(), Some("entry-1"));

    first.shutdown_async().await;
    served_by(&caller, 2).await;

    second.shutdown_async().await;
    caller.shutdown_async().await;
}
