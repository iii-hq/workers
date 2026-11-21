//! A small MCP server over stdio for the mcp-client tests. Its behaviour comes
//! from `FAKE_MCP_*` environment variables, with the `KEY=VALUE` lines of
//! `FAKE_MCP_STATE_FILE` over them, read at each start.
//!
//! Every reply is MCP wire JSON turned into rmcp's type with serde, so the
//! fixture derives nothing (rmcp's `server` feature brings its own schemars).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, GetPromptRequestParams, GetPromptResponse,
    ListPromptsRequestMethod, ListPromptsResult, ListResourceTemplatesRequestMethod,
    ListResourceTemplatesResult, ListResourcesRequestMethod, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ReadResourceRequestMethod, ReadResourceRequestParams,
    ReadResourceResponse, ServerConfig,
};
use rmcp::service::{Peer, RequestContext};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

/// Process env, then each `KEY=VALUE` line of `FAKE_MCP_STATE_FILE` over it.
fn knobs() -> HashMap<String, String> {
    let mut knobs: HashMap<String, String> = std::env::vars().collect();
    let file = knobs.get("FAKE_MCP_STATE_FILE").cloned();
    if let Some(text) = file.and_then(|path| std::fs::read_to_string(path).ok()) {
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            knobs.insert(key.to_owned(), value.to_owned());
        }
    }
    knobs
}

fn wire<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("fixture JSON matches rmcp's type")
}

fn text_result(text: &str, is_error: bool) -> CallToolResponse {
    wire::<rmcp::model::CallToolResult>(
        json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
    )
    .into()
}

fn tool(name: &str, properties: Value) -> Value {
    json!({"name": name, "inputSchema": {"type": "object", "properties": properties}})
}

const RESOURCES: [(&str, &str, Option<&str>); 5] = [
    ("file:///a.txt", "a.txt", Some("text/plain")),
    ("file:///b.txt", "b.txt", Some("text/plain")),
    ("file:///img.png", "img.png", Some("image/png")),
    (
        "file:///bin.dat",
        "bin.dat",
        Some("application/octet-stream"),
    ),
    ("file:///pair", "pair", None),
];

/// Wire JSON, mutated by the `mutate` tool.
struct Lists {
    extra_tool: bool,
    prompts: Vec<Value>,
    template: bool,
}

struct Fake {
    knobs: HashMap<String, String>,
    lists: Mutex<Lists>,
}

impl Fake {
    fn new(knobs: HashMap<String, String>) -> Self {
        let template = knobs.get("FAKE_MCP_TEMPLATES").is_some_and(|v| v == "1");
        let prompts = vec![
            json!({"name": "code_review", "description": "Review code", "arguments": [
                {"name": "language", "description": "Language", "required": true},
                {"name": "focus"},
            ]}),
            json!({"name": "summary", "title": "Summarize"}),
            json!({"name": "bad.name", "description": "Bad"}),
        ];
        let lists = Mutex::new(Lists {
            extra_tool: false,
            prompts,
            template,
        });
        Fake { knobs, lists }
    }

    /// A flag knob is on only when its value is exactly `1`.
    fn on(&self, knob: &str) -> bool {
        self.knobs.get(knob).is_some_and(|v| v == "1")
    }

    fn num<T: FromStr>(&self, knob: &str) -> Option<T> {
        self.knobs.get(knob)?.parse().ok()
    }

    async fn mutate(&self, what: &str, peer: &Peer<RoleServer>) -> Result<(), McpError> {
        {
            let mut lists = self.lists.lock().unwrap();
            match what {
                "add_tool" => lists.extra_tool = true,
                "add_prompt" => lists
                    .prompts
                    .push(json!({"name": "extra_prompt", "description": "Extra"})),
                "change_prompt" => {
                    let review = lists
                        .prompts
                        .iter_mut()
                        .find(|p| p["name"] == "code_review");
                    review.unwrap()["arguments"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"name": "style"}));
                }
                "remove_prompt" => lists.prompts.retain(|p| p["name"] != "summary"),
                "add_template" => lists.template = true,
                other => {
                    return Err(McpError::invalid_params(
                        format!("unknown mutation: {other}"),
                        None,
                    ));
                }
            }
        }
        let sent = match what {
            "add_tool" => peer.notify_tool_list_changed().await,
            "add_template" => peer.notify_resource_list_changed().await,
            _ => peer.notify_prompt_list_changed().await,
        };
        sent.map_err(|e| McpError::internal_error(e.to_string(), None))
    }
}

impl ServerHandler for Fake {
    fn get_info(&self) -> ServerConfig {
        let mut capabilities = json!({"tools": {"listChanged": true}});
        if self.on("FAKE_MCP_RESOURCES") {
            capabilities["resources"] = json!({"listChanged": true});
        }
        if self.on("FAKE_MCP_PROMPTS") {
            capabilities["prompts"] = json!({"listChanged": true});
        }
        wire(json!({
            "protocolVersion": ProtocolVersion::default(),
            "capabilities": capabilities,
            "serverInfo": {"name": "fake-mcp-stdio", "version": "0.0.0"},
            "instructions": self.knobs.get("FAKE_MCP_INSTRUCTIONS"),
        }))
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let text = json!({"text": {"type": "string"}});
        let mut tools = vec![
            tool("echo", text.clone()),
            tool("env", json!({"name": {"type": "string"}})),
            tool("cwd", json!({})),
            tool("fail", json!({})),
            tool("big", json!({"bytes": {"type": "integer"}})),
            tool("stderr", text),
            tool("exit", json!({"code": {"type": "integer"}})),
            tool("mutate", json!({"what": {"type": "string"}})),
        ];
        if self.lists.lock().unwrap().extra_tool {
            tools.push(tool("extra", json!({})));
        }
        Ok(wire(json!({"tools": tools})))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        let arg = |key: &str| args[key].as_str().unwrap_or_default().to_owned();
        let text = match request.name.as_ref() {
            "echo" => arg("text"),
            "env" => std::env::var(arg("name")).unwrap_or_else(|_| "<unset>".into()),
            "cwd" => std::env::current_dir().unwrap().display().to_string(),
            "fail" => return Ok(text_result("boom", true)),
            "big" => "x".repeat(args["bytes"].as_u64().unwrap_or(0) as usize),
            "stderr" => {
                eprintln!("{}", arg("text"));
                "ok".into()
            }
            // Exits without answering: the client sees the transport close.
            "exit" => std::process::exit(args["code"].as_i64().unwrap_or(0) as i32),
            "mutate" => {
                self.mutate(&arg("what"), &context.peer).await?;
                "ok".into()
            }
            "extra" if self.lists.lock().unwrap().extra_tool => "extra".into(),
            other => {
                return Err(McpError::invalid_params(
                    format!("unknown tool: {other}"),
                    None,
                ));
            }
        };
        Ok(text_result(&text, false))
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        if !self.on("FAKE_MCP_RESOURCES") {
            return Err(McpError::method_not_found::<ListResourcesRequestMethod>());
        }
        let all: Vec<Value> = RESOURCES
            .iter()
            .map(|(uri, name, mime)| json!({"uri": uri, "name": name, "mimeType": mime}))
            .collect();
        let Some(page) = self.num::<usize>("FAKE_MCP_RESOURCE_PAGE") else {
            return Ok(wire(json!({"resources": all})));
        };
        // The cursor is the index of the next resource.
        let cursor = request.and_then(|r| r.cursor);
        let start = cursor
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
            .min(all.len());
        let end = (start + page).min(all.len());
        let mut out = json!({"resources": all[start..end]});
        if end < all.len() {
            out["nextCursor"] = json!(end.to_string());
        }
        Ok(wire(out))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        if !self.on("FAKE_MCP_RESOURCES") {
            return Err(McpError::method_not_found::<ReadResourceRequestMethod>());
        }
        let uri = request.uri.as_str();
        let contents = match uri {
            "file:///a.txt" => json!([{"uri": uri, "mimeType": "text/plain", "text": "alpha"}]),
            "file:///b.txt" => json!([{"uri": uri, "mimeType": "text/plain", "text": "beta"}]),
            "file:///img.png" => {
                json!([{"uri": uri, "mimeType": "image/png", "blob": "iVBORw0KGgo="}])
            }
            "file:///bin.dat" => {
                json!([{"uri": uri, "mimeType": "application/octet-stream", "blob": "AAECAw=="}])
            }
            "file:///pair" => json!([
                {"uri": "file:///pair/1", "text": "one"},
                {"uri": "file:///pair/2", "text": "two"},
            ]),
            _ => {
                let message = format!("resource not found: {uri}");
                return Err(McpError::resource_not_found(message, None));
            }
        };
        Ok(wire::<rmcp::model::ReadResourceResult>(json!({"contents": contents})).into())
    }

    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        if !self.lists.lock().unwrap().template {
            return Err(McpError::method_not_found::<
                ListResourceTemplatesRequestMethod,
            >());
        }
        Ok(wire(json!({"resourceTemplates": [
            {"uriTemplate": "file:///{name}.txt", "name": "text file"},
        ]})))
    }

    async fn list_prompts(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        if !self.on("FAKE_MCP_PROMPTS") {
            return Err(McpError::method_not_found::<ListPromptsRequestMethod>());
        }
        if self.on("FAKE_MCP_PROMPTS_FAIL") {
            return Err(McpError::internal_error("prompts unavailable", None));
        }
        Ok(wire(json!({"prompts": self.lists.lock().unwrap().prompts})))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        let result = match request.name.as_str() {
            "code_review" => {
                let language = args["language"].as_str().unwrap_or_default();
                let focus = args["focus"].as_str().unwrap_or("all");
                let text = format!("Review {language} code, focus: {focus}");
                json!({"description": "Review code", "messages": [
                    {"role": "user", "content": {"type": "text", "text": text}},
                ]})
            }
            "summary" => json!({"messages": [
                {"role": "user", "content":
                    {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"}},
                {"role": "assistant", "content": {"type": "resource", "resource":
                    {"uri": "file:///a.txt", "mimeType": "text/plain", "text": "alpha"}}},
            ]}),
            other => {
                return Err(McpError::invalid_params(
                    format!("unknown prompt: {other}"),
                    None,
                ));
            }
        };
        Ok(wire::<rmcp::model::GetPromptResult>(result).into())
    }
}

/// A pending read on tokio's stdin cannot be cancelled, so the fixture never
/// returns from `main`: it exits, or parks when told to ignore stdin EOF.
fn end(ignore_term: bool) -> ! {
    if ignore_term {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
    std::process::exit(0)
}

fn main() {
    let fake = Fake::new(knobs());
    // SAFETY: single-threaded until the exit timer and the runtime start below.
    unsafe { std::env::set_var("FAKE_MCP_PID", std::process::id().to_string()) };
    if fake.on("FAKE_MCP_GRANDCHILD") {
        let pid = std::process::Command::new("sleep")
            .arg("300")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sleep")
            .id();
        // SAFETY: still single-threaded.
        unsafe { std::env::set_var("FAKE_MCP_GRANDCHILD_PID", pid.to_string()) };
        eprintln!("grandchild {pid}");
    }
    let ignore_term = fake.on("FAKE_MCP_IGNORE_TERM");
    #[cfg(unix)]
    if ignore_term {
        // SAFETY: installs SIG_IGN, no handler code runs.
        unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
    }
    if let Some(banner) = fake.knobs.get("FAKE_MCP_STDERR_BANNER") {
        eprintln!("{banner}");
    }
    if let Some(banner) = fake.knobs.get("FAKE_MCP_STDOUT_BANNER") {
        println!("{banner}");
    }
    if let Some(ms) = fake.num::<u64>("FAKE_MCP_EXIT_AFTER_MS") {
        let code = fake.num("FAKE_MCP_EXIT_CODE").unwrap_or(1);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            std::process::exit(code)
        });
    }

    if fake.on("FAKE_MCP_NO_HANDSHAKE") {
        let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
        end(ignore_term);
    }
    // Never dropped: `end` does not return, so runtime shutdown never waits on stdin.
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let served = runtime.block_on(async {
        fake.serve(rmcp::transport::stdio())
            .await?
            .waiting()
            .await?;
        anyhow::Ok(())
    });
    if let Err(e) = served {
        eprintln!("fake-mcp-stdio: {e}");
    }
    end(ignore_term)
}
