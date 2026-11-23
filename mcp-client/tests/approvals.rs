mod support;

use std::sync::Arc;
use std::time::Duration;

use mcp_client::config::{RawConfig, fingerprint};
use mcp_client::functions::{SecretChange, catalog};
use mcp_client::server::{Manager, State};
use serde_json::{Value, json};
use support::mcp_mock::McpMock;
use support::{
    ECHO, FakeHost, WAIT, bearer, connect, connect_unapproved, eventually, fast_timings, hi,
    server, tool, unregistered,
};

const APPROVALS: &str = "MCP_CLIENT_APPROVALS";

/// The entry of `srv` in `bearer(url)`.
fn entry(url: &str) -> Value {
    json!({"type": "http", "url": url, "auth": {"type": "bearer", "token": "secret://K"}})
}

/// The envelope text this worker writes for these approvals.
fn stored(approved: &[(&str, &str)]) -> String {
    let pairs: Vec<String> = approved
        .iter()
        .map(|(name, fp)| format!(r#""{name}":"{fp}""#))
        .collect();
    format!(r#"{{"version":1,"approved":{{{}}}}}"#, pairs.join(","))
}

fn approvals_changed() -> SecretChange {
    SecretChange {
        name: Some(APPROVALS.into()),
        reference: Some(format!("secret://{APPROVALS}")),
        action: Some("rotated".into()),
    }
}

/// A started mock with the tool `echo` answering `hi()`.
async fn mock() -> McpMock {
    let mock = McpMock::start().await;
    mock.tools(json!([tool("echo")])).await;
    mock.on_call("echo", hi()).await;
    mock
}

fn host() -> Arc<FakeHost> {
    let host = FakeHost::new();
    host.set_secret("secret://K", "k1");
    host
}

async fn settle(manager: &Arc<Manager>, state: State) -> Value {
    let reached = manager.wait_for("srv", state, WAIT).await;
    let row = server(manager, "srv").await;
    assert!(reached, "{row}");
    row
}

#[tokio::test]
async fn unapproved_entry_makes_no_request_and_resolves_nothing() {
    let mock = mock().await;
    let host = host();
    let manager = connect_unapproved(&host, bearer(&mock.url())).await;
    let view = settle(&manager, State::Unapproved).await;
    assert_eq!(view["stage"], "approval", "{view}");
    assert_eq!(view["error"]["code"], "unapproved", "{view}");
    assert_eq!(view["approved"], false, "{view}");
    assert_eq!(view["fingerprint"], fingerprint(&entry(&mock.url())));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(mock.request_count().await, 0);
    assert_eq!(host.resolve_count("secret://K"), 0);
    assert!(host.registered_ids().is_empty());
    assert!(matches!(
        manager.reconnect("srv").await,
        Ok(State::Unapproved)
    ));
    assert_eq!(
        mock.request_count().await,
        0,
        "reconnect does not pass the gate"
    );
}

#[tokio::test]
async fn approve_starts_the_entry_and_is_stored_in_the_vault() {
    let mock = mock().await;
    let host = host();
    let manager = connect_unapproved(&host, bearer(&mock.url())).await;
    settle(&manager, State::Unapproved).await;
    manager.approve("srv", None).await.unwrap();
    let view = settle(&manager, State::Ready).await;
    assert_eq!(view["approved"], true, "{view}");
    let fp = fingerprint(&entry(&mock.url()));
    assert_eq!(host.vault(APPROVALS).unwrap(), stored(&[("srv", &fp)]));
    assert!(
        manager.approve("nope", None).await.is_err(),
        "no entry of that name is loaded"
    );
    assert!(
        manager.approve("srv", Some("abc")).await.is_err(),
        "not a fingerprint"
    );
}

#[tokio::test]
async fn approve_with_a_fingerprint_works_before_the_entry_is_written() {
    let mock = mock().await;
    let host = host();
    let manager = connect_unapproved(&host, json!({"mcpServers": {}})).await;
    // The page's order: approve, then write.
    manager
        .approve("srv", Some(&fingerprint(&entry(&mock.url()))))
        .await
        .unwrap();
    manager
        .apply_config(RawConfig::from_value(&bearer(&mock.url())))
        .await;
    settle(&manager, State::Ready).await;
}

#[tokio::test]
async fn approving_a_new_fingerprint_keeps_the_running_entry_approved_until_the_write() {
    let (mock, other) = (mock().await, mock().await);
    let host = host();
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, State::Ready).await;
    let (old, new) = (
        fingerprint(&entry(&mock.url())),
        fingerprint(&entry(&other.url())),
    );

    manager.approve("srv", Some(&new)).await.unwrap();
    let view = server(&manager, "srv").await;
    assert_eq!(view["state"], "ready", "{view}");
    assert_eq!(view["approved"], true, "{view}");
    assert_eq!(host.vault(APPROVALS).unwrap(), stored(&[("srv", &old)]));
    assert!(matches!(
        manager.reconnect("srv").await,
        Ok(State::Connecting)
    ));
    settle(&manager, State::Ready).await;
    assert_eq!(mock.requests("initialize").await.len(), 2);

    // The write: an unapproved entry would wait there for an approve call.
    manager
        .apply_config(RawConfig::from_value(&bearer(&other.url())))
        .await;
    let view = settle(&manager, State::Ready).await;
    assert_eq!(view["target"], other.url(), "{view}");
    assert_eq!(other.requests("initialize").await.len(), 1);
    assert_eq!(host.vault(APPROVALS).unwrap(), stored(&[("srv", &new)]));
}

#[tokio::test]
async fn changed_entry_goes_unapproved_and_keeps_its_functions() {
    let (mock, other) = (mock().await, mock().await);
    let host = host();
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, State::Ready).await;
    let resolved = host.resolve_count("secret://K");

    // Somebody rewrote the entry and nobody approved it.
    manager
        .apply_config(RawConfig::from_value(&bearer(&other.url())))
        .await;
    settle(&manager, State::Unapproved).await;
    assert_eq!(host.registered_ids(), [ECHO]);
    assert!(unregistered(&host).is_empty());
    let error = host.call(ECHO, json!({})).await.unwrap_err();
    assert!(
        error.starts_with("`srv` MCP unavailable (`unapproved`), for "),
        "{error}"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(other.request_count().await, 0);
    assert_eq!(
        host.resolve_count("secret://K"),
        resolved,
        "nothing was resolved for it"
    );
}

#[tokio::test]
async fn disabled_flip_needs_no_new_approval() {
    let mock = mock().await;
    let host = host();
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, State::Ready).await;
    let mut off = entry(&mock.url());
    off["disabled"] = json!(true);
    manager
        .apply_config(RawConfig::from_value(&json!({"mcpServers": {"srv": off}})))
        .await;
    let view = settle(&manager, State::Disabled).await;
    assert_eq!(view["approved"], true, "{view}");
    manager
        .apply_config(RawConfig::from_value(&bearer(&mock.url())))
        .await;
    settle(&manager, State::Ready).await;
}

#[tokio::test]
async fn foreign_write_to_the_approvals_unapproves_everything() {
    let mock = mock().await;
    let host = host();
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, State::Ready).await;
    let fp = fingerprint(&entry(&mock.url()));

    // The change event of this worker's own write is not a tamper.
    manager.on_secret_change(&approvals_changed()).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server(&manager, "srv").await["state"], "ready");

    // A foreign write, even one that approves the very same fingerprint: the
    // text differs from what this worker wrote.
    let foreign = format!(r#"{{"approved":{{"srv":"{fp}"}},"version":1}}"#);
    host.set_secret("secret://MCP_CLIENT_APPROVALS", &foreign);
    manager.on_secret_change(&approvals_changed()).await;
    settle(&manager, State::Unapproved).await;
    assert_eq!(host.registered_ids(), [ECHO]);
    assert!(unregistered(&host).is_empty());

    // The way back; it replaces the foreign text.
    manager.approve("srv", None).await.unwrap();
    settle(&manager, State::Ready).await;
    assert_eq!(host.vault(APPROVALS).unwrap(), stored(&[("srv", &fp)]));

    // Tamper detection lasts until the worker restarts: at start it trusts
    // what is stored.
    host.set_secret("secret://MCP_CLIENT_APPROVALS", &foreign);
    manager.on_secret_change(&approvals_changed()).await;
    settle(&manager, State::Unapproved).await;
    manager.shutdown().await;
    let manager = connect_unapproved(&host, bearer(&mock.url())).await;
    settle(&manager, State::Ready).await;
}

#[tokio::test]
async fn a_foreign_approval_before_the_first_load_is_not_trusted() {
    let mock = mock().await;
    let host = host();
    let manager = connect_unapproved(&host, json!({"mcpServers": {}})).await;
    let fp = fingerprint(&entry(&mock.url()));
    host.set_secret("secret://MCP_CLIENT_APPROVALS", &stored(&[("srv", &fp)]));
    manager.on_secret_change(&approvals_changed()).await;
    manager
        .apply_config(RawConfig::from_value(&bearer(&mock.url())))
        .await;
    settle(&manager, State::Unapproved).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(mock.request_count().await, 0);
    assert_eq!(host.resolve_count("secret://K"), 0);
}

#[tokio::test]
async fn approvals_are_read_at_start_without_any_entry() {
    let host = host();
    let manager = connect_unapproved(&host, json!({"mcpServers": {}})).await;
    let reads = || host.resolve_count("secret://MCP_CLIENT_APPROVALS");
    eventually("the approvals are read at start", || async { reads() == 1 }).await;
    tokio::time::sleep(fast_timings().backoff_max * 2).await;
    assert_eq!(reads(), 1, "read once");
    manager.shutdown().await;

    // Unreadable: tried again with backoff until the worker shuts down.
    let host = FakeHost::new();
    host.set_unreachable(true);
    let manager = connect_unapproved(&host, json!({"mcpServers": {}})).await;
    let reads = || host.resolve_count("secret://MCP_CLIENT_APPROVALS");
    eventually("the read is retried", || async { reads() >= 3 }).await;
    manager.shutdown().await;
    let after = reads();
    tokio::time::sleep(fast_timings().backoff_max * 2).await;
    assert_eq!(reads(), after, "shutdown stops the retry");
}

#[tokio::test]
async fn approvals_unreadable_at_boot_are_retried() {
    let mock = mock().await;
    let host = host();
    let fp = fingerprint(&entry(&mock.url()));
    host.set_secret("secret://MCP_CLIENT_APPROVALS", &stored(&[("srv", &fp)]));
    host.set_unreachable(true);
    let manager = connect_unapproved(&host, bearer(&mock.url())).await;
    assert!(manager.wait_for("srv", State::Connecting, WAIT).await);
    tokio::time::sleep(fast_timings().backoff_max * 2).await;
    let view = server(&manager, "srv").await;
    assert_eq!(view["state"], "connecting", "{view}");
    assert_eq!(view["stage"], "approval", "{view}");
    assert_eq!(view["error"]["code"], "connect_failed", "{view}");
    assert_eq!(mock.request_count().await, 0);
    host.set_unreachable(false);
    settle(&manager, State::Ready).await;
}

#[tokio::test]
async fn approvals_not_shared_with_the_worker_stay_at_the_approval_stage() {
    let mock = mock().await;
    let host = host();
    host.forbid("secret://MCP_CLIENT_APPROVALS");
    let manager = connect_unapproved(&host, bearer(&mock.url())).await;
    assert!(manager.wait_for("srv", State::Connecting, WAIT).await);
    eventually("the error names the secret", || async {
        server(&manager, "srv").await["error"]["code"] == "secret_not_shared"
    })
    .await;
    let view = server(&manager, "srv").await;
    assert_eq!(view["state"], "connecting", "{view}");
    assert_eq!(view["stage"], "approval", "{view}");
    let message = view["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("`secret://MCP_CLIENT_APPROVALS`") && message.contains("`mcp-client`"),
        "{message}"
    );
    assert_eq!(mock.request_count().await, 0);
}

#[tokio::test]
async fn removed_entry_forgets_its_approval() {
    let mock = mock().await;
    let host = host();
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, State::Ready).await;
    manager
        .apply_config(RawConfig::from_value(&json!({"mcpServers": {}})))
        .await;
    eventually("the approval is gone from the vault", || async {
        host.vault(APPROVALS).as_deref() == Some(stored(&[]).as_str())
    })
    .await;
    manager
        .apply_config(RawConfig::from_value(&bearer(&mock.url())))
        .await;
    settle(&manager, State::Unapproved).await;
}

#[tokio::test]
async fn reconnect_cuts_a_wait_short_and_leaves_auth_failed() {
    let mock = mock().await;
    mock.http_status("initialize", 429, &[("retry-after", "30")])
        .await;
    let host = host();
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, State::Unavailable).await;
    mock.clear_http_status().await;
    assert!(matches!(
        manager.reconnect("srv").await,
        Ok(State::Connecting)
    ));
    assert!(
        manager.wait_for("srv", State::Ready, WAIT).await,
        "30 s were not waited"
    );

    mock.http_status(
        "tools/call",
        401,
        &[("www-authenticate", r#"Bearer error="invalid_token""#)],
    )
    .await;
    assert!(host.call(ECHO, json!({})).await.is_err());
    settle(&manager, State::AuthFailed).await;
    mock.clear_http_status().await;
    assert!(matches!(
        manager.reconnect("srv").await,
        Ok(State::Connecting)
    ));
    settle(&manager, State::Ready).await;
    assert!(manager.reconnect("nope").await.is_err());
}

#[test]
fn approve_and_reconnect_are_in_the_catalog() {
    let ids: Vec<&str> = catalog().into_iter().map(|(id, ..)| id).collect();
    for id in [
        "mcp-client::servers::approve",
        "mcp-client::servers::reconnect",
    ] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
}
