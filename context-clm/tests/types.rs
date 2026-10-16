//! The wire types are lossless copies of what the harness and
//! context-manager exchange.
use context_clm::types::{AssembleOptions, AssembleRequest, AssembleResponse, Block, Msg};
use serde_json::{json, Value};

const WINDOW: &str = include_str!("fixtures/window.json");

#[test]
fn harness_messages_round_trip_losslessly() {
    let raw: Value = serde_json::from_str(WINDOW).unwrap();
    let msgs: Vec<Msg> = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(&msgs).unwrap(), raw);
}

#[test]
fn helpers_read_calls_results_and_text() {
    let msgs: Vec<Msg> = serde_json::from_str(WINDOW).unwrap();
    assert!(!msgs[0].is_assistant_call());
    assert!(msgs[1].is_assistant_call());
    assert!(!msgs[3].is_assistant_call());
    assert_eq!(msgs[1].call_ids(), vec!["toolu_018DCMmnN8C2x4b1TKYoVgJ9"]);
    assert!(msgs[0].call_ids().is_empty());
    assert_eq!(
        msgs[2].function_call_id(),
        Some("toolu_018DCMmnN8C2x4b1TKYoVgJ9")
    );
    assert_eq!(msgs[1].function_call_id(), None);
    assert!(msgs[2].result_text().starts_with("{\"duration_ms\":14"));
    assert_eq!(msgs[1].content[2].text(), Some("Looking for the handler."));
    assert_eq!(msgs[1].content[0].text(), None, "thinking is not text");
    assert_eq!(
        serde_json::to_value(Block::new_text("hi")).unwrap(),
        json!({"type": "text", "text": "hi"})
    );
}

#[test]
fn assemble_request_keeps_harness_fields_and_unknown_keys() {
    let messages: Value = serde_json::from_str(WINDOW).unwrap();
    let raw = json!({
        "messages": messages,
        "model": {"id": "claude-opus-5-5", "provider": "anthropic"},
        "options": {
            "lease_key": "s_1",
            "request_overhead_tokens": 120,
            "previous_summary": "earlier",
            "thinking_level": "high",
            "allow_prune": true,
            "session_id": "s_1",
            "turn_id": "t_1",
            "step": 3,
            "filesystem_root": "/work/s_1",
            "budget_tokens": 16384,
            "reserved_tokens": 900
        },
        "tools": [{"name": "shell::exec", "description": "run", "parameters": {"type": "object"}}],
        "system_prompt": "You are helpful.",
        "parts": {"skills": "listing"}
    });
    let req: AssembleRequest = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(req.messages.len(), 4);
    assert_eq!(req.model.provider.as_deref(), Some("anthropic"));
    assert_eq!(req.options.session_id.as_deref(), Some("s_1"));
    assert_eq!(req.options.turn_id.as_deref(), Some("t_1"));
    assert_eq!(req.options.step, Some(3));
    assert_eq!(req.options.filesystem_root.as_deref(), Some("/work/s_1"));
    assert_eq!(req.options.budget_tokens, Some(16384));
    assert_eq!(req.options.rest["reserved_tokens"], 900);
    assert_eq!(req.rest["parts"], json!({"skills": "listing"}));
    assert_eq!(serde_json::to_value(&req).unwrap(), raw);
}

#[test]
fn minimal_request_defaults_and_sends_no_nulls() {
    let req: AssembleRequest =
        serde_json::from_value(json!({"messages": [], "model": {"id": "m"}})).unwrap();
    assert_eq!(req.options, AssembleOptions::default());
    assert!(req.tools.is_empty());
    assert_eq!(
        serde_json::to_value(&req).unwrap(),
        json!({"messages": [], "model": {"id": "m"}, "options": {}, "tools": []})
    );
}

#[test]
fn assemble_response_round_trips_context_manager_output() {
    let raw = json!({
        "system_prompt": "sp",
        "messages": [],
        "token_count": 10,
        "usable": 100,
        "effective_max_output_tokens": 32000,
        "model_resolved": "router",
        "applied": {"initial_token_count": 10, "pruned": false, "pruned_tokens": 0,
                    "capped_parts": 0, "capped_tokens": 0, "compacted": false},
        "breakdown": {"system_prompt_tokens": 1, "tools_tokens": 2, "by_role": {},
                      "estimator": "heuristic"}
    });
    let res: AssembleResponse = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(&res).unwrap(), raw);

    let bare = AssembleResponse {
        system_prompt: String::new(),
        messages: vec![],
        token_count: 0,
        usable: 0,
        effective_max_output_tokens: 0,
        applied: Value::Null,
        rest: Default::default(),
    };
    assert!(serde_json::to_value(&bare)
        .unwrap()
        .get("applied")
        .is_none());
}

#[test]
fn request_and_response_schemas_are_typed() {
    for schema in [
        schemars::schema_for!(AssembleRequest),
        schemars::schema_for!(AssembleResponse),
    ] {
        let schema = serde_json::to_value(schema).unwrap();
        assert!(schema["properties"]["messages"].is_object(), "{schema}");
    }
}
