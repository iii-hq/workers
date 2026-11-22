use super::*;
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message;

#[derive(Default)]
struct Store {
    rows: BTreeMap<String, Value>,
    sends: BTreeMap<String, String>,
    state: BTreeMap<(String, String), Value>,
    events: Vec<Value>,
    fail_sends: usize,
    expire_on_reserve: bool,
    idem_reads: Vec<String>,
    fail_close: bool,
    fail_send: bool,
    gone: bool,
    calls: Vec<String>,
    withdrawn: Option<Value>,
    metadata: Option<Value>,
}

async fn fixture() -> (Deps, Arc<Mutex<Store>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let store = Arc::new(Mutex::new(Store::default()));
    let db = store.clone();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        socket.send(Message::Text(json!({"type":"workerregistered", "worker_id":"ask-test", "features":["observe.v2"]}).to_string().into())).await.unwrap();
        while let Some(Ok(frame)) = socket.next().await {
            let Ok(msg) = serde_json::from_str::<Value>(frame.to_text().unwrap_or("")) else {
                continue;
            };
            if msg["type"] == "registerfunction" && msg["id"] == crate::events::ON_ASK_SETTLED_FN {
                let registered = json!({"type":"functionregistered", "id":msg["id"],
                    "revision":1, "observers":[{"id":"settled", "target":"test::settled"}]});
                socket
                    .send(Message::Text(registered.to_string().into()))
                    .await
                    .unwrap();
                continue;
            }
            if msg["type"] != "invokefunction" {
                continue;
            }
            let function = msg["function_id"].as_str().unwrap();
            let data = &msg["data"];
            let mut rejected = false;
            let result = {
                let mut db = db.lock().unwrap();
                db.calls.push(function.into());
                let key = data["key"].as_str().unwrap_or_default().to_owned();
                let scope = data["scope"].as_str().unwrap_or_default().to_owned();
                if scope == crate::state::IDEM_SCOPE && function.ends_with("::get") {
                    db.idem_reads.push(key.clone());
                    if db.fail_send || db.fail_sends > 0 {
                        db.fail_send = false;
                        db.fail_sends = db.fail_sends.saturating_sub(1);
                        rejected = true;
                    }
                }
                match function {
                    "state::get" | "harness::state::get" if scope == SCOPE => {
                        db.rows.get(&key).cloned().unwrap_or(Value::Null)
                    }
                    "state::get" | "harness::state::get" => {
                        db.state.get(&(scope, key)).cloned().unwrap_or(Value::Null)
                    }
                    "harness::state::list" if scope == SCOPE => {
                        json!({"values": db.rows.values().collect::<Vec<_>>()})
                    }
                    "state::list" | "harness::state::list" => json!([]),
                    "state::set" => {
                        record_message(&mut db, data);
                        db.state.insert((scope, key), data["value"].clone());
                        json!({})
                    }
                    "harness::state::compare-and-set" => {
                        let current = if scope == SCOPE {
                            db.rows.get(&key).cloned()
                        } else {
                            db.state.get(&(scope.clone(), key.clone())).cloned()
                        }
                        .unwrap_or(Value::Null);
                        if scope == SCOPE
                            && db.expire_on_reserve
                            && data["value"]["state"] == "answering"
                        {
                            db.expire_on_reserve = false;
                            let mut expired = current;
                            expired["state"] = json!("expired");
                            expired["by"] = json!({"channel":"expiry"});
                            db.rows.insert(key, expired.clone());
                            json!({"swapped":false, "current":expired})
                        } else if scope == SCOPE
                            && db.fail_close
                            && data["value"]["state"] == "answered"
                        {
                            db.fail_close = false;
                            rejected = true;
                            Value::Null
                        } else if data.get("expected").unwrap_or(&Value::Null) == &current {
                            if scope == SCOPE {
                                if data["value"].is_null() {
                                    db.rows.remove(&key);
                                } else {
                                    db.rows.insert(key, data["value"].clone());
                                }
                            } else {
                                db.state.insert((scope, key), data["value"].clone());
                            }
                            json!({"swapped": true})
                        } else {
                            json!({"swapped": false, "current": current})
                        }
                    }
                    "session::get" if db.gone => Value::Null,
                    "session::get" => {
                        let metadata = db
                            .metadata
                            .clone()
                            .unwrap_or_else(|| json!({"kanban_ticket":"KAN-597"}));
                        json!({"meta":{"session_id":data["session_id"],"metadata":metadata}})
                    }
                    "engine::workers::list" => json!({"workers":[]}),
                    "engine::workers::register" => json!({"success":true}),
                    "engine::functions::list" => json!({"functions":[]}),
                    "harness::ask::withdraw" => {
                        db.withdrawn = Some(data.clone());
                        data.clone()
                    }
                    "session::append" => {
                        record_message(&mut db, data);
                        json!({"entry_id":data["entry_id"]})
                    }
                    "session::messages" => json!({"messages":[]}),
                    "session::set-status" | "session::set-meta" => json!({}),
                    "harness::turn" => json!({}),
                    crate::events::ON_ASK_SETTLED_FN => {
                        db.events.push(data.clone());
                        data.clone()
                    }
                    other => panic!("unexpected function {other}"),
                }
            };
            if msg["invocation_id"].is_null() {
                continue;
            }
            let mut reply = json!({"type":"invocationresult", "function_id":function, "invocation_id":msg["invocation_id"]});
            if rejected {
                reply["error"] = json!({"code":"rejected", "message":"injected close failure"});
            } else {
                reply["result"] = result;
            }
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
    for session in ["s", "other"] {
        let mut record = crate::types::turn::tests::record();
        record.session_id = session.into();
        record.status = crate::types::turn::TurnStatus::Completed;
        record.options.system_prompt =
            Some(format!("Frozen ask test prompt {}", uuid::Uuid::new_v4()));
        crate::state::put_turn(&deps.iii, &record, 1000)
            .await
            .unwrap();
    }
    (deps, store, server)
}

fn record_message(db: &mut Store, data: &Value) {
    let row = data.get("value").unwrap_or(data);
    let Some(id) = row["entry_id"].as_str() else {
        return;
    };
    let keys = db
        .rows
        .values()
        .filter_map(|value| serde_json::from_value::<Entry>(value.clone()).ok())
        .flat_map(|entry| {
            [
                format!("ask:{}", entry.key()),
                format!("ask-expiry:{}", entry.key()),
            ]
        });
    if let Some(key) = keys
        .into_iter()
        .find(|key| crate::ids::idem_user_entry_id(key) == id)
    {
        let text = row["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned();
        db.sends.entry(key).or_insert(text);
    }
}

fn request(extra: Value) -> AskRequest {
    let mut req = json!({"questions":[{"header":"Builds","question":"How?","options":[{"label":"Isolate"},{"label":"Allow-list"}]}]});
    req.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(req).unwrap()
}

fn response(session: &str, question: &str) -> AnswerRequest {
    serde_json::from_value(json!({"session_id":session,"question_id":question,
        "answers":[{"header":"Builds","choice":"Isolate"}],"by":{"channel":"tower","user":"person"}})).unwrap()
}

#[tokio::test]
async fn asking_profile_is_stored_and_preserved_when_settled() {
    let (deps, db, server) = fixture().await;
    db.lock().unwrap().metadata = Some(json!({
        "agent_profile":{"id":"backend-engineer", "name":"Backend engineer"},
        "kanban_ticket":"KAN-597"
    }));
    // An explicit card must not skip looking up the asking profile.
    let req = request(json!({"about":{"ticket":"KAN-598"}}));
    accept(&deps, "s", "t", "q", &req).await.unwrap();
    let entry = get(&deps, "s", "q").await.unwrap();
    assert_eq!(entry.asked_by.as_deref(), Some("backend-engineer"));
    assert_eq!(entry.about.ticket.as_deref(), Some("KAN-598"));
    assert_eq!(
        db.lock().unwrap().rows["s/q"]["asked_by"],
        "backend-engineer"
    );
    db.lock().unwrap().metadata = Some(json!({"agent_profile":{"id":"other"}}));
    accept(&deps, "s", "t", "q", &req).await.unwrap();
    let settled = answer(&deps, response("s", "q")).await.unwrap();
    assert_eq!(settled.asked_by.as_deref(), Some("backend-engineer"));
    let payload = serde_json::to_value(&settled).unwrap();
    assert_eq!(payload["asked_by"], "backend-engineer");
    // Older entries and sessions without profiles omit attribution.
    let mut legacy = payload;
    legacy.as_object_mut().unwrap().remove("asked_by");
    assert!(
        serde_json::from_value::<Entry>(legacy)
            .unwrap()
            .asked_by
            .is_none()
    );
    server.abort();
}

#[tokio::test]
async fn accepts_lists_counts_and_preserves_closed_entries_until_purge() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    accept(&deps, "other", "t", "q", &request(json!({})))
        .await
        .unwrap();
    assert_eq!(open_count(&deps, "s").await.unwrap(), 1);
    let page = list(
        &deps,
        ListRequest {
            limit: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].about.ticket.as_deref(), Some("KAN-597"));
    assert!(page.entries[0].asked_by.is_none());
    assert!(db.lock().unwrap().rows["s/q"].get("asked_by").is_none());
    let page = list(
        &deps,
        ListRequest {
            cursor: page.cursor,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.entries[0].session_id, "s");
    answer(&deps, response("s", "q")).await.unwrap();
    assert_eq!(open_count(&deps, "s").await.unwrap(), 0);
    assert_eq!(
        get(&deps, "s", "q").await.unwrap().state,
        AskState::Answered
    );
    // Legacy settlement arrays remain readable by their original functions.
    db.lock().unwrap().rows.insert("s".into(), json!([]));
    purge(&deps, "s").await.unwrap();
    assert!(get(&deps, "s", "q").await.is_err());
    assert_eq!(
        get(&deps, "other", "q").await.unwrap().state,
        AskState::Open
    );
    assert!(
        db.lock()
            .unwrap()
            .calls
            .iter()
            .filter(|id| id.contains("state::")
                && !matches!(id.as_str(), "state::set" | "state::get" | "state::list"))
            .all(|id| id.starts_with("harness::state::"))
    );
    server.abort();
}

#[tokio::test]
async fn concurrent_answers_reserve_once_and_report_the_winning_channel() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        answer(&deps, response("s", "q")),
        answer(&deps, response("s", "q"))
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let err = a.err().or(b.err()).unwrap().to_string();
    assert!(err.contains("already answered by tower"), "{err}");
    let store = db.lock().unwrap();
    assert_eq!(store.idem_reads, ["ask:s/q", "ask:s/q"]);
    assert_eq!(store.sends["ask:s/q"], "Builds: Isolate");
    server.abort();
}

#[tokio::test]
async fn recovery_replays_a_sent_reservation_with_the_same_key() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    db.lock().unwrap().fail_close = true;
    assert!(answer(&deps, response("s", "q")).await.is_err());
    assert_eq!(
        get(&deps, "s", "q").await.unwrap().state,
        AskState::Answering
    );
    recover(&deps).await.unwrap();
    assert_eq!(
        get(&deps, "s", "q").await.unwrap().state,
        AskState::Answered
    );
    let store = db.lock().unwrap();
    assert_eq!(store.idem_reads, ["ask:s/q", "ask:s/q", "ask:s/q"]);
    assert_eq!(store.sends.len(), 1);
    server.abort();
}

#[tokio::test]
async fn expiry_block_is_claimed_once_and_still_accepts_a_late_answer() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({"expires_in_min":1})))
        .await
        .unwrap();
    let due = get(&deps, "s", "q").await.unwrap().expires_at.unwrap();
    let (a, b) = tokio::join!(expire(&deps, "s", "q", due), expire(&deps, "s", "q", due));
    a.unwrap();
    b.unwrap();
    assert_eq!(get(&deps, "s", "q").await.unwrap().state, AskState::Expired);
    assert_eq!(open_count(&deps, "s").await.unwrap(), 1);
    assert_eq!(
        db.lock().unwrap().sends["ask-expiry:s/q"],
        "No answer within 1 min to: Builds. Do not decide it yourself. If you cannot go on without it, finish blocked; the answer will arrive as a message."
    );
    answer(&deps, response("s", "q")).await.unwrap();
    assert_eq!(
        db.lock().unwrap().idem_reads,
        ["ask-expiry:s/q", "ask-expiry:s/q", "ask:s/q", "ask:s/q"]
    );
    server.abort();
}

#[tokio::test]
async fn timer_rearmed_at_boot_applies_the_validated_default() {
    let (deps, db, server) = fixture().await;
    let req = request(
        json!({"expires_in_min":1,"about":{"class":"routine"},"on_expiry":{"answer":"Isolate"}}),
    );
    crate::ask::validate(&req).unwrap();
    accept(&deps, "s", "t", "q", &req).await.unwrap();
    deps.ask_timers.cancel("ask-expiry:s/q");
    let mut past = get(&deps, "s", "q").await.unwrap();
    let before = past.clone();
    past.expires_at = Some(AgentMessage::now_ms() - 1);
    assert!(cas(&deps, Some(&before), &past).await.unwrap());
    recover(&deps).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if get(&deps, "s", "q").await.unwrap().state == AskState::Answered {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let entry = get(&deps, "s", "q").await.unwrap();
    assert_eq!(entry.by.unwrap().channel, Channel::Expiry);
    assert_eq!(entry.outcome, Some(Outcome::ExpiredDefault));
    assert_eq!(
        db.lock().unwrap().sends["ask:s/q"],
        "Builds: Isolate (no answer within 1 min; default applied)"
    );
    server.abort();
}

#[tokio::test]
async fn withdrawal_cancels_deadline_and_cannot_close_another_session() {
    let (deps, db, server) = fixture().await;
    let req = request(json!({"expires_in_min":1}));
    accept(&deps, "s", "t", "q", &req).await.unwrap();
    accept(&deps, "other", "t", "q", &req).await.unwrap();
    let due = get(&deps, "s", "q").await.unwrap().expires_at.unwrap();
    let entry = withdraw(
        &deps,
        WithdrawRequest {
            session_id: "s".into(),
            question_id: "q".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(entry.state, AskState::Withdrawn);
    assert_eq!(entry.outcome, Some(Outcome::Withdrawn));
    assert!(!deps.ask_timers.is_armed("ask-expiry:s/q"));
    expire(&deps, "s", "q", due).await.unwrap();
    assert!(db.lock().unwrap().sends.is_empty());
    assert_eq!(
        get(&deps, "other", "q").await.unwrap().state,
        AskState::Open
    );
    assert!(
        answer(&deps, response("s", "q"))
            .await
            .unwrap_err()
            .to_string()
            .contains("withdrawn")
    );
    server.abort();
}

#[tokio::test]
async fn missing_session_does_not_get_recreated_by_an_answer() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    db.lock().unwrap().gone = true;
    let entry = answer(&deps, response("s", "q")).await.unwrap();
    assert_eq!(entry.outcome, Some(Outcome::NotDelivered));
    assert!(db.lock().unwrap().sends.is_empty());
    recover(&deps).await.unwrap();
    assert!(all(&deps).await.unwrap().is_empty());
    server.abort();
}

#[test]
fn structured_answers_match_the_chat_formatter_and_reject_invalid_answers() {
    let entry: Entry = serde_json::from_value(json!({"session_id":"s","question_id":"q","turn_id":"t",
        "questions":[{"header":"Builds","question":"How?","options":[{"label":"Isolate"},{"label":"Allow-list"}]},
        {"header":"Channels","question":"Where?","multi_select":true,"options":[{"label":"Console"},{"label":"CLI"}]},
        {"header":"Region","question":"Which?","options":[{"label":"US"},{"label":"EU"}]},
        {"header":"Budget","question":"How much?","options":[{"label":"Low"},{"label":"High"}]}],
        "about":{},"asked_at":0,"on_expiry":"block","state":"open"})).unwrap();
    let answers = serde_json::from_value::<Vec<Answer>>(json!([
        {"header":"Builds","choice":"Isolate"},
        {"header":"Channels","choices":["CLI","Console"],"other":"  Mobile  "},
        {"header":"Region","other":"  eu-west-2  "},
        {"header":"Budget","skipped":true}
    ]))
    .unwrap();
    assert_eq!(
        answer_text(&entry, &answers).unwrap(),
        "Builds: Isolate\nChannels: Console, CLI, Mobile\nRegion: eu-west-2\nBudget: (skipped)"
    );
    let mut bad = answers.clone();
    bad[0].choice = Some("invented".into());
    assert!(answer_text(&entry, &bad).is_err());
    bad = answers.clone();
    bad[0].choices = Some(vec!["Isolate".into()]);
    assert!(answer_text(&entry, &bad).is_err());
    bad = answers.clone();
    bad[1].choices = Some(vec!["CLI".into(), "CLI".into()]);
    assert!(answer_text(&entry, &bad).is_err());
    bad = answers.clone();
    bad[0].header = "Wrong".into();
    assert!(answer_text(&entry, &bad).is_err());
    bad = answers.clone();
    bad[3].choice = Some("Low".into());
    assert!(answer_text(&entry, &bad).is_err());
    assert!(answer_text(&entry, &answers[..3]).is_err());
}

#[test]
fn dismissal_only_allows_the_three_approved_classes() {
    let req = response("s", "q");
    let mut entry: Entry = serde_json::from_value(json!({"session_id":"s","question_id":"q","turn_id":"t",
        "questions":request(json!({})).questions,"about":{},"asked_at":0,"on_expiry":"block","state":"open"})).unwrap();
    let dismiss = || AnswerRequest {
        answers: None,
        dismissed: true,
        ..req.clone()
    };
    for class in [AskClass::Scope, AskClass::Routine, AskClass::RunHealth] {
        entry.about.class = class;
        assert_eq!(
            reserve(&entry, dismiss()).unwrap().answer_text.as_deref(),
            Some(DISMISS_TEXT)
        );
    }
    for class in [
        AskClass::Security,
        AskClass::Publish,
        AskClass::ExternalCost,
        AskClass::Data,
    ] {
        entry.about.class = class;
        assert!(reserve(&entry, dismiss()).is_err());
    }
}

#[tokio::test]
async fn an_agent_cannot_forge_the_withdrawal_session() {
    let (deps, db, server) = fixture().await;
    deps.liveness.remember_live("mine", deps.liveness.stamp());
    let policy = crate::policy::CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
        allow: vec!["*".into()],
        deny: vec![],
        expose: Default::default(),
    }));
    let result = crate::functions::observe::invoke(
        &deps,
        &deps.engine().await,
        &policy,
        WITHDRAW_ID,
        None,
        &json!({"session_id":"someone-else", "question_id":"q"}),
        "mine",
        false,
        None,
    )
    .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        db.lock().unwrap().withdrawn,
        Some(json!({"session_id":"mine", "question_id":"q"}))
    );
    server.abort();
}

#[tokio::test]
async fn a_transient_delivery_error_retries_without_a_reboot_or_worker_event() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    db.lock().unwrap().fail_send = true;
    assert!(answer(&deps, response("s", "q")).await.is_err());
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop {
            if get(&deps, "s", "q").await.unwrap().state == AskState::Answered {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        db.lock().unwrap().idem_reads,
        ["ask:s/q", "ask:s/q", "ask:s/q"]
    );
    assert_eq!(db.lock().unwrap().sends.len(), 1);
    server.abort();
}

#[tokio::test]
async fn delivery_keeps_rearming_after_more_than_five_failures() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    let before = get(&deps, "s", "q").await.unwrap();
    let reserved = reserve(&before, response("s", "q")).unwrap();
    assert!(cas(&deps, Some(&before), &reserved).await.unwrap());
    db.lock().unwrap().fail_sends = 7;
    // Zero delay exercises successive re-armings without waiting out the backoff.
    retry(&deps, &reserved, 0);
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop {
            if get(&deps, "s", "q").await.unwrap().state == AskState::Answered {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Seven failed lookups, then delivery lookup and the private ledger write read.
    assert_eq!(db.lock().unwrap().idem_reads.len(), 9);
    assert_eq!(db.lock().unwrap().sends.len(), 1);
    assert!(!deps.ask_timers.is_armed("ask-retry:s/q"));
    server.abort();
}

#[tokio::test]
async fn failed_recovery_delivery_keeps_booting_and_retries() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    let before = get(&deps, "s", "q").await.unwrap();
    let reserved = reserve(&before, response("s", "q")).unwrap();
    assert!(cas(&deps, Some(&before), &reserved).await.unwrap());
    db.lock().unwrap().fail_send = true;
    recover(&deps).await.unwrap();
    assert!(deps.ask_timers.is_armed("ask-retry:s/q"));
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop {
            if get(&deps, "s", "q").await.unwrap().state == AskState::Answered {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(db.lock().unwrap().idem_reads.len(), 3);
    assert_eq!(db.lock().unwrap().sends.len(), 1);
    server.abort();
}

#[tokio::test]
async fn purge_withdraws_open_expired_and_reserved_questions_before_removing_them() {
    let (deps, db, server) = fixture().await;
    for (id, state) in [
        ("open", AskState::Open),
        ("expired", AskState::Expired),
        ("reserved", AskState::Answering),
    ] {
        accept(&deps, "s", "t", id, &request(json!({"expires_in_min":1})))
            .await
            .unwrap();
        db.lock().unwrap().rows.get_mut(&format!("s/{id}")).unwrap()["state"] = json!(state);
    }
    purge(&deps, "s").await.unwrap();
    assert_eq!(open_count(&deps, "s").await.unwrap(), 0);
    let store = db.lock().unwrap();
    assert!(store.rows.is_empty());
    assert_eq!(store.events.len(), 3);
    for payload in &store.events {
        assert_eq!(payload["state"], "withdrawn");
        assert_eq!(payload["outcome"], "withdrawn");
        assert_eq!(payload["about"]["ticket"], "KAN-597");
        assert!(payload["settled_at"].is_i64());
    }
    assert!(deps.ask_timers.is_empty());
    server.abort();
}

#[tokio::test]
async fn malformed_rows_do_not_break_listing_counts_recovery_or_deletion() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    db.lock()
        .unwrap()
        .rows
        .insert("broken".into(), json!({"state":"unknown"}));
    assert_eq!(
        list(&deps, ListRequest::default())
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
    assert_eq!(open_count(&deps, "s").await.unwrap(), 1);
    recover(&deps).await.unwrap();
    purge(&deps, "s").await.unwrap();
    assert!(all(&deps).await.unwrap().is_empty());
    assert!(db.lock().unwrap().rows.contains_key("broken"));
    server.abort();
}

#[tokio::test]
async fn purge_preserves_reserved_answers_and_dismissals_as_not_delivered() {
    let (deps, db, server) = fixture().await;
    for dismissed in [false, true] {
        let id = if dismissed { "dismissed" } else { "answered" };
        accept(&deps, "s", "t", id, &request(json!({})))
            .await
            .unwrap();
        let before = get(&deps, "s", id).await.unwrap();
        let mut req = response("s", id);
        if dismissed {
            req.answers = None;
            req.dismissed = true;
        }
        let reserved = reserve(&before, req).unwrap();
        assert!(cas(&deps, Some(&before), &reserved).await.unwrap());
    }
    purge(&deps, "s").await.unwrap();
    assert!(all(&deps).await.unwrap().is_empty());
    let store = db.lock().unwrap();
    assert_eq!(store.events.len(), 2);
    for event in &store.events {
        assert_eq!(event["outcome"], "not_delivered");
        assert_eq!(event["by"], json!({"channel":"tower", "user":"person"}));
        if event["question_id"] == "dismissed" {
            assert_eq!(event["state"], "dismissed");
            assert_eq!(event["answer_text"], DISMISS_TEXT);
        } else {
            assert_eq!(event["state"], "answered");
            assert_eq!(event["answer_text"], "Builds: Isolate");
        }
    }
    server.abort();
}

#[tokio::test]
async fn answer_during_block_expiry_delivery_is_busy_then_accepts_retry() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({"expires_in_min":1})))
        .await
        .unwrap();
    let before = get(&deps, "s", "q").await.unwrap();
    let reserved = expiry_reservation(&before).unwrap();
    assert!(cas(&deps, Some(&before), &reserved).await.unwrap());
    let error = answer(&deps, response("s", "q")).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("ask_busy: expiry notice in delivery, retry")
    );
    assert_eq!(open_count(&deps, "s").await.unwrap(), 1);
    deliver_once(&deps, reserved).await.unwrap();
    let answered = answer(&deps, response("s", "q")).await.unwrap();
    assert_eq!(answered.outcome, Some(Outcome::Answered));
    assert_eq!(
        db.lock().unwrap().idem_reads,
        ["ask-expiry:s/q", "ask-expiry:s/q", "ask:s/q", "ask:s/q"]
    );
    server.abort();
}

#[tokio::test]
async fn answer_retries_reservation_when_expiry_completed_after_its_read() {
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({"expires_in_min":1})))
        .await
        .unwrap();
    db.lock().unwrap().expire_on_reserve = true;
    let entry = answer(&deps, response("s", "q")).await.unwrap();
    assert_eq!(entry.outcome, Some(Outcome::Answered));
    assert_eq!(db.lock().unwrap().idem_reads, ["ask:s/q", "ask:s/q"]);
    server.abort();
}

#[test]
fn answer_request_tolerates_the_engine_caller_stamp() {
    // kanban::question::answer forwards worker-to-worker; the engine stamps
    // _caller_worker_id on the payload, which must not fail the parse.
    let request: AnswerRequest = serde_json::from_value(serde_json::json!({
        "session_id": "s",
        "question_id": "q",
        "dismissed": true,
        "by": { "channel": "kanban", "user": "user" },
        "_caller_worker_id": "kanban-1",
    }))
    .unwrap();
    assert!(request.dismissed);
}

#[tokio::test]
async fn an_answer_being_delivered_still_counts_as_open() {
    // Board dispatch reads open_asks when a turn ends; a reservation not yet
    // delivered must keep the card in progress.
    let (deps, db, server) = fixture().await;
    accept(&deps, "s", "t", "q", &request(json!({})))
        .await
        .unwrap();
    db.lock().unwrap().rows.get_mut("s/q").unwrap()["state"] = json!("answering");
    assert_eq!(open_count(&deps, "s").await.unwrap(), 1);
    server.abort();
}
