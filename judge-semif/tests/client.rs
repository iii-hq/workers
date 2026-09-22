//! Contract behaviour of the in-process client on the tiny GGUF.
mod support;

use judge_contract::{Answer, EvaluateRequest, EvaluateResponse, ModelsRequest, ModelsResponse};
use serde_json::{json, Value};
use support::tiny_client;

fn request(extra: Value) -> EvaluateRequest {
    let mut value = json!({
        "timeout_ms": 60000,
        "evaluations": [{
            "id": "ticket", "state": {"body": "Billed twice, refund now or we cancel."},
            "questions": {
                "department": {"type": "choice", "instructions": "Which team?", "criteria": {"billing": "refunds", "technical": null, "sales": {"scope": "new contracts"}}},
                "urgency": {"type": "score", "instructions": ["How urgent?"], "criteria": ["low", {"impact": "blocking"}, "critical"]},
                "churn": {"type": "noul", "instructions": "Threatens to cancel?", "criteria": {"true": "explicit threat"}}
            }
        }, {
            "id": "second", "state": "Plain text state", "questions": {"ok": {"type": "noul", "instructions": "Fine?"}}
        }]
    });
    if let (Some(base), Some(over)) = (value.as_object_mut(), extra.as_object()) {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(value).unwrap()
}

fn ok(
    response: EvaluateResponse,
) -> (
    String,
    std::collections::BTreeMap<String, judge_contract::EvaluationResult>,
    judge_contract::Stats,
) {
    match response {
        EvaluateResponse::Ok {
            model,
            results,
            stats,
        } => (model, results, stats),
        other => panic!("{other:?}"),
    }
}

fn code(response: EvaluateResponse) -> judge_contract::ErrorCode {
    match response {
        EvaluateResponse::Error { code, .. } => code,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn mixed_evaluation_answers_every_question_with_valid_distributions_and_usage() {
    let (model, results, stats) = ok(tiny_client().evaluate(request(json!({}))).await);
    assert_eq!(model, "qwen3.5-4b");
    let ticket = &results["ticket"];
    match &ticket.answers["department"] {
        Answer::Choice {
            choice,
            probabilities,
            confidence,
        } => {
            assert_eq!(
                probabilities.keys().cloned().collect::<Vec<_>>(),
                ["billing", "sales", "technical"]
            );
            assert!((probabilities.values().sum::<f64>() - 1.0).abs() < 1e-6);
            assert!(probabilities.contains_key(choice));
            assert!((0.0..=1.0).contains(confidence));
        }
        other => panic!("{other:?}"),
    }
    match &ticket.answers["urgency"] {
        Answer::Score {
            score,
            probabilities,
            legend,
            ..
        } => {
            assert!((0.0..=2.0).contains(score));
            assert_eq!(probabilities.len(), 3);
            assert_eq!(
                legend["1"],
                serde_json::from_value(json!({"impact": "blocking"})).unwrap()
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(ticket.answers["churn"], Answer::Noul { noul } if (0.0..=1.0).contains(&noul))
    );
    // One request per evaluation; tokens counted once per shared prefix.
    assert_eq!((stats.attempts, stats.requests, stats.questions), (1, 2, 4));
    let per_evaluation: u64 = results
        .values()
        .map(|r| r.usage.as_ref().unwrap().input_tokens.unwrap())
        .sum();
    assert_eq!(stats.input_tokens, per_evaluation);
    assert!(stats.input_tokens > 0 && stats.usage_complete);
}

#[tokio::test]
async fn shared_prefix_reuse_matches_fresh_scoring() {
    let client = tiny_client();
    let state = json!({"body": "The deployment finished and health checks passed in every zone."});
    let questions: serde_json::Map<String, Value> = (0..5)
        .map(|i| {
            (
                format!("q{i}"),
                json!({"type": "noul", "instructions": format!("Question number {i}?")}),
            )
        })
        .collect();
    let together =
        request(json!({"evaluations": [{"id": "all", "state": state, "questions": questions}]}));
    let (_, shared, stats) = ok(client.evaluate(together).await);
    for (qid, question) in &questions {
        let alone = request(
            json!({"evaluations": [{"id": "one", "state": state, "questions": {qid.clone(): question}}]}),
        );
        let (_, fresh, _) = ok(client.evaluate(alone).await);
        let (Answer::Noul { noul: a }, Answer::Noul { noul: b }) =
            (&shared["all"].answers[qid], &fresh["one"].answers[qid])
        else {
            panic!("noul answers")
        };
        assert!((a - b).abs() < 1e-3, "{qid}: shared {a} vs fresh {b}");
    }
    // Five suffixes after one prefix cost less than five full prompts.
    let (_, single, _) = ok(client.evaluate(request(json!({"evaluations": [{"id": "one", "state": state, "questions": {"q0": questions["q0"]}}]}))).await);
    assert!(stats.input_tokens < 5 * single["one"].usage.as_ref().unwrap().input_tokens.unwrap());
}

#[tokio::test]
async fn deadlines_model_names_and_oversized_prompts_fail_typed() {
    let client = tiny_client();
    assert_eq!(
        code(
            client
                .evaluate(request(json!({"expires_at_unix_ms": 1})))
                .await
        ),
        judge_contract::ErrorCode::Deadline
    );
    assert_eq!(
        code(client.evaluate(request(json!({"model": "other"}))).await),
        judge_contract::ErrorCode::InvalidRequest
    );
    // The tiny vocabulary is byte-level: 5000 characters overflow 2048 tokens.
    let long = request(
        json!({"evaluations": [{"id": "e", "state": "x".repeat(5000), "questions": {"q": {"type": "noul", "instructions": "?"}}}]}),
    );
    assert_eq!(
        code(client.evaluate(long).await),
        judge_contract::ErrorCode::PayloadTooLarge
    );
    let many: serde_json::Map<String, Value> =
        (0..17).map(|i| (format!("o{i:02}"), Value::Null)).collect();
    let wide = request(
        json!({"evaluations": [{"id": "e", "state": "s", "questions": {"q": {"type": "choice", "criteria": many}}}]}),
    );
    assert_eq!(
        code(client.evaluate(wide).await),
        judge_contract::ErrorCode::PayloadTooLarge
    );
}

#[tokio::test]
async fn a_short_deadline_stops_the_engine_and_the_next_call_still_answers() {
    let client = tiny_client();
    let questions: serde_json::Map<String, Value> = (0..40)
        .map(|i| {
            (
                format!("q{i:02}"),
                json!({"type": "noul", "instructions": format!("Question {i}?")}),
            )
        })
        .collect();
    let slow = request(
        json!({"timeout_ms": 1, "evaluations": [{"id": "e", "state": "y".repeat(1500), "questions": questions}]}),
    );
    assert_eq!(
        code(client.evaluate(slow).await),
        judge_contract::ErrorCode::Deadline
    );
    ok(client.evaluate(request(json!({}))).await);
}

#[tokio::test]
async fn model_listing_describes_the_loaded_model() {
    let ModelsResponse::Ok { models, stats } =
        tiny_client().list_models(ModelsRequest::default()).await
    else {
        panic!("listing succeeds");
    };
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "qwen3.5-4b");
    assert!(
        models[0].description.contains("2048-token window"),
        "{}",
        models[0].description
    );
    assert!(models[0].release_date.starts_with("local:"));
    assert!(stats.usage_complete);
}
