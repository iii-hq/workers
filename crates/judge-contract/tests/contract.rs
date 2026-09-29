use judge_contract::*;
const DEFAULT_MODEL: &str = "jev-latest";
use serde_json::json;

fn request() -> EvaluateRequest {
    serde_json::from_value(json!({
        "timeout_ms": 3000,
        "evaluations": [{
            "id": "ticket", "state": {"message": "I cannot sign in"},
            "questions": {"auth": {"type": "noul", "instructions": "Is message about signing in?"}}
        }]
    }))
    .unwrap()
}

#[test]
fn empty_batch_is_invalid_instead_of_a_successful_no_match() {
    let mut request = request();
    request.evaluations.clear();
    assert_eq!(validate_request(&request), Err(ErrorCode::InvalidRequest));
}

#[test]
fn optional_criteria_and_arbitrary_state_reach_provider_without_bus_fields() {
    let mut request = request();
    request.expires_at_unix_ms = Some(1234);
    validate_request(&request).unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&encode_evaluation(DEFAULT_MODEL, &request.evaluations[0]).unwrap())
            .unwrap();
    assert_eq!(
        json,
        json!({
            "model": DEFAULT_MODEL, "state": {"message": "I cannot sign in"},
            "questions": {"auth": {"type":"noul","instructions":"Is message about signing in?"}}
        })
    );
}

#[test]
fn rejects_ambiguous_inputs_before_sending_any_evaluations() {
    let original = serde_json::to_value(request()).unwrap();
    let mut invalid = Vec::new();
    let mut zero_timeout = original.clone();
    zero_timeout["timeout_ms"] = json!(0);
    invalid.push(zero_timeout);
    for (pointer, value) in [
        ("/model", json!(" ")),
        ("/evaluations/0/id", json!(" ")),
        ("/evaluations/0/questions", json!({})),
        (
            "/evaluations/0/questions/auth/criteria",
            json!({"unknown":"yes"}),
        ),
    ] {
        let mut candidate = original.clone();
        if pointer == "/model" {
            candidate["model"] = value;
        } else if pointer.ends_with("/criteria") {
            candidate["evaluations"][0]["questions"]["auth"]["criteria"] = value;
        } else {
            *candidate.pointer_mut(pointer).unwrap() = value;
        }
        invalid.push(candidate);
    }
    let mut duplicated = original.clone();
    duplicated["evaluations"]
        .as_array_mut()
        .unwrap()
        .push(original["evaluations"][0].clone());
    invalid.push(duplicated);
    for value in invalid {
        assert_eq!(
            validate_request(&serde_json::from_value(value).unwrap()),
            Err(ErrorCode::InvalidRequest)
        );
    }
}

#[test]
fn byte_guards_account_for_utf8_and_all_questions() {
    let limits = 16 * 1024;
    let mut evaluation = request().evaluations.remove(0);
    evaluation.state = json!({"message": "🦀".repeat(5000)});
    assert_eq!(
        encode_evaluation_with_limits(DEFAULT_MODEL, &evaluation, limits),
        Err(ErrorCode::PayloadTooLarge)
    );
    evaluation.state = json!({});
    let question = evaluation.questions["auth"].clone();
    evaluation.questions = (0..1000)
        .map(|n| (format!("q{n}"), question.clone()))
        .collect();
    assert_eq!(
        encode_evaluation_with_limits(DEFAULT_MODEL, &evaluation, limits),
        Err(ErrorCode::PayloadTooLarge)
    );
}

#[test]
fn prevents_unbounded_batch_and_rejects_unknown_primitives() {
    let mut request = request();
    let evaluation = request.evaluations[0].clone();
    request.evaluations = (0..=MAX_EVALUATIONS)
        .map(|n| Evaluation {
            id: n.to_string(),
            ..evaluation.clone()
        })
        .collect();
    assert_eq!(validate_request(&request), Err(ErrorCode::InvalidRequest));
    let mut question = serde_json::to_value(&evaluation.questions["auth"]).unwrap();
    question["type"] = json!("unrecognized");
    assert!(serde_json::from_value::<Question>(question).is_err());
}

#[test]
fn absent_http_status_is_omitted_and_roundtrips() {
    let error = EvaluateResponse::Error {
        code: ErrorCode::MissingKey,
        http_status: None,
        provider_error: None,
        retry_after_ms: None,
        stats: Stats::default(),
    };
    let value = serde_json::to_value(error).unwrap();
    assert!(value.get("http_status").is_none());
    assert!(matches!(
        serde_json::from_value::<EvaluateResponse>(value).unwrap(),
        EvaluateResponse::Error {
            http_status: None,
            ..
        }
    ));
}
