use judge_contract::{EvaluateRequest, EvaluateResponse, EvaluationResult, ModelsRequest};
use serde_json::json;

#[test]
fn request_controls_accept_partial_overrides_and_preserve_legacy_calls() {
    let legacy = json!({"timeout_ms":1000,"evaluations":[{"id":"ticket","state":"Checkout is down","questions":{"urgent":{"type":"noul"}}}]});
    let parsed: EvaluateRequest = serde_json::from_value(legacy.clone()).unwrap();
    let encoded = serde_json::to_value(parsed).unwrap();
    assert_eq!(encoded["options"], json!({}));
    let mut call = legacy;
    call["request_id"] = json!("job-42");
    call["options"] = json!({"attempt_timeout_ms":500});
    let parsed: EvaluateRequest = serde_json::from_value(call).unwrap();
    let wire = serde_json::to_value(parsed).unwrap();
    assert_eq!(wire["options"]["attempt_timeout_ms"], 500);
    assert_eq!(wire["request_id"], "job-42");
}

#[test]
fn usage_distinguishes_missing_counters_from_zero() {
    let parsed: EvaluationResult = serde_json::from_value(
        json!({"answers":{},"usage":{"input_tokens":0,"output_tokens":null}}),
    )
    .unwrap();
    let wire = serde_json::to_value(parsed).unwrap();
    assert_eq!(wire["usage"]["input_tokens"], 0);
    assert_eq!(wire["usage"]["output_tokens"], serde_json::Value::Null);
    assert!(serde_json::from_value::<EvaluationResult>(json!({"answers":{}})).is_ok());
}

#[test]
fn provider_errors_keep_validation_detail_and_retry_delay() {
    let wire = json!({"status":"error","code":"http","http_status":422,"provider_error":{"detail":[{"loc":["body","state"],"msg":"Invalid state"}],"truncated":false},"retry_after_ms":1200,"stats":{"attempts":1,"requests":0,"questions":0,"input_tokens":0,"output_tokens":0,"elapsed_ms":12,"usage_complete":false}});
    let parsed: EvaluateResponse = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
}

#[test]
fn models_accept_the_same_controls() {
    let request: ModelsRequest = serde_json::from_value(
        json!({"request_id":"catalog-refresh","options":{"attempt_timeout_ms":250}}),
    )
    .unwrap();
    let wire = serde_json::to_value(request).unwrap();
    assert_eq!(wire["options"]["attempt_timeout_ms"], 250);
}

#[test]
fn controls_reject_unknown_fields_and_advertise_cancellation_and_usage() {
    for bad in [
        json!({"options":{"retry":{"max_retries":0}}}),
        json!({"options":{"headers":{"x-tag":"a"}}}),
    ] {
        assert!(serde_json::from_value::<ModelsRequest>(bad).is_err());
    }
    let schema = serde_json::to_value(schemars::schema_for!(EvaluateRequest)).unwrap();
    assert_eq!(schema["properties"]["request_id"]["maxLength"], 128);
    assert_eq!(
        schema["definitions"]["RequestOptions"]["additionalProperties"],
        false
    );
    let cancel: judge_contract::CancelRequest =
        serde_json::from_value(json!({"request_id":"job-42"})).unwrap();
    assert_eq!(cancel.request_id, "job-42");
    assert!(serde_json::from_value::<judge_contract::CancelRequest>(
        json!({"request_id":"job-42","caller_id":"someone-else"})
    )
    .is_err());
}
