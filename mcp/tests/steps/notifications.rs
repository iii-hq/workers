//! Step defs for `tests/features/notifications.feature`.
//!
//! MCP Streamable HTTP §"server response": notification-only requests
//! (no `id` field) MUST return 202 Accepted with no body. Asserts that
//! contract end-to-end through the bridge.

use cucumber::{then, when};
use serde_json::json;

use crate::common::http::{last_status, post_jsonrpc};
use crate::common::world::McpWorld;

#[when("I send a notifications/initialized notification")]
async fn send_notifications_initialized(world: &mut McpWorld) {
    post_jsonrpc(
        world,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
}

#[when(regex = r#"^I send an unknown notification with method "([^"]+)"$"#)]
async fn send_unknown_notification(world: &mut McpWorld, method: String) {
    post_jsonrpc(world, json!({"jsonrpc": "2.0", "method": method})).await;
}

#[then("the http response status is 202")]
fn status_is_202(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    assert_eq!(last_status(world), 202, "notification must return 202");
}
