use std::collections::BTreeMap;

use serde_json::json;

use crate::types::scenario::{
    AuthoredScenarioV1, RouterReplyV1, ScenarioGenerationV1, ScenarioRouterV1, ScenarioSendV1,
};
use crate::types::script::{JsonMatcherV1, SchemaVersion1};

use super::{
    compile_scenario, render_authored_yaml, render_compiled, scenario_template, Placeholders,
    ScenarioTemplateKind,
};

fn minimal(reply: RouterReplyV1) -> AuthoredScenarioV1 {
    AuthoredScenarioV1 {
        schema_version: SchemaVersion1::V1,
        id: "C-E2E-T".to_string(),
        description: "A focused compiler fixture.".to_string(),
        quarantine: false,
        send: ScenarioSendV1 {
            message: "hello".to_string(),
            allow: None,
            idempotency_key: None,
        },
        functions: BTreeMap::new(),
        router: ScenarioRouterV1 {
            model: None,
            generations: vec![ScenarioGenerationV1 {
                reply,
                match_overrides: Default::default(),
            }],
        },
        bindings: Vec::new(),
        release: None,
        fault: None,
        timeouts: Default::default(),
        expect: Default::default(),
    }
}

#[test]
fn text_reply_compiles_stream_and_common_defaults() {
    let authored = minimal(RouterReplyV1::Text {
        text: "hello".to_string(),
        chunks: vec!["hel".to_string(), "lo".to_string()],
        usage: None,
    });
    let compiled = compile_scenario(&authored, "base\n").unwrap();
    assert_eq!(compiled.script.generations[0].frames.len(), 7);
    assert!(compiled.scenario.send.options.functions.allow.is_empty());
    assert_eq!(compiled.scenario.deadlines.teardown_ms, 15_000);
    assert!(compiled
        .scenario
        .invariants
        .iter()
        .any(|invariant| invariant.id == "target.calls" && invariant.parameters["count"] == 0));
    assert!(compiled
        .system_prompt_template
        .ends_with("Function dispatch is entirely disabled this turn — do not call any function."));
}

#[test]
fn mismatched_chunks_and_unknown_aliases_fail_before_runtime() {
    let authored = minimal(RouterReplyV1::Text {
        text: "hello".to_string(),
        chunks: vec!["wrong".to_string()],
        usage: None,
    });
    assert!(
        format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
            .contains("chunks concatenate")
    );

    let mut authored = minimal(RouterReplyV1::FunctionCall {
        id: None,
        function: "missing".to_string(),
        arguments: json!({}),
        usage: None,
    });
    authored.send.allow = Some(vec![]);
    assert!(
        format!("{:#}", compile_scenario(&authored, "base").unwrap_err()).contains("unknown alias")
    );
}

#[test]
fn compile_rejects_unknown_placeholders_and_unsafe_ids() {
    let mut authored = minimal(RouterReplyV1::Text {
        text: "hello".to_string(),
        chunks: Vec::new(),
        usage: None,
    });
    authored.send.message = "{{unknown}}".to_string();
    assert!(
        format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
            .contains("unexpanded placeholder")
    );

    authored.send.message = "hello".to_string();
    authored.id = "../../escape".to_string();
    assert!(
        format!("{:#}", compile_scenario(&authored, "base").unwrap_err()).contains("scenario id")
    );
}

#[test]
fn function_call_defaults_are_ordered_by_call_and_validated() {
    let mut authored = scenario_template(
        "C-E2E-CALLS",
        "Validate generated function call ids.",
        ScenarioTemplateKind::Function,
    );
    authored.router.generations.insert(
        0,
        ScenarioGenerationV1 {
            reply: RouterReplyV1::Text {
                text: "before call".to_string(),
                chunks: Vec::new(),
                usage: None,
            },
            match_overrides: Default::default(),
        },
    );
    let compiled = compile_scenario(&authored, "base").unwrap();
    let messages = match &compiled.script.generations[2].match_.messages {
        JsonMatcherV1::Exact { expected, .. } => expected.as_array().unwrap(),
        other => panic!("expected exact history, got {other:?}"),
    };
    assert_eq!(messages[2]["content"][0]["id"], "call-1");
    assert_eq!(messages[3]["function_call_id"], "call-1");

    let RouterReplyV1::FunctionCall { id, .. } = &mut authored.router.generations[1].reply else {
        unreachable!()
    };
    *id = Some(String::new());
    assert!(
        format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
            .contains("must not be empty")
    );
}

#[test]
fn tools_are_canonicalized_and_function_contracts_validate() {
    let mut authored = scenario_template(
        "C-E2E-TOOLS",
        "Validate tool ordering and arguments.",
        ScenarioTemplateKind::Function,
    );
    let record = authored.functions["record"].clone();
    authored
        .functions
        .insert("zeta".to_string(), record.clone());
    authored.functions.insert("alpha".to_string(), record);
    authored.send.allow = Some(vec![
        "zeta".to_string(),
        "record".to_string(),
        "alpha".to_string(),
    ]);
    let compiled = compile_scenario(&authored, "base").unwrap();
    let allowed = &compiled.scenario.send.options.functions.allow;
    assert_eq!(
        allowed,
        &[
            "{{run_id}}::alpha".to_string(),
            "{{run_id}}::record".to_string(),
            "{{run_id}}::zeta".to_string()
        ]
    );
    let JsonMatcherV1::Exact { expected, .. } = &compiled.script.generations[0].match_.tools else {
        panic!("tools must use an exact matcher");
    };
    assert_eq!(
        expected
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "{{run_id}}::alpha",
            "{{run_id}}::record",
            "{{run_id}}::zeta"
        ]
    );

    let RouterReplyV1::FunctionCall { arguments, .. } = &mut authored.router.generations[0].reply
    else {
        unreachable!()
    };
    *arguments = json!({});
    assert!(
        format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
            .contains("do not match request_schema")
    );

    authored.functions.get_mut("record").unwrap().request_schema =
        json!({ "type": "not-a-json-schema-type" })
            .as_object()
            .unwrap()
            .clone();
    assert!(
        format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
            .contains("invalid request_schema")
    );
}

#[test]
fn function_response_history_matches_harness_normalization() {
    let mut authored = scenario_template(
        "C-E2E-RESPONSE",
        "Validate normalized function results.",
        ScenarioTemplateKind::Function,
    );
    authored.functions.get_mut("record").unwrap().response = json!("ok");
    let compiled = compile_scenario(&authored, "base").unwrap();
    let JsonMatcherV1::Exact { expected, .. } = &compiled.script.generations[1].match_.messages
    else {
        panic!("function history must be exact");
    };
    assert_eq!(
        expected[2]["content"],
        json!([{ "type": "text", "text": "ok" }])
    );

    authored.functions.get_mut("record").unwrap().response = json!({ "value": 1 });
    let compiled = compile_scenario(&authored, "base").unwrap();
    let JsonMatcherV1::Exact { expected, .. } = &compiled.script.generations[1].match_.messages
    else {
        unreachable!()
    };
    assert_eq!(
        expected[2]["content"],
        json!([{ "type": "text", "text": "{\"value\":1}" }])
    );
}

#[test]
fn bindings_release_and_fault_fail_fast_when_incoherent() {
    let mut hook = scenario_template(
        "C-E2E-HOOK",
        "Validate hook relationships.",
        ScenarioTemplateKind::Hook,
    );
    hook.bindings[0].functions.clear();
    assert!(
        format!("{:#}", compile_scenario(&hook, "base").unwrap_err())
            .contains("select at least one")
    );

    hook.bindings[0].functions = vec!["record".to_string()];
    hook.functions.get_mut("hook").unwrap().response = json!({ "decision": "continue" });
    assert!(
        format!("{:#}", compile_scenario(&hook, "base").unwrap_err())
            .contains("requires a selected hook with decision: hold")
    );

    let mut crash = scenario_template(
        "C-E2E-CRASH",
        "Validate fault relationships.",
        ScenarioTemplateKind::Crash,
    );
    let compiled = compile_scenario(&crash, "base").unwrap();
    assert_eq!(
        compiled.scenario.recorder.target.hold_response_at,
        Some(1),
        "fault targets receive the compiler-owned response-gate ordinal"
    );

    let second_call = crash.router.generations[0].clone();
    crash.router.generations.insert(1, second_call);
    crash.fault.as_mut().unwrap().after_target_calls = 2;
    let compiled = compile_scenario(&crash, "base").unwrap();
    assert_eq!(
        compiled.scenario.recorder.target.hold_response_at,
        Some(2),
        "earlier calls must return before the selected fault ordinal is held"
    );

    crash.fault.as_mut().unwrap().after_target_calls = 0;
    assert!(
        format!("{:#}", compile_scenario(&crash, "base").unwrap_err())
            .contains("after_target_calls must be greater than zero")
    );
}

#[test]
fn templates_compile_and_render_deterministically() {
    for kind in [
        ScenarioTemplateKind::Text,
        ScenarioTemplateKind::Function,
        ScenarioTemplateKind::Hook,
        ScenarioTemplateKind::Crash,
    ] {
        let authored = scenario_template("C-E2E-NEW", "A generated scenario.", kind);
        let yaml = render_authored_yaml(&authored).unwrap();
        let reparsed: AuthoredScenarioV1 = serde_yaml::from_str(&yaml).unwrap();
        let fixture = compile_scenario(&reparsed, "base\n").unwrap();
        assert_eq!(
            render_compiled(&fixture).unwrap(),
            render_compiled(&fixture).unwrap()
        );
        assert!(fixture
            .scenario
            .invariants
            .iter()
            .any(|invariant| invariant.id == "transcript.message_counts"));
        if kind != ScenarioTemplateKind::Text {
            assert!(fixture
                .scenario
                .invariants
                .iter()
                .any(|invariant| invariant.id == "target.calls"));
        }
        if matches!(
            kind,
            ScenarioTemplateKind::Hook | ScenarioTemplateKind::Crash
        ) {
            assert!(matches!(
                fixture.script.generations[1].match_.messages,
                JsonMatcherV1::Present
            ));
        }
    }
}

#[test]
fn expands_strings_keys_and_rejects_unknown_tokens() {
    let placeholders = Placeholders::new("r1", "s_abc");
    let mut value = json!({
        "idempotency_key": "{{run_id}}:streamed-text",
        "session_id": "{{session_id}}",
        "{{run_id}}::record": { "count": 1 }
    });
    placeholders.expand_value(&mut value).unwrap();
    assert_eq!(value["idempotency_key"], "r1:streamed-text");
    assert_eq!(value["session_id"], "s_abc");
    assert!(value.get("r1::record").is_some());

    let mut bad = json!("{{unknown_token}}");
    assert!(placeholders.expand_value(&mut bad).is_err());
}
