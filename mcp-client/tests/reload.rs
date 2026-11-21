mod support;

use std::sync::Arc;
use std::time::Duration;

use mcp_client::functions::SecretChange;
use mcp_client::server::{Manager, State};
use serde_json::{Value, json};
use support::mcp_mock::McpMock;
use support::{
    ECHO, FakeHost, WAIT, apply, bearer, connect, eventually, server, tool, unregistered,
};

fn change(name: &str) -> SecretChange {
    SecretChange {
        name: Some(name.into()),
        reference: Some(format!("secret://{name}")),
        action: Some("rotated".into()),
    }
}

/// A started mock with the tool `echo`.
async fn mock() -> McpMock {
    let mock = McpMock::start().await;
    mock.tools(json!([tool("echo")])).await;
    mock
}

/// The config with `a` on that mock and `b` the given entry.
fn two(a: &McpMock, b: Value) -> Value {
    json!({"mcpServers": {"a": {"type": "http", "url": a.url()}, "b": b}})
}

async fn settle(manager: &Arc<Manager>, name: &str, state: State) -> Value {
    let reached = manager.wait_for(name, state, WAIT).await;
    let row = server(manager, name).await;
    assert!(reached, "{row}");
    row
}

#[tokio::test]
async fn only_the_changed_entry_reconnects() {
    let (a, b) = (mock().await, mock().await);
    let entry = |w: &str| json!({"type": "http", "url": b.url(), "headers": {"X-Workspace": w}});
    let host = FakeHost::new();
    let manager = connect(&host, two(&a, entry("one"))).await;
    settle(&manager, "a", State::Ready).await;
    settle(&manager, "b", State::Ready).await;
    let (events, ops) = (host.events().len(), host.ops().len());

    // The same value again: nothing happens at all.
    apply(&manager, two(&a, entry("one"))).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(b.requests("initialize").await.len(), 1);
    assert_eq!(host.events().len(), events);

    apply(&manager, two(&a, entry("two"))).await;
    eventually("b reconnects", || async {
        b.requests("initialize").await.len() == 2
    })
    .await;
    settle(&manager, "b", State::Ready).await;
    assert_eq!(
        b.headers_of("tools/list").await.last().unwrap()["x-workspace"],
        "two"
    );
    assert_eq!(a.requests("initialize").await.len(), 1, "a was not touched");
    assert_eq!(
        host.ops().len(),
        ops,
        "the same tools: no registry operation"
    );
}

#[tokio::test]
async fn removed_and_disabled_entries_lose_their_functions() {
    let (a, b) = (mock().await, mock().await);
    let host = FakeHost::new();
    let manager = connect(&host, two(&a, json!({"type": "http", "url": b.url()}))).await;
    settle(&manager, "a", State::Ready).await;
    settle(&manager, "b", State::Ready).await;

    apply(
        &manager,
        json!({"mcpServers": {"b": {"type": "http", "url": b.url(), "disabled": true}}}),
    )
    .await;
    eventually("both servers' functions are gone", || async {
        host.registered_ids().is_empty()
    })
    .await;
    let mut gone = unregistered(&host);
    gone.sort();
    assert_eq!(gone, ["mcp::a::echo", "mcp::b::echo"]);
    let list = serde_json::to_value(manager.list().await).unwrap();
    let servers = list["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 1, "{list}");
    assert_eq!(servers[0]["name"], "b");
    assert_eq!(servers[0]["state"], "disabled");

    apply(
        &manager,
        json!({"mcpServers": {"b": {"type": "http", "url": b.url()}}}),
    )
    .await;
    settle(&manager, "b", State::Ready).await;
    assert_eq!(host.registered_ids(), ["mcp::b::echo"]);
}

#[tokio::test]
async fn rotated_secret_reconnects_with_the_new_value() {
    let mock = mock().await;
    let host = FakeHost::new();
    host.set_secret("secret://K", "k1");
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, "srv", State::Ready).await;

    host.set_secret("secret://K", "k2");
    manager.on_secret_change(&change("K")).await;
    eventually("srv reconnects", || async {
        mock.requests("initialize").await.len() == 2
    })
    .await;
    assert_eq!(
        mock.headers_of("initialize").await[1]["authorization"],
        "Bearer k2"
    );
    settle(&manager, "srv", State::Ready).await;

    // A secret no entry names, and a change that did not change the value.
    manager.on_secret_change(&change("OTHER")).await;
    manager.on_secret_change(&change("K")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mock.requests("initialize").await.len(), 2);
    assert!(unregistered(&host).is_empty());
}

#[tokio::test]
async fn secret_change_is_the_way_out_of_config_error_and_auth_failed() {
    let mock = mock().await;
    let host = FakeHost::new();
    let manager = connect(&host, bearer(&mock.url())).await;
    let row = settle(&manager, "srv", State::ConfigError).await;
    assert_eq!(row["error"]["code"], "secret_missing", "{row}");
    assert_eq!(row["stage"], "secret", "{row}");

    host.set_secret("secret://K", "k1");
    manager.on_secret_change(&change("K")).await;
    settle(&manager, "srv", State::Ready).await;

    mock.http_status(
        "initialize",
        401,
        &[("www-authenticate", r#"Bearer error="invalid_token""#)],
    )
    .await;
    host.set_secret("secret://K", "revoked");
    manager.on_secret_change(&change("K")).await;
    settle(&manager, "srv", State::AuthFailed).await;
    assert_eq!(
        host.registered_ids(),
        [ECHO],
        "auth_failed keeps the functions"
    );

    mock.clear_http_status().await;
    host.set_secret("secret://K", "k3");
    manager.on_secret_change(&change("K")).await;
    settle(&manager, "srv", State::Ready).await;
    assert!(unregistered(&host).is_empty());
}
