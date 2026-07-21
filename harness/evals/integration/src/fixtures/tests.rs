use std::collections::BTreeSet;

use serde_json::json;

use super::*;
use crate::types::script::{JsonMatcherV1, RouterScriptV1};

fn minimal_script(mutate: impl FnOnce(&mut serde_json::Value)) -> serde_json::Value {
    let match_ = serde_json::to_value(crate::types::script::GenerationMatchV1::uniform(
        JsonMatcherV1::Absent,
    ))
    .unwrap();
    let mut script = json!({
        "schema_version": "1",
        "scenario_id": "T",
        "model": {
            "id": "m", "provider": "p",
            "context_window": 1000, "max_output_tokens": 100
        },
        "generations": [{
            "ordinal": 1,
            "match": match_,
            "frames": [{
                "type": "done",
                "message": {
                    "role": "assistant", "content": [], "stop_reason": "end",
                    "model": "m", "provider": "p", "timestamp": 1
                }
            }],
            "response": { "ok": true, "provider": "p", "model": "m", "stop_reason": "end" }
        }]
    });
    mutate(&mut script);
    script
}

fn validate(value: serde_json::Value) -> anyhow::Result<()> {
    let script: RouterScriptV1 = serde_json::from_value(value)?;
    validate_script(&script)
}

/// Full anyhow chain (`{:#}`): plain Display shows only the outermost
/// context, hiding the root-cause text the assertions look for.
fn error_chain(value: serde_json::Value) -> String {
    format!("{:#}", validate(value).unwrap_err())
}

#[test]
fn a_wellformed_script_validates() {
    validate(minimal_script(|_| {})).unwrap();
}

#[test]
fn duplicate_ordinals_are_rejected() {
    let script = minimal_script(|script| {
        let generation = script["generations"][0].clone();
        script["generations"]
            .as_array_mut()
            .unwrap()
            .push(generation);
    });
    assert!(error_chain(script).contains("duplicate"));
}

#[test]
fn missing_terminal_frame_is_rejected() {
    let script = minimal_script(|script| {
        script["generations"][0]["frames"] = json!([{ "type": "ping" }]);
    });
    assert!(error_chain(script).contains("no terminal"));
}

#[test]
fn terminal_frame_must_be_last() {
    let script = minimal_script(|script| {
        let done = script["generations"][0]["frames"][0].clone();
        script["generations"][0]["frames"] = json!([done, { "type": "ping" }]);
    });
    assert!(error_chain(script).contains("not the last"));
}

#[test]
fn response_frame_disagreement_is_rejected() {
    let script = minimal_script(|script| {
        script["generations"][0]["response"]["ok"] = json!(false);
    });
    assert!(error_chain(script).contains("ok:false"));

    let script = minimal_script(|script| {
        script["generations"][0]["response"]["stop_reason"] = json!("length");
    });
    assert!(error_chain(script).contains("disagrees"));

    let script = minimal_script(|script| {
        script["generations"][0]["response"]["provider"] = json!("other");
    });
    assert!(error_chain(script).contains("provider"));

    let script = minimal_script(|script| {
        script["generations"][0]["response"]["usage"] = json!({ "input": 1 });
    });
    assert!(error_chain(script).contains("usage"));
}

#[test]
fn invalid_matchers_and_normalizers_are_rejected() {
    let script = minimal_script(|script| {
        script["generations"][0]["match"]["model"] = json!({ "mode": "regex", "pattern": "(" });
    });
    assert!(validate(script).is_err());

    let script = minimal_script(|script| {
        script["generations"][0]["match"]["messages"] = json!({
            "mode": "exact", "expected": [],
            "normalize": [{ "pointer": "/0/x", "operation": "replace" }]
        });
    });
    assert!(error_chain(script).contains("replacement"));

    let script = minimal_script(|script| {
        script["generations"][0]["match"]["messages"] = json!({
            "mode": "exact", "expected": [],
            "normalize": [{ "pointer": "no-slash", "operation": "delete" }]
        });
    });
    assert!(validate(script).is_err());
}

#[test]
fn unknown_fields_and_bad_sha256_are_rejected_by_schema() {
    let script = minimal_script(|script| {
        script["generations"][0]["surprise"] = json!(true);
    });
    assert!(validate(script).is_err());

    let script = minimal_script(|script| {
        script["generations"][0]["match"]["system_prompt"] =
            json!({ "mode": "sha256", "expected": "nothex" });
    });
    assert!(error_chain(script).contains("64 hex"));
}

#[test]
fn removed_barriers_are_rejected_as_unknown_fields() {
    let script = minimal_script(|script| {
        script["generations"][0]["barriers"] =
            json!([{ "before_frame": 5, "id": "b", "timeout_ms": 100 }]);
    });
    assert!(error_chain(script).contains("barrier"));
}

/// Both delta wire forms are valid fixtures: the slim delta (no
/// `partial`) the router emits today, and the legacy fat delta carrying
/// a full snapshot.
#[test]
fn slim_and_fat_deltas_both_validate() {
    let slim = minimal_script(|script| {
        let mut done = script["generations"][0]["frames"][0].clone();
        done["message"]["content"] = json!([{ "type": "text", "text": "x" }]);
        script["generations"][0]["frames"] = json!([{ "type": "text_delta", "delta": "x" }, done]);
    });
    validate(slim).unwrap();

    let fat = minimal_script(|script| {
        let mut done = script["generations"][0]["frames"][0].clone();
        done["message"]["content"] = json!([{ "type": "text", "text": "x" }]);
        let partial = done["message"].clone();
        script["generations"][0]["frames"] =
            json!([{ "type": "text_delta", "partial": partial, "delta": "x" }, done]);
    });
    validate(fat).unwrap();
}

#[test]
fn streamed_text_must_agree_with_terminal_message() {
    let script = minimal_script(|script| {
        let mut done = script["generations"][0]["frames"][0].clone();
        done["message"]["content"] = json!([{ "type": "text", "text": "different" }]);
        script["generations"][0]["frames"] =
            json!([{ "type": "text_delta", "delta": "streamed" }, done]);
    });
    assert!(error_chain(script).contains("streamed content disagrees"));
}

#[test]
fn block_end_cannot_hide_incorrect_text_or_thinking_deltas() {
    for (start_type, delta_type, end_type, block_type) in [
        ("text_start", "text_delta", "text_end", "text"),
        (
            "thinking_start",
            "thinking_delta",
            "thinking_end",
            "thinking",
        ),
    ] {
        let script = minimal_script(|script| {
            let mut done = script["generations"][0]["frames"][0].clone();
            done["message"]["content"] = json!([{ "type": block_type, "text": "authoritative" }]);
            let mut start = done["message"].clone();
            start["content"] = json!([{ "type": block_type, "text": "" }]);
            let end = done["message"].clone();
            script["generations"][0]["frames"] = json!([
                { "type": start_type, "partial": start },
                { "type": delta_type, "delta": "contradictory" },
                { "type": end_type, "partial": end },
                done
            ]);
        });
        assert!(
            error_chain(script).contains("deltas disagree"),
            "{block_type} stream was accepted"
        );
    }
}

#[test]
fn function_call_end_cannot_hide_incorrect_argument_deltas() {
    let script = minimal_script(|script| {
        let mut done = script["generations"][0]["frames"][0].clone();
        done["message"]["stop_reason"] = json!("function_call");
        done["message"]["content"] = json!([{
            "type": "function_call",
            "id": "call-1",
            "function_id": "run::target",
            "arguments": { "value": "authoritative" }
        }]);
        script["generations"][0]["response"]["stop_reason"] = json!("function_call");
        let mut start = done["message"].clone();
        start["content"][0]["arguments"] = json!({});
        let end = done["message"].clone();
        script["generations"][0]["frames"] = json!([
            { "type": "functioncall_start", "partial": start },
            {
                "type": "functioncall_delta",
                "id": "call-1",
                "delta": "{\"value\":\"contradictory\"}"
            },
            { "type": "functioncall_end", "partial": end },
            done
        ]);
    });
    assert!(error_chain(script).contains("deltas disagree"));
}

fn registered_text_scenario(slug: &str, id: &str) -> crate::scenarios::RegisteredScenario {
    crate::scenarios::RegisteredScenario {
        slug: slug.to_string(),
        authored: crate::expand::scenario_template(
            id,
            "A fixture selection test.",
            crate::expand::ScenarioTemplateKind::Text,
        ),
        driver: crate::scenarios::ScenarioDriver::Direct,
    }
}

#[test]
fn all_selector_with_no_runnable_scenario_is_an_error() {
    let empty_error = super::discovery::select_fixtures(Vec::new(), "all", true).unwrap_err();
    assert!(format!("{empty_error:#}").contains("matched no registered scenario"));

    let mut quarantined = registered_text_scenario("only-quarantined", "C-E2E-Q");
    quarantined.authored.quarantine = true;
    let error = super::discovery::select_fixtures(vec![quarantined], "all", false).unwrap_err();
    assert!(format!("{error:#}").contains("no non-quarantined registered scenario"));
}

#[test]
fn explicit_selection_isolated_from_unrelated_invalid_fixtures() {
    // Well-typed but semantically broken: the function call resolves no
    // registered alias, so only compilation can reject it.
    let mut invalid = registered_text_scenario("a-invalid", "C-E2E-INVALID");
    invalid.authored.router.generations[0].reply =
        crate::types::scenario::RouterReplyV1::FunctionCall {
            id: None,
            function: "missing".to_string(),
            arguments: json!({}),
            usage: None,
        };
    let registered = vec![
        invalid,
        registered_text_scenario("z-selected", "C-E2E-SELECTED"),
    ];

    let by_slug =
        super::discovery::select_fixtures(registered.clone(), "z-selected", false).unwrap();
    assert_eq!(by_slug[0].scenario.id, "C-E2E-SELECTED");
    let by_id =
        super::discovery::select_fixtures(registered.clone(), "C-E2E-SELECTED", false).unwrap();
    assert_eq!(by_id[0].scenario.id, "C-E2E-SELECTED");
    assert!(super::discovery::select_fixtures(registered, "all", true).is_err());
}

#[test]
fn duplicate_identities_are_rejected_for_every_selector() {
    let registered = vec![
        registered_text_scenario("first", "C-E2E-DUPLICATE"),
        registered_text_scenario("second", "C-E2E-DUPLICATE"),
    ];

    for selector in ["all", "C-E2E-DUPLICATE", "first"] {
        let error =
            super::discovery::select_fixtures(registered.clone(), selector, true).unwrap_err();
        assert!(
            format!("{error:#}").contains("duplicate scenario id"),
            "{selector}: {error:#}"
        );
    }

    let duplicate_slug = vec![
        registered_text_scenario("twice", "C-E2E-FIRST"),
        registered_text_scenario("twice", "C-E2E-SECOND"),
    ];
    let error = super::discovery::select_fixtures(duplicate_slug, "all", true).unwrap_err();
    assert!(format!("{error:#}").contains("duplicate scenario slug"));
}

#[test]
fn all_selection_can_include_quarantines_for_validation() {
    let all = scenario_fixtures("all", true).unwrap();
    let runnable = scenario_fixtures("all", false).unwrap();
    let all_ids = all
        .iter()
        .map(|fixture| fixture.scenario.id.as_str())
        .collect::<BTreeSet<_>>();
    let runnable_ids = runnable
        .iter()
        .map(|fixture| fixture.scenario.id.as_str())
        .collect::<BTreeSet<_>>();
    let quarantined_ids = all
        .iter()
        .filter(|fixture| fixture.scenario.quarantine)
        .map(|fixture| fixture.scenario.id.as_str())
        .collect::<BTreeSet<_>>();
    let partition = runnable_ids
        .union(&quarantined_ids)
        .copied()
        .collect::<BTreeSet<_>>();

    assert!(
        !all_ids.is_empty(),
        "expected at least one checked-in scenario"
    );
    assert!(runnable_ids.is_disjoint(&quarantined_ids));
    assert_eq!(partition, all_ids);
    assert!(
        scenario_fixtures("crash-recovery-507", false).unwrap()[0]
            .scenario
            .quarantine
    );
}
