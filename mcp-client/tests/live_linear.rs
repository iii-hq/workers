#![allow(dead_code)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use mcp_client::server::{Manager, State, Timings};
use serde_json::{Map, Value, json};
use support::{FakeHost, connect_with, server};

/// The variable's value. Unset or blank panics naming the variable, never a value.
fn need(var: &str) -> String {
    match std::env::var(var) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => panic!("{var} is not set"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Read,
    Write,
    Destructive,
}

/// The table of 5.6. The second value is true when the class was guessed from the name.
fn classify(name: &str, annotations: Option<&Value>) -> (Class, bool) {
    match annotations {
        Some(a) if a["readOnlyHint"] == true => (Class::Read, false),
        Some(a) if a["destructiveHint"] == false => (Class::Write, false),
        Some(_) => (Class::Destructive, false),
        None => {
            let prefixes = ["get_", "list_", "search_", "read_", "find_", "describe_"];
            let read = name == "whoami" || prefixes.iter().any(|p| name.starts_with(p));
            let class = if read {
                Class::Read
            } else {
                Class::Destructive
            };
            (class, true)
        }
    }
}

/// Connects `name` to `url` with the Bearer key from `LINEAR_API_KEY`, ready or panic.
async fn linear(host: &Arc<FakeHost>, name: &str, url: &str) -> Arc<Manager> {
    host.set_secret("env://LINEAR_API_KEY", &need("LINEAR_API_KEY"));
    let entry = json!({"type": "http", "url": url,
                       "auth": {"type": "bearer", "token": "env://LINEAR_API_KEY"}});
    // The production timings: Linear's slowest logged call takes 32 s.
    let manager = connect_with(
        host,
        json!({"mcpServers": {name: entry}}),
        Timings::default(),
    )
    .await;
    if !manager
        .wait_for(name, State::Ready, Duration::from_secs(60))
        .await
    {
        let r = server(&manager, name).await;
        panic!(
            "{name} is not ready: state={} stage={} error.code={} error.message={}",
            r["state"], r["stage"], r["error"]["code"], r["error"]["message"]
        );
    }
    manager
}

/// The JSON of the first text block of a tool answer.
fn parse_text(result: &Value) -> Option<Value> {
    let blocks = result["content"].as_array()?;
    let text = blocks.iter().find(|b| b["type"] == "text")?["text"].as_str()?;
    serde_json::from_str(text).ok()
}

fn text_json(tool: &str, result: &Value) -> Value {
    parse_text(result).unwrap_or_else(|| {
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        let head: String = text.chars().take(200).collect();
        panic!("{tool} answered without JSON text: {head}")
    })
}

/// A value that is only read, found by key at any depth.
fn first_str(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| map.values().find_map(|v| first_str(v, key))),
        Value::Array(items) => items.iter().find_map(|v| first_str(v, key)),
        _ => None,
    }
}

/// The answer's own object: its top level, or its one wrapper object (`{"issue": {..}}`).
fn own_obj(answer: &Value) -> Option<&Map<String, Value>> {
    let top = answer.as_object()?;
    match top.values().next() {
        Some(Value::Object(inner)) if top.len() == 1 => Some(inner),
        _ => Some(top),
    }
}

fn own_str(answer: &Value, key: &str) -> Option<String> {
    own_obj(answer)?.get(key)?.as_str().map(String::from)
}

/// The id of something this run created. Never searched deeper: sorted keys put
/// `assignee.id` or `parent.id` before the issue's own `id`.
fn own_id(answer: &Value) -> Option<String> {
    own_str(answer, "id")
}

const MCP_URL: &str = "https://mcp.linear.app/mcp";
const ISSUE_FIELDS: [&str; 4] = ["id", "title", "team", "teamId"];

fn http_client() -> Option<reqwest::Client> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .ok()
}

/// An `Authorization` value that `Debug` never shows.
fn secret_header(value: &str) -> Option<reqwest::header::HeaderValue> {
    let mut header = reqwest::header::HeaderValue::from_str(value).ok()?;
    header.set_sensitive(true);
    Some(header)
}

/// The payload of the first complete `data:` line of an SSE body.
fn sse_data(body: &str) -> Option<&str> {
    body.split_inclusive('\n')
        .find(|l| l.starts_with("data:") && l.ends_with('\n'))
        .map(|l| l["data:".len()..].trim())
}

fn has_content(result: &Result<Value, String>) -> bool {
    result
        .as_ref()
        .is_ok_and(|r| r["content"].as_array().is_some_and(|c| !c.is_empty()))
}

/// The JSON-RPC `initialize` sent straight to Linear, before any worker code runs.
async fn raw_probe() {
    let key = need("LINEAR_API_KEY");
    let auth = secret_header(&format!("Bearer {key}"))
        .expect("LINEAR_API_KEY is not a valid header value");
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"iii-mcp-client-live","version":"0"}}}"#;
    let mut resp = http_client()
        .expect("reqwest client")
        .post(MCP_URL)
        .header("Authorization", auth)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("raw probe failed: {e}"));
    let status = resp.status().as_u16();
    let present = |h: &str| match resp.headers().contains_key(h) {
        true => "present",
        false => "absent",
    };
    println!("LIVE raw_status={status}");
    println!("LIVE raw_www_authenticate={}", present("www-authenticate"));
    println!("LIVE raw_session_id={}", present("mcp-session-id"));
    assert_ne!(
        status, 401,
        "LINEAR_API_KEY was rejected as Bearer on /mcp (HTTP 401)"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .unwrap_or_else(|e| panic!("raw probe body: {e}"))
    {
        bytes.extend_from_slice(&chunk);
        // An SSE stream may stay open after the answer.
        if sse_data(&String::from_utf8_lossy(&bytes)).is_some() {
            break;
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    let answer: Value = serde_json::from_str(&text)
        .ok()
        .or_else(|| sse_data(&text).and_then(|d| serde_json::from_str(d).ok()))
        .unwrap_or_else(|| panic!("raw probe answer is not JSON-RPC (HTTP {status})"));
    let version = first_str(&answer, "protocolVersion");
    println!(
        "LIVE raw_protocol_version={}",
        version.as_deref().unwrap_or("absent")
    );
    let list_changed = match &answer["result"]["capabilities"]["tools"]["listChanged"] {
        Value::Bool(b) => b.to_string(),
        _ => "absent".into(),
    };
    println!("LIVE listChanged={list_changed}");
}

fn names_or_none(names: &[String]) -> String {
    match names.is_empty() {
        true => "none".into(),
        false => names.join(","),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn live_bearer_read() {
    raw_probe().await;

    let host = FakeHost::new();
    let manager = linear(&host, "linear", MCP_URL).await;
    let row = server(&manager, "linear").await;
    let tools = row["tools"].as_array().cloned().unwrap_or_default();
    let skipped = row["skipped"].as_array().cloned().unwrap_or_default();
    let version = row["protocol_version"].as_str();
    println!("LIVE protocol_version={}", version.unwrap_or("none"));
    println!("LIVE tools={}", tools.len());
    let skipped_entries: Vec<String> = skipped
        .iter()
        .map(|s| {
            format!(
                "{}:{}",
                s["kind"].as_str().unwrap_or("?"),
                s["name"].as_str().unwrap_or("?")
            )
        })
        .collect();
    println!("LIVE skipped={}", names_or_none(&skipped_entries));
    println!("LIVE resources={}", row["capabilities"]["resources"]);
    println!("LIVE prompts={}", row["capabilities"]["prompts"]);
    assert!(version.is_some(), "no protocol version");
    assert!(tools.len() >= 60, "only {} tools", tools.len());

    let name = |t: &Value| t["name"].as_str().unwrap_or_default().to_string();
    let failing: Vec<String> = tools
        .iter()
        .chain(&skipped)
        .map(name)
        .filter(|n| mcp_client::reconcile::check_name("mcp__linear__", n).is_err())
        .collect();
    println!("LIVE names_failing={}", names_or_none(&failing));

    let schemas: Vec<String> = tools
        .iter()
        .filter_map(|t| host.def(t["function_id"].as_str()?))
        .map(|d| d.request_format.to_string())
        .collect();
    let with = |needle: &str| schemas.iter().filter(|s| s.contains(needle)).count();
    println!("LIVE schemas_with_ref={}", with("\"$ref\""));
    println!("LIVE schemas_with_defs={}", with("\"$defs\""));

    let annotations = |t: &Value| Some(t["annotations"].clone()).filter(|a| !a.is_null());
    let annotated = tools.iter().filter_map(annotations).count();
    let classes: Vec<(String, (Class, bool))> = tools
        .iter()
        .map(|t| (name(t), classify(&name(t), annotations(t).as_ref())))
        .collect();
    let count = |c: Class| classes.iter().filter(|(_, (k, _))| *k == c).count();
    let guessed: Vec<String> = classes
        .iter()
        .filter(|(_, (_, g))| *g)
        .map(|(n, _)| n.clone())
        .collect();
    println!("LIVE annotated={annotated}/{}", tools.len());
    println!("LIVE class_read={}", count(Class::Read));
    println!("LIVE class_write={}", count(Class::Write));
    println!("LIVE class_destructive={}", count(Class::Destructive));
    println!("LIVE guessed_from_name={}", names_or_none(&guessed));

    // With an API key Linear lists no `whoami`; `get_user` with `me` is the identity
    // read. It names the user: its answer and error are never shown.
    let me = host
        .call("mcp::linear::get_user", json!({"query": "me"}))
        .await;
    let whoami = has_content(&me);
    println!("LIVE whoami={}", if whoami { "ok" } else { "failed" });
    let teams = host
        .call("mcp::linear::list_teams", json!({"limit": 1}))
        .await;
    let teams = has_content(&teams);
    println!("LIVE list_teams={}", if teams { "ok" } else { "failed" });
    assert!(whoami, "whoami did not succeed");
    assert!(teams, "list_teams did not succeed");

    let bad = host.call("mcp::linear::get_issue", json!({})).await;
    let channel = match &bad {
        Ok(_) => "accepted",
        Err(e) if e.starts_with("invalid_argument: ") => "json_rpc_32602",
        Err(e) if e.starts_with("MCP error ") => "json_rpc_other",
        Err(_) => "is_error",
    };
    println!("LIVE bad_argument={channel}");
    assert!(bad.is_err(), "get_issue without an id succeeded");

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/live");
    std::fs::create_dir_all(&dir).unwrap();
    for (tool, file) in [
        ("save_issue", "linear-save-issue.schema.json"),
        ("list_issues", "linear-list-issues.schema.json"),
    ] {
        let schema = host
            .def(&format!("mcp::linear::{tool}"))
            .unwrap_or_else(|| panic!("{tool} is not registered"))
            .request_format;
        let text = serde_json::to_string_pretty(&schema).unwrap();
        std::fs::write(dir.join(file), text + "\n").unwrap();
        let properties = schema["properties"].as_object().map_or(0, Map::len);
        println!("LIVE {tool}_properties={properties}");
    }
    println!("LIVE schema_saved=mcp-client/target/live");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn live_bearer_readonly_endpoint() {
    let host = FakeHost::new();
    let manager = linear(&host, "linear-ro", "https://mcp.linear.app/mcp/readonly").await;
    let row = server(&manager, "linear-ro").await;
    let names: Vec<String> = row["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect();
    let write_like: Vec<String> = names
        .iter()
        .filter(|n| {
            ["save_", "create_", "update_", "delete_"]
                .iter()
                .any(|p| n.starts_with(p))
        })
        .cloned()
        .collect();
    println!("LIVE readonly_tools={}", names.len());
    println!("LIVE readonly_write_like={}", names_or_none(&write_like));
    let saves: Vec<&String> = names.iter().filter(|n| n.starts_with("save_")).collect();
    assert!(saves.is_empty(), "the read-only endpoint lists {saves:?}");
}

/// Every write of the suite goes through here: only into the team whose key
/// equals `LINEAR_TEST_TEAM`, and only on what this run created. No `Debug`:
/// it holds the API key.
struct WriteGuard {
    host: Arc<FakeHost>,
    server: String,
    team_id: String,
    team_key: String,
    /// The title `create_issue` used; clean-up checks it before deleting.
    title: String,
    issue: Option<String>,
    comment: Option<String>,
    api_key: Option<String>,
}

/// Every object in `value`, at any depth, whose string `key` equals `want`.
fn with_key<'a>(value: &'a Value, want: &str, out: &mut Vec<&'a Map<String, Value>>) {
    match value {
        Value::Object(map) => {
            if map.get("key").and_then(Value::as_str) == Some(want) {
                out.push(map);
            }
            map.values().for_each(|v| with_key(v, want, out));
        }
        Value::Array(items) => items.iter().for_each(|v| with_key(v, want, out)),
        _ => {}
    }
}

async fn issue_delete(key: &str, id: &str) -> bool {
    let attempt = async {
        // A personal API key goes without `Bearer`.
        let auth = secret_header(key)?;
        let body = json!({
            "query": "mutation($id: String!) { issueDelete(id: $id) { success } }",
            "variables": {"id": id}
        });
        let resp = http_client()?
            .post("https://api.linear.app/graphql")
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .ok()?;
        if resp.status() != 200 {
            return None;
        }
        let answer: Value = serde_json::from_str(&resp.text().await.ok()?).ok()?;
        Some(answer["data"]["issueDelete"]["success"] == true)
    };
    attempt.await == Some(true)
}

impl WriteGuard {
    async fn new(host: &Arc<FakeHost>, server: &str) -> WriteGuard {
        let team_key = need("LINEAR_TEST_TEAM");
        let id = format!("mcp::{server}::list_teams");
        let result = host
            .call(&id, json!({"limit": 250}))
            .await
            .unwrap_or_else(|e| panic!("list_teams failed: {e}"));
        let answer = text_json("list_teams", &result);
        let mut teams = Vec::new();
        with_key(&answer, &team_key, &mut teams);
        assert_eq!(
            teams.len(),
            1,
            "expected exactly one team whose key equals LINEAR_TEST_TEAM, found {}",
            teams.len()
        );
        let team_id = teams[0]
            .get("id")
            .and_then(Value::as_str)
            .expect("the test team has no id")
            .to_string();
        WriteGuard {
            host: host.clone(),
            server: server.into(),
            team_id,
            team_key,
            title: String::new(),
            issue: None,
            comment: None,
            api_key: std::env::var("LINEAR_API_KEY").ok(),
        }
    }

    async fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        let id = format!("mcp::{}::{tool}", self.server);
        self.host.call(&id, args).await
    }

    /// `get_issue`'s answer when it carries `self.title`.
    async fn read_own(&self, id: &str) -> Option<Value> {
        let args = json!({"id": id, "fields": ISSUE_FIELDS});
        let answer = parse_text(&self.call("get_issue", args).await.ok()?)?;
        (own_str(&answer, "title")? == self.title).then_some(answer)
    }

    async fn create_issue(&mut self) -> String {
        use std::hash::{BuildHasher, RandomState};
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now();
        let secs = now.duration_since(UNIX_EPOCH).unwrap().as_secs();
        let random = RandomState::new().hash_one(now) & 0xff_ffff;
        let title = format!("mcp-client live test {secs} {random:06x}");
        self.title = title.clone();

        let args = json!({"title": title, "team": self.team_id});
        let created = self.call("save_issue", args).await;
        let Some(id) = created.ok().and_then(|r| own_id(&parse_text(&r)?)) else {
            println!("LIVE leftover_title={title}");
            panic!("save_issue gave no issue id");
        };
        let Some(answer) = self.read_own(&id).await else {
            println!("LIVE leftover_title={title}");
            panic!("get_issue does not show this run's title for the new issue");
        };
        self.issue = Some(id.clone());

        let own = own_obj(&answer).unwrap();
        let ours = |v: &Value| {
            v.as_str()
                .is_some_and(|s| s == self.team_id || s == self.team_key)
        };
        let in_team = own.get("teamId").and_then(Value::as_str) == Some(self.team_id.as_str())
            || own
                .get("team")
                .is_some_and(|t| ours(t) || ["id", "key", "name"].iter().any(|k| ours(&t[*k])));
        assert!(in_team, "issue {id} is not in the test team");
        id
    }

    async fn comment(&mut self, issue: &str) -> String {
        assert_eq!(
            self.issue.as_deref(),
            Some(issue),
            "a comment only goes on the issue this guard created"
        );
        let args = json!({"issueId": issue, "body": "mcp-client live test comment"});
        let result = self
            .call("save_comment", args)
            .await
            .unwrap_or_else(|e| panic!("save_comment failed: {e}"));
        let id = own_id(&text_json("save_comment", &result)).expect("save_comment gave no id");
        self.comment = Some(id.clone());
        id
    }

    async fn delete_comment(&mut self) {
        let id = self.comment.clone().expect("no comment to delete");
        self.call("delete_comment", json!({"id": id}))
            .await
            .unwrap_or_else(|e| panic!("delete_comment failed: {e}"));
        self.comment = None;
    }

    /// Never panics.
    async fn clean_up(&mut self) {
        if let Some(comment) = self.comment.take() {
            let _ = self.call("delete_comment", json!({"id": comment})).await;
        }
        let Some(id) = self.issue.take() else { return };
        if self.read_own(&id).await.is_none() {
            println!("LIVE leftover={id} title differs, check it by hand");
            return;
        }
        let deleted = match &self.api_key {
            Some(key) => issue_delete(key, &id).await,
            None => false,
        };
        if deleted {
            return;
        }
        let cancel = json!({"id": id, "state": "canceled"});
        match self.call("save_issue", cancel).await {
            Ok(_) => println!("LIVE leftover={id} canceled, delete it by hand"),
            Err(_) => println!("LIVE leftover={id} still open, delete it by hand"),
        }
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        if self.issue.is_none() && self.comment.is_none() {
            return;
        }
        // The MCP session lives on the multi-thread runtime.
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(self.clean_up()));
    }
}

/// Whether `list_comments` of `issue` names the comment `id`.
async fn lists(host: &FakeHost, issue: &str, id: &str) -> bool {
    let result = host
        .call("mcp::linear::list_comments", json!({"issueId": issue}))
        .await
        .unwrap_or_else(|e| panic!("list_comments failed: {e}"));
    let answer = text_json("list_comments", &result);
    answer.to_string().contains(&format!("\"{id}\""))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn live_write_in_test_team() {
    need("LINEAR_TEST_TEAM");
    let host = FakeHost::new();
    let _manager = linear(&host, "linear", MCP_URL).await;
    let mut guard = WriteGuard::new(&host, "linear").await;
    let issue = guard.create_issue().await;
    println!("LIVE issue_created={issue}");
    let comment = guard.comment(&issue).await;

    assert!(
        lists(&host, &issue, &comment).await,
        "list_comments does not show the new comment"
    );
    guard.delete_comment().await;
    assert!(
        !lists(&host, &issue, &comment).await,
        "the comment is still listed after delete_comment"
    );
    guard.clean_up().await;
    println!("LIVE clean_up=done");
}

#[test]
#[should_panic(expected = "MCP_CLIENT_LIVE_VARIABLE_THAT_IS_NEVER_SET is not set")]
fn need_names_the_missing_variable() {
    need("MCP_CLIENT_LIVE_VARIABLE_THAT_IS_NEVER_SET");
}

#[test]
fn classify_follows_the_table() {
    assert_eq!(
        classify("save_issue", Some(&json!({ "readOnlyHint": true }))),
        (Class::Read, false)
    );
    assert_eq!(
        classify("save_issue", Some(&json!({ "destructiveHint": false }))),
        (Class::Write, false)
    );
    assert_eq!(
        classify("list_issues", Some(&json!({ "readOnlyHint": false }))),
        (Class::Destructive, false)
    );
    assert_eq!(
        classify("save_issue", Some(&json!({ "destructiveHint": true }))),
        (Class::Destructive, false)
    );
    for name in [
        "get_issue",
        "list_teams",
        "search_documentation",
        "read_x",
        "find_x",
        "describe_x",
        "whoami",
    ] {
        assert_eq!(classify(name, None), (Class::Read, true), "{name}");
    }
    assert_eq!(classify("save_issue", None), (Class::Destructive, true));
    assert_eq!(classify("getaway", None), (Class::Destructive, true));
}

#[test]
fn own_id_reads_the_top_level_only() {
    let issue = json!({
        "assignee": { "id": "user-1" },
        "createdBy": { "id": "user-2" },
        "id": "issue-1",
        "parent": { "id": "issue-0" },
        "title": "t",
        "uuid": "u-1"
    });
    assert_eq!(own_id(&issue), Some("issue-1".into()));
    assert_eq!(own_id(&json!({ "issue": issue })), Some("issue-1".into()));
    let no_id = json!({
        "assignee": { "id": "user-1" },
        "createdBy": { "id": "user-2" },
        "parent": { "id": "issue-0" },
        "uuid": "u-1"
    });
    assert_eq!(own_id(&no_id), None);
    assert_eq!(own_id(&json!({ "issue": no_id })), None);
    assert_eq!(own_id(&json!({ "data": { "issue": issue } })), None);
}
