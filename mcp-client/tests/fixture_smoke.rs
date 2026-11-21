#![cfg(feature = "test-fixtures")]
//! Proves the fake stdio MCP server with rmcp's client alone, over the same
//! `(ChildStdout, ChildStdin)` pair the worker serves.

use std::process::Stdio;
use std::time::Duration;

use rmcp::model::ReadResourceRequestParams;
use rmcp::service::{NotificationContext, RunningService};
use rmcp::{ClientHandler, RoleClient, ServiceError, ServiceExt};
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::timeout;

const BIN: &str = env!("CARGO_BIN_EXE_fake-mcp-stdio");

/// Forwards "tools" | "prompts" | "resources" for each list_changed notification.
struct Seen(UnboundedSender<&'static str>);

impl ClientHandler for Seen {
    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
        let _ = self.0.send("tools");
    }
    async fn on_prompt_list_changed(&self, _: NotificationContext<RoleClient>) {
        let _ = self.0.send("prompts");
    }
    async fn on_resource_list_changed(&self, _: NotificationContext<RoleClient>) {
        let _ = self.0.send("resources");
    }
}

struct Fake {
    client: RunningService<RoleClient, Seen>,
    child: tokio::process::Child,
    seen: UnboundedReceiver<&'static str>,
}

fn command(env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

async fn fake(env: &[(&str, &str)]) -> Fake {
    let mut child = command(env).spawn().expect("spawn fake-mcp-stdio");
    let io = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let (tx, seen) = unbounded_channel();
    let client = Seen(tx).serve(io).await.expect("handshake");
    Fake {
        client,
        child,
        seen,
    }
}

/// The wire JSON of the `CallToolResult`.
async fn call(f: &Fake, tool: &str, args: Value) -> Result<Value, ServiceError> {
    let params = serde_json::from_value(json!({"name": tool, "arguments": args})).unwrap();
    Ok(serde_json::to_value(f.client.call_tool(params).await?).unwrap())
}

fn text(result: Value) -> String {
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

fn mcp_error(result: Result<impl std::fmt::Debug, ServiceError>) -> (i32, String) {
    match result {
        Err(ServiceError::McpError(e)) => (e.code.0, e.message.to_string()),
        other => panic!("expected an MCP error, got {other:?}"),
    }
}

#[tokio::test]
async fn lists_the_eight_tools_and_echoes() {
    let f = fake(&[]).await;
    let mut names: Vec<String> = f
        .client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "big", "cwd", "echo", "env", "exit", "fail", "mutate", "stderr"
        ]
    );
    assert_eq!(
        text(call(&f, "echo", json!({"text": "hi"})).await.unwrap()),
        "hi"
    );
    assert_eq!(call(&f, "fail", json!({})).await.unwrap()["isError"], true);
    assert_eq!(
        text(call(&f, "big", json!({"bytes": 5})).await.unwrap()),
        "xxxxx"
    );
    assert_eq!(
        text(call(&f, "env", json!({"name": "NOPE"})).await.unwrap()),
        "<unset>"
    );
    assert_eq!(
        text(
            call(&f, "env", json!({"name": "FAKE_MCP_PID"}))
                .await
                .unwrap()
        ),
        f.child.id().unwrap().to_string()
    );
}

#[tokio::test]
async fn state_file_overrides_env_at_each_start() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        "FAKE_MCP_INSTRUCTIONS=from-file\nFAKE_MCP_PROMPTS=0\n",
    )
    .unwrap();
    let path = file.path().to_str().unwrap();
    let f = fake(&[
        ("FAKE_MCP_INSTRUCTIONS", "from-env"),
        ("FAKE_MCP_PROMPTS", "1"),
        ("FAKE_MCP_STATE_FILE", path),
    ])
    .await;
    let info = serde_json::to_value(&*f.client.peer_info().unwrap()).unwrap();
    assert_eq!(info["instructions"], "from-file");
    assert!(info["capabilities"].get("prompts").is_none(), "{info}");
}

#[tokio::test]
async fn mutate_sends_the_matching_list_changed() {
    let mut f = fake(&[("FAKE_MCP_PROMPTS", "1"), ("FAKE_MCP_RESOURCES", "1")]).await;
    for (what, kind) in [
        ("add_tool", "tools"),
        ("add_prompt", "prompts"),
        ("change_prompt", "prompts"),
        ("remove_prompt", "prompts"),
        ("add_template", "resources"),
    ] {
        let reply = call(&f, "mutate", json!({"what": what})).await.unwrap();
        assert_eq!(text(reply), "ok");
        let got = timeout(Duration::from_secs(2), f.seen.recv()).await;
        assert_eq!(got.expect(what), Some(kind), "{what}");
    }
    let names: Vec<String> = f
        .client
        .list_all_prompts()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["code_review", "bad.name", "extra_prompt"]);
}

#[tokio::test]
async fn resources_page_templates_and_prompt_failure() {
    let f = fake(&[
        ("FAKE_MCP_RESOURCES", "1"),
        ("FAKE_MCP_RESOURCE_PAGE", "2"),
        ("FAKE_MCP_PROMPTS", "1"),
        ("FAKE_MCP_PROMPTS_FAIL", "1"),
    ])
    .await;
    let page = f.client.list_resources(None).await.unwrap();
    assert_eq!(page.resources.len(), 2);
    assert_eq!(serde_json::to_value(&page).unwrap()["nextCursor"], "2");
    let pair = f
        .client
        .read_resource(ReadResourceRequestParams::new("file:///pair"))
        .await
        .unwrap();
    assert_eq!(pair.contents.len(), 2);
    assert_eq!(
        mcp_error(f.client.list_resource_templates(None).await).0,
        -32601
    );
    assert_eq!(
        mcp_error(
            f.client
                .read_resource(ReadResourceRequestParams::new("file:///nope"))
                .await
        ),
        (-32002, "resource not found: file:///nope".to_owned())
    );
    assert!(f.client.list_prompts(None).await.is_err());
}

#[tokio::test]
async fn exit_paths_never_wait_on_stdin() {
    let mut f = fake(&[]).await;
    assert!(call(&f, "exit", json!({"code": 7})).await.is_err());
    assert_eq!(f.child.wait().await.unwrap().code(), Some(7));

    // Not served: stdin stays open, so a cold start slower than 50 ms cannot fail a handshake.
    let mut child = command(&[
        ("FAKE_MCP_EXIT_AFTER_MS", "50"),
        ("FAKE_MCP_EXIT_CODE", "4"),
    ])
    .spawn()
    .unwrap();
    // Held outside the Child: tokio's `wait` closes a stdin it still owns.
    let _stdin = child.stdin.take();
    let status = timeout(Duration::from_secs(5), child.wait()).await;
    assert_eq!(status.unwrap().unwrap().code(), Some(4));

    let Fake {
        client, mut child, ..
    } = fake(&[]).await;
    client.cancel().await.unwrap();
    let status = timeout(Duration::from_secs(5), child.wait()).await;
    assert_eq!(status.unwrap().unwrap().code(), Some(0));
}

#[cfg(unix)]
#[tokio::test]
async fn ignore_term_survives_stdin_eof_and_sigterm() {
    let Fake {
        client, mut child, ..
    } = fake(&[("FAKE_MCP_IGNORE_TERM", "1")]).await;
    let pid = child.id().unwrap() as libc::pid_t;
    client.cancel().await.unwrap();
    // SAFETY: plain kill(2) on our own child's pid.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "still running after EOF and SIGTERM"
    );
    child.kill().await.unwrap();
}
