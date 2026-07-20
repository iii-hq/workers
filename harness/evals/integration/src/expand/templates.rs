use std::collections::BTreeMap;

use serde_json::json;

use crate::types::scenario::{
    ExpectationsV1, FunctionResultExpectationV1, IntegrationScenarioV1, MessageCountsExpectationV1,
    RouterReplyV1, ScenarioFunctionV1, TargetCallsExpectationV1,
};
use crate::types::script::{JsonMatcherV1, SchemaVersion1};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioTemplateKind {
    Text,
    Function,
    Hook,
    Crash,
}

/// Minimal valid authored scenario used by the non-interactive `init` CLI.
pub fn scenario_template(
    id: &str,
    description: &str,
    kind: ScenarioTemplateKind,
) -> IntegrationScenarioV1 {
    let mut functions = BTreeMap::new();
    let record = ScenarioFunctionV1 {
        description: "Record one integration fixture value.".to_string(),
        request_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        })
        .as_object()
        .cloned()
        .expect("object"),
        response: json!({
            "content": [{ "type": "text", "text": "recorded" }],
            "is_error": false
        }),
        response_delay_ms: None,
        expose: true,
    };
    if kind != ScenarioTemplateKind::Text {
        functions.insert("record".to_string(), record);
    }

    let mut bindings = Vec::new();
    let mut release = None;
    let mut fault = None;
    let mut timeouts = crate::types::scenario::DeadlinesV1::default();
    if kind == ScenarioTemplateKind::Hook {
        functions.insert(
            "hook".to_string(),
            ScenarioFunctionV1 {
                description: "Hold the controlled call for explicit release.".to_string(),
                request_schema: json!({ "type": "object" })
                    .as_object()
                    .cloned()
                    .expect("object"),
                response: json!({ "decision": "hold" }),
                response_delay_ms: None,
                expose: false,
            },
        );
        bindings.push(crate::types::scenario::TriggerBindingSpecV1 {
            trigger: crate::types::scenario::TriggerKindV1::HookPreTrigger,
            function: "hook".to_string(),
            functions: vec!["record".to_string()],
            priority: 10,
        });
        release = Some(crate::types::scenario::ReleaseV1 {
            function_call_id: "call-1".to_string(),
            action: crate::types::scenario::ReleaseActionV1::Execute,
        });
    }
    if kind == ScenarioTemplateKind::Crash {
        functions
            .get_mut("record")
            .expect("crash template has record")
            .response_delay_ms = Some(8_000);
        fault = Some(crate::types::scenario::FaultV1 {
            kind: crate::types::scenario::FaultKind::EngineSigkill,
            function: Some("record".to_string()),
            after_target_calls: 1,
            restart_delay_ms: 1_500,
        });
        timeouts.scenario_ms = 120_000;
    }

    let first_reply = match kind {
        ScenarioTemplateKind::Text => RouterReplyV1::Text {
            text: "fixture complete".to_string(),
            chunks: vec!["fixture ".to_string(), "complete".to_string()],
            usage: None,
        },
        _ => RouterReplyV1::FunctionCall {
            id: None,
            function: "record".to_string(),
            arguments: json!({ "value": "expected" }),
            usage: None,
        },
    };
    let mut generations = vec![crate::types::scenario::ScenarioGenerationV1 {
        reply: first_reply,
        match_overrides: Default::default(),
    }];
    if kind != ScenarioTemplateKind::Text {
        let match_overrides = if matches!(
            kind,
            ScenarioTemplateKind::Hook | ScenarioTemplateKind::Crash
        ) {
            crate::types::scenario::GenerationMatchOverridesV1 {
                request_id: Some(JsonMatcherV1::Regex {
                    pattern: "^t_[0-9a-f]{32}:[0-9]+$".to_string(),
                }),
                system_prompt: Some(JsonMatcherV1::Present),
                messages: Some(JsonMatcherV1::Present),
                tools: Some(JsonMatcherV1::Present),
                ..Default::default()
            }
        } else {
            Default::default()
        };
        generations.push(crate::types::scenario::ScenarioGenerationV1 {
            reply: RouterReplyV1::Text {
                text: "recorded once".to_string(),
                chunks: Vec::new(),
                usage: None,
            },
            match_overrides,
        });
    }

    let expect = if kind == ScenarioTemplateKind::Text {
        ExpectationsV1 {
            message_counts: Some(MessageCountsExpectationV1 {
                user: 1,
                assistant: 1,
                function_result: 0,
            }),
            assistant_text: Some("fixture complete".to_string()),
            ..Default::default()
        }
    } else {
        let function_result = FunctionResultExpectationV1 {
            function_call_id: "call-1".to_string(),
            function: (kind != ScenarioTemplateKind::Crash).then(|| "record".to_string()),
            content: (kind != ScenarioTemplateKind::Crash)
                .then(|| vec![json!({ "type": "text", "text": "recorded" })]),
            is_error: (kind != ScenarioTemplateKind::Crash).then_some(false),
        };
        let mut calls = vec![TargetCallsExpectationV1 {
            function: "record".to_string(),
            count: 1,
            payload: Some(json!({ "value": "expected" })),
            payload_subset: None,
        }];
        if kind == ScenarioTemplateKind::Hook {
            calls.push(TargetCallsExpectationV1 {
                function: "hook".to_string(),
                count: 1,
                payload: None,
                payload_subset: None,
            });
        }
        ExpectationsV1 {
            message_counts: Some(MessageCountsExpectationV1 {
                user: 1,
                assistant: 2,
                function_result: 1,
            }),
            assistant_text: Some("recorded once".to_string()),
            function_results: vec![function_result],
            calls_closed: true,
            calls,
            ..Default::default()
        }
    };

    IntegrationScenarioV1 {
        schema_version: SchemaVersion1::V1,
        id: id.to_string(),
        description: description.to_string(),
        quarantine: false,
        send: crate::types::scenario::ScenarioSendV1 {
            message: if kind == ScenarioTemplateKind::Text {
                "Return the fixture phrase.".to_string()
            } else {
                "Call the recorder once.".to_string()
            },
            allow: None,
            idempotency_key: None,
        },
        functions,
        router: crate::types::scenario::ScenarioRouterV1 {
            model: None,
            generations,
        },
        bindings,
        release,
        fault,
        timeouts,
        expect,
    }
}
