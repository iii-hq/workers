#![cfg(feature = "test-fixtures")]
//! The stdio runtime against the fake stdio MCP server: spawn, environment,
//! handshake, exit, restart and stop.

mod support;

use std::sync::Arc;
use std::time::Duration;

use mcp_client::config::{RawConfig, StdioEntry, fingerprint};
use mcp_client::functions::SecretChange;
use mcp_client::refs::{Resolved, Text};
use mcp_client::server::{Manager, ManagerOptions, State, Timings};
use serde_json::{Value, json};
use support::stdio_fixture::{BIN, cfg, opts};
use support::{FakeHost, RegOp, WAIT, connect_opts, server, stdio_timings, until};

/// Backoff of 5 s, so a test can look at `unavailable` before the restart.
fn held() -> Timings {
    Timings {
        backoff_min: Duration::from_secs(5),
        backoff_max: Duration::from_secs(5),
        ..stdio_timings()
    }
}

const READY: Duration = Duration::from_secs(10);

async fn reached(m: &Arc<Manager>, name: &str, state: State) -> Value {
    let within = if state == State::Ready { READY } else { WAIT };
    let ok = m.wait_for(name, state, within).await;
    let row = server(m, name).await;
    assert!(ok, "{name} never reached {state:?}: {row}");
    row
}

async fn ready(host: &Arc<FakeHost>, env: Value) -> Arc<Manager> {
    let m = connect_opts(host, &cfg(env), opts(stdio_timings())).await;
    reached(&m, "fake", State::Ready).await;
    m
}

async fn text(host: &FakeHost, tool: &str, args: Value) -> String {
    let id = if tool.starts_with("mcp::") {
        tool.to_string()
    } else {
        format!("mcp::fake::{tool}")
    };
    let out = host.call(&id, args).await.unwrap();
    out["content"][0]["text"].as_str().unwrap().to_string()
}

async fn pid(host: &FakeHost, var: &str) -> i32 {
    pid_of(host, "mcp::fake::env", var).await
}

async fn pid_of(host: &FakeHost, id: &str, var: &str) -> i32 {
    text(host, id, json!({"name": var})).await.parse().unwrap()
}

fn has_unregister(host: &FakeHost) -> bool {
    host.ops()
        .iter()
        .any(|op| matches!(op, RegOp::Unregister(_)))
}

/// kill(pid, 0), and on Linux not a zombie (field 3 of /proc/<pid>/stat is not 'Z'): a killed grandchild
/// stays a zombie until its new parent reaps it, and some CI containers have no reaping init.
#[cfg(unix)]
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 sends nothing; it only checks that the pid exists.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        let state = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next());
        return state != Some("Z");
    }
    true
}

#[tokio::test]
async fn spawns_lists_and_calls() {
    let host = FakeHost::new();
    let m = ready(&host, json!({})).await;
    assert!(
        host.registered_ids()
            .contains(&"mcp::fake::echo".to_string())
    );
    let echo = host.call("mcp::fake::echo", json!({"text": "hi"})).await;
    assert_eq!(
        echo,
        Ok(json!({"content": [{"type": "text", "text": "hi"}]}))
    );
    let fail = host.call("mcp::fake::fail", json!({})).await;
    assert_eq!(fail, Err("boom".to_string()));
    let v = server(&m, "fake").await;
    assert_eq!(
        (&v["transport"], &v["target"]),
        (&json!("stdio"), &json!(BIN))
    );
    m.shutdown().await;
}

#[tokio::test]
async fn env_is_the_base_plus_the_entry_and_cwd_is_applied() {
    assert!(std::env::var("CARGO_MANIFEST_DIR").is_ok());
    let host = FakeHost::new();
    host.set_secret("secret://FAKE_GH", "tok-123");
    let dir = tempfile::tempdir().unwrap();
    let config = json!({"mcpServers": {
        "fake": {
            "command": BIN,
            "env": {"FOO": "bar", "GITHUB_TOKEN": "secret://FAKE_GH"},
            "cwd": dir.path(),
        },
        "plain": {"command": BIN},
    }});
    let m = connect_opts(&host, &config, opts(stdio_timings())).await;
    reached(&m, "fake", State::Ready).await;
    reached(&m, "plain", State::Ready).await;
    let env = async |name: &str| text(&host, "env", json!({"name": name})).await;
    assert_eq!(env("FOO").await, "bar");
    assert_eq!(env("GITHUB_TOKEN").await, "tok-123");
    assert_eq!(env("PATH").await, std::env::var("PATH").unwrap());
    assert_eq!(env("CARGO_MANIFEST_DIR").await, "<unset>");
    let canon = |p: &str| std::fs::canonicalize(p).unwrap();
    let cwd = text(&host, "cwd", json!({})).await;
    assert_eq!(canon(&cwd), dir.path().canonicalize().unwrap());
    let plain = text(&host, "mcp::plain::cwd", json!({})).await;
    assert_eq!(canon(&plain), std::env::temp_dir().canonicalize().unwrap());
    m.shutdown().await;
}

#[tokio::test]
async fn an_exit_is_unavailable_with_the_code_and_the_last_stderr_line() {
    let host = FakeHost::new();
    let m = connect_opts(&host, &cfg(json!({})), opts(held())).await;
    reached(&m, "fake", State::Ready).await;
    assert_eq!(
        text(&host, "stderr", json!({"text": "about to die"})).await,
        "ok"
    );
    let exit = host.call("mcp::fake::exit", json!({"code": 3})).await;
    assert!(exit.is_err(), "{exit:?}");
    let v = reached(&m, "fake", State::Unavailable).await;
    assert_eq!(
        (&v["stage"], &v["error"]["code"]),
        (&json!("connect"), &json!("exited")),
        "{v}"
    );
    assert_eq!(v["error"]["message"], "exited with code 3: about to die");
    assert!(
        host.registered_ids()
            .contains(&"mcp::fake::echo".to_string())
    );
    assert!(!has_unregister(&host));
    let echo = host.call("mcp::fake::echo", json!({"text": "x"})).await;
    let error = echo.unwrap_err();
    assert!(error.contains("MCP unavailable (`unavailable`)"), "{error}");
    m.shutdown().await;
}

#[tokio::test]
async fn it_restarts_after_an_exit() {
    let host = FakeHost::new();
    let m = ready(&host, json!({})).await;
    let first = pid(&host, "FAKE_MCP_PID").await;
    let exit = host.call("mcp::fake::exit", json!({"code": 1})).await;
    assert!(exit.is_err(), "{exit:?}");
    let restarted = until(Duration::from_secs(10), || async {
        let out = host
            .call("mcp::fake::env", json!({"name": "FAKE_MCP_PID"}))
            .await;
        out.is_ok_and(|v| v["content"][0]["text"] != json!(first.to_string()))
    })
    .await;
    assert!(restarted, "{}", server(&m, "fake").await);
    assert!(!has_unregister(&host));
    m.shutdown().await;
}

#[tokio::test]
async fn no_handshake_is_handshake_failed_with_the_stderr_tail() {
    let host = FakeHost::new();
    let timings = Timings {
        stdio_start: Duration::from_secs(2),
        ..held()
    };
    let env = json!({"FAKE_MCP_NO_HANDSHAKE": "1", "FAKE_MCP_STDERR_BANNER": "starting up"});
    let m = connect_opts(&host, &cfg(env), opts(timings)).await;
    let v = reached(&m, "fake", State::Unavailable).await;
    assert_eq!(v["error"]["code"], "handshake_failed", "{v}");
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.ends_with(": starting up"), "{message}");
    m.shutdown().await;
}

/// Pins rmcp 3.11.0: a stdout line that is not JSON is dropped with a debug
/// log and the read goes on (`src/transport/async_rw.rs:314-323`). If an rmcp
/// bump changes that, this test turns red.
#[tokio::test]
async fn a_stdout_banner_before_the_handshake_is_ignored() {
    let host = FakeHost::new();
    let m = ready(
        &host,
        json!({"FAKE_MCP_STDOUT_BANNER": "Fake MCP server v0 listening"}),
    )
    .await;
    assert_eq!(text(&host, "echo", json!({"text": "hi"})).await, "hi");
    m.shutdown().await;
}

#[tokio::test]
async fn a_missing_command_is_spawn_failed_naming_the_command_and_the_path() {
    let host = FakeHost::new();
    let config = json!({"mcpServers": {"fake": {
        "command": "no-such-command-7f3a",
        "env": {"PATH": "/nonexistent-bin"},
    }}});
    let m = connect_opts(&host, &config, opts(held())).await;
    let v = reached(&m, "fake", State::Unavailable).await;
    assert_eq!(v["error"]["code"], "spawn_failed", "{v}");
    assert_eq!(
        v["error"]["message"],
        "command `no-such-command-7f3a` not found on PATH `/nonexistent-bin`"
    );
    m.shutdown().await;
}

#[tokio::test]
async fn stdio_off_rejects_the_entry() {
    let host = FakeHost::new();
    let options = ManagerOptions {
        stdio_enabled: false,
        ..opts(stdio_timings())
    };
    let m = connect_opts(&host, &cfg(json!({})), options).await;
    let v = reached(&m, "fake", State::ConfigError).await;
    assert_eq!(
        (&v["stage"], &v["error"]["code"]),
        (&json!("config"), &json!("stdio_disabled")),
        "{v}"
    );
    assert!(host.registered_ids().is_empty());
    m.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn stop_kills_a_child_that_ignores_sigterm() {
    let host = FakeHost::new();
    let env = json!({"FAKE_MCP_IGNORE_TERM": "1", "FAKE_MCP_GRANDCHILD": "1"});
    let m = ready(&host, env.clone()).await;
    let child = pid(&host, "FAKE_MCP_PID").await;
    let grandchild = pid(&host, "FAKE_MCP_GRANDCHILD_PID").await;
    assert!(alive(child) && alive(grandchild));
    let disabled = json!({"mcpServers": {"fake": {"command": BIN, "env": env, "disabled": true}}});
    m.apply_config(RawConfig::from_value(&disabled)).await;
    let gone = until(Duration::from_millis(1500), || async {
        !alive(child) && !alive(grandchild)
    })
    .await;
    assert!(
        gone,
        "child alive {}, grandchild alive {}",
        alive(child),
        alive(grandchild)
    );
    assert_eq!(server(&m, "fake").await["state"], "disabled");
    assert!(host.registered_ids().is_empty());
    m.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn removal_and_shutdown_stop_the_process() {
    let host = FakeHost::new();
    let config = json!({"mcpServers": {"a": {"command": BIN}, "b": {"command": BIN}}});
    let m = connect_opts(&host, &config, opts(stdio_timings())).await;
    reached(&m, "a", State::Ready).await;
    reached(&m, "b", State::Ready).await;
    let a = pid_of(&host, "mcp::a::env", "FAKE_MCP_PID").await;
    let b = pid_of(&host, "mcp::b::env", "FAKE_MCP_PID").await;
    let only_b = json!({"mcpServers": {"b": {"command": BIN}}});
    m.apply_config(RawConfig::from_value(&only_b)).await;
    assert!(until(Duration::from_millis(1500), || async { !alive(a) }).await);
    assert!(alive(b));
    m.shutdown().await;
    assert!(until(Duration::from_millis(1500), || async { !alive(b) }).await);
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_the_guard_kills_the_group() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let literal = |v: &str| Text::Literal(v.into());
    let entry = StdioEntry {
        command: BIN.into(),
        args: vec![],
        cwd: None,
        env: [
            ("FAKE_MCP_IGNORE_TERM".to_string(), literal("1")),
            ("FAKE_MCP_GRANDCHILD".to_string(), literal("1")),
        ]
        .into(),
    };
    let resolved = Resolved {
        values: Default::default(),
    };
    let mut s = mcp_client::stdio::spawn(&entry, &resolved, &std::env::temp_dir()).unwrap();
    let child = s.pid.unwrap() as i32;
    let mut line = String::new();
    BufReader::new(&mut s.stderr)
        .read_line(&mut line)
        .await
        .unwrap();
    let grandchild: i32 = line
        .trim()
        .strip_prefix("grandchild ")
        .unwrap()
        .parse()
        .unwrap();
    assert!(alive(child) && alive(grandchild));
    drop(s);
    let gone = until(Duration::from_secs(1), || async {
        !alive(child) && !alive(grandchild)
    })
    .await;
    assert!(
        gone,
        "child alive {}, grandchild alive {}",
        alive(child),
        alive(grandchild)
    );
}

#[tokio::test]
async fn an_unapproved_stdio_entry_spawns_nothing_until_approved() {
    let host = FakeHost::new();
    let m = Manager::new(host.clone(), opts(stdio_timings()));
    m.apply_config(RawConfig::from_value(&cfg(json!({})))).await;
    let v = reached(&m, "fake", State::Unapproved).await;
    assert_eq!(
        (&v["stage"], &v["error"]["code"]),
        (&json!("approval"), &json!("unapproved")),
        "{v}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(host.registered_ids().is_empty());
    let env = host
        .call("mcp::fake::env", json!({"name": "FAKE_MCP_PID"}))
        .await;
    assert!(env.is_err(), "{env:?}");

    m.approve("fake", None).await.unwrap();
    reached(&m, "fake", State::Ready).await;
    let child = pid(&host, "FAKE_MCP_PID").await;

    // A foreign write to the approvals unapproves everything and stops the child.
    let foreign = format!(
        r#"{{"approved":{{"fake":"{}"}},"version":1}}"#,
        fingerprint(&RawConfig::from_value(&cfg(json!({}))).0["fake"])
    );
    host.set_secret("secret://MCP_CLIENT_APPROVALS", &foreign);
    let change = SecretChange {
        name: Some("MCP_CLIENT_APPROVALS".into()),
        reference: Some("secret://MCP_CLIENT_APPROVALS".into()),
        action: Some("rotated".into()),
    };
    m.on_secret_change(&change).await;
    reached(&m, "fake", State::Unapproved).await;
    #[cfg(unix)]
    assert!(until(Duration::from_millis(1500), || async { !alive(child) }).await);
    let _ = child;
    m.shutdown().await;
}
