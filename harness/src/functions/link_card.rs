//! Link the calling session tree to a kanban card.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deps::Deps;
use crate::error::HarnessError;

pub const ID: &str = "harness::link-card";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkCardRequest {
    /// Stamped by the harness for an agent call; operators supply it explicitly.
    #[serde(default)]
    pub session_id: String,
    pub ticket: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct LinkCardResponse {
    pub session_id: String,
    pub ticket: String,
}

pub async fn handle(deps: &Deps, req: LinkCardRequest) -> Result<LinkCardResponse, HarnessError> {
    if req.ticket.trim().is_empty() || req.ticket.chars().count() > 64 {
        return Err(HarnessError::InvalidRequest(
            "ticket must be non-empty and at most 64 characters".into(),
        ));
    }
    let root = super::metrics::resolve_root(deps, &req.session_id).await?;
    let session = deps.session().await;
    let mut metadata = session
        .metadata_of(&root)
        .await?
        .ok_or_else(|| HarnessError::InvalidRequest(format!("session `{root}` does not exist")))?;
    if let Some(existing) = metadata.get("kanban_ticket").and_then(Value::as_str) {
        if existing != req.ticket {
            return Err(HarnessError::InvalidRequest(format!(
                "session `{root}` already links ticket `{existing}`"
            )));
        }
        return Ok(LinkCardResponse {
            session_id: root,
            ticket: req.ticket,
        });
    }
    metadata.insert("kanban_ticket".into(), Value::String(req.ticket.clone()));
    // ponytail: session::set-meta replaces the whole object, so this read-modify-write can
    // lose a concurrent metadata update; make session-manager expose an atomic merge if needed.
    session.set_metadata(&root, metadata).await?;
    Ok(LinkCardResponse {
        session_id: root,
        ticket: req.ticket,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Map, json};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tokio_tungstenite::tungstenite::Message;

    #[derive(Default)]
    struct Store {
        sessions: BTreeMap<String, Map<String, Value>>,
        state: BTreeMap<(String, String), Value>,
        dispatched_link: Option<Value>,
        metadata_writes: usize,
    }

    struct Fixture {
        deps: Deps,
        store: Arc<Mutex<Store>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.deps.iii.shutdown();
            self.server.abort();
        }
    }

    async fn fixture() -> Fixture {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let store = Arc::new(Mutex::new(Store::default()));
        let shared = store.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(msg) = serde_json::from_str::<Value>(frame.to_text().unwrap_or("")) else {
                    continue;
                };
                if msg["type"] != "invokefunction" || msg["invocation_id"].is_null() {
                    continue;
                }
                let function = msg["function_id"].as_str().unwrap();
                let data = &msg["data"];
                let result = {
                    let mut store = shared.lock().unwrap();
                    match function {
                        "session::get" => store
                            .sessions
                            .get(data["session_id"].as_str().unwrap_or_default())
                            .map(|metadata| {
                                json!({"meta": {
                                    "session_id": data["session_id"],
                                    "metadata": metadata,
                                }})
                            })
                            .unwrap_or(Value::Null),
                        "session::set-meta" => {
                            store.metadata_writes += 1;
                            store.sessions.insert(
                                data["session_id"].as_str().unwrap().into(),
                                data["metadata"].as_object().unwrap().clone(),
                            );
                            json!({})
                        }
                        "harness::state::compare-and-set" => {
                            let key = (
                                data["scope"].as_str().unwrap_or_default().into(),
                                data["key"].as_str().unwrap_or_default().into(),
                            );
                            let current = store.state.get(&key).cloned().unwrap_or(Value::Null);
                            if data.get("expected").unwrap_or(&Value::Null) == &current {
                                if data["value"].is_null() {
                                    store.state.remove(&key);
                                } else {
                                    store.state.insert(key, data["value"].clone());
                                }
                                json!({"swapped": true})
                            } else {
                                json!({"swapped": false, "current": current})
                            }
                        }
                        "harness::link-card" => {
                            store.dispatched_link = Some(data.clone());
                            json!({"session_id": data["session_id"], "ticket": data["ticket"]})
                        }
                        "engine::workers::list" => json!({"workers": []}),
                        other => panic!("unexpected function {other}"),
                    }
                };
                let reply = json!({
                    "type": "invocationresult",
                    "function_id": function,
                    "invocation_id": msg["invocation_id"],
                    "result": result,
                });
                if socket
                    .send(Message::Text(reply.to_string().into()))
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
        let deps = Deps::new(
            iii.clone(),
            Arc::new(tokio::sync::RwLock::new(Arc::new(
                crate::config::WorkerConfig::default(),
            ))),
            crate::discovery::new_cell(),
            crate::skills::new_cell(),
            crate::events::TurnEvents::register(&iii),
            crate::hooks::HookRegistry::register(&iii),
        );
        Fixture {
            deps,
            store,
            server,
        }
    }

    #[tokio::test]
    async fn links_only_the_callers_root_and_preserves_its_metadata() {
        let fixture = fixture().await;
        {
            let mut store = fixture.store.lock().unwrap();
            store.sessions.insert(
                "child".into(),
                Map::from_iter([
                    ("parent_session_id".into(), json!("root")),
                    ("child_only".into(), json!(true)),
                ]),
            );
            store.sessions.insert(
                "root".into(),
                Map::from_iter([("tenant".into(), json!("acme"))]),
            );
            store.sessions.insert(
                "other".into(),
                Map::from_iter([("kanban_ticket".into(), json!("KAN-OLD"))]),
            );
        }

        let linked = handle(
            &fixture.deps,
            LinkCardRequest {
                session_id: "child".into(),
                ticket: "KAN-597".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            linked,
            LinkCardResponse {
                session_id: "root".into(),
                ticket: "KAN-597".into(),
            }
        );
        let store = fixture.store.lock().unwrap();
        assert_eq!(store.sessions["root"]["tenant"], "acme");
        assert_eq!(store.sessions["root"]["kanban_ticket"], "KAN-597");
        assert_eq!(store.sessions["child"]["child_only"], true);
        assert!(store.sessions["child"].get("kanban_ticket").is_none());
        assert_eq!(store.sessions["other"]["kanban_ticket"], "KAN-OLD");
    }

    #[tokio::test]
    async fn refuses_relinking_a_root_but_accepts_the_same_ticket() {
        let fixture = fixture().await;
        fixture.store.lock().unwrap().sessions.insert(
            "root".into(),
            Map::from_iter([("kanban_ticket".into(), json!("KAN-597"))]),
        );
        let error = handle(
            &fixture.deps,
            LinkCardRequest {
                session_id: "root".into(),
                ticket: "KAN-598".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("already links ticket `KAN-597`"));
        assert_eq!(
            fixture.store.lock().unwrap().sessions["root"]["kanban_ticket"],
            "KAN-597"
        );
        let linked = handle(
            &fixture.deps,
            LinkCardRequest {
                session_id: "root".into(),
                ticket: "KAN-597".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(linked.ticket, "KAN-597");
        assert_eq!(fixture.store.lock().unwrap().metadata_writes, 0);
    }

    #[tokio::test]
    async fn agent_call_stamps_the_caller_over_a_forged_session() {
        let fixture = fixture().await;
        fixture
            .deps
            .liveness
            .remember_live("mine", fixture.deps.liveness.stamp());
        let policy =
            crate::policy::CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
                allow: vec!["*".into()],
                deny: vec![],
                expose: Default::default(),
            }));

        let result = crate::functions::observe::invoke(
            &fixture.deps,
            &fixture.deps.engine().await,
            &policy,
            ID,
            None,
            &json!({"session_id": "someone-else", "ticket": "KAN-597"}),
            "mine",
            false,
            None,
        )
        .await;

        assert!(!result.is_error, "{result:?}");
        assert_eq!(
            fixture.store.lock().unwrap().dispatched_link,
            Some(json!({"session_id": "mine", "ticket": "KAN-597"}))
        );
    }

    #[tokio::test]
    async fn rejects_empty_or_overlong_tickets_without_reading_a_session() {
        let fixture = fixture().await;
        for ticket in ["   ".to_string(), "x".repeat(65)] {
            let error = handle(
                &fixture.deps,
                LinkCardRequest {
                    session_id: "missing".into(),
                    ticket,
                },
            )
            .await
            .unwrap_err();
            assert_eq!(error.code(), "harness/invalid_request");
            assert!(error.to_string().contains("at most 64 characters"));
        }
    }
}
