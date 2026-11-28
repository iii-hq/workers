//! Accounts over an isolated SDK transport: a fake engine with in-memory
//! state, sessions, and tmux functions that are present or not. No live
//! engine, no tmux, no model.

use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message as Frame;

use crate::types::turn::ParentLink;

const TIMEOUT: u64 = 2_000;
const S: &str = "s_conv";

// ── pure rules ──

fn options(provider: Option<&str>, model: &str) -> TurnOptions {
    let mut options = crate::types::turn::tests::record().options;
    options.provider = provider.map(str::to_string);
    options.model = model.into();
    options
}

#[test]
fn only_claude_code_turns_run_on_an_account() {
    assert!(runs_on_claude_code(&options(
        Some("claude-code"),
        "claude-code/claude-opus-4-8"
    )));
    assert!(runs_on_claude_code(&options(
        None,
        "claude-code/claude-opus-4-8"
    )));
    assert!(!runs_on_claude_code(&options(None, "claude-codex/x")));
    assert!(!runs_on_claude_code(&options(
        Some("anthropic"),
        "claude-code/claude-opus-4-8"
    )));
    assert!(!runs_on_claude_code(&options(None, "claude-opus-4-8")));
}

#[test]
fn the_account_option_is_written_over_any_and_stripped_alone() {
    let mut map = Some(BTreeMap::from([(
        "claude-code".to_string(),
        json!({ "thinking": { "type": "enabled" }, "account": "client" }),
    )]));
    set_account_option(&mut map, "work");
    assert_eq!(account_option(map.as_ref()), Some("work"));
    assert_eq!(
        map.as_ref().unwrap()["claude-code"]["thinking"],
        json!({ "type": "enabled" }),
        "the rest of the namespace stays"
    );

    let mut map = map.unwrap();
    assert!(strip_account_option(&mut map));
    assert_eq!(
        map["claude-code"],
        json!({ "thinking": { "type": "enabled" } })
    );
    assert!(!strip_account_option(&mut map), "nothing left to drop");

    let mut only = BTreeMap::from([("claude-code".to_string(), json!({ "account": "x" }))]);
    assert!(strip_account_option(&mut only));
    assert!(
        only.is_empty(),
        "a namespace that held only the account goes"
    );

    // Another provider's own `account` option is not the harness's.
    let mut other = BTreeMap::from([("acme".to_string(), json!({ "account": "x" }))]);
    assert!(!strip_account_option(&mut other));

    let mut none = None;
    set_account_option(&mut none, "personal");
    assert_eq!(account_option(none.as_ref()), Some("personal"));
}

#[test]
fn options_compare_without_the_account() {
    let locked = BTreeMap::from([(
        "claude-code".to_string(),
        json!({ "account": "work", "effort": "high" }),
    )]);
    let sent = BTreeMap::from([(
        "claude-code".to_string(),
        json!({ "account": "personal", "effort": "high" }),
    )]);
    assert_eq!(without_account(Some(&locked)), without_account(Some(&sent)));
    let only_account = BTreeMap::from([("claude-code".to_string(), json!({ "account": "w" }))]);
    assert_eq!(without_account(Some(&only_account)), None);
    assert_eq!(without_account(Some(&BTreeMap::new())), None);
    assert_eq!(without_account(None), None);
    let effort = BTreeMap::from([("claude-code".to_string(), json!({ "effort": "low" }))]);
    assert_ne!(
        without_account(Some(&locked)),
        without_account(Some(&effort))
    );
}

fn project(id: &str, account: Value) -> Project {
    serde_json::from_value(json!({
        "id": id, "name": id, "color": "teal", "folders": ["/w"], "account": account,
        "created_at_ms": 1, "updated_at_ms": 1
    }))
    .unwrap()
}

fn metadata(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

#[test]
fn the_session_pins_before_its_project_and_the_project_before_the_global() {
    let projects = [
        project(
            "prj_a",
            json!({ "mode": "pinned", "account_id": "personal" }),
        ),
        project("prj_b", json!({ "mode": "global" })),
    ];
    let pin = |meta: Value| pin_of(&metadata(meta), &projects);

    assert_eq!(
        pin(json!({ "project_id": "prj_a", "claude_account": "work" })),
        Pin {
            account: Some(("work".into(), AccountSource::Session)),
            project_id: Some("prj_a".into()),
        }
    );
    assert_eq!(
        pin(json!({ "project_id": "prj_a" })),
        Pin {
            account: Some(("personal".into(), AccountSource::Project)),
            project_id: Some("prj_a".into()),
        }
    );
    // "global" follows the global account, even in a pinned project.
    assert_eq!(
        pin(json!({ "project_id": "prj_a", "claude_account": "global" })),
        Pin {
            account: None,
            project_id: Some("prj_a".into()),
        }
    );
    assert_eq!(pin(json!({ "project_id": "prj_b" })).account, None);
    assert_eq!(pin(json!({ "project_id": "none" })), Pin::default());
    // A project gone from the store pins nothing; its tokens count under
    // its id, which the split shows as no project.
    assert_eq!(
        pin(json!({ "project_id": "prj_gone" })),
        Pin {
            account: None,
            project_id: Some("prj_gone".into()),
        }
    );
    assert_eq!(pin(json!({ "claude_account": "  " })), Pin::default());
}

fn tmux(global: &str, ids: &[&str]) -> TmuxClaude {
    serde_json::from_value(json!({
        "available": true, "global": global,
        "accounts": ids.iter().map(|id| json!({ "id": id })).collect::<Vec<_>>()
    }))
    .unwrap()
}

#[test]
fn a_pin_resolves_to_its_account_or_to_unknown_and_no_pin_to_the_global() {
    let list = tmux("work", &["work", "personal"]);
    let pinned = |id: &str, source| Pin {
        account: Some((id.into(), source)),
        project_id: Some("prj_a".into()),
    };
    assert_eq!(
        pick(&list, pinned("personal", AccountSource::Project)),
        Resolved::Account(TurnAccount {
            id: "personal".into(),
            source: AccountSource::Project,
            project_id: Some("prj_a".into()),
        })
    );
    assert_eq!(
        pick(&list, pinned("gone", AccountSource::Session)),
        Resolved::Unknown(TurnAccount {
            id: "gone".into(),
            source: AccountSource::Session,
            project_id: Some("prj_a".into()),
        })
    );
    assert_eq!(
        pick(&list, Pin::default()),
        Resolved::Account(TurnAccount {
            id: "work".into(),
            source: AccountSource::Global,
            project_id: None,
        })
    );
    assert_eq!(pick(&tmux("", &[]), Pin::default()), Resolved::None);
}

#[test]
fn a_project_account_is_pinned_only_with_an_id() {
    assert_eq!(
        pinned_account(&json!({ "mode": "pinned", "account_id": "w" })),
        Some("w")
    );
    assert_eq!(
        pinned_account(&json!({ "mode": "pinned", "account_id": "" })),
        None
    );
    assert_eq!(
        pinned_account(&json!({ "mode": "global", "account_id": "w" })),
        None
    );
    assert_eq!(pinned_account(&json!({})), None);
}

#[test]
fn the_retry_gate_wants_a_failed_turn_that_is_done() {
    let code = |result: Result<(), HarnessError>| result.unwrap_err().rejected_code().unwrap();
    let mut record = crate::types::turn::tests::record();
    assert_eq!(code(retry_gate(None, S)), "nothing_to_retry");
    record.status = TurnStatus::Running;
    assert_eq!(code(retry_gate(Some(&record), S)), "busy");
    record.status = TurnStatus::AwaitingFunctions;
    assert_eq!(code(retry_gate(Some(&record), S)), "busy");
    record.status = TurnStatus::Completed;
    assert_eq!(code(retry_gate(Some(&record), S)), "nothing_to_retry");
    record.status = TurnStatus::Failed;
    assert!(retry_gate(Some(&record), S).is_ok());
    record.continues = true;
    assert_eq!(
        code(retry_gate(Some(&record), S)),
        "busy",
        "a reseed is on its way"
    );
}

#[test]
fn a_model_is_priced_under_its_anthropic_id() {
    assert_eq!(
        anthropic_model("claude-code/claude-opus-4-8"),
        "claude-opus-4-8"
    );
    assert_eq!(
        anthropic_model("claude-sonnet-4-5-20250929"),
        "claude-sonnet-4-5"
    );
    assert_eq!(
        anthropic_model("claude-code/claude-opus-4-8[1m]"),
        "claude-opus-4-8"
    );
    assert_eq!(anthropic_model("claude-haiku-4-5"), "claude-haiku-4-5");
    // Any id is safe to cut, a non-ASCII one included.
    for id in [
        "modèle-é",
        "ç",
        "ççççççççç",
        "claude-ünïcødé-x",
        "-2025092",
        "x-2025092é",
    ] {
        assert_eq!(anthropic_model(id), id);
    }
    assert_eq!(anthropic_model("modèle-20250929"), "modèle");
}

#[test]
fn a_long_prompt_counts_apart_and_prices_on_its_long_card() {
    let short = Tokens {
        input: 40_000,
        cache_read: 60_000,
        ..Tokens::default()
    };
    assert_eq!(
        ledger_model("claude-code/claude-haiku-5-5", &short),
        "claude-haiku-5-5"
    );
    let long = Tokens {
        cache_write: 1,
        ..short
    };
    assert_eq!(
        ledger_model("claude-code/claude-haiku-5-5", &long),
        "claude-haiku-5-5#long"
    );
    assert_eq!(
        ledger_model("claude-opus-4-8", &long),
        "claude-opus-4-8",
        "no long-prompt card"
    );

    let mut doc = LedgerDay {
        account: "work".into(),
        project_id: None,
        day: 0,
        hours: BTreeMap::new(),
    };
    add_to_ledger(&mut doc, 0, "claude-haiku-5-5", short);
    add_to_ledger(&mut doc, 0, "claude-haiku-5-5", long);
    let models = &doc.hours[&0];
    assert_eq!(models["claude-haiku-5-5"], short);
    assert_eq!(models["claude-haiku-5-5#long"], long);

    let prices = price_models(&models.keys().cloned().collect(), &BTreeMap::new());
    assert_eq!(prices["claude-haiku-5-5"], Price::new(0.10, 0.50, 0.01));
    assert_eq!(
        prices["claude-haiku-5-5#long"],
        Price::new(0.50, 2.50, 0.05)
    );
    // The short prompt on the short card; the long one, its cache write
    // included, on the long card. Per MTok.
    let expected =
        (40_000.0 * 0.10 + 60_000.0 * 0.01) + (40_000.0 * 0.50 + 60_000.0 * 0.05 + 0.50 * 1.25);
    assert!((cost_of(models, &prices) - expected / 1e6).abs() < 1e-12);
}

#[test]
fn a_model_without_a_list_price_takes_its_nearest_relatives() {
    let models: BTreeSet<String> = [
        "claude-opus-4-8",
        "claude-opus-4-9",
        "claude-sonnet-6",
        "claude-haiku-4-5",
        "claude-custom-1",
        "gpt-5",
        "claude-opus-7",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let catalog = BTreeMap::from([(
        "claude-haiku-4-5".to_string(),
        Some(Price::new(2.0, 8.0, 0.2)),
    )]);
    let prices = price_models(&models, &catalog);
    assert_eq!(prices["claude-opus-4-8"], Price::new(5.0, 25.0, 0.50));
    assert_eq!(
        prices["claude-opus-4-9"],
        Price::new(5.0, 25.0, 0.50),
        "4.9 is nearest 4.8"
    );
    assert_eq!(
        prices["claude-opus-7"],
        Price::new(4.0, 20.0, 0.20),
        "7 is nearest 5.5"
    );
    assert_eq!(
        prices["claude-sonnet-6"],
        Price::new(2.0, 10.0, 0.20),
        "6 is nearest 5.5"
    );
    assert_eq!(
        prices["claude-haiku-4-5"],
        Price::new(2.0, 8.0, 0.2),
        "the router's catalog wins over the built-in list"
    );
    // No family to go by: the default (Sonnet) price, never free.
    let sonnet = list_price(DEFAULT_PRICE_MODEL).unwrap();
    assert_eq!(prices["claude-custom-1"], sonnet);
    assert_eq!(prices["gpt-5"], sonnet);
}

#[test]
fn a_price_counts_every_kind_of_token() {
    let price = Price::new(5.0, 25.0, 0.5);
    let tokens = Tokens {
        input: 1_000_000,
        output: 1_000_000,
        cache_read: 1_000_000,
        cache_write: 1_000_000,
    };
    assert!((price.cost(&tokens) - (5.0 + 25.0 + 0.5 + 6.25)).abs() < 1e-9);
    let catalog = Price::from_catalog(&crate::types::model::Pricing {
        input: Some(3.0),
        output: Some(15.0),
        cache_read: Some(0.3),
        cache_write: None,
    })
    .unwrap();
    assert_eq!(
        catalog,
        Price::new(3.0, 15.0, 0.3),
        "cache writes at 1.25x input"
    );
    let bare = Price::from_catalog(&crate::types::model::Pricing {
        input: Some(10.0),
        output: None,
        cache_read: None,
        cache_write: None,
    })
    .unwrap();
    assert!(close(bare.cache_read, 1.0) && close(bare.output, 50.0));
}

fn tokens(input: u64) -> Tokens {
    Tokens {
        input,
        ..Tokens::default()
    }
}

#[test]
fn the_ledger_counts_by_hour_and_model_and_reads_back_from_a_start() {
    let day = 20_000_i64;
    let at = |hour: i64| day * DAY_MS + hour * HOUR_MS + 60_000;
    let mut doc = LedgerDay {
        account: "work".into(),
        project_id: Some("prj_a".into()),
        day,
        hours: BTreeMap::new(),
    };
    add_to_ledger(&mut doc, at(9), "claude-code/claude-opus-4-8", tokens(10));
    add_to_ledger(&mut doc, at(9), "claude-opus-4-8-20260101", tokens(5));
    add_to_ledger(&mut doc, at(15), "claude-code/claude-haiku-4-5", tokens(7));
    assert_eq!(doc.hours[&9]["claude-opus-4-8"], tokens(15));
    assert_eq!(doc.key(), format!("{day}:work:prj_a"));
    // Hours are stored as JSON object keys and read back.
    let stored = serde_json::to_value(&doc).unwrap();
    assert_eq!(serde_json::from_value::<LedgerDay>(stored).unwrap(), doc);

    let other = LedgerDay {
        account: "personal".into(),
        project_id: None,
        day,
        hours: BTreeMap::from([(9, BTreeMap::from([("claude-opus-4-8".into(), tokens(99))]))]),
    };
    let ledger = [doc, other];
    let all = chat_tokens(&ledger, "work", at(0));
    assert_eq!(all[&Some("prj_a".into())]["claude-opus-4-8"], tokens(15));
    assert_eq!(all[&Some("prj_a".into())]["claude-haiku-4-5"], tokens(7));
    assert!(
        !all.contains_key(&None),
        "another account's tokens stay out"
    );
    // From 10:00, hour 9 is before the week.
    let later = chat_tokens(&ledger, "work", day * DAY_MS + 10 * HOUR_MS);
    assert!(!later[&Some("prj_a".into())].contains_key("claude-opus-4-8"));
    assert_eq!(later[&Some("prj_a".into())]["claude-haiku-4-5"], tokens(7));
}

#[test]
fn the_week_starts_seven_days_before_its_reset() {
    let now = 100 * DAY_MS;
    let reset = (now + DAY_MS) as u64;
    let week = week_of(
        Some(&TmuxLimit {
            used_percent: 64.0,
            resets_at_ms: Some(reset),
            reset_since_read: false,
        }),
        now,
    );
    assert_eq!(
        week,
        Week {
            start_ms: now + DAY_MS - WEEK_MS,
            resets_at_ms: Some(reset),
            percent: 64.0,
        }
    );
    // Reset since tmux read it: a new week, nothing used yet.
    let past = (now - DAY_MS) as u64;
    let week = week_of(
        Some(&TmuxLimit {
            used_percent: 99.0,
            resets_at_ms: Some(past),
            reset_since_read: true,
        }),
        now,
    );
    assert_eq!(week.start_ms, now - DAY_MS);
    assert_eq!(week.resets_at_ms, Some(past + WEEK_MS as u64));
    assert_eq!(week.percent, 0.0);
    // tmux drops the reset it passed: the new week starts now, unmeasured.
    let week = week_of(
        Some(&TmuxLimit {
            used_percent: 0.0,
            resets_at_ms: None,
            reset_since_read: true,
        }),
        now,
    );
    assert_eq!(
        week,
        Week {
            start_ms: now,
            resets_at_ms: None,
            percent: 0.0,
        }
    );
    assert_eq!(week_of(None, now).start_ms, now - WEEK_MS);
}

fn percents(segments: &[SplitSegment]) -> Vec<(String, f64)> {
    segments
        .iter()
        .map(|segment| match segment {
            SplitSegment::Project { name, percent, .. } => (name.clone(), *percent),
            SplitSegment::NoProject { percent } => ("no project".into(), *percent),
            SplitSegment::Outside { percent } => ("outside".into(), *percent),
        })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn the_segments_add_up_to_the_week() {
    let a = project("prj_a", json!({ "mode": "global" }));
    let mut b = project("prj_b", json!({ "mode": "global" }));
    b.name = "mobile".into();
    let split = segments(64.0, vec![(b, 1.0), (a, 3.0)], 2.0, Some(2.0));
    let got = percents(&split);
    assert_eq!(
        got.iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["prj_a", "mobile", "no project", "outside"],
        "projects by cost, then no project, then the outside"
    );
    assert!(close(got[0].1, 24.0));
    assert!(close(got[1].1, 8.0));
    assert!(close(got[2].1, 16.0));
    assert!(close(got[3].1, 16.0));
    assert!(close(got.iter().map(|(_, p)| p).sum::<f64>(), 64.0));

    // The outside unmeasured: the Chat splits the week alone.
    let alone = percents(&segments(
        30.0,
        vec![(project("prj_a", json!({})), 1.0)],
        1.0,
        None,
    ));
    assert_eq!(alone.len(), 2);
    assert!(close(alone[0].1 + alone[1].1, 30.0));

    // Weekly use but no Chat spend: all outside.
    assert_eq!(
        segments(40.0, Vec::new(), 0.0, Some(0.0)),
        vec![SplitSegment::Outside { percent: 40.0 }]
    );
    assert!(segments(40.0, Vec::new(), 0.0, None).is_empty());
    assert!(segments(0.0, Vec::new(), 0.0, Some(0.0)).is_empty());
}

// ── over a fake engine ──

#[derive(Default)]
struct Data {
    state: BTreeMap<(String, String), Value>,
    sessions: BTreeMap<String, Value>,
    entries: BTreeMap<String, Vec<Value>>,
    calls: Vec<(String, Value)>,
    /// `tmux::accounts::list`'s answer; `None`: no tmux worker.
    tmux: Option<Value>,
    /// tmux is there but `tmux::accounts::list` fails (a timeout, say).
    tmux_fails: bool,
    /// `tmux::usage::claude_outside`'s answers by account; a missing one fails.
    outside: BTreeMap<String, Value>,
}

impl Data {
    fn get(&self, scope: &str, key: &str) -> Value {
        self.state
            .get(&(scope.into(), key.into()))
            .cloned()
            .unwrap_or(Value::Null)
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
        let missing = || {
            Err((
                "function_not_found".to_string(),
                format!("Function {function} not found"),
            ))
        };
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
            "session::patch-meta" => {
                let meta = self
                    .sessions
                    .get_mut(&sid)
                    .ok_or(("session/not_found".to_string(), sid.clone()))?;
                let metadata = meta["metadata"].as_object_mut().unwrap();
                for (key, value) in p["set"].as_object().into_iter().flatten() {
                    metadata.insert(key.clone(), value.clone());
                }
                json!({ "meta": meta })
            }
            "session::append" => {
                let entries = self.entries.entry(sid).or_default();
                entries.push(json!({ "entry_id": p["entry_id"], "message": p["message"] }));
                json!({ "entry_id": p["entry_id"] })
            }
            "tmux::accounts::list" if self.tmux_fails => {
                return Err(("timeout".to_string(), "tmux did not answer".to_string()));
            }
            "tmux::accounts::list" => match &self.tmux {
                Some(list) => list.clone(),
                None => return missing(),
            },
            "tmux::usage::read" => match &self.tmux {
                Some(list) => json!({ "claude": list, "generated_at_ms": 1 }),
                None => return missing(),
            },
            "tmux::usage::claude_outside" => {
                if self.tmux.is_none() {
                    return missing();
                }
                let account = p["account"].as_str().unwrap_or_default();
                match self.outside.get(account) {
                    Some(reading) => reading.clone(),
                    None => {
                        return Err((
                            "tmux/unreadable".to_string(),
                            "transcripts unreadable".to_string(),
                        ));
                    }
                }
            }
            "router::models::get" => json!({ "model": null }),
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

impl Fake {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let data = Arc::new(Mutex::new(Data::default()));
        let shared = data.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let hello = json!({ "type": "workerregistered", "worker_id": "accounts-offline",
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

    /// Two tmux accounts, `work` global; a session in `metadata`.
    async fn with_accounts(metadata: Value) -> Self {
        let fake = Self::new().await;
        fake.tmux(Some(json!({
            "available": true, "global": "work",
            "accounts": [
                { "id": "work", "usage": { "week": {
                    "used_percent": 60.0, "resets_at_ms": AgentMessage::now_ms() + DAY_MS,
                    "reset_since_read": false } } },
                { "id": "personal" }
            ]
        })));
        fake.session(S, metadata);
        fake
    }

    fn tmux(&self, list: Option<Value>) {
        self.data.lock().unwrap().tmux = list;
    }

    fn tmux_fails(&self, fails: bool) {
        self.data.lock().unwrap().tmux_fails = fails;
    }

    fn session(&self, id: &str, metadata: Value) {
        self.data.lock().unwrap().sessions.insert(
            id.into(),
            json!({ "session_id": id, "title": "t", "status": "idle", "metadata": metadata,
                    "message_count": 0, "created_at": 1, "updated_at": 1 }),
        );
    }

    async fn project(&self, account: Value) -> String {
        let path = self.deps.cfg().await.projects_file_path.clone();
        self.deps
            .projects
            .create(
                &path,
                crate::projects::NewProject {
                    name: "acme".into(),
                    color: "teal".into(),
                    folders: vec!["/w/acme".into()],
                    account: Some(account),
                    autocompact: None,
                },
                None,
            )
            .await
            .unwrap()
            .project
            .id
    }

    async fn resolve(&self, record: &mut TurnRecord) -> TurnResolution {
        let session = self.deps.session().await;
        resolve_turn(&self.deps, &session, record).await.unwrap()
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
}

/// A new `claude-code` turn of [`S`] that inherited the previous turn's
/// options, an account among them.
fn claude_turn() -> TurnRecord {
    let mut record = crate::types::turn::tests::record();
    record.session_id = S.into();
    record.turn_id = "t_new".into();
    record.status = TurnStatus::Running;
    record.step = 0;
    record.turn_count = 0;
    record.options.provider = Some("claude-code".into());
    record.options.model = "claude-code/claude-opus-4-8".into();
    record.options.provider_options = Some(BTreeMap::from([(
        "claude-code".to_string(),
        json!({ "account": "inherited", "effort": "high" }),
    )]));
    record
}

fn turn_account(id: &str, source: AccountSource, project_id: Option<&str>) -> TurnAccount {
    TurnAccount {
        id: id.into(),
        source,
        project_id: project_id.map(str::to_string),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_runs_on_the_global_account_and_overwrites_an_inherited_one() {
    let fake = Fake::with_accounts(json!({})).await;
    let mut record = claude_turn();
    assert_eq!(fake.resolve(&mut record).await, TurnResolution::Changed);
    assert_eq!(
        record.account,
        Some(turn_account("work", AccountSource::Global, None))
    );
    assert_eq!(
        record.options.provider_options.as_ref().unwrap()["claude-code"],
        json!({ "account": "work", "effort": "high" })
    );
    // Frozen: a second pass (a redelivered first step) changes nothing.
    fake.tmux(Some(json!({ "available": true, "global": "personal",
        "accounts": [{ "id": "work" }, { "id": "personal" }] })));
    assert_eq!(fake.resolve(&mut record).await, TurnResolution::Unchanged);
    assert_eq!(record.account.as_ref().unwrap().id, "work");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pinned_project_and_then_the_session_decide_the_account() {
    let fake = Fake::with_accounts(json!({})).await;
    let project = fake
        .project(json!({ "mode": "pinned", "account_id": "personal" }))
        .await;
    fake.session(S, json!({ "project_id": project }));
    let mut record = claude_turn();
    fake.resolve(&mut record).await;
    assert_eq!(
        record.account,
        Some(turn_account(
            "personal",
            AccountSource::Project,
            Some(&project)
        ))
    );

    fake.session(
        S,
        json!({ "project_id": project, "claude_account": "work" }),
    );
    let mut record = claude_turn();
    fake.resolve(&mut record).await;
    assert_eq!(
        record.account,
        Some(turn_account("work", AccountSource::Session, Some(&project)))
    );
    assert_eq!(
        account_option(record.options.provider_options.as_ref()),
        Some("work")
    );

    fake.session(
        S,
        json!({ "project_id": project, "claude_account": "global" }),
    );
    let mut record = claude_turn();
    fake.resolve(&mut record).await;
    assert_eq!(
        record.account,
        Some(turn_account("work", AccountSource::Global, Some(&project)))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pin_to_a_removed_account_is_unknown_and_sends_no_account() {
    let fake = Fake::with_accounts(json!({ "claude_account": "old" })).await;
    let mut record = claude_turn();
    assert_eq!(fake.resolve(&mut record).await, TurnResolution::Unknown);
    assert_eq!(
        record.account,
        Some(turn_account("old", AccountSource::Session, None))
    );
    assert_eq!(
        account_option(record.options.provider_options.as_ref()),
        None
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_tmux_no_account_is_set_and_an_inherited_one_goes() {
    let fake = Fake::with_accounts(json!({ "claude_account": "work" })).await;
    fake.tmux(None);
    let mut record = claude_turn();
    assert_eq!(fake.resolve(&mut record).await, TurnResolution::Changed);
    assert_eq!(record.account, None);
    assert_eq!(
        record.options.provider_options.as_ref().unwrap()["claude-code"],
        json!({ "effort": "high" })
    );
    // Nothing left to change on the next pass.
    assert_eq!(fake.resolve(&mut record).await, TurnResolution::Unchanged);

    // Claude Code missing on tmux's computer reads the same.
    fake.tmux(Some(
        json!({ "available": false, "global": "default", "accounts": [] }),
    ));
    let mut record = claude_turn();
    fake.resolve(&mut record).await;
    assert_eq!(record.account, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_provider_never_keeps_an_inherited_account() {
    let fake = Fake::with_accounts(json!({})).await;
    let mut record = claude_turn();
    record.options.provider = Some("anthropic".into());
    record.options.model = "claude-opus-4-8".into();
    assert_eq!(fake.resolve(&mut record).await, TurnResolution::Changed);
    assert_eq!(record.account, None);
    assert_eq!(
        account_option(record.options.provider_options.as_ref()),
        None
    );
    assert!(fake.calls_to("tmux::accounts::list").is_empty());
}

fn child_of(parent: &TurnRecord, inherited: &str) -> TurnRecord {
    let mut child = claude_turn();
    child.session_id = "s_child".into();
    child.depth = 1;
    child.parent = Some(ParentLink {
        session_id: parent.session_id.clone(),
        turn_id: parent.turn_id.clone(),
        function_call_id: "fc_1".into(),
    });
    set_account_option(&mut child.options.provider_options, inherited);
    child
}

fn parent_turn(account: Option<TurnAccount>) -> TurnRecord {
    let mut parent = claude_turn();
    parent.session_id = "s_parent".into();
    parent.account = account;
    parent
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sub_agent_runs_on_its_parents_account() {
    let fake = Fake::with_accounts(json!({})).await;
    fake.session("s_child", json!({}));
    let parent = parent_turn(Some(turn_account(
        "personal",
        AccountSource::Project,
        Some("prj_x"),
    )));
    crate::state::put_turn(&fake.deps.iii, &parent, TIMEOUT)
        .await
        .unwrap();
    let mut child = child_of(&parent, "personal");
    fake.resolve(&mut child).await;
    assert_eq!(child.account, parent.account);
    assert_eq!(
        account_option(child.options.provider_options.as_ref()),
        Some("personal")
    );

    // tmux gone since: no account goes to the provider, the inherited one
    // is dropped (the provider fails a pinned id it cannot run).
    fake.tmux(None);
    let mut child = child_of(&parent, "personal");
    assert_eq!(fake.resolve(&mut child).await, TurnResolution::Changed);
    assert_eq!(child.account, None);
    assert_eq!(
        account_option(child.options.provider_options.as_ref()),
        None
    );

    // The parent's account removed from tmux: the child fails like a pin.
    fake.tmux(Some(json!({ "available": true, "global": "work",
        "accounts": [{ "id": "work" }] })));
    let mut child = child_of(&parent, "personal");
    assert_eq!(fake.resolve(&mut child).await, TurnResolution::Unknown);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sub_agent_follows_its_parents_current_account_after_a_switch() {
    let fake = Fake::with_accounts(json!({})).await;
    fake.session("s_child", json!({}));
    // Spawned on `personal`; the global moved to `work` since, and the
    // parent's current turn (the one waking the child) runs on it.
    let parent = parent_turn(Some(turn_account("work", AccountSource::Global, None)));
    crate::state::put_turn(&fake.deps.iii, &parent, TIMEOUT)
        .await
        .unwrap();
    let mut child = child_of(&parent, "personal");
    assert_eq!(fake.resolve(&mut child).await, TurnResolution::Changed);
    assert_eq!(
        child.account,
        Some(turn_account("work", AccountSource::Global, None))
    );
    assert_eq!(
        account_option(child.options.provider_options.as_ref()),
        Some("work")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sub_agent_keeps_the_inherited_id_only_when_its_parent_has_no_turn() {
    let fake = Fake::with_accounts(json!({})).await;
    fake.session("s_child", json!({}));
    let parent = parent_turn(None);
    let mut child = child_of(&parent, "personal");
    fake.resolve(&mut child).await;
    assert_eq!(
        child.account,
        Some(turn_account("personal", AccountSource::Session, None))
    );

    // A parent turn with no account (another provider): the child resolves
    // as a conversation of its own, on the global.
    crate::state::put_turn(&fake.deps.iii, &parent, TIMEOUT)
        .await
        .unwrap();
    let mut child = child_of(&parent, "personal");
    fake.resolve(&mut child).await;
    assert_eq!(
        child.account,
        Some(turn_account("work", AccountSource::Global, None))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tmux_that_cannot_answer_leaves_the_turn_unresolved() {
    let fake = Fake::with_accounts(json!({})).await;
    fake.tmux_fails(true);
    let mut record = claude_turn();
    let before = record.clone();
    assert!(matches!(
        fake.resolve(&mut record).await,
        TurnResolution::Unavailable(detail) if detail.contains("tmux did not answer")
    ));
    assert_eq!(record, before, "nothing is decided, nothing is dropped");

    // Once tmux answers, the same turn resolves.
    fake.tmux_fails(false);
    assert_eq!(fake.resolve(&mut record).await, TurnResolution::Changed);
    assert_eq!(record.account.as_ref().unwrap().id, "work");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_whose_account_cannot_be_decided_resumes_then_fails_retryable() {
    let fake = Fake::with_accounts(json!({})).await;
    fake.tmux_fails(true);
    let mut record = claude_turn();
    record.options.max_transient_resumes = 1;
    crate::state::put_turn(&fake.deps.iii, &record, TIMEOUT)
        .await
        .unwrap();
    let step = || crate::turn_loop::TurnStepPayload {
        session_id: S.into(),
        turn_id: record.turn_id.clone(),
        step: 0,
        message_preview: None,
        depth: 0,
    };

    // First: one transient resume, the first step queued again.
    let first = crate::turn_loop::run_step(&fake.deps, step())
        .await
        .unwrap();
    assert_eq!(first.status, TurnStatus::Running);
    assert_eq!(first.next_step, Some(0));
    let stored = crate::state::get_turn_unhydrated(&fake.deps.iii, S, TIMEOUT)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.transient_resumes, 1);
    assert_eq!(stored.account, None);
    let queued = fake.calls_to("harness::turn");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0]["step"], 0);
    let appended = fake.calls_to("session::append");
    assert!(
        appended.iter().any(|entry| {
            entry["entry_id"] == "e_t_new_transient_recovery_1_recovering"
                && entry.to_string().contains(ACCOUNTS_UNAVAILABLE_CODE)
        }),
        "a recovery entry says why: {appended:?}"
    );

    // The resumes spent: a retryable failure.
    let second = crate::turn_loop::run_step(&fake.deps, step())
        .await
        .unwrap();
    assert_eq!(second.status, TurnStatus::Failed);
    let stored = crate::state::get_turn_unhydrated(&fake.deps.iii, S, TIMEOUT)
        .await
        .unwrap()
        .unwrap();
    let failure = stored.failure.clone().unwrap();
    assert_eq!(failure.code, ACCOUNTS_UNAVAILABLE_CODE);
    assert!(failure.retryable);
    assert!(
        retry_gate(Some(&stored), S).is_ok(),
        "harness::retry takes it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreadable_projects_leave_a_project_conversation_unresolved() {
    let fake = Fake::with_accounts(json!({ "project_id": "prj_a" })).await;
    let path = fake.deps.cfg().await.projects_file_path.clone();
    std::fs::write(&path, b"{ not json").unwrap();
    let mut record = claude_turn();
    assert!(matches!(
        fake.resolve(&mut record).await,
        TurnResolution::Unavailable(detail) if detail.contains("projects")
    ));
    assert!(record.account.is_none());

    // A conversation with an account of its own never reads its project.
    fake.session(
        S,
        json!({ "project_id": "prj_a", "claude_account": "personal" }),
    );
    let mut record = claude_turn();
    fake.resolve(&mut record).await;
    assert_eq!(
        record.account,
        Some(turn_account(
            "personal",
            AccountSource::Session,
            Some("prj_a")
        ))
    );
}

fn side_lock(account: Option<TurnAccount>) -> crate::side_chat::SideChatLock {
    let mut options = claude_turn().options;
    if let Some(account) = &account {
        set_account_option(&mut options.provider_options, &account.id);
    } else if let Some(map) = options.provider_options.as_mut() {
        strip_account_option(map);
    }
    serde_json::from_value(json!({
        "session_id": "s_side", "parent_session_id": S, "model": options.model,
        "provider": options.provider, "provider_options": options.provider_options,
        "account": account, "created_at_ms": 1, "source": "open"
    }))
    .unwrap()
}

async fn save_side_lock(fake: &Fake, lock: &crate::side_chat::SideChatLock) {
    crate::state::state_set(
        &fake.deps.iii,
        "harness_side_chat",
        "s_side",
        serde_json::to_value(lock).unwrap(),
        TIMEOUT,
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_chat_runs_on_its_locks_account() {
    let fake = Fake::with_accounts(json!({ "claude_account": "work" })).await;
    fake.session("s_side", json!({ "side_chat_of": S }));
    save_side_lock(
        &fake,
        &side_lock(Some(turn_account("personal", AccountSource::Project, None))),
    )
    .await;
    let mut record = claude_turn();
    record.session_id = "s_side".into();
    fake.resolve(&mut record).await;
    assert_eq!(
        record.account,
        Some(turn_account("personal", AccountSource::Project, None)),
        "the lock's, not the conversation's current one"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_chat_keeps_its_locks_account_when_tmux_cannot_answer() {
    let fake = Fake::with_accounts(json!({})).await;
    fake.session("s_side", json!({ "side_chat_of": S }));
    let locked = turn_account("personal", AccountSource::Session, None);
    save_side_lock(&fake, &side_lock(Some(locked.clone()))).await;
    fake.tmux_fails(true);
    let mut record = claude_turn();
    record.session_id = "s_side".into();
    fake.resolve(&mut record).await;
    assert_eq!(record.account, Some(locked));
    assert_eq!(
        account_option(record.options.provider_options.as_ref()),
        Some("personal")
    );

    // No tmux at all: the side chat runs with no account, the lock's dropped
    // from the options sent.
    fake.tmux_fails(false);
    fake.tmux(None);
    let mut record = claude_turn();
    record.session_id = "s_side".into();
    fake.resolve(&mut record).await;
    assert_eq!(record.account, None);
    assert_eq!(
        account_option(record.options.provider_options.as_ref()),
        None
    );

    // An old lock with no account has nothing to fall back on.
    fake.tmux(Some(json!({ "available": true, "global": "work",
        "accounts": [{ "id": "work" }, { "id": "personal" }] })));
    fake.tmux_fails(true);
    save_side_lock(&fake, &side_lock(None)).await;
    let mut record = claude_turn();
    record.session_id = "s_side".into();
    assert!(matches!(
        fake.resolve(&mut record).await,
        TurnResolution::Unavailable(_)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_old_side_chat_lock_takes_the_conversations_resolution_and_keeps_it() {
    let fake = Fake::with_accounts(json!({ "claude_account": "personal" })).await;
    fake.session("s_side", json!({ "side_chat_of": S }));
    save_side_lock(&fake, &side_lock(None)).await;
    let mut record = claude_turn();
    record.session_id = "s_side".into();
    fake.resolve(&mut record).await;
    let expected = turn_account("personal", AccountSource::Session, None);
    assert_eq!(record.account, Some(expected.clone()));

    let lock = crate::side_chat::load_lock(&fake.deps, "s_side")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lock.account, Some(expected));
    assert_eq!(
        account_option(lock.provider_options.as_ref()),
        Some("personal")
    );
    let shown = fake.data.lock().unwrap().sessions["s_side"]["metadata"]["side_chat_lock"].clone();
    assert_eq!(shown["account"], "personal");
}

fn failed_turn(partial: bool) -> TurnRecord {
    let mut record = claude_turn();
    record.turn_id = "t_failed".into();
    record.status = TurnStatus::Failed;
    record.step = 2;
    record.turn_count = 1;
    record.message_preview = Some("Fix the webhook".into());
    record.account = Some(turn_account("work", AccountSource::Global, None));
    set_account_option(&mut record.options.provider_options, "work");
    record.result_error = Some("This Claude Code account reached its usage limit.".into());
    record.failure = Some(crate::types::turn::TurnFailure {
        code: USAGE_LIMIT_CODE.into(),
        retryable: false,
        details: None,
    });
    if partial {
        record.result = Some(json!("Half of the answer"));
    }
    record
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_starts_a_turn_from_the_transcript_with_no_account_yet() {
    let fake = Fake::with_accounts(json!({})).await;
    crate::state::put_turn(&fake.deps.iii, &failed_turn(false), TIMEOUT)
        .await
        .unwrap();
    let retried = retry(
        &fake.deps,
        RetryRequest {
            session_id: S.into(),
        },
    )
    .await
    .unwrap();
    assert_ne!(retried.turn_id, "t_failed");
    let record = crate::state::get_turn(&fake.deps.iii, S, TIMEOUT)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.turn_id, retried.turn_id);
    assert_eq!(record.status, TurnStatus::Running);
    assert_eq!((record.step, record.turn_count), (0, 0));
    assert_eq!(record.failure, None);
    assert_eq!(record.account, None, "its first step resolves anew");
    assert_eq!(
        record.options.provider_options.as_ref().unwrap()["claude-code"],
        json!({ "effort": "high" })
    );
    assert_eq!(record.message_preview.as_deref(), Some("Fix the webhook"));
    assert!(
        fake.calls_to("session::append").is_empty(),
        "no new user message"
    );

    // The new turn runs: no second retry, nothing to retry when it completes.
    let busy = retry(
        &fake.deps,
        RetryRequest {
            session_id: S.into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(busy.rejected_code().as_deref(), Some("busy"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_runs_on_the_options_a_plain_send_would_take() {
    let fake = Fake::with_accounts(json!({})).await;
    let cfg = fake.deps.cfg().await;
    let mut failed = failed_turn(false);
    // What the failed turn's own send asked, beyond the session's options.
    failed.options.max_turns = 3;
    failed.options.max_transient_resumes = 0;
    failed.options.max_output_tokens = Some(512);
    failed.options.metadata = Some(json!({ "fs_scope": { "root": "/w/acme" }, "trace": "x" }));
    failed.options.thinking_level = Some(crate::types::model::ThinkingLevel::High);
    crate::state::put_turn(&fake.deps.iii, &failed, TIMEOUT)
        .await
        .unwrap();
    retry(
        &fake.deps,
        RetryRequest {
            session_id: S.into(),
        },
    )
    .await
    .unwrap();
    let record = crate::state::get_turn(&fake.deps.iii, S, TIMEOUT)
        .await
        .unwrap()
        .unwrap();
    let options = &record.options;
    assert_eq!(options.model, failed.options.model);
    assert_eq!(options.provider, failed.options.provider);
    assert_eq!(options.thinking_level, failed.options.thinking_level);
    assert_eq!(options.system_prompt, failed.options.system_prompt);
    assert_eq!(options.functions, failed.options.functions);
    assert_eq!(options.max_turns, cfg.default_max_turns);
    assert_eq!(options.max_transient_resumes, cfg.max_transient_resumes);
    assert_eq!(options.max_output_tokens, None);
    assert_eq!(
        options.metadata,
        Some(json!({ "fs_scope": { "root": "/w/acme" } })),
        "only the working folder outlives its send"
    );
    assert_eq!(account_option(options.provider_options.as_ref()), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sub_agents_retry_resolves_from_its_parents_current_turn() {
    let fake = Fake::with_accounts(json!({})).await;
    fake.session("s_child", json!({}));
    let parent = parent_turn(Some(turn_account("personal", AccountSource::Session, None)));
    crate::state::put_turn(&fake.deps.iii, &parent, TIMEOUT)
        .await
        .unwrap();
    let mut failed = failed_turn(false);
    failed.session_id = "s_child".into();
    failed.depth = 1;
    failed.parent = child_of(&parent, "work").parent;
    crate::state::put_turn(&fake.deps.iii, &failed, TIMEOUT)
        .await
        .unwrap();
    retry(
        &fake.deps,
        RetryRequest {
            session_id: "s_child".into(),
        },
    )
    .await
    .unwrap();
    let mut record = crate::state::get_turn(&fake.deps.iii, "s_child", TIMEOUT)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.parent, failed.parent);
    assert_eq!(record.account, None);
    fake.resolve(&mut record).await;
    assert_eq!(
        record.account, parent.account,
        "not the `work` it failed on"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_resumes_from_partial_output_with_the_nudge() {
    let fake = Fake::with_accounts(json!({})).await;
    crate::state::put_turn(&fake.deps.iii, &failed_turn(true), TIMEOUT)
        .await
        .unwrap();
    let retried = retry(
        &fake.deps,
        RetryRequest {
            session_id: S.into(),
        },
    )
    .await
    .unwrap();
    let appended = fake.calls_to("session::append");
    assert_eq!(appended.len(), 1);
    assert_eq!(
        appended[0]["entry_id"],
        crate::ids::transient_resume_nudge_entry_id(&retried.turn_id, 1)
    );
    let text = appended[0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("Resume from the transcript"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_refuses_a_turn_that_did_not_fail() {
    let fake = Fake::with_accounts(json!({})).await;
    let missing = retry(
        &fake.deps,
        RetryRequest {
            session_id: S.into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(missing.rejected_code().as_deref(), Some("nothing_to_retry"));
    let mut done = failed_turn(false);
    done.status = TurnStatus::Completed;
    done.failure = None;
    crate::state::put_turn(&fake.deps.iii, &done, TIMEOUT)
        .await
        .unwrap();
    let completed = retry(
        &fake.deps,
        RetryRequest {
            session_id: S.into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        completed.rejected_code().as_deref(),
        Some("nothing_to_retry")
    );
}

fn usage_message(model: &str, input: u64, output: u64) -> AssistantMessage {
    let mut message = crate::types::message::empty_assistant("claude-code", model);
    message.usage = Some(crate::types::event::Usage {
        input: Some(input),
        output: Some(output),
        ..Default::default()
    });
    message
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_ledger_and_the_outside_split_the_accounts_week() {
    let fake = Fake::with_accounts(json!({})).await;
    let project = fake.project(json!({ "mode": "global" })).await;
    let mut in_project = claude_turn();
    in_project.account = Some(turn_account("work", AccountSource::Global, Some(&project)));
    let mut no_project = claude_turn();
    no_project.account = Some(turn_account("work", AccountSource::Global, None));
    // Opus at 5/25 per MTok: 1M in + 0 out = $5, twice in the project.
    let opus = usage_message("claude-code/claude-opus-4-8", 1_000_000, 0);
    record_usage(&fake.deps, &in_project, &opus).await;
    record_usage(&fake.deps, &in_project, &opus).await;
    record_usage(&fake.deps, &no_project, &opus).await;
    // No account (no tmux, or another provider): not counted.
    record_usage(&fake.deps, &claude_turn(), &opus).await;
    fake.data.lock().unwrap().outside.insert(
        "work".into(),
        json!({ "account": "work", "since_ms": 0, "as_of_ms": 1, "attributed": true,
                "by_model": [{ "model": "claude-opus-4-8-20260101", "input": 1_000_000,
                               "output": 0, "cache_read": 0, "cache_write": 0,
                               "messages": 3 }] }),
    );

    let split = account_split(&fake.deps, AccountSplitRequest {})
        .await
        .unwrap();
    assert_eq!(split.len(), 2);
    let work = &split[0];
    assert_eq!(work.account, "work");
    assert_eq!(work.week_percent, 60.0);
    assert!(work.outside_measured);
    let got = percents(&work.segments);
    assert_eq!(
        got.iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["acme", "no project", "outside"]
    );
    assert!(close(got[0].1, 30.0), "{got:?}");
    assert!(close(got[1].1, 15.0), "{got:?}");
    assert!(close(got[2].1, 15.0), "{got:?}");
    if let SplitSegment::Project {
        project_id, color, ..
    } = &work.segments[0]
    {
        assert_eq!(
            (project_id.as_str(), color.as_str()),
            (project.as_str(), "teal")
        );
    }
    // `personal`: no usage reading, no Chat spend, its outside unreadable.
    let personal = &split[1];
    assert_eq!(personal.week_percent, 0.0);
    assert!(!personal.outside_measured);
    assert!(personal.segments.is_empty());

    // The outside unreadable: the Chat splits the week alone.
    fake.data.lock().unwrap().outside.clear();
    let split = account_split(&fake.deps, AccountSplitRequest {})
        .await
        .unwrap();
    let got = percents(&split[0].segments);
    assert!(!split[0].outside_measured);
    assert_eq!(got.len(), 2);
    assert!(close(got[0].1, 40.0) && close(got[1].1, 20.0), "{got:?}");

    // The week reset since tmux read it, and tmux dropped the reset: a week
    // that starts now, with nothing used and nothing to split.
    let before = AgentMessage::now_ms();
    fake.tmux(Some(
        json!({ "available": true, "global": "work", "accounts": [
        { "id": "work", "usage": { "week": {
            "used_percent": 0.0, "resets_at_ms": null, "reset_since_read": true } } }
    ] }),
    ));
    let split = account_split(&fake.deps, AccountSplitRequest {})
        .await
        .unwrap();
    assert_eq!(split[0].week_percent, 0.0);
    assert_eq!(split[0].resets_at_ms, None);
    assert!(split[0].segments.is_empty());
    let since = fake.calls_to("tmux::usage::claude_outside").last().unwrap()["since_ms"]
        .as_i64()
        .unwrap();
    assert!(since >= before, "no outside window of the week before");

    // No tmux: no accounts to split.
    fake.tmux(None);
    assert!(
        account_split(&fake.deps, AccountSplitRequest {})
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_ledger_forgets_days_past_its_window() {
    let fake = Fake::with_accounts(json!({})).await;
    let today = AgentMessage::now_ms().div_euclid(DAY_MS);
    let old = LedgerDay {
        account: "work".into(),
        project_id: None,
        day: today - LEDGER_DAYS,
        hours: BTreeMap::from([(1, BTreeMap::from([("claude-opus-4-8".into(), tokens(1))]))]),
    };
    // Today and the 34 days before it: 35 days.
    let kept = LedgerDay {
        day: today - (LEDGER_DAYS - 1),
        ..old.clone()
    };
    for doc in [&old, &kept] {
        crate::state::state_set(
            &fake.deps.iii,
            LEDGER_SCOPE,
            &doc.key(),
            serde_json::to_value(doc).unwrap(),
            TIMEOUT,
        )
        .await
        .unwrap();
    }
    // An unreadable document is skipped, not deleted, and fails nothing.
    crate::state::state_set(
        &fake.deps.iii,
        LEDGER_SCOPE,
        "garbled",
        json!({ "day": "?" }),
        TIMEOUT,
    )
    .await
    .unwrap();
    let ledger = load_ledger(&fake.deps, AgentMessage::now_ms())
        .await
        .unwrap();
    assert_eq!(ledger, vec![kept.clone()]);
    let data = fake.data.lock().unwrap();
    assert!(data.get(LEDGER_SCOPE, &old.key()).is_null());
    assert!(!data.get(LEDGER_SCOPE, &kept.key()).is_null());
    assert!(!data.get(LEDGER_SCOPE, "garbled").is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_ledger_day_is_moved_aside_not_counted_over() {
    let fake = Fake::with_accounts(json!({})).await;
    let mut record = claude_turn();
    record.account = Some(turn_account("work", AccountSource::Global, None));
    let day = AgentMessage::now_ms().div_euclid(DAY_MS);
    let key = ledger_key(day, "work", None);
    let garbled = json!({ "account": "work", "day": day, "hours": "lost" });
    crate::state::state_set(&fake.deps.iii, LEDGER_SCOPE, &key, garbled.clone(), TIMEOUT)
        .await
        .unwrap();
    record_usage(
        &fake.deps,
        &record,
        &usage_message("claude-opus-4-8", 10, 5),
    )
    .await;
    let data = fake.data.lock().unwrap();
    let aside: Vec<_> = data
        .state
        .iter()
        .filter(|((scope, _), _)| scope == LEDGER_UNREADABLE_SCOPE)
        .collect();
    assert_eq!(aside.len(), 1);
    assert!(aside[0].0.1.starts_with(&format!("{key}@")));
    assert_eq!(aside[0].1, &garbled);
    let doc: LedgerDay = serde_json::from_value(data.get(LEDGER_SCOPE, &key)).unwrap();
    assert_eq!(
        doc.hours.values().next().unwrap()["claude-opus-4-8"],
        Tokens {
            input: 10,
            output: 5,
            ..Tokens::default()
        }
    );
}
