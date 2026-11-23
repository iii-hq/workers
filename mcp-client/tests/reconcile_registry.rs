mod support;

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use mcp_client::host::{EventKind, FunctionDef, Handler, Host, SecretError, ServerEvent};
use mcp_client::reconcile::{
    Plan, Registry, SkipKind, Skipped, Target, ToolInfo, Wanted, tool_response_format, tool_wanted,
};
use serde_json::{Value, json};
use support::{FakeHost, RegOp};

fn tools(v: Value) -> Vec<ToolInfo> {
    serde_json::from_value(v).unwrap()
}

fn echo(description: &str) -> Value {
    json!({"name": "echo", "description": description, "inputSchema": {"type": "object"}})
}

fn handler_for(wanted: &Wanted) -> Handler {
    let id = wanted.def.id.clone();
    Arc::new(move |payload| {
        let id = id.clone();
        Box::pin(async move { Ok(json!({"id": id, "payload": payload})) })
    })
}

#[tokio::test]
async fn duplicate_names_register_once_and_the_rest_are_skipped() {
    let host = FakeHost::new();
    let registry = Registry::new();
    let id = "mcp::s::echo";

    let (wanted, skipped) = tool_wanted("s", &tools(json!([echo("one"), echo("two")])), false);
    assert_eq!(wanted.len(), 1);
    assert_eq!(wanted[id].def.description, "one");
    assert_eq!(
        skipped,
        vec![Skipped {
            kind: SkipKind::Tool,
            name: "echo".into(),
            reason: "duplicate name".into()
        }]
    );

    let first = registry
        .apply(host.as_ref(), wanted.clone(), &|_| false, &handler_for)
        .await;
    assert_eq!(first.add, [id]);
    assert_eq!(host.ops(), vec![RegOp::Register(id.into())]);

    let again = registry
        .apply(host.as_ref(), wanted, &|_| false, &handler_for)
        .await;
    assert_eq!(again, Plan::default());
    assert_eq!(host.ops().len(), 1);

    // FakeHost panics on a live id, as the SDK does, so reaching the asserts
    // is the proof that the change unregistered first.
    let (wanted, _) = tool_wanted("s", &tools(json!([echo("three")])), false);
    let third = registry
        .apply(host.as_ref(), wanted, &|_| false, &handler_for)
        .await;
    assert_eq!(third.change, [id]);
    assert_eq!(
        host.ops(),
        vec![
            RegOp::Register(id.into()),
            RegOp::Unregister(id.into()),
            RegOp::Register(id.into())
        ]
    );
    assert_eq!(host.def(id).unwrap().description, "three");
}

#[tokio::test]
async fn apply_adds_removes_and_leaves_untouched_ids() {
    let host = FakeHost::new();
    let registry = Registry::new();
    let tool = |name: &str| json!({"name": name, "inputSchema": {"type": "object"}});
    let prompt_id = "mcp::s::prompts::p";
    let prompt = Wanted {
        def: FunctionDef {
            id: prompt_id.into(),
            description: "Prompt `p` from the `s` MCP server.".into(),
            request_format: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            response_format: json!({"type": "object", "required": ["messages"]}),
            metadata: json!({"mcp": {"server": "s", "prompt": "p"}}),
        },
        target: Target::Prompt { name: "p".into() },
    };

    let (mut wanted, _) = tool_wanted("s", &tools(json!([tool("a"), tool("b")])), false);
    wanted.insert(prompt_id.into(), prompt);
    let plan = registry
        .apply(host.as_ref(), wanted, &|_| false, &handler_for)
        .await;
    assert_eq!(plan.add, ["mcp::s::a", "mcp::s::b", "mcp::s::prompts::p"]);

    let (only_a, _) = tool_wanted("s", &tools(json!([tool("a")])), false);
    let plan = registry
        .apply(
            host.as_ref(),
            only_a.clone(),
            &|id| id.starts_with("mcp::s::prompts::"),
            &handler_for,
        )
        .await;
    assert_eq!(plan.remove, ["mcp::s::b"]);
    assert!(plan.change.is_empty() && plan.add.is_empty());
    assert_eq!(host.registered_ids(), ["mcp::s::a", "mcp::s::prompts::p"]);
    assert_eq!(
        registry.target(prompt_id).await,
        Some(Target::Prompt { name: "p".into() })
    );
    assert_eq!(registry.target("mcp::s::b").await, None);
    assert_eq!(
        host.call("mcp::s::a", json!({"x": 1})).await,
        Ok(json!({"id": "mcp::s::a", "payload": {"x": 1}}))
    );
    assert_eq!(
        host.call("mcp::s::b", json!({})).await,
        Err("function not found: mcp::s::b".to_string())
    );

    let plan = registry
        .apply(host.as_ref(), only_a, &|_| false, &handler_for)
        .await;
    assert_eq!(plan.remove, [prompt_id]);
    assert_eq!(host.registered_ids(), ["mcp::s::a"]);
    assert_eq!(
        registry.defs().await.keys().collect::<Vec<_>>(),
        ["mcp::s::a"]
    );

    registry.clear().await;
    assert!(registry.defs().await.is_empty());
    assert!(host.registered_ids().is_empty());
    assert_eq!(
        host.ops().last(),
        Some(&RegOp::Unregister("mcp::s::a".into()))
    );
}

#[tokio::test]
async fn apply_reports_a_tool_that_lost_read_only() {
    let host = FakeHost::new();
    let registry = Registry::new();
    let id = "mcp::s::t";
    let hinted = |hint: bool| {
        tool_wanted(
            "s",
            &tools(json!([{"name": "t", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": hint}}])),
            false,
        )
        .0
    };

    registry
        .apply(host.as_ref(), hinted(true), &|_| false, &handler_for)
        .await;
    let second = registry
        .apply(host.as_ref(), hinted(false), &|_| false, &handler_for)
        .await;
    assert_eq!(second.change, [id]);
    assert_eq!(second.lost_read_only, [id]);

    let third = registry
        .apply(host.as_ref(), hinted(false), &|_| false, &handler_for)
        .await;
    assert!(third.change.is_empty() && third.lost_read_only.is_empty());
}

#[tokio::test]
async fn fake_host_keeps_one_secret_store_and_records_events() {
    let host = FakeHost::new();

    // One store keyed by reference text; `env://` references live in it too.
    host.set_secret("secret://K", "v1");
    host.set_secret("env://E", "e");
    assert_eq!(host.vault("K"), Some("v1".into()));
    assert_eq!(host.resolve("secret://K").await, Ok("v1".into()));
    assert_eq!(host.compare_and_set("K", Some("v1"), "v2").await, Ok(true));
    assert_eq!(host.vault("K"), Some("v2".into()));
    assert_eq!(host.resolve("env://E").await, Ok("e".into()));
    assert_eq!(host.vault("E"), None);
    assert_eq!(
        host.resolve("secret://MISSING").await,
        Err(SecretError::NotFound)
    );

    // compare_and_set swaps only when the current value equals `expected`.
    assert_eq!(
        host.compare_and_set("K", Some("stale"), "x").await,
        Ok(false)
    );
    assert_eq!(host.vault("K"), Some("v2".into()));
    assert_eq!(host.compare_and_set("NEW", None, "n").await, Ok(true));
    assert_eq!(host.compare_and_set("NEW", None, "m").await, Ok(false));
    assert_eq!(host.vault("NEW"), Some("n".into()));

    // `forbid` makes resolve of that reference answer Forbidden.
    host.forbid("secret://K");
    assert_eq!(
        host.resolve("secret://K").await,
        Err(SecretError::Forbidden)
    );

    // resolve_count counts every resolve, failed ones included.
    assert_eq!(host.resolve_count("secret://K"), 2); // one Ok, one Forbidden
    assert_eq!(host.resolve_count("env://never"), 0);

    // set_unreachable makes resolve and compare_and_set answer Unreachable.
    host.set_unreachable(true);
    assert!(matches!(
        host.resolve("env://E").await,
        Err(SecretError::Unreachable(_))
    ));
    assert!(matches!(
        host.compare_and_set("K", Some("v2"), "y").await,
        Err(SecretError::Unreachable(_))
    ));
    host.set_unreachable(false);
    assert_eq!(host.resolve("env://E").await, Ok("e".into()));
    assert_eq!(host.vault("K"), Some("v2".into()));

    // register panics on a live id, as the SDK does.
    let def = FunctionDef {
        id: "mcp::s::x".into(),
        description: String::new(),
        request_format: json!({"type": "object"}),
        response_format: tool_response_format(),
        metadata: json!({"mcp": {"server": "s", "tool": "x"}}),
    };
    let wanted = Wanted {
        def: def.clone(),
        target: Target::Instructions,
    };
    let registration = host.register(&def, handler_for(&wanted));
    let panic = catch_unwind(AssertUnwindSafe(|| {
        host.register(&def, handler_for(&wanted))
    }))
    .err()
    .expect("a second register of a live id panics");
    assert_eq!(
        panic.downcast_ref::<String>().map(String::as_str),
        Some("already registered: mcp::s::x")
    );
    assert_eq!(host.registered().await, HashSet::from(["mcp::s::x".into()]));

    // drop_registration hides the id without an op and keeps its handler:
    // the engine no longer lists it, the worker still holds it.
    host.drop_registration("mcp::s::x");
    assert!(host.registered().await.is_empty());
    assert!(host.registered_ids().is_empty());
    assert_eq!(host.ops(), vec![RegOp::Register("mcp::s::x".into())]);
    assert!(host.call("mcp::s::x", json!({})).await.is_ok());
    assert!(
        catch_unwind(AssertUnwindSafe(
            || host.register(&def, handler_for(&wanted))
        ))
        .is_err()
    );

    // unregister records the op and drops the handler, also for a hidden id.
    registration.unregister();
    assert_eq!(
        host.ops(),
        vec![
            RegOp::Register("mcp::s::x".into()),
            RegOp::Unregister("mcp::s::x".into())
        ]
    );
    assert_eq!(
        host.call("mcp::s::x", json!({})).await,
        Err("function not found: mcp::s::x".to_string())
    );
    assert_eq!(host.def("mcp::s::x"), None);

    // Events and the http port.
    let event = ServerEvent {
        server: "linear".into(),
        kind: EventKind::Tools,
    };
    host.emit(event.clone());
    assert_eq!(host.events(), vec![event]);
    assert_eq!(
        serde_json::to_value(&ServerEvent {
            server: "linear".into(),
            kind: EventKind::Tools
        })
        .unwrap(),
        json!({"server": "linear", "kind": "tools"})
    );
    assert_eq!(host.http_port().await, None);
    host.set_http_port(Some(3111));
    assert_eq!(host.http_port().await, Some(3111));
}
