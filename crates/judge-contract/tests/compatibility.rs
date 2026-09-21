use judge_contract::*;
const DEFAULT_MODEL: &str = "jev-1.13.0";
use serde_json::{json, Value};

#[test]
fn mixed_questions_accept_structured_partial_null_and_omitted_descriptions() {
    let value = json!({
        "timeout_ms": 60000,
        "evaluations": [{
            "id": "ticket", "state": {"body": "Invoice question"},
            "questions": {
                "department": {"type": "choice", "criteria": {
                    "billing": null, "support": {"scope": "bugs"}
                }},
                "severity": {"type": "score", "instructions": ["Assess impact"],
                    "criteria": ["routine", {"impact": "blocking"}]},
                "urgent": {"type": "noul", "instructions": null,
                    "criteria": {"true": ["urgent"]}}
            }
        }]
    });
    let request = serde_json::from_value::<EvaluateRequest>(value);
    assert!(request.is_ok(), "documented mixed request: {request:?}");
    validate_request(&request.unwrap()).unwrap();
}

fn request(question: Value) -> EvaluateRequest {
    serde_json::from_value(json!({
        "timeout_ms": 3000,
        "evaluations": [{"id":"ticket", "state":"Customer cannot pay", "questions":{"q":question}}]
    }))
    .unwrap()
}

#[test]
fn all_documented_description_forms_and_partial_noul_criteria_reach_upstream() {
    let forms = [
        json!(null),
        json!(""),
        json!("Is this urgent?"),
        json!({"question":"Is this urgent?", "flags":[true,1,null]}),
        json!(["urgent",{"examples":["outage"]}]),
    ];
    for instructions in &forms {
        for criteria in [
            json!(null),
            json!({}),
            json!({"true":instructions}),
            json!({"false":instructions}),
            json!({"true":instructions,"false":instructions}),
        ] {
            let question = json!({"type":"noul","instructions":instructions,"criteria":criteria});
            let request = request(question.clone());
            validate_request(&request).unwrap();
            let upstream: Value = serde_json::from_slice(
                &encode_evaluation(DEFAULT_MODEL, &request.evaluations[0]).unwrap(),
            )
            .unwrap();
            assert_eq!(upstream["questions"]["q"]["instructions"], *instructions);
            if !criteria.is_null() {
                assert_eq!(upstream["questions"]["q"]["criteria"], criteria);
            }
        }
    }
    for kind in ["choice", "score"] {
        let criteria = if kind == "choice" {
            json!({"a":null,"b":{"facts":forms}})
        } else {
            json!(["routine",{"facts":forms}])
        };
        let request = request(json!({"type":kind,"criteria":criteria}));
        validate_request(&request).unwrap();
        let upstream: Value = serde_json::from_slice(
            &encode_evaluation(DEFAULT_MODEL, &request.evaluations[0]).unwrap(),
        )
        .unwrap();
        assert_eq!(upstream["questions"]["q"]["criteria"], criteria);
    }
}

#[test]
fn invalid_description_shapes_cardinalities_and_unknown_fields_are_rejected() {
    for instructions in [json!(true), json!(42)] {
        assert!(serde_json::from_value::<Question>(
            json!({"type":"noul","instructions":instructions})
        )
        .is_err());
    }
    for count in [0, 256] {
        let criteria: serde_json::Map<String, Value> =
            (0..count).map(|i| (i.to_string(), Value::Null)).collect();
        assert_eq!(
            validate_request(&request(json!({"type":"choice","criteria":criteria}))),
            Err(ErrorCode::InvalidRequest)
        );
    }
    for count in [0, 1, 11] {
        assert_eq!(
            validate_request(&request(
                json!({"type":"score","criteria":vec![json!("level");count]})
            )),
            Err(ErrorCode::InvalidRequest)
        );
    }
    for count in [1, 255] {
        let criteria: serde_json::Map<String, Value> =
            (0..count).map(|i| (i.to_string(), Value::Null)).collect();
        validate_request(&request(json!({"type":"choice","criteria":criteria}))).unwrap();
    }
    for count in [2, 10] {
        validate_request(&request(
            json!({"type":"score","criteria":vec![json!("level");count]}),
        ))
        .unwrap();
    }
    assert!(serde_json::from_value::<Question>(
        json!({"type":"noul","endpoint":"http://example.invalid"})
    )
    .is_err());
    assert_eq!(
        validate_request(&request(json!({"type":"noul","criteria":{"maybe":null}}))),
        Err(ErrorCode::InvalidRequest)
    );
}

#[test]
fn callers_can_apply_smaller_limits_than_the_generic_defaults() {
    let caller_limits = EncodingLimits {
        max_body_bytes: 48 * 1024,
        max_state_question_bytes: Some(16 * 1024),
    };
    let mut request = request(json!({"type":"noul","instructions":"Is checkout failing?"}));
    request.timeout_ms = 60000;
    request.evaluations[0].state = json!({"events":"Checkout unavailable. ".repeat(4000)});
    validate_request(&request).unwrap();
    let evaluation = &request.evaluations[0];
    let bytes = encode_evaluation(DEFAULT_MODEL, evaluation).unwrap();
    assert!(bytes.len() > caller_limits.max_body_bytes);
    assert_eq!(
        encode_evaluation_with_limits(DEFAULT_MODEL, evaluation, caller_limits),
        Err(ErrorCode::PayloadTooLarge)
    );
    let limit = EncodingLimits {
        max_body_bytes: bytes.len(),
        max_state_question_bytes: None,
    };
    assert_eq!(
        encode_evaluation_with_limits(DEFAULT_MODEL, evaluation, limit).unwrap(),
        bytes
    );
    assert_eq!(
        encode_evaluation_with_limits(
            DEFAULT_MODEL,
            evaluation,
            EncodingLimits {
                max_body_bytes: bytes.len() - 1,
                ..limit
            }
        ),
        Err(ErrorCode::PayloadTooLarge)
    );
}

#[test]
fn every_answer_preserves_provider_fields_and_checks_request_domain() {
    let cases = [
        (json!({"type":"noul"}), json!({"type":"noul","noul":0.95})),
        (
            json!({"type":"choice","criteria":{"billing":null,"support":null,"other":null}}),
            json!({"type":"choice","choice":"billing","probabilities":{"billing":0.34,"support":0.33,"other":0.33},"confidence":0.02}),
        ),
        (
            json!({"type":"score","criteria":["routine",{"impact":"blocking"}]}),
            json!({"type":"score","score":0.8,"probabilities":{"0":0.2,"1":0.8},"confidence":0.4,"legend":{"0":"routine","1":{"impact":"blocking"}}}),
        ),
    ];
    for (q, a) in cases {
        let question: Question = serde_json::from_value(q).unwrap();
        let answer: Answer = serde_json::from_value(a.clone()).unwrap();
        validate_answer(&question, &answer).unwrap();
        assert_eq!(serde_json::to_value(&answer).unwrap(), a);
        let wrong = Question::Choice {
            instructions: Content::Null,
            criteria: std::collections::BTreeMap::from([("different".into(), Content::Null)]),
        };
        assert_eq!(
            validate_answer(&wrong, &answer),
            Err(ErrorCode::InvalidResponse)
        );
    }
}

#[test]
fn invalid_answer_scores_distributions_and_level_sets_are_rejected() {
    let question: Question =
        serde_json::from_value(json!({"type":"score","criteria":["routine","blocking"]})).unwrap();
    let good = json!({"type":"score","score":0.8,"probabilities":{"0":0.2,"1":0.8},"confidence":0.4,"legend":{"0":"routine","1":"blocking"}});
    for (pointer, value) in [
        ("/score", json!(2)),
        ("/score", json!(-1)),
        ("/confidence", json!(1.1)),
        ("/probabilities", json!({"0":0.2})),
        ("/probabilities", json!({"0":0.2,"1":0.3})),
        ("/probabilities", json!({"0":-0.1,"1":1.1})),
        ("/legend", json!({"0":"routine","other":"blocking"})),
    ] {
        let mut invalid = good.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            validate_answer(&question, &serde_json::from_value(invalid).unwrap()),
            Err(ErrorCode::InvalidResponse)
        );
    }
    let rounded:Answer=serde_json::from_value(json!({"type":"score","score":0.5,"probabilities":{"0":0.49,"1":0.5},"confidence":0.0,"legend":{"0":"routine","1":"blocking"}})).unwrap();
    validate_answer(&question, &rounded).unwrap();
    assert_eq!(
        validate_answer(
            &Question::Noul {
                instructions: Content::Null,
                criteria: None
            },
            &Answer::Noul { noul: f64::NAN }
        ),
        Err(ErrorCode::InvalidResponse)
    );
}

#[test]
fn duplicate_answer_distribution_and_legend_keys_are_rejected() {
    for text in [
        r#"{"type":"choice","choice":"a","confidence":1,"probabilities":{"a":0.1,"a":1}}"#,
        r#"{"type":"score","score":0,"confidence":1,"probabilities":{"0":1,"1":0},"legend":{"0":"a","0":"b"}}"#,
    ] {
        assert!(serde_json::from_str::<Answer>(text).is_err());
    }
    assert!(serde_json::from_str::<EvaluationResult>(
        r#"{"answers":{"q":{"type":"noul","noul":0},"q":{"type":"noul","noul":1}}}"#
    )
    .is_err());
}

#[test]
fn model_listing_has_defaults_strict_request_and_typed_error() {
    let request: ModelsRequest = serde_json::from_value(json!({})).unwrap();
    assert_eq!(request.timeout_ms, 30000);
    assert!(
        serde_json::from_value::<ModelsRequest>(json!({"api_key":"not-a-caller-option"})).is_err()
    );
    let response = ModelsResponse::Error {
        code: ErrorCode::Http,
        http_status: Some(429),
        provider_error: None,
        retry_after_ms: None,
        stats: Stats::default(),
    };
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(value["code"], "http");
    assert_eq!(value["http_status"], 429);
    let _: ModelsResponse = serde_json::from_value(value).unwrap();
}

#[test]
fn score_levels_reject_null_as_confirmed_by_provider_and_sdk_schema() {
    assert!(serde_json::from_value::<Question>(
        json!({"type":"score","criteria":[null,"blocking"]})
    )
    .is_err());
}

#[test]
fn score_legend_must_preserve_each_requested_level_and_reject_null() {
    let question: Question = serde_json::from_value(
        json!({"type":"score","criteria":["routine",{"impact":"blocking"}]}),
    )
    .unwrap();
    let swapped=serde_json::from_value::<Answer>(json!({"type":"score","score":1,"confidence":1,"probabilities":{"0":0,"1":1},"legend":{"0":{"impact":"blocking"},"1":"routine"}})).unwrap();
    assert_eq!(
        validate_answer(&question, &swapped),
        Err(ErrorCode::InvalidResponse)
    );
    assert!(serde_json::from_value::<Answer>(json!({"type":"score","score":1,"confidence":1,"probabilities":{"0":0,"1":1},"legend":{"0":null,"1":{"impact":"blocking"}}})).is_err());
}

#[test]
fn schemas_express_state_shapes_noul_keys_and_question_cardinalities() {
    let schema = serde_json::to_value(schemars::schema_for!(EvaluateRequest)).unwrap();
    let definitions = &schema["definitions"];
    assert_eq!(
        definitions["Evaluation"]["properties"]["state"]["$ref"],
        "#/definitions/ScoreLevel"
    );
    assert_eq!(
        definitions["Evaluation"]["properties"]["questions"]["minProperties"],
        1
    );
    let variants = definitions["Question"]["oneOf"].as_array().unwrap();
    for (kind, min, max) in [("choice", 1, 255), ("score", 2, 10)] {
        let variant = variants
            .iter()
            .find(|variant| variant["properties"]["type"]["enum"] == json!([kind]))
            .unwrap();
        let criteria = &variant["properties"]["criteria"];
        let (minimum, maximum) = if kind == "choice" {
            ("minProperties", "maxProperties")
        } else {
            ("minItems", "maxItems")
        };
        assert_eq!(criteria[minimum], min);
        assert_eq!(criteria[maximum], max);
    }
    let noul = variants
        .iter()
        .find(|variant| variant["properties"]["type"]["enum"] == json!(["noul"]))
        .unwrap();
    let criteria = &noul["properties"]["criteria"]["anyOf"][0];
    assert_eq!(criteria["additionalProperties"], false);
    assert_eq!(
        criteria["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["false", "true"]
    );
}
