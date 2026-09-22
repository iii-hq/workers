//! Contract behaviour of the in-process client on the tiny checkpoint.
mod support;

use judge_contract::{Answer, EvaluateRequest, EvaluateResponse, ModelsRequest, ModelsResponse};
use serde_json::{json, Value};
use support::tiny_client;

fn request(extra: Value) -> EvaluateRequest {
    let mut value = json!({
        "timeout_ms": 30000,
        "evaluations": [{
            "id": "ticket", "state": {"body": "Billed twice, refund now or we cancel."},
            "questions": {
                "department": {"type": "choice", "criteria": {"billing": "refunds", "technical": null, "sales": {"scope": "new contracts"}}},
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

fn ok(response: EvaluateResponse) -> EvaluateResponse {
    assert!(
        matches!(response, EvaluateResponse::Ok { .. }),
        "{response:?}"
    );
    response
}

#[tokio::test]
async fn mixed_evaluation_answers_every_question_with_valid_distributions_and_complete_usage() {
    let client = tiny_client();
    let response = ok(client.evaluate(request(json!({}))).await);
    let EvaluateResponse::Ok {
        model,
        results,
        stats,
    } = response
    else {
        unreachable!()
    };
    assert_eq!(model, "laya");
    assert_eq!(results.len(), 2);
    let ticket = &results["ticket"];
    assert_eq!(ticket.answers.len(), 3);
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
    let usage = ticket.usage.as_ref().unwrap();
    assert!(usage.input_tokens.unwrap() > 0);
    assert_eq!(usage.output_tokens, Some(0));
    assert_eq!((stats.requests, stats.questions), (1, 4));
    assert_eq!(
        stats.input_tokens,
        results
            .values()
            .map(|r| r.usage.as_ref().unwrap().input_tokens.unwrap())
            .sum::<u64>()
    );
    assert!(stats.usage_complete);
}

#[tokio::test]
async fn batches_split_by_the_configured_size_and_respect_deadlines_and_model_names() {
    let client = tiny_client().with_limits(judge_laya::Limits {
        batch_questions: 1,
        ..Default::default()
    });
    let response = ok(client.evaluate(request(json!({}))).await);
    let EvaluateResponse::Ok { stats, .. } = response else {
        unreachable!()
    };
    assert_eq!((stats.attempts, stats.requests, stats.questions), (4, 4, 4));

    let expired = request(json!({"expires_at_unix_ms": 1}));
    let response = tiny_client().evaluate(expired).await;
    assert!(
        matches!(
            response,
            EvaluateResponse::Error {
                code: judge_contract::ErrorCode::Deadline,
                ..
            }
        ),
        "{response:?}"
    );

    let other = request(json!({"model": "laya-multilingual"}));
    let response = tiny_client().evaluate(other).await;
    assert!(
        matches!(
            response,
            EvaluateResponse::Error {
                code: judge_contract::ErrorCode::InvalidRequest,
                ..
            }
        ),
        "{response:?}"
    );
    let same = request(json!({"model": "laya"}));
    ok(tiny_client().evaluate(same).await);
}

#[tokio::test]
async fn options_that_overflow_the_head_window_are_payload_too_large() {
    // laya shrinks options to fit `head_max_len` (48 here), but 40 four-token
    // options overflow the 128-token window, so later markers fall outside it.
    let criteria: serde_json::Map<String, Value> = (0..40)
        .map(|i| {
            (
                format!("option-{i}"),
                json!("a fairly long description of this option ".repeat(3)),
            )
        })
        .collect();
    let request = request(
        json!({"evaluations": [{"id": "wide", "state": "x", "questions": {"pick": {"type": "choice", "criteria": criteria}}}]}),
    );
    let response = tiny_client().evaluate(request).await;
    assert!(
        matches!(
            response,
            EvaluateResponse::Error {
                code: judge_contract::ErrorCode::PayloadTooLarge,
                ..
            }
        ),
        "{response:?}"
    );
}

#[tokio::test]
async fn model_listing_describes_the_loaded_checkpoint() {
    let ModelsResponse::Ok { models, stats } =
        tiny_client().list_models(ModelsRequest::default()).await
    else {
        panic!("listing succeeds");
    };
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "laya");
    assert!(models[0].description.contains("CPU"));
    assert!(models[0].release_date.starts_with("local:"));
    assert!(stats.usage_complete);
}
