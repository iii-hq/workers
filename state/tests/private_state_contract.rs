//! Real SDK-registered state handlers and KvStoreAdapter on an isolated wire
//! peer. The peer emulates routing/caller lookup, not state authorization.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::{IIIClient, InitOptions, register_worker};
use iii_state::adapters::{KvStoreAdapter, StateAdapter};
use iii_state::config::StateConfig;
use iii_state::events::Invoker;
use iii_state::functions::{self, PrivateNamespaces, StateCtx};
use iii_state::trigger::{StateTriggerEntry, StateTriggerHandler, StateTriggerSpec};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

struct NoFanOut;
#[async_trait::async_trait]
impl Invoker for NoFanOut {
    async fn call(&self, _: &str, _: Value, _: Option<Value>) -> Result<Value, String> {
        panic!("private state must never fan out");
    }
}

struct Peer {
    iii: Arc<IIIClient>,
    socket: WebSocketStream<TcpStream>,
    registrations: BTreeMap<String, Value>,
    ctx: Arc<StateCtx>,
    calls: usize,
    last_frame_bytes: usize,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.iii.shutdown();
    }
}
impl Peer {
    async fn new(adapter: Arc<dyn StateAdapter>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let iii = Arc::new(register_worker(
            &format!("ws://{}", listener.local_addr().unwrap()),
            InitOptions::default(),
        ));
        let (tcp, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let triggers = StateTriggerHandler::new().triggers;
        triggers.write().await.insert(
            "catchall".into(),
            StateTriggerEntry {
                config: StateTriggerSpec {
                    scope: None,
                    key: None,
                    condition_function_id: None,
                },
                function_id: "must-not-fire".into(),
                metadata: None,
            },
        );
        let ctx = Arc::new(StateCtx {
            pages: Default::default(),
            adapter,
            triggers,
            config: Arc::new(RwLock::new(Arc::new(StateConfig::default()))),
            invoker: Arc::new(NoFanOut),
            private: Arc::new(PrivateNamespaces::default()),
        });
        functions::restore_persisted_claims(&iii, &ctx)
            .await
            .unwrap();
        functions::register_functions(&iii, ctx.clone());
        Self {
            iii,
            socket,
            registrations: BTreeMap::new(),
            ctx,
            calls: 0,
            last_frame_bytes: 0,
        }
    }
    async fn call(&mut self, function: &str, mut data: Value) -> Value {
        if function.ends_with("::list_entries") && data.get("_caller_worker_id").is_none() {
            data["_caller_worker_id"] = json!("owner-id");
        }
        self.calls += 1;
        let id = uuid::Uuid::new_v4().to_string();
        self.socket
            .send(Message::Text(
                json!({"type":"invokefunction", "invocation_id":id,
            "function_id":function, "data":data})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = self.socket.next().await.unwrap().unwrap();
                let Ok(text) = frame.to_text() else { continue; };
                let Ok(message) = serde_json::from_str::<Value>(text) else { continue; };
                match message["type"].as_str() {
                    Some("registerfunction") => {
                        self.registrations.insert(message["id"].as_str().unwrap().to_owned(), message);
                    }
                    Some("invokefunction") => {
                        if message["function_id"] == "engine::workers::register" {
                            self.socket.send(Message::Text(json!({"type":"invocationresult",
                                "invocation_id":message["invocation_id"], "function_id":message["function_id"],
                                "result":{"worker_id":"state-test"}}).to_string().into())).await.unwrap();
                            continue;
                        }
                        assert_eq!(message["function_id"], "engine::workers::list");
                        self.socket.send(Message::Text(json!({"type":"invocationresult",
                            "invocation_id":message["invocation_id"], "function_id":message["function_id"],
                            "result":{"workers":[{"id":"owner-id", "name":"harness"},
                                {"id":"other-id", "name":"other"}]}}).to_string().into())).await.unwrap();
                    }
                    Some("invocationresult") if message["invocation_id"] == id => { self.last_frame_bytes=text.len(); return message; },
                    _ => {}
                }
            }
        }).await.expect("real handler must answer")
    }
    async fn result(&mut self, function: &str, data: Value) -> Value {
        let reply = self.call(function, data).await;
        assert!(reply["error"].is_null(), "{reply}");
        reply["result"].clone()
    }
    async fn error(&mut self, function: &str, data: Value, code: &str) {
        let reply = self.call(function, data).await;
        assert!(
            reply["error"]["message"].as_str().unwrap().contains(code),
            "{reply}"
        );
        assert!(
            reply["result"].is_null(),
            "must not expose private keys: {reply}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_key_listing_uses_real_handlers_contracts_and_restored_namespace() {
    let adapter: Arc<dyn StateAdapter> = Arc::new(KvStoreAdapter::new(Some(
        json!({"store_method":"in_memory"}),
    )));
    let mut peer = Peer::new(adapter.clone()).await;
    let scope = "harness_deletion_dispatch";
    let claim = json!({"functions_prefix":"harness", "scopes":[scope]});
    peer.error("state::claim-namespace", claim.clone(), "UNKNOWN_CALLER")
        .await;
    let mut wrong = claim.clone();
    wrong["_caller_worker_id"] = json!("other-id");
    peer.error("state::claim-namespace", wrong, "FORBIDDEN")
        .await;
    let mut owned = claim;
    owned["_caller_worker_id"] = json!("owner-id");
    let claimed = peer.result("state::claim-namespace", owned.clone()).await;
    assert!(
        claimed["functions"]
            .as_array()
            .unwrap()
            .contains(&json!("harness::state::list_keys"))
    );
    for (key, sid) in [("legacy-key", "selected"), ("foreign-key", "other-session")] {
        assert_eq!(
            peer.result(
                "harness::state::compare-and-set",
                json!({"scope":scope,
            "key":key, "value":{"session_id":sid,"function_id":"slow::work"}})
            )
            .await["swapped"],
            true
        );
    }
    for function in [
        "state::get",
        "state::list",
        "state::list_keys",
        "state::list_entries",
        "state::set",
        "state::delete",
        "state::update",
    ] {
        peer.error(
            function,
            json!({"scope":scope,"key":"legacy-key","value":{},"ops":[]}),
            "RESERVED_SCOPE",
        )
        .await;
    }
    for invalid in ["public", "__state_private_namespaces"] {
        peer.error(
            "harness::state::list_keys",
            json!({"scope":invalid}),
            "INVALID_SCOPE",
        )
        .await;
    }
    peer.result(
        "state::claim-namespace",
        json!({"functions_prefix":"other", "scopes":["other-private"],
        "_caller_worker_id":"other-id"}),
    )
    .await;
    peer.error(
        "harness::state::list_keys",
        json!({"scope":"other-private"}),
        "INVALID_SCOPE",
    )
    .await;
    for invalid in ["public", "__state_private_namespaces", "other-private"] {
        peer.error(
            "harness::state::list_entries",
            json!({"scope":invalid}),
            "INVALID_SCOPE",
        )
        .await;
    }
    let entries = peer
        .result("harness::state::list_entries", json!({"scope":scope}))
        .await;
    assert_eq!(
        entries,
        json!({"done":true,"next_cursor":null,"offset":0,"total":2,"entries":[
            ["legacy-key", {"session_id":"selected","function_id":"slow::work"}],
            ["foreign-key", {"session_id":"other-session","function_id":"slow::work"}]
        ]})
    );
    let bulk_contract = &peer.registrations["harness::state::list_entries"];
    assert_eq!(
        bulk_contract["metadata"],
        json!({"internal":true,"trace_hidden":true})
    );
    assert_eq!(
        bulk_contract["request_format"]["required"],
        json!(["scope"])
    );
    assert_eq!(
        bulk_contract["response_format"]["properties"]["entries"]["type"],
        "array"
    );
    let listed = peer
        .result("harness::state::list_keys", json!({"scope":scope}))
        .await;
    assert_eq!(listed, json!({"keys":["legacy-key","foreign-key"]}));
    let contract = &peer.registrations["harness::state::list_keys"];
    assert_eq!(
        contract["metadata"],
        json!({"internal":true,"trace_hidden":true})
    );
    assert_eq!(contract["request_format"]["required"], json!(["scope"]));
    assert_eq!(
        contract["response_format"]["properties"]["keys"]["type"],
        "array"
    );
    peer.result(
        "state::claim-namespace",
        json!({"functions_prefix":"harness", "scopes":[scope,"extra-private"],
        "_caller_worker_id":"owner-id"}),
    )
    .await;
    assert_eq!(
        peer.result(
            "harness::state::list_keys",
            json!({"scope":"extra-private"})
        )
        .await,
        json!({"keys":[]})
    );
    assert_eq!(
        peer.result("state::claim-namespace", owned).await["claimed"],
        false
    );
    // Fresh SDK registration + namespace registry, same real adapter. Not a
    // deployed engine/DB restart; restore executes the production boot path.
    drop(peer);
    let mut restored = Peer::new(adapter).await;
    assert!(restored.ctx.private.is_reserved(scope));
    restored
        .error("state::list_keys", json!({"scope":scope}), "RESERVED_SCOPE")
        .await;
    assert_eq!(
        restored
            .result("harness::state::list_keys", json!({"scope":scope}))
            .await,
        listed
    );
    assert_eq!(
        restored
            .result("harness::state::list_entries", json!({"scope":scope}))
            .await,
        entries
    );
    restored
        .error(
            "state::list_entries",
            json!({"scope":scope}),
            "RESERVED_SCOPE",
        )
        .await;
    let expected = restored
        .result(
            "harness::state::get",
            json!({"scope":scope,"key":"legacy-key"}),
        )
        .await;
    assert_eq!(
        restored
            .result(
                "harness::state::compare-and-set",
                json!({"scope":scope,"key":"legacy-key",
        "expected":expected,"value":null})
            )
            .await["swapped"],
        true
    );
    let bulk = restored
        .result("harness::state::list_entries", json!({"scope":scope}))
        .await;
    assert_eq!(bulk["entries"][0], json!(["legacy-key", null]));
    for i in 0..150 {
        restored
            .ctx
            .adapter
            .set(scope, &format!("history-{i}"), Value::Null)
            .await
            .unwrap();
    }
    let offset = restored.calls;
    let bulk = restored
        .result("harness::state::list_entries", json!({"scope":scope}))
        .await;
    assert_eq!(bulk["entries"].as_array().unwrap().len(), 100);
    assert_eq!(bulk["done"], false);
    let next = restored
        .result(
            "harness::state::list_entries",
            json!({"scope":scope,"cursor":bulk["next_cursor"]}),
        )
        .await;
    assert_eq!(next["entries"].as_array().unwrap().len(), 52);
    assert_eq!(next["done"], true);
    assert!(bulk["entries"].as_array().unwrap().iter().any(|row| row
        == &json!(["foreign-key", {"session_id":"other-session","function_id":"slow::work"}])));
    assert_eq!(restored.calls - offset, 2);
    // A confirmed late reply sees null and cannot recreate the witness.
    let late = restored
        .result(
            "harness::state::compare-and-set",
            json!({"scope":scope,"key":"legacy-key",
        "expected":expected,"value":null}),
        )
        .await;
    assert_eq!(late, json!({"swapped":false,"current":null}));
    assert_eq!(
        restored
            .result(
                "harness::state::get",
                json!({"scope":scope,"key":"foreign-key"})
            )
            .await["session_id"],
        "other-session"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_registered_handlers_bound_frames_and_fail_closed() {
    let adapter: Arc<dyn StateAdapter> = Arc::new(KvStoreAdapter::new(Some(
        json!({"store_method":"in_memory"}),
    )));
    let mut peer = Peer::new(adapter.clone()).await;
    peer.result("state::claim-namespace",json!({"functions_prefix":"harness","scopes":["private","extra-private"],"_caller_worker_id":"owner-id"})).await;
    for (function, scope) in [
        ("state::list_entries", "public"),
        ("harness::state::list_entries", "private"),
    ] {
        let empty = peer.result(function, json!({"scope":scope})).await;
        assert_eq!(
            empty,
            json!({"entries":[],"done":true,"next_cursor":null,"offset":0,"total":0})
        );
        for i in 0..101 {
            adapter
                .set(scope, &format!("{i:03}"), Value::Null)
                .await
                .unwrap();
        }
        let first = peer.result(function, json!({"scope":scope})).await;
        assert_eq!(first["entries"].as_array().unwrap().len(), 100);
        assert_eq!(first["done"], false);
        let token = first["next_cursor"].clone();
        peer.error(
            function,
            json!({"scope":scope,"cursor":token,"_caller_worker_id":"other-id"}),
            "INVALID_CURSOR",
        )
        .await;
        peer.error(
            function,
            json!({"scope":scope,"cursor":"tampered"}),
            "INVALID_CURSOR",
        )
        .await;
        peer.error(
            function,
            json!({"scope":scope,"cursor":token,"limit":1}),
            "INVALID_CURSOR",
        )
        .await;
        adapter.delete(scope, "100").await.unwrap();
        adapter.set(scope, "inserted", json!("new")).await.unwrap();
        let next = peer
            .result(function, json!({"scope":scope,"cursor":token}))
            .await;
        assert_eq!(next["entries"], json!([["100", null]]));
        assert_eq!(next["done"], true);
        peer.error(
            function,
            json!({"scope":scope,"cursor":token}),
            "INVALID_CURSOR",
        )
        .await;
        for request in [
            json!({"limit":0}),
            json!({"limit":1001}),
            json!({"max_bytes":255}),
            json!({"max_bytes":8_000_001}),
            json!({"limit":-1}),
        ] {
            let mut input = request;
            input["scope"] = json!(scope);
            let reply = peer.call(function, input).await;
            assert!(!reply["error"].is_null());
        }
        peer.error(
            function,
            json!({"scope":scope,"_caller_worker_id":null}),
            "UNKNOWN_CALLER",
        )
        .await;
        let schema = &peer.registrations[function];
        for field in ["cursor", "limit", "max_bytes"] {
            assert!(schema["request_format"]["properties"][field].is_object());
        }
        for field in ["done", "next_cursor", "offset", "total", "entries"] {
            assert!(
                schema["response_format"]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(field)),
                "{schema}"
            );
        }
    }
    // Actual SDK invocationresult frame near the hard page cap, not a mocked
    // serde shape or a count-only proxy. Values are seeded via the adapter.
    for (function, scope) in [
        ("state::list_entries", "frame-public"),
        ("harness::state::list_entries", "extra-private"),
    ] {
        adapter
            .set(
                scope,
                "escaped\n\"中文",
                json!(format!(
                    "{}{}",
                    "\"\n中文".repeat(780_000),
                    "x".repeat(199_875)
                )),
            )
            .await
            .unwrap();
        adapter.set(scope, "later", Value::Null).await.unwrap();
        let reply = peer
            .call(
                function,
                json!({"scope":scope,"max_bytes":8_000_000,"limit":1}),
            )
            .await;
        assert!(reply["error"].is_null(), "{function}");
        let result_bytes = serde_json::to_vec(&reply["result"]).unwrap().len();
        let frame_bytes = peer.last_frame_bytes;
        assert_eq!(frame_bytes, serde_json::to_vec(&reply).unwrap().len());
        assert_eq!(result_bytes, 8_000_000);
        assert!(frame_bytes < 16_000_000);
        assert_eq!(reply["result"]["done"], false);
        println!(
            "{function}: response={result_bytes} invocationresult={frame_bytes} bytes (<16000000)"
        );
        if function == "harness::state::list_entries" {
            // Keep two non-null rows for the same exact nonterminal boundary,
            // plus a null which only the explicit private opt-in excludes.
            adapter.set(scope, "later", json!(false)).await.unwrap();
            adapter.set(scope, "retired", Value::Null).await.unwrap();
            let filtered = peer
                .call(
                    function,
                    json!({"scope":scope,"max_bytes":8_000_000,"limit":1,"non_null_only":true}),
                )
                .await;
            assert!(filtered["error"].is_null());
            let payload_bytes = serde_json::to_vec(&filtered["result"]).unwrap().len();
            let frame_bytes = peer.last_frame_bytes;
            assert_eq!(payload_bytes, 8_000_000);
            assert_eq!(frame_bytes, serde_json::to_vec(&filtered).unwrap().len());
            assert!(frame_bytes < 16_000_000);
            assert_eq!(filtered["result"]["total"], 2);
            println!(
                "{function} non_null_only=true: response={payload_bytes} invocationresult={frame_bytes} bytes (<16000000)"
            );
            let end = peer.result(function, json!({"scope":scope,"max_bytes":8_000_000,"limit":1,"non_null_only":true,"cursor":filtered["result"]["next_cursor"]})).await;
            assert_eq!(end["entries"], json!([["later", false]]));
            assert_eq!(end["done"], true);
        }
        let token = reply["result"]["next_cursor"].clone();
        let other = if function == "state::list_entries" {
            "public"
        } else {
            "private"
        };
        peer.error(
            function,
            json!({"scope":other,"cursor":token,"max_bytes":8_000_000,"limit":1}),
            "INVALID_CURSOR",
        )
        .await;
        let huge_scope = if function == "state::list_entries" {
            "huge-public"
        } else {
            "private"
        };
        adapter
            .set(huge_scope, "z-huge", json!("x".repeat(8_000_000)))
            .await
            .unwrap();
        let error = peer
            .call(function, json!({"scope":huge_scope,"max_bytes":8_000_000}))
            .await;
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("ROW_TOO_LARGE")
        );
        let error_bytes = serde_json::to_vec(&error).unwrap().len();
        println!(
            "{function}: bounded ROW_TOO_LARGE error frame={error_bytes} bytes (SDK includes stacktrace)"
        );
        assert!(error["error"]["message"].as_str().unwrap().len() < 256);
        assert!(error_bytes < 64_000);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_cursor_cannot_cross_namespace_even_with_same_caller_and_empty_owned_scope() {
    let adapter: Arc<dyn StateAdapter> = Arc::new(KvStoreAdapter::new(Some(
        json!({"store_method":"in_memory"}),
    )));
    let mut peer = Peer::new(adapter.clone()).await;
    peer.result(
        "state::claim-namespace",
        json!({"functions_prefix":"harness","scopes":["h-private"],"_caller_worker_id":"owner-id"}),
    )
    .await;
    peer.result("state::claim-namespace",json!({"functions_prefix":"other","scopes":["other-private"],"_caller_worker_id":"other-id"})).await;
    adapter.set("h-private", "a", Value::Null).await.unwrap();
    adapter.set("h-private", "b", json!(42)).await.unwrap();
    let first = peer
        .result(
            "harness::state::list_entries",
            json!({"scope":"h-private","limit":1}),
        )
        .await;
    peer.error(
        "other::state::list_entries",
        json!({"scope":"other-private","limit":1,"cursor":first["next_cursor"]}),
        "INVALID_CURSOR",
    )
    .await;
    peer.error(
        "state::list_entries",
        json!({"scope":"public","limit":1,"cursor":first["next_cursor"]}),
        "INVALID_CURSOR",
    )
    .await;
    assert_eq!(
        peer.result(
            "harness::state::list_entries",
            json!({"scope":"h-private","limit":1,"cursor":first["next_cursor"]})
        )
        .await["done"],
        true
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_non_null_handler_survives_retired_history_and_keeps_public_default_contracts() {
    let adapter: Arc<dyn StateAdapter> = Arc::new(KvStoreAdapter::new(Some(
        json!({"store_method":"in_memory"}),
    )));
    let mut peer = Peer::new(adapter.clone()).await;
    peer.result("state::claim-namespace",json!({"functions_prefix":"harness","scopes":["history-private"],"_caller_worker_id":"owner-id"})).await;
    for i in 0..100_001 {
        adapter
            .set("history-private", &format!("retired-{i:06}"), Value::Null)
            .await
            .unwrap();
    }
    let original = [
        (
            "target",
            json!({"session_id":"target","function_id":"slow::target"}),
        ),
        ("ambiguous", json!({"function_id":"no-owner"})),
        (
            "foreign",
            json!({"session_id":"foreign","function_id":"foreign::call"}),
        ),
    ];
    for (key, value) in &original {
        adapter
            .set("history-private", key, value.clone())
            .await
            .unwrap();
    }
    let keys_before = adapter.list_keys("history-private").await.unwrap();
    assert_eq!(keys_before.len(), 100_004);
    peer.error(
        "harness::state::list_entries",
        json!({"scope":"history-private"}),
        "SNAPSHOT_CAPACITY",
    )
    .await;
    let first = peer
        .result(
            "harness::state::list_entries",
            json!({"scope":"history-private","non_null_only":true,"limit":1}),
        )
        .await;
    assert_eq!(first["total"], 3);
    assert_eq!(first["entries"], json!([[original[0].0, original[0].1]]));
    for mode in [json!({}), json!({"non_null_only":false})] {
        let mut request = mode;
        request["scope"] = json!("history-private");
        request["limit"] = json!(1);
        request["cursor"] = first["next_cursor"].clone();
        peer.error("harness::state::list_entries", request, "INVALID_CURSOR")
            .await;
    }
    // A separate concurrent writer flips both directions after initial capture.
    let writer = adapter.clone();
    tokio::spawn(async move {
        writer
            .set("history-private", "ambiguous", Value::Null)
            .await
            .unwrap();
        writer
            .set(
                "history-private",
                "foreign",
                json!({"session_id":"replacement"}),
            )
            .await
            .unwrap();
        writer
            .set(
                "history-private",
                "retired-000000",
                json!({"session_id":"new"}),
            )
            .await
            .unwrap();
    })
    .await
    .unwrap();
    let mut cursor = first["next_cursor"].clone();
    let mut seen = first["entries"].as_array().unwrap().clone();
    loop {
        let page = peer
            .result(
                "harness::state::list_entries",
                json!({"scope":"history-private","non_null_only":true,"limit":1,"cursor":cursor}),
            )
            .await;
        assert_eq!(page["offset"], json!(seen.len()));
        assert_eq!(page["total"], 3);
        assert!(serde_json::to_vec(&page).unwrap().len() <= 1_000_000);
        assert!(peer.last_frame_bytes < 16_000_000);
        seen.extend(page["entries"].as_array().unwrap().clone());
        cursor = page["next_cursor"].clone();
        if page["done"] == true {
            assert!(cursor.is_null());
            break;
        }
    }
    assert_eq!(
        seen,
        original
            .iter()
            .map(|(k, v)| json!([k, v]))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        adapter.list_keys("history-private").await.unwrap(),
        keys_before
    );
    assert!(
        adapter
            .get("history-private", "retired-000001")
            .await
            .unwrap()
            .unwrap()
            .is_null()
    );
    // No private opt-in leaks into the public schema; unknown extra input does
    // not activate it in the existing permissive public runtime contract.
    for (function, scope) in [
        ("state::list_entries", "default-public"),
        ("harness::state::list_entries", "history-private"),
    ] {
        let schema = &peer.registrations[function]["request_format"];
        assert!(schema["properties"]["scope"].is_object());
        assert_eq!(
            schema["properties"]["non_null_only"].is_object(),
            function != "state::list_entries"
        );
        if function == "state::list_entries" {
            adapter.set(scope, "null", Value::Null).await.unwrap();
            adapter.set(scope, "non-null", json!(false)).await.unwrap();
            let page = peer
                .result(function, json!({"scope":scope,"non_null_only":true}))
                .await;
            assert_eq!(page["total"], 2);
            assert_eq!(page["entries"][0], json!(["null", null]));
        }
    }
    peer.error(
        "harness::state::list_entries",
        json!({"scope":"history-private","non_null_only":"true"}),
        "invalid type",
    )
    .await;
    for i in 0..100_001 {
        adapter
            .set("public-history", &format!("retired-{i:06}"), Value::Null)
            .await
            .unwrap();
    }
    peer.error(
        "state::list_entries",
        json!({"scope":"public-history","non_null_only":true}),
        "SNAPSHOT_CAPACITY",
    )
    .await;
    // Explicit false and omission still include nulls on a small private scope.
    peer.result("state::claim-namespace",json!({"functions_prefix":"harness","scopes":["default-private"],"_caller_worker_id":"owner-id"})).await;
    adapter
        .set("default-private", "null", Value::Null)
        .await
        .unwrap();
    adapter
        .set("default-private", "active", json!(false))
        .await
        .unwrap();
    for request in [
        json!({"scope":"default-private"}),
        json!({"scope":"default-private","non_null_only":false}),
    ] {
        let page = peer.result("harness::state::list_entries", request).await;
        assert_eq!(page["total"], 2);
        assert_eq!(page["entries"][0], json!(["null", null]));
    }
    let default = peer
        .result(
            "harness::state::list_entries",
            json!({"scope":"default-private","limit":1}),
        )
        .await;
    peer.error("harness::state::list_entries",json!({"scope":"default-private","limit":1,"cursor":default["next_cursor"],"non_null_only":true}),"INVALID_CURSOR").await;
    assert_eq!(
        peer.result(
            "harness::state::list_entries",
            json!({"scope":"default-private","limit":1,"cursor":default["next_cursor"]})
        )
        .await["done"],
        true
    );
    println!(
        "B1 real handlers: 100001 retired keys + 3 non-null witnesses; exact filter, immutable membership, mode binding, public/default caps and physical history verified"
    );
}
