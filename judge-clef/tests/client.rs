//! Contract behaviour of the in-process client on the tiny checkpoint.
mod support;

use judge_contract::{
    Answer, ErrorCode, EvaluateRequest, EvaluateResponse, EvaluationResult, ModelsRequest,
    ModelsResponse, Stats,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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

/// One evaluation `e` with state `state` and questions `questions`.
fn one(state: Value, questions: Value) -> EvaluateRequest {
    request(json!({"evaluations": [{"id": "e", "state": state, "questions": questions}]}))
}

fn ok(response: EvaluateResponse) -> (String, BTreeMap<String, EvaluationResult>, Stats) {
    match response {
        EvaluateResponse::Ok {
            model,
            results,
            stats,
        } => (model, results, stats),
        other => panic!("{other:?}"),
    }
}

fn code(response: EvaluateResponse) -> ErrorCode {
    match response {
        EvaluateResponse::Error { code, .. } => code,
        other => panic!("{other:?}"),
    }
}

fn input_tokens(result: &EvaluationResult) -> u64 {
    result.usage.as_ref().unwrap().input_tokens.unwrap()
}

#[tokio::test]
async fn mixed_evaluation_answers_every_question_with_valid_distributions_and_usage() {
    let (model, results, stats) = ok(tiny_client().evaluate(request(json!({}))).await);
    assert_eq!(model, "clef-flash");
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
            confidence,
            legend,
        } => {
            assert!((0.0..=2.0).contains(score));
            assert_eq!(
                probabilities.keys().cloned().collect::<Vec<_>>(),
                ["0", "1", "2"]
            );
            assert!((probabilities.values().sum::<f64>() - 1.0).abs() < 1e-6);
            assert!((0.0..=1.0).contains(confidence));
            assert_eq!(
                legend["1"],
                serde_json::from_value(json!({"impact": "blocking"})).unwrap()
            );
        }
        other => panic!("{other:?}"),
    }
    for (evaluation, qid) in [("ticket", "churn"), ("second", "ok")] {
        assert!(
            matches!(results[evaluation].answers[qid], Answer::Noul { noul } if (0.0..=1.0).contains(&noul))
        );
    }
    // One backbone forward per evaluation, each counting its whole prompt.
    assert_eq!((stats.attempts, stats.requests, stats.questions), (2, 2, 4));
    assert_eq!(
        stats.input_tokens,
        results.values().map(input_tokens).sum::<u64>()
    );
    assert!(results.values().all(|r| input_tokens(r) > 0));
    assert_eq!(stats.output_tokens, 0);
    assert!(stats.usage_complete);
}

#[tokio::test]
async fn a_one_option_choice_answers_with_certainty() {
    let (_, results, _) = ok(tiny_client()
        .evaluate(one(
            json!("s"),
            json!({"q": {"type": "choice", "instructions": "Which?", "criteria": {"only": "the one field"}}}),
        ))
        .await);
    match &results["e"].answers["q"] {
        Answer::Choice {
            choice,
            probabilities,
            confidence,
        } => {
            assert_eq!(choice, "only");
            assert_eq!(probabilities.len(), 1);
            assert!(
                (probabilities["only"] - 1.0).abs() < 1e-9,
                "{probabilities:?}"
            );
            assert_eq!(*confidence, 1.0);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn deadlines_model_names_and_oversized_schemas_fail_typed() {
    let client = tiny_client();
    let noul = |instructions: Value| json!({"type": "noul", "instructions": instructions});
    assert_eq!(
        code(
            client
                .evaluate(request(json!({"expires_at_unix_ms": 1})))
                .await
        ),
        ErrorCode::Deadline
    );
    assert_eq!(
        code(client.evaluate(request(json!({"model": "other"}))).await),
        ErrorCode::InvalidRequest
    );
    // The tiny vocabulary is byte-level: 3000 characters of schema overflow
    // 2048 tokens, and only the state is ever cut.
    let wide = one(json!("s"), json!({"q": noul(json!("x".repeat(3000)))}));
    assert_eq!(
        code(client.evaluate(wide).await),
        ErrorCode::PayloadTooLarge
    );
    // max_request_bytes bounds each evaluation like a provider request body.
    let small = client.with_limits(judge_clef::Limits {
        max_request_bytes: 64,
        ..Default::default()
    });
    assert_eq!(
        code(small.evaluate(request(json!({}))).await),
        ErrorCode::PayloadTooLarge
    );
    // An empty id without instructions leaves an empty question span (NaN in
    // the reference), even beside a valid evaluation.
    let mut empty = request(json!({}));
    empty.evaluations[1] = serde_json::from_value(
        json!({"id": "second", "state": "s", "questions": {"": noul(Value::Null)}}),
    )
    .unwrap();
    assert_eq!(
        code(client.evaluate(empty).await),
        ErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn a_long_state_keeps_its_beginning_within_the_window() {
    let long = one(
        json!("x".repeat(5000)),
        json!({"q": {"type": "noul", "instructions": "?"}}),
    );
    let (_, results, stats) = ok(tiny_client().evaluate(long).await);
    assert_eq!(input_tokens(&results["e"]), 2048);
    assert_eq!(stats.input_tokens, 2048);
}

#[tokio::test]
async fn a_short_deadline_stops_the_engine_and_the_next_call_still_answers() {
    let client = tiny_client();
    // A schema that fits the 2048-token window; the state fills the rest.
    let questions: serde_json::Map<String, Value> = (0..5)
        .map(|i| {
            (
                format!("q{i:02}"),
                json!({"type": "noul", "instructions": format!("Question {i}?")}),
            )
        })
        .collect();
    let mut slow = one(json!("y".repeat(1500)), json!(questions));
    slow.timeout_ms = 1;
    assert_eq!(
        code(client.evaluate(slow.clone()).await),
        ErrorCode::Deadline
    );
    slow.timeout_ms = 60000;
    ok(client.evaluate(slow).await);
}

#[tokio::test]
async fn model_listing_describes_the_loaded_model() {
    let ModelsResponse::Ok { models, stats } =
        tiny_client().list_models(ModelsRequest::default()).await
    else {
        panic!("listing succeeds");
    };
    assert_eq!(models.len(), 1);
    let card = &models[0];
    assert_eq!(card.name, "clef-flash");
    assert!(
        card.description.starts_with("Clef-Flash")
            && card.description.contains("2048-token window"),
        "{}",
        card.description
    );
    assert!(card.release_date.starts_with("local:"));
    assert_eq!(
        (card.context_window, card.max_options),
        (Some(2048), Some(255))
    );
    assert!(stats.usage_complete);
}
