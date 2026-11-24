//! Sidechats over an isolated SDK transport: a fake engine with an
//! in-memory session-manager (fork, patch-meta, list, transcript) and state.
//! No live engine, no model.

use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message as Frame;

use crate::functions::send;
use crate::types::turn::{ParentLink, SkillContext};

const MAIN: &str = "s_main";
const TIMEOUT: u64 = 2_000;

#[derive(Default)]
struct Data {
    state: BTreeMap<(String, String), Value>,
    sessions: BTreeMap<String, Value>,
    entries: BTreeMap<String, Vec<Value>>,
    calls: Vec<(String, Value)>,
    clock: i64,
    forks: u64,
}

impl Data {
    fn get(&self, scope: &str, key: &str) -> Value {
        self.state
            .get(&(scope.into(), key.into()))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn tick(&mut self) -> i64 {
        self.clock += 1_000;
        self.clock
    }

    fn rpc(
        &mut self,
        function: &str,
        p: &Value,
        action: &Value,
    ) -> Result<Value, (String, String)> {
        self.calls.push((function.into(), p.clone()));
        if action["type"] == "enqueue" {
            return Ok(json!({ "message_receipt_id": "receipt" }));
        }
        let scope = p["scope"].as_str().unwrap_or_default().to_string();
        let key = p["key"].as_str().unwrap_or_default().to_string();
        let sid = p["session_id"].as_str().unwrap_or_default().to_string();
        Ok(match function {
            "state::get" | "harness::state::get" => self.get(&scope, &key),
            "state::set" | "harness::state::set" => {
                self.state.insert((scope, key), p["value"].clone());
                json!({})
            }
            "state::delete" | "harness::state::delete" => {
                self.state.remove(&(scope, key));
                json!({})
            }
            "state::list" | "harness::state::list" => Value::Array(
                self.state
                    .iter()
                    .filter(|((s, _), _)| *s == scope)
                    .map(|(_, v)| v.clone())
                    .collect(),
            ),
            "state::list_keys" => json!({ "keys": self
                .state
                .keys()
                .filter(|(s, _)| *s == scope)
                .map(|(_, k)| k.clone())
                .collect::<Vec<_>>() }),
            "harness::state::compare-and-set" => {
                let old = self.get(&scope, &key);
                let swapped = old == p["expected"];
                if swapped {
                    self.state.insert((scope, key), p["value"].clone());
                }
                json!({ "swapped": swapped, "current": old })
            }
            "state::claim-namespace" => json!({ "claimed": true }),
            "engine::functions::list" => json!({ "functions": [] }),
            "engine::workers::register" => json!({ "success": true }),
            "session::get" => self
                .sessions
                .get(&sid)
                .map(|meta| json!({ "meta": meta }))
                .unwrap_or(Value::Null),
            "session::ensure" => {
                let meta = self.sessions.get(&sid).cloned().ok_or((
                    "session/not_found".to_string(),
                    format!("ensure must not create {sid}"),
                ))?;
                json!({ "created": false, "meta": meta })
            }
            "session::list" => {
                let filter = p["metadata"].as_object().cloned().unwrap_or_default();
                let sessions: Vec<Value> = self
                    .sessions
                    .values()
                    .filter(|s| filter.iter().all(|(k, v)| s["metadata"].get(k) == Some(v)))
                    .cloned()
                    .collect();
                json!({ "sessions": sessions })
            }
            "session::patch-meta" => {
                let now = self.tick();
                let meta = self
                    .sessions
                    .get_mut(&sid)
                    .ok_or(("session/not_found".to_string(), sid.clone()))?;
                let metadata = meta["metadata"].as_object_mut().unwrap();
                for key in p["unset"].as_array().into_iter().flatten() {
                    metadata.remove(key.as_str().unwrap());
                }
                for (key, value) in p["set"].as_object().into_iter().flatten() {
                    metadata.insert(key.clone(), value.clone());
                }
                for field in ["title", "description"] {
                    if let Some(value) = p.get(field) {
                        meta[field] = value.clone();
                    }
                }
                if p["touch"] != json!(false) {
                    meta["updated_at"] = json!(now);
                }
                json!({ "meta": meta })
            }
            "session::fork" => {
                let entry = p["entry_id"].as_str().unwrap_or_default();
                let source = self.entries.get(&sid).cloned().unwrap_or_default();
                let Some(at) = source.iter().position(|e| e["entry_id"] == entry) else {
                    return Err((
                        "session/entry_not_found".into(),
                        format!("entry {entry} does not exist in session {sid}"),
                    ));
                };
                self.forks += 1;
                let new_id = format!("s_fork{}", self.forks);
                let copies: Vec<Value> = source[..=at]
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let mut copy = e.clone();
                        copy["entry_id"] = json!(format!("e_{new_id}_{i}"));
                        copy
                    })
                    .collect();
                let now = self.tick();
                let mut meta = self.sessions[&sid].clone();
                meta["session_id"] = json!(new_id);
                meta["title"] = p.get("title").cloned().unwrap_or(meta["title"].clone());
                meta["forked_from"] = json!(sid);
                meta["status"] = json!("idle");
                meta["created_at"] = json!(now);
                meta["updated_at"] = json!(now);
                meta["message_count"] =
                    json!(copies.iter().filter(|e| !e["message"].is_null()).count());
                self.entries.insert(new_id.clone(), copies);
                self.sessions.insert(new_id.clone(), meta.clone());
                json!({ "session_id": new_id, "meta": meta })
            }
            "session::messages-tail" => {
                let limit = p["limit"].as_u64().unwrap_or(20) as usize;
                let spoken: Vec<Value> = self
                    .entries
                    .get(&sid)
                    .into_iter()
                    .flatten()
                    .filter(|e| !e["message"].is_null())
                    .cloned()
                    .collect();
                let skip = spoken.len().saturating_sub(limit);
                json!({ "messages": spoken[skip..] })
            }
            "session::messages" => {
                json!({ "messages": self.entries.get(&sid).cloned().unwrap_or_default() })
            }
            "session::append" => {
                let now = self.tick();
                let entries = self.entries.entry(sid.clone()).or_default();
                let id = p["entry_id"].clone();
                if !entries.iter().any(|e| e["entry_id"] == id) {
                    entries.push(json!({ "entry_id": id, "message": p["message"] }));
                    if let Some(meta) = self.sessions.get_mut(&sid) {
                        let count = meta["message_count"].as_u64().unwrap_or(0);
                        meta["message_count"] = json!(count + 1);
                        meta["updated_at"] = json!(now);
                    }
                }
                json!({ "entry_id": id })
            }
            "session::delete" => json!({ "deleted": self.sessions.remove(&sid).is_some() }),
            _ => Value::Null,
        })
    }
}

struct Fake {
    deps: Deps,
    data: Arc<Mutex<Data>>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}

fn user(text: &str) -> Value {
    serde_json::to_value(AgentMessage::user_text(text)).unwrap()
}

fn assistant(text: &str) -> Value {
    json!({ "role": "assistant", "content": [{ "type": "text", "text": text }],
            "stop_reason": "end", "model": "claude-opus-5-5", "provider": "anthropic",
            "timestamp": 1 })
}

/// The conversation's last turn: a model, effort, policy, skills, folder and
/// prompt of its own, a budget, a parent link and work in flight — what a
/// side chat must copy and what it must not.
fn main_record() -> TurnRecord {
    let mut record = crate::types::turn::tests::record();
    record.session_id = MAIN.into();
    record.turn_id = "t_main".into();
    record.status = TurnStatus::Completed;
    record.step = 3;
    record.options.model = "claude-opus-5-5".into();
    record.options.provider = Some("anthropic".into());
    record.options.thinking_level = Some(ThinkingLevel::High);
    record.options.provider_options = Some(BTreeMap::from([(
        "anthropic".to_string(),
        json!({ "reasoning_effort": "high", "account": "work" }),
    )]));
    record.options.functions = Some(FunctionPolicy {
        allow: vec!["*".into()],
        ..FunctionPolicy::default()
    });
    record.options.skill_context = Some(SkillContext {
        filter: Some(vec!["iii/state".into()]),
        baseline: Some("<skills>iii/state</skills>".into()),
        baseline_ref: None,
    });
    record.options.system_prompt = Some("Frozen system prompt of the conversation".into());
    record.options.metadata = Some(json!({ "fs_scope": { "root": "/w/web" }, "trace": "x" }));
    record.options.budget_root_session_id = Some(MAIN.into());
    record.options.max_total_tokens = Some(100_000);
    record.skills_started = true;
    record.parent = Some(ParentLink {
        session_id: "s_root".into(),
        turn_id: "t_root".into(),
        function_call_id: "fc_1".into(),
    });
    record
}

fn meta(id: &str, title: &str, metadata: Value, message_count: u64) -> Value {
    json!({ "session_id": id, "title": title, "description": "Webhook retries",
            "status": "done", "metadata": metadata, "message_count": message_count,
            "created_at": 100, "updated_at": 500 })
}

impl Fake {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let data = Arc::new(Mutex::new(Data {
            clock: 10_000,
            ..Data::default()
        }));
        let shared = data.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let hello = json!({ "type": "workerregistered", "worker_id": "side-chat-offline",
                "features": ["observe.v1", "observe.v2", "local-dispatch.v1"] });
            socket
                .send(Frame::Text(hello.to_string().into()))
                .await
                .unwrap();
            let _ = ready.send(());
            let mut revision = 0_u64;
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(text) = frame.to_text() else { continue };
                let Ok(request) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                let reply = if request["type"] == "registerfunction" {
                    revision += 1;
                    json!({ "type": "functionregistered", "id": request["id"],
                        "namespace": "default", "revision": revision,
                        "local_dispatch": true, "observers": [] })
                } else if request["type"] == "invokefunction" {
                    let function = request["function_id"].as_str().unwrap_or_default();
                    let result =
                        shared
                            .lock()
                            .unwrap()
                            .rpc(function, &request["data"], &request["action"]);
                    if request["invocation_id"].is_null() {
                        continue;
                    }
                    let mut reply = json!({ "type": "invocationresult",
                        "invocation_id": request["invocation_id"], "function_id": function });
                    match result {
                        Ok(value) => reply["result"] = value,
                        Err((code, message)) => {
                            reply["error"] = json!({ "code": code, "message": message })
                        }
                    }
                    reply
                } else {
                    continue;
                };
                if socket
                    .send(Frame::Text(reply.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let iii = Arc::new(iii_sdk::register_worker(
            &url,
            iii_sdk::InitOptions::default(),
        ));
        wait.await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cfg = crate::config::WorkerConfig {
            session_timeout_ms: TIMEOUT,
            folders_file_path: dir.path().join("folders.json").to_string_lossy().into(),
            projects_file_path: dir.path().join("projects.json").to_string_lossy().into(),
            ..crate::config::WorkerConfig::default()
        };
        let deps = Deps::new(
            iii.clone(),
            Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
            crate::discovery::new_cell(),
            crate::skills::new_cell(),
            crate::events::TurnEvents::register(&iii),
            crate::hooks::HookRegistry::register(&iii),
        );
        Self {
            deps,
            data,
            server,
            _dir: dir,
        }
    }

    /// The conversation: four messages, its turn record, a project, a name
    /// and a description the user wrote.
    async fn with_conversation() -> Self {
        let fake = Self::new().await;
        {
            let mut data = fake.data.lock().unwrap();
            data.sessions.insert(
                MAIN.into(),
                meta(
                    MAIN,
                    "Webhook retries",
                    json!({ "project_id": "prj_1", "title_manual": true,
                            "description_manual": true, "description_by": "user",
                            "fs_scope": { "root": "/w/web" } }),
                    4,
                ),
            );
            data.entries.insert(
                MAIN.into(),
                vec![
                    json!({ "entry_id": "e_u1", "message": user("Why do webhooks retry?") }),
                    json!({ "entry_id": "e_rc", "custom": { "custom_type": "runtime_context",
                            "data": { "aid": "Your session id is s_main." } } }),
                    json!({ "entry_id": "e_a1", "message": assistant("Five attempts.") }),
                    json!({ "entry_id": "e_u2", "message": user("With what backoff?") }),
                    json!({ "entry_id": "e_a2", "message": assistant("Exponential.") }),
                ],
            );
        }
        fake.put_turn(&main_record()).await;
        fake
    }

    async fn put_turn(&self, record: &TurnRecord) {
        crate::state::put_turn(&self.deps.iii, record, TIMEOUT)
            .await
            .unwrap();
    }

    async fn turn(&self, session_id: &str) -> Option<TurnRecord> {
        crate::state::get_turn(&self.deps.iii, session_id, TIMEOUT)
            .await
            .unwrap()
    }

    fn session(&self, id: &str) -> Value {
        self.data.lock().unwrap().sessions[id].clone()
    }

    fn state(&self, scope: &str, key: &str) -> Value {
        self.data.lock().unwrap().get(scope, key)
    }

    fn calls_to(&self, function: &str) -> Vec<Value> {
        self.data
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(f, _)| f == function)
            .map(|(_, data)| data.clone())
            .collect()
    }

    async fn open(&self) -> SideChatOpenResponse {
        open(
            &self.deps,
            SideChatOpenRequest {
                session_id: MAIN.into(),
                entry_id: None,
                quote: None,
            },
        )
        .await
        .unwrap()
    }

    async fn send(&self, session_id: &str, req: Value) -> Result<send::SendResponse, HarnessError> {
        let mut req = req;
        req["session_id"] = json!(session_id);
        send::handle(&self.deps, serde_json::from_value(req).unwrap()).await
    }
}

fn code(error: &HarnessError) -> Option<String> {
    error.rejected_code()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_forks_at_the_last_reply_and_locks_the_conversations_turn() {
    let fake = Fake::with_conversation().await;
    let opened = fake.open().await;
    let sid = opened.session_id.clone();

    let fork = &fake.calls_to("session::fork")[0];
    assert_eq!(fork["entry_id"], "e_a2", "the last spoken entry");
    assert_eq!(fork["title"], TITLE);

    let metadata = &opened.meta["metadata"];
    assert_eq!(metadata[SIDE_CHAT_OF], MAIN);
    assert_eq!(
        metadata[SIDE_CHAT_FROM],
        format!("e_{sid}_4"),
        "the fork's copy of e_a2"
    );
    assert_eq!(
        metadata[SIDE_CHAT_LOCK],
        json!({ "model": "claude-opus-5-5", "provider": "anthropic",
                "thinking_level": "high", "account": "work" })
    );
    assert_eq!(
        metadata[SIDE_CHAT_FORK_POINT],
        json!({ "entry_id": "e_a2", "message_count": 4 })
    );
    assert_eq!(metadata["project_id"], "prj_1", "a fork keeps the project");
    for key in ["title_manual", "description_manual", "description_by"] {
        assert!(
            metadata.get(key).is_none(),
            "{key} is the conversation's own"
        );
    }
    assert_eq!(opened.meta["title"], TITLE);
    assert_eq!(opened.meta["description"], "");

    let lock = load_lock(&fake.deps, &sid).await.unwrap().unwrap();
    assert_eq!(lock.parent_session_id, MAIN);
    assert_eq!(lock.parent_entry_id.as_deref(), Some("e_a2"));
    assert_eq!(lock.model, "claude-opus-5-5");
    assert_eq!(lock.provider.as_deref(), Some("anthropic"));
    assert_eq!(lock.thinking_level, Some(ThinkingLevel::High));
    assert_eq!(
        lock.provider_options,
        main_record().options.provider_options
    );
    assert_eq!(lock.functions, main_record().options.functions);
    assert_eq!(lock.skills, Some(vec!["iii/state".to_string()]));
    assert_eq!(lock.root(), Some("/w/web"));
    assert_eq!(lock.source, "open");

    // The side chat starts from the conversation's turn, finished.
    let seeded = fake.turn(&sid).await.unwrap();
    let parent = fake.turn(MAIN).await.unwrap();
    assert_eq!(seeded.status, TurnStatus::Completed);
    assert_ne!(seeded.turn_id, parent.turn_id);
    assert_eq!(seeded.options.system_prompt, parent.options.system_prompt);
    assert_eq!(seeded.options.skill_context, parent.options.skill_context);
    assert_eq!(seeded.options.model, parent.options.model);
    assert_eq!(seeded.options.functions, parent.options.functions);
    assert!(seeded.skills_started);
    assert!(seeded.parent.is_none(), "a side chat is a root of its own");
    assert!(seeded.options.budget_root_session_id.is_none());
    assert!(seeded.options.max_total_tokens.is_none());
    assert_eq!(
        seeded.options.metadata,
        Some(json!({ "fs_scope": { "root": "/w/web" } })),
        "only the folder survives from the turn's metadata"
    );
    // Nothing was written into the conversation.
    assert!(
        fake.calls_to("session::patch-meta")
            .iter()
            .all(|p| p["session_id"] != MAIN)
    );
    assert!(fake.calls_to("session::append").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_at_an_entry_with_a_quote() {
    let fake = Fake::with_conversation().await;
    let quote = SideChatQuote {
        message_id: "e_a1".into(),
        text: "Five attempts.".into(),
        agent: Some("Claude".into()),
    };
    let opened = open(
        &fake.deps,
        SideChatOpenRequest {
            session_id: MAIN.into(),
            entry_id: Some("e_a1".into()),
            quote: Some(quote.clone()),
        },
    )
    .await
    .unwrap();
    let metadata = &opened.meta["metadata"];
    assert_eq!(metadata[SIDE_CHAT_QUOTES], json!([quote]));
    assert_eq!(
        metadata[SIDE_CHAT_FORK_POINT],
        json!({ "entry_id": "e_a1", "message_count": 2 })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_refuses_a_side_chat_a_conversation_without_a_turn_and_unknown_ids() {
    let fake = Fake::with_conversation().await;
    let opened = fake.open().await;
    let request = |session_id: &str, entry_id: Option<&str>| SideChatOpenRequest {
        session_id: session_id.into(),
        entry_id: entry_id.map(str::to_string),
        quote: None,
    };

    let nested = open(&fake.deps, request(&opened.session_id, None))
        .await
        .unwrap_err();
    assert_eq!(code(&nested).as_deref(), Some("nested_side_chat"));

    fake.data
        .lock()
        .unwrap()
        .sessions
        .insert("s_new".into(), meta("s_new", "New chat", json!({}), 0));
    let empty = open(&fake.deps, request("s_new", None)).await.unwrap_err();
    assert_eq!(code(&empty).as_deref(), Some("nothing_to_fork"));

    let unknown = open(&fake.deps, request("s_gone", None)).await.unwrap_err();
    assert_eq!(code(&unknown).as_deref(), Some("not_found"));

    let entry = open(&fake.deps, request(MAIN, Some("e_missing")))
        .await
        .unwrap_err();
    assert_eq!(code(&entry).as_deref(), Some("not_found"));
    assert_eq!(
        fake.calls_to("session::fork").len(),
        2,
        "only the first open and the unknown entry reached session::fork"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_while_the_conversation_runs_skips_the_reply_still_streaming() {
    let fake = Fake::with_conversation().await;
    let mut running = main_record();
    running.status = TurnStatus::Running;
    running.turn_id = "t_run".into();
    running.step = 1;
    fake.put_turn(&running).await;
    {
        let mut data = fake.data.lock().unwrap();
        let entries = data.entries.get_mut(MAIN).unwrap();
        entries.push(json!({ "entry_id": "e_u3", "message": user("And the jitter?") }));
        entries.push(json!({ "entry_id": "e_t_run_1_assistant", "message": assistant("Jit") }));
    }
    fake.open().await;
    assert_eq!(fake.calls_to("session::fork")[0]["entry_id"], "e_u3");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_runs_on_the_lock_names_the_side_chat_and_refuses_another_model() {
    let fake = Fake::with_conversation().await;
    let sid = fake.open().await.session_id;

    fake.send(&sid, json!({ "message": "Why 5 attempts   and not 8?" }))
        .await
        .unwrap();
    let turn = fake.turn(&sid).await.unwrap();
    let parent = fake.turn(MAIN).await.unwrap().options;
    assert_eq!(turn.status, TurnStatus::Running);
    assert_eq!(turn.options.model, parent.model);
    assert_eq!(turn.options.provider, parent.provider);
    assert_eq!(turn.options.thinking_level, parent.thinking_level);
    assert_eq!(turn.options.provider_options, parent.provider_options);
    assert_eq!(turn.options.functions, parent.functions);
    assert_eq!(turn.options.skill_context, parent.skill_context);
    assert_eq!(turn.options.system_prompt, parent.system_prompt);
    assert_eq!(turn.options.filesystem_root(), Some("/w/web"));
    assert_eq!(fake.session(&sid)["title"], "Why 5 attempts and not 8?");

    let other = fake
        .send(&sid, json!({ "message": "again", "model": "gpt-5" }))
        .await
        .unwrap_err();
    assert_eq!(code(&other).as_deref(), Some("side_chat_locked"));
    let wire: Value = serde_json::from_str(&other.to_string()).unwrap();
    assert_eq!(wire["field"], "model");
    assert_eq!(wire["locked"], "claude-opus-5-5");
    assert_eq!(wire["message"], LOCKED_MESSAGE);

    let effort = fake
        .send(
            &sid,
            json!({ "message": "again", "options": { "thinking_level": "low" } }),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&effort).as_deref(), Some("side_chat_locked"));

    // Naming the lock's own values is accepted; the title stays.
    fake.send(
        &sid,
        json!({ "message": "and 8?", "model": "claude-opus-5-5", "provider": "anthropic",
                "options": { "thinking_level": "high" } }),
    )
    .await
    .unwrap();
    assert_eq!(fake.session(&sid)["title"], "Why 5 attempts and not 8?");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_chat_forked_before_phase_2_is_locked_at_its_next_send() {
    let fake = Fake::with_conversation().await;
    {
        let mut data = fake.data.lock().unwrap();
        // Forked by the ADE, never sent in: no lock, no turn record.
        data.sessions.insert(
            "s_old".into(),
            meta(
                "s_old",
                "Side chat · Webhook retries",
                json!({ SIDE_CHAT_OF: MAIN, SIDE_CHAT_FROM: "e_c4", "project_id": "prj_1" }),
                4,
            ),
        );
        // Forked by the ADE and used: its own turn ran on another model.
        data.sessions.insert(
            "s_used".into(),
            meta(
                "s_used",
                "Side chat · Webhook retries",
                json!({ SIDE_CHAT_OF: MAIN, "project_id": "prj_1" }),
                6,
            ),
        );
    }
    let mut used = main_record();
    used.session_id = "s_used".into();
    used.turn_id = "t_used".into();
    used.parent = None;
    used.options.budget_root_session_id = None;
    used.options.max_total_tokens = None;
    used.options.model = "claude-haiku-5-5".into();
    used.options.thinking_level = Some(ThinkingLevel::Low);
    used.options.provider_options = None;
    fake.put_turn(&used).await;

    for sid in ["s_old", "s_used"] {
        fake.send(sid, json!({ "message": "hello" })).await.unwrap();
        let lock = load_lock(&fake.deps, sid).await.unwrap().unwrap();
        assert_eq!(lock.source, "lazy");
        assert_eq!(
            lock.model, "claude-opus-5-5",
            "{sid}: the conversation's now"
        );
        assert!(lock.parent_entry_id.is_none());
        let turn = fake.turn(sid).await.unwrap();
        assert_eq!(turn.options.model, "claude-opus-5-5", "{sid}");
        assert_eq!(
            turn.options.thinking_level,
            Some(ThinkingLevel::High),
            "{sid}"
        );
        assert_eq!(
            turn.options.provider_options,
            main_record().options.provider_options,
            "{sid}"
        );
        assert_eq!(
            fake.session(sid)["metadata"][SIDE_CHAT_LOCK]["model"],
            "claude-opus-5-5"
        );
        assert_eq!(
            fake.session(sid)["title"],
            "Side chat · Webhook retries",
            "{sid}: an old title is not the placeholder"
        );
    }
    // The never-sent one started from the conversation's record.
    assert_eq!(
        fake.turn("s_old").await.unwrap().options.system_prompt,
        main_record().options.system_prompt
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_manual_title_is_never_replaced() {
    let fake = Fake::with_conversation().await;
    let sid = fake.open().await.session_id;
    fake.deps
        .session()
        .await
        .patch_meta(
            &sid,
            &MetaPatch {
                title: Some("Backoff".into()),
                ..MetaPatch::set("title_manual", json!(true))
            },
        )
        .await
        .unwrap();
    fake.send(&sid, json!({ "message": "Why 5?" }))
        .await
        .unwrap();
    assert_eq!(fake.session(&sid)["title"], "Backoff");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_deletes_an_empty_side_chat_and_keeps_one_with_messages() {
    let fake = Fake::with_conversation().await;
    let empty = fake.open().await.session_id;
    let used = fake.open().await.session_id;
    fake.send(&used, json!({ "message": "Why 5?" }))
        .await
        .unwrap();

    let closed = close(
        &fake.deps,
        SideChatRequest {
            session_id: used.clone(),
        },
    )
    .await
    .unwrap();
    assert!(!closed.deleted && closed.operation_id.is_none());

    let closed = close(
        &fake.deps,
        SideChatRequest {
            session_id: empty.clone(),
        },
    )
    .await
    .unwrap();
    assert!(closed.deleted);
    let operation = closed.operation_id.unwrap();
    let stored = fake.state(
        crate::functions::delete_session_tree::OPERATIONS,
        &operation,
    );
    assert_eq!(stored["snapshot"]["session_id"], empty);
    assert_eq!(stored["snapshot"]["status"], "deleting");
    assert_eq!(
        fake.state(crate::functions::delete_session_tree::GUARDS, &empty),
        json!(operation),
        "tombstoned before anything else"
    );
    assert!(
        fake.calls_to(crate::functions::delete_session_tree::RUN_ID)
            .iter()
            .any(|p| p["operation_id"] == operation),
        "the deletion runner is enqueued"
    );

    let plain = close(
        &fake.deps,
        SideChatRequest {
            session_id: MAIN.into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(code(&plain).as_deref(), Some("not_side_chat"));
    let gone = close(
        &fake.deps,
        SideChatRequest {
            session_id: "s_gone".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(code(&gone).as_deref(), Some("not_found"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_reads_an_old_side_chats_transcript_after_its_fork_point() {
    let fake = Fake::with_conversation().await;
    {
        let mut data = fake.data.lock().unwrap();
        for (id, after) in [("s_quiet", false), ("s_talked", true)] {
            data.sessions.insert(
                id.into(),
                meta(
                    id,
                    "Side chat · x",
                    json!({ SIDE_CHAT_OF: MAIN, SIDE_CHAT_FROM: "e_c2" }),
                    2,
                ),
            );
            let mut entries = vec![
                json!({ "entry_id": "e_c1", "message": user("Why?") }),
                json!({ "entry_id": "e_c2", "message": assistant("Because.") }),
            ];
            if after {
                entries.push(json!({ "entry_id": "e_c3", "message": user("Really?") }));
            }
            data.entries.insert(id.into(), entries);
        }
    }
    let talked = close(
        &fake.deps,
        SideChatRequest {
            session_id: "s_talked".into(),
        },
    )
    .await
    .unwrap();
    assert!(!talked.deleted);
    let quiet = close(
        &fake.deps,
        SideChatRequest {
            session_id: "s_quiet".into(),
        },
    )
    .await
    .unwrap();
    assert!(quiet.deleted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_makes_it_a_conversation_in_its_conversations_project() {
    let fake = Fake::with_conversation().await;
    let sid = fake.open().await.session_id;
    // The conversation moved to another project after the fork.
    fake.deps
        .session()
        .await
        .patch_meta(MAIN, &MetaPatch::set("project_id", json!("prj_2")))
        .await
        .unwrap();
    let before = fake.session(&sid)["updated_at"].clone();

    let kept = keep(
        &fake.deps,
        SideChatRequest {
            session_id: sid.clone(),
        },
    )
    .await
    .unwrap();
    let metadata = &kept.meta["metadata"];
    for key in [
        SIDE_CHAT_OF,
        SIDE_CHAT_FROM,
        SIDE_CHAT_LOCK,
        SIDE_CHAT_FORK_POINT,
    ] {
        assert!(metadata.get(key).is_none(), "{key} went");
    }
    assert_eq!(metadata["project_id"], "prj_2");
    assert_eq!(
        fake.session(&sid)["updated_at"],
        before,
        "touch: false (D50)"
    );
    assert!(load_lock(&fake.deps, &sid).await.unwrap().is_none());
    let patch = fake
        .calls_to("session::patch-meta")
        .last()
        .cloned()
        .unwrap();
    assert_eq!(patch["touch"], false);

    // A kept conversation is no side chat: it runs unlocked.
    fake.send(&sid, json!({ "message": "hi", "model": "gpt-5" }))
        .await
        .unwrap();
    let again = keep(&fake.deps, SideChatRequest { session_id: sid })
        .await
        .unwrap_err();
    assert_eq!(code(&again).as_deref(), Some("not_side_chat"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summarize_sends_the_fixed_prompt_and_its_reply_joins_the_summaries() {
    let fake = Fake::with_conversation().await;
    let sid = fake.open().await.session_id;
    let summarized = summarize(
        &fake.deps,
        SideChatRequest {
            session_id: sid.clone(),
        },
    )
    .await
    .unwrap();
    let turn = fake.turn(&sid).await.unwrap();
    assert_eq!(turn.turn_id, summarized.turn_id);
    assert_eq!(
        turn.options.metadata.as_ref().unwrap()[SUMMARY_TURN_KEY],
        json!(true)
    );
    assert_eq!(turn.options.filesystem_root(), Some("/w/web"));
    let appended = fake.calls_to("session::append");
    assert_eq!(
        appended[0]["message"]["content"][0]["text"], SUMMARY_PROMPT,
        "the fixed prompt, exactly"
    );
    assert_eq!(
        fake.session(&sid)["title"],
        TITLE,
        "the prompt names nothing"
    );

    // A second one while the turn runs is refused.
    let busy = summarize(
        &fake.deps,
        SideChatRequest {
            session_id: sid.clone(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(code(&busy).as_deref(), Some("side_chat_busy"));

    // The reply completes: its entry id joins `side_chat_summaries`.
    let reply = crate::ids::assistant_entry_id(&turn.turn_id, 0);
    fake.data
        .lock()
        .unwrap()
        .entries
        .get_mut(&sid)
        .unwrap()
        .push(json!({ "entry_id": reply, "message": assistant("Five, then a dead letter.") }));
    let session = fake.deps.session().await;
    record_summary_reply(&session, &turn).await;
    record_summary_reply(&session, &turn).await;
    assert_eq!(
        fake.session(&sid)["metadata"][SIDE_CHAT_SUMMARIES],
        json!([reply]),
        "once, on a redelivered step too"
    );
    // Any other turn is left alone.
    let mut plain = turn.clone();
    plain.options.metadata = None;
    plain.turn_id = "t_other".into();
    record_summary_reply(&session, &plain).await;
    assert_eq!(
        fake.session(&sid)["metadata"][SIDE_CHAT_SUMMARIES],
        json!([reply])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summaries_keep_the_newest_twenty() {
    let fake = Fake::with_conversation().await;
    let sid = fake.open().await.session_id;
    let old: Vec<String> = (0..SUMMARIES_KEPT).map(|i| format!("e_old_{i}")).collect();
    fake.deps
        .session()
        .await
        .patch_meta(&sid, &MetaPatch::set(SIDE_CHAT_SUMMARIES, json!(old)))
        .await
        .unwrap();
    let mut turn = fake.turn(&sid).await.unwrap();
    turn.turn_id = "t_sum".into();
    turn.options.metadata = Some(json!({ SUMMARY_TURN_KEY: true }));
    fake.data
        .lock()
        .unwrap()
        .entries
        .get_mut(&sid)
        .unwrap()
        .push(json!({ "entry_id": "e_t_sum_2_assistant", "message": assistant("Done.") }));
    record_summary_reply(&fake.deps.session().await, &turn).await;
    let ids = fake.session(&sid)["metadata"][SIDE_CHAT_SUMMARIES].clone();
    let ids = ids.as_array().unwrap();
    assert_eq!(ids.len(), SUMMARIES_KEPT);
    assert_eq!(ids[0], "e_old_1");
    assert_eq!(ids[SUMMARIES_KEPT - 1], "e_t_sum_2_assistant");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_items_list_the_side_chats_newest_first() {
    let fake = Fake::with_conversation().await;
    let first = fake.open().await.session_id;
    let second = fake.open().await.session_id;
    fake.send(&first, json!({ "message": "Why 5?" }))
        .await
        .unwrap();
    fake.data.lock().unwrap().sessions.get_mut(&first).unwrap()["status"] = json!("working");
    let items = crate::context_items::handle(
        &fake.deps,
        crate::context_items::ContextItemsRequest {
            session_id: MAIN.into(),
        },
    )
    .await
    .unwrap();
    let rows = serde_json::to_value(&items.side_chats).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(
        rows[0]["session_id"],
        first.as_str(),
        "most recently active first"
    );
    assert_eq!(rows[0]["title"], "Why 5?");
    assert_eq!(rows[0]["message_count"], 1, "only its own messages");
    assert_eq!(rows[0]["from_entry_id"], "e_a2");
    assert_eq!(rows[0]["status"], "running");
    assert_eq!(rows[0]["running"], true);
    assert!(rows[0]["updated_at_ms"].is_i64() && rows[0]["created_at_ms"].is_i64());
    assert_eq!(rows[1]["session_id"], second.as_str());
    assert_eq!(rows[1]["title"], TITLE);
    assert_eq!(rows[1]["message_count"], 0);
    assert_eq!(rows[1]["running"], false);
    // The side chat's own context lists none.
    let own = crate::context_items::handle(
        &fake.deps,
        crate::context_items::ContextItemsRequest { session_id: first },
    )
    .await
    .unwrap();
    assert!(own.side_chats.is_empty());
}

// ── pure rules ──

fn lock() -> SideChatLock {
    SideChatLock::from_options(
        "s_side",
        MAIN,
        Some("e_a2".into()),
        &main_record().options,
        "open",
    )
}

fn request(value: Value) -> SendRequest {
    let mut value = value;
    value["message"] = json!("hi");
    serde_json::from_value(value).unwrap()
}

#[test]
fn the_lock_refuses_each_field_that_differs_and_accepts_its_own_values() {
    let lock = lock();
    let refused = |value: Value| {
        let error = check_lock(&lock, &request(value.clone())).unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("side_chat_locked"), "{value}");
        serde_json::from_str::<Value>(&error.to_string()).unwrap()["field"].clone()
    };
    assert_eq!(refused(json!({ "model": "gpt-5" })), "model");
    assert_eq!(refused(json!({ "provider": "openai" })), "provider");
    assert_eq!(
        refused(json!({ "options": { "thinking_level": "low" } })),
        "thinking_level"
    );
    assert_eq!(
        refused(json!({ "options": { "provider_options": {} } })),
        "provider_options"
    );
    assert_eq!(
        refused(json!({ "options": { "reasoning": "lowest" } })),
        "reasoning"
    );
    assert_eq!(
        refused(json!({ "options": { "functions": { "allow": [] } } })),
        "functions"
    );
    assert_eq!(
        refused(json!({ "options": { "skills": ["x/y"] } })),
        "skills"
    );
    assert_eq!(
        refused(json!({ "options": { "metadata": { "fs_scope": { "root": "/w/api" } } } })),
        "fs_scope"
    );
    assert_eq!(
        refused(json!({ "options": { "metadata": { "fs_scope": {} } } })),
        "fs_scope"
    );
    assert_eq!(refused(json!({ "options": { "agent": "coder" } })), "agent");
    assert_eq!(
        refused(json!({ "options": { "system_prompt": "x" } })),
        "system_prompt"
    );

    for value in [
        json!({}),
        json!({ "model": "claude-opus-5-5", "provider": "anthropic" }),
        json!({ "options": { "thinking_level": "high",
            "provider_options": main_record().options.provider_options,
            "functions": { "allow": ["*"] }, "skills": ["iii/state"],
            "metadata": { "fs_scope": { "root": "/w/web" }, "trace": "t" } } }),
    ] {
        check_lock(&lock, &request(value.clone())).unwrap_or_else(|e| panic!("{value}: {e}"));
    }
    // `{}` provider options and an empty skills list are "none".
    let mut bare = lock.clone();
    bare.provider_options = None;
    bare.skills = None;
    check_lock(
        &bare,
        &request(json!({ "options": { "provider_options": {}, "skills": [] } })),
    )
    .unwrap();
}

#[test]
fn the_lock_fills_what_a_send_leaves_out_only_where_the_last_turn_differs() {
    let lock = lock();
    // The last turn is the conversation's own (a side chat `open` made):
    // nothing is written into the request.
    let mut same = main_record();
    same.session_id = "s_side".into();
    let mut req = request(json!({}));
    fill_from_lock(&lock, Some(&same), &mut req);
    assert!(req.model.is_none() && req.provider.is_none());
    let options = req.options.as_ref().unwrap();
    assert!(options.thinking_level.is_none() && options.provider_options.is_none());
    assert!(options.functions.is_none() && options.skills.is_none());
    assert!(requested_root(options).is_none());

    // A last turn on other options: the lock's are written in.
    let mut other = same.clone();
    other.options.model = "claude-haiku-5-5".into();
    other.options.thinking_level = None;
    other.options.provider_options = None;
    other.options.functions = None;
    other.options.set_filesystem_root("/w/elsewhere");
    other.options.skill_context.as_mut().unwrap().filter = None;
    let mut req = request(json!({}));
    fill_from_lock(&lock, Some(&other), &mut req);
    assert_eq!(req.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(req.provider.as_deref(), Some("anthropic"));
    let options = req.options.as_ref().unwrap();
    assert_eq!(options.thinking_level, Some(ThinkingLevel::High));
    assert_eq!(options.provider_options, lock.provider_options);
    assert_eq!(options.functions, lock.functions);
    assert_eq!(requested_root(options), Some(Some("/w/web")));
    assert_eq!(options.skills, Some(vec!["iii/state".to_string()]));

    // A running last turn never gets a skills change.
    other.status = TurnStatus::Running;
    let mut req = request(json!({}));
    fill_from_lock(&lock, Some(&other), &mut req);
    assert!(req.options.unwrap().skills.is_none());
}

#[test]
fn a_first_message_names_the_side_chat_like_the_clients_do() {
    assert_eq!(
        derive_title("  Why 5\n attempts?  ").as_deref(),
        Some("Why 5 attempts?")
    );
    assert_eq!(derive_title(" \n "), None);
    let long = "word ".repeat(30);
    let title = derive_title(&long).unwrap();
    assert!(title.ends_with('…'));
    assert_eq!(
        title.chars().count(),
        TITLE_MAX_CHARS,
        "59 kept, the space trimmed, the ellipsis"
    );
    let exact = "x".repeat(TITLE_MAX_CHARS);
    assert_eq!(derive_title(&exact).as_deref(), Some(exact.as_str()));
}

#[test]
fn the_display_lock_names_the_account_when_the_provider_options_do() {
    let mut lock = lock();
    assert_eq!(lock.summary()["account"], "work");
    lock.provider_options = None;
    assert!(lock.summary().get("account").is_none());
}
