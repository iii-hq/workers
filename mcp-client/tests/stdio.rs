#![cfg(feature = "test-fixtures")]
//! The stdio runtime against the fake stdio MCP server: spawn, environment,
//! handshake, exit, restart and stop.

mod support;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mcp_client::config::{RawConfig, StdioEntry, fingerprint};
use mcp_client::functions::SecretChange;
use mcp_client::host::{EventKind, ServerEvent};
use mcp_client::refs::{Resolved, Text};
use mcp_client::server::{Manager, ManagerOptions, State, Timings};
use serde_json::{Value, json};
use support::stdio_fixture::{BIN, cfg, opts};
use support::{FakeHost, RegOp, WAIT, connect_opts, server, stdio_timings, until};
use tokio::time::timeout;

/// Backoff of 5 s, so a test can look at `unavailable` before the restart.
fn held() -> Timings {
    Timings {
        backoff_min: Duration::from_secs(5),
        backoff_max: Duration::from_secs(5),
        ..stdio_timings()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
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
async fn a_tail_comes_only_from_the_process_that_printed_it() {
    let host = FakeHost::new();
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("knobs");
    std::fs::write(&state, "FAKE_MCP_STDERR_BANNER=from the first process\n").unwrap();
    let m = ready(&host, json!({"FAKE_MCP_STATE_FILE": state})).await;
    // Every later start prints nothing and exits at once.
    std::fs::write(&state, "FAKE_MCP_EXIT_AFTER_MS=0\nFAKE_MCP_EXIT_CODE=7\n").unwrap();
    let exit = host.call("mcp::fake::exit", json!({"code": 3})).await;
    assert!(exit.is_err(), "{exit:?}");
    let message = async || server(&m, "fake").await["error"]["message"].clone();
    let restarted = until(Duration::from_secs(5), || async {
        message()
            .await
            .as_str()
            .is_some_and(|t| t.contains("code 7"))
    })
    .await;
    assert!(restarted, "{}", server(&m, "fake").await);
    assert_eq!(message().await, "exited with code 7");
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
    // stop_grace plus one second: a stuck stop fails here instead of hanging.
    let bound = stdio_timings().stop_grace + Duration::from_secs(1);
    let removal = m.apply_config(RawConfig::from_value(&only_b));
    timeout(bound, removal)
        .await
        .expect("the removal stopped in time");
    assert!(until(Duration::from_millis(1500), || async { !alive(a) }).await);
    assert!(alive(b));
    timeout(bound, m.shutdown())
        .await
        .expect("the shutdown stopped in time");
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
async fn a_config_after_shutdown_starts_nothing() {
    let host = FakeHost::new();
    let m = connect_opts(&host, &json!({"mcpServers": {}}), opts(stdio_timings())).await;
    m.shutdown().await;
    let raw = RawConfig::from_value(&cfg(json!({})));
    let fp = fingerprint(&raw.0["fake"]);
    m.approve("fake", Some(&fp)).await.unwrap();
    m.apply_config(raw).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let list = serde_json::to_value(m.list().await).unwrap();
    assert_eq!(list["servers"], json!([]), "{list}");
    assert!(host.registered_ids().is_empty());
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

#[tokio::test]
async fn process_is_reported_in_servers_list() {
    let host = FakeHost::new();
    let m = ready(&host, json!({})).await;
    let child = pid(&host, "FAKE_MCP_PID").await;
    let v = server(&m, "fake").await;
    let p = &v["process"];
    assert_eq!(p["pid"], json!(child), "{v}");
    assert!(p["started_at"].as_u64().unwrap() <= now_ms(), "{v}");
    assert_eq!(
        (&p["restarts"], &p["restart_budget"]),
        (&json!(0), &json!(5)),
        "{v}"
    );
    assert!(p["next_restart_at"].is_null(), "{v}");
    assert!(p["last_exit"].is_null(), "{v}");
    let process = ServerEvent {
        server: "fake".into(),
        kind: EventKind::Process,
    };
    assert!(host.events().contains(&process));
    m.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_kill_is_reported_as_a_signal_with_the_next_restart() {
    let host = FakeHost::new();
    let m = connect_opts(&host, &cfg(json!({})), opts(held())).await;
    reached(&m, "fake", State::Ready).await;
    let child = pid(&host, "FAKE_MCP_PID").await;
    // SAFETY: a signal to the fixture's own pid, read from its environment.
    assert_eq!(unsafe { libc::kill(child, libc::SIGKILL) }, 0);
    let v = reached(&m, "fake", State::Unavailable).await;
    let p = &v["process"];
    assert_eq!(v["error"]["message"], "killed by signal 9", "{v}");
    assert_eq!(p["last_exit"], json!({"code": null, "signal": 9}), "{v}");
    assert!(p["pid"].is_null(), "{v}");
    assert_eq!(p["restarts"], 1, "{v}");
    assert!(p["next_restart_at"].as_u64().unwrap() > now_ms(), "{v}");
    m.shutdown().await;
}

#[tokio::test]
async fn the_budget_is_spent_then_cooldown_and_reconnect_resets_it() {
    let host = FakeHost::new();
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("knobs");
    // Every start dies at once.
    std::fs::write(&state, "FAKE_MCP_EXIT_AFTER_MS=0\nFAKE_MCP_EXIT_CODE=2\n").unwrap();
    let ms = Duration::from_millis;
    let timings = Timings {
        backoff_min: ms(10),
        backoff_max: ms(40),
        budget_starts: 3,
        budget_window: Duration::from_secs(60),
        cooldown: Duration::from_secs(30),
        ..stdio_timings()
    };
    let config = cfg(json!({"FAKE_MCP_STATE_FILE": state}));
    let m = connect_opts(&host, &config, opts(timings)).await;
    let process = async || server(&m, "fake").await["process"].clone();
    let spent = until(Duration::from_secs(10), || async {
        process().await["restarts"] == 3
    })
    .await;
    let v = server(&m, "fake").await;
    assert!(spent, "{v}");
    let p = &v["process"];
    assert_eq!(
        (&v["state"], &v["error"]["code"]),
        (&json!("unavailable"), &json!("exited")),
        "{v}"
    );
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.starts_with("exited with code 2"), "{v}");
    assert_eq!(p["last_exit"]["code"], 2, "{v}");
    assert_eq!(p["restart_budget"], 3, "{v}");
    // In the cooldown, not in the backoff.
    assert!(
        p["next_restart_at"].as_u64().unwrap() > now_ms() + 20_000,
        "{v}"
    );
    let started = p["started_at"].clone();
    tokio::time::sleep(ms(300)).await;
    assert_eq!(
        process().await["started_at"],
        started,
        "a start in cooldown"
    );
    // The state file is read at each start.
    std::fs::write(&state, "").unwrap();
    m.reconnect("fake").await.unwrap();
    let v = reached(&m, "fake", State::Ready).await;
    assert_eq!(v["process"]["restarts"], 0, "{v}");
    m.shutdown().await;
}

#[tokio::test]
async fn staying_ready_resets_the_budget_and_a_later_exit_is_not_a_failed_start() {
    let host = FakeHost::new();
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("knobs");
    std::fs::write(&state, "FAKE_MCP_EXIT_AFTER_MS=0\n").unwrap();
    // Never in cooldown here.
    let timings = Timings {
        stable_after: Duration::from_millis(200),
        budget_starts: 100,
        ..stdio_timings()
    };
    let config = cfg(json!({"FAKE_MCP_STATE_FILE": state}));
    let m = connect_opts(&host, &config, opts(timings)).await;
    let restarts = async || server(&m, "fake").await["process"]["restarts"].as_u64();
    let failing = until(Duration::from_secs(10), || async {
        restarts().await >= Some(2)
    })
    .await;
    assert!(failing, "{}", server(&m, "fake").await);
    std::fs::write(&state, "").unwrap();
    reached(&m, "fake", State::Ready).await;
    let reset = until(Duration::from_secs(2), || async {
        restarts().await == Some(0)
    })
    .await;
    assert!(reset, "{}", server(&m, "fake").await);
    // After the 200 ms of `ready`.
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
    assert_eq!(restarts().await, Some(0), "{}", server(&m, "fake").await);
    m.shutdown().await;
}

#[tokio::test]
async fn logs_return_the_tail_and_stderr_events_fire() {
    let host = FakeHost::new();
    let m = ready(&host, json!({"FAKE_MCP_STDERR_BANNER": "hello from fake"})).await;
    assert_eq!(text(&host, "stderr", json!({"text": "second"})).await, "ok");
    let two = until(Duration::from_secs(2), || async {
        m.logs("fake").await.unwrap().lines.len() == 2
    })
    .await;
    let logs = m.logs("fake").await.unwrap();
    assert!(two, "{logs:?}");
    let texts: Vec<&str> = logs.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["hello from fake", "second"]);
    assert!(logs.lines[0].ts > 0, "{logs:?}");
    assert_eq!(logs.dropped, 0);
    let stderr = ServerEvent {
        server: "fake".into(),
        kind: EventKind::Stderr,
    };
    let fired = until(Duration::from_secs(2), || async {
        host.events().contains(&stderr)
    })
    .await;
    assert!(fired, "{:?}", host.events());
    // stderr output never changes the state.
    assert_eq!(server(&m, "fake").await["state"], "ready");
    assert_eq!(m.logs("nope").await.unwrap_err(), "no server named `nope`");
    m.shutdown().await;
}

#[tokio::test]
async fn a_changed_env_secret_restarts_the_process() {
    let host = FakeHost::new();
    host.set_secret("secret://FAKE_TOKEN", "v1");
    let m = ready(&host, json!({"API_TOKEN": "secret://FAKE_TOKEN"})).await;
    assert_eq!(text(&host, "env", json!({"name": "API_TOKEN"})).await, "v1");
    host.set_secret("secret://FAKE_TOKEN", "v2");
    let change = SecretChange {
        name: Some("FAKE_TOKEN".into()),
        reference: Some("secret://FAKE_TOKEN".into()),
        action: Some("set".into()),
    };
    m.on_secret_change(&change).await;
    let rotated = until(Duration::from_secs(10), || async {
        let out = host
            .call("mcp::fake::env", json!({"name": "API_TOKEN"}))
            .await;
        out.is_ok_and(|v| v["content"][0]["text"] == "v2")
    })
    .await;
    assert!(rotated, "{}", server(&m, "fake").await);
    m.shutdown().await;
}

#[tokio::test]
async fn a_secret_that_cannot_be_read_is_not_a_failed_start() {
    let host = FakeHost::new();
    host.set_secret("secret://FAKE_TOKEN", "v1");
    let raw = RawConfig::from_value(&cfg(json!({"API_TOKEN": "secret://FAKE_TOKEN"})));
    let m = Manager::new(host.clone(), opts(stdio_timings()));
    m.approve("fake", Some(&fingerprint(&raw.0["fake"])))
        .await
        .unwrap();
    // The approvals are loaded; every attempt now fails at the secret step.
    host.set_unreachable(true);
    m.apply_config(raw).await;
    // More attempts than the budget of 5 allows before a cooldown.
    let retried = until(Duration::from_secs(5), || async {
        host.resolve_count("secret://FAKE_TOKEN") >= 6
    })
    .await;
    let v = server(&m, "fake").await;
    assert!(retried, "{v}");
    assert_eq!(
        (&v["state"], &v["stage"], &v["error"]["code"]),
        (
            &json!("unavailable"),
            &json!("secret"),
            &json!("connect_failed")
        ),
        "{v}"
    );
    let p = &v["process"];
    assert_eq!(p["restarts"], 0, "{v}");
    assert!(p["started_at"].is_null() && p["pid"].is_null(), "{v}");
    assert!(p["next_restart_at"].is_null(), "{v}");
    let process = ServerEvent {
        server: "fake".into(),
        kind: EventKind::Process,
    };
    assert!(!host.events().contains(&process), "a process was spawned");
    m.shutdown().await;
}
