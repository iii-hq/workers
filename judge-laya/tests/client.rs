//! Contract behaviour of the in-process client on the tiny checkpoint.
mod support;

use judge_contract::{Answer, EvaluateRequest, EvaluateResponse, ModelsRequest, ModelsResponse};
use serde_json::{json, Value};
use support::{tiny_client, tiny_client_named};

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
    // One request per evaluation (two here), whatever the batch layout.
    assert_eq!((stats.requests, stats.questions), (2, 4));
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
    assert_eq!((stats.attempts, stats.requests, stats.questions), (4, 2, 4));

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
    assert!(models[0].description.contains("in-process"));
    assert!(models[0].release_date.starts_with("local:"));
    assert_eq!(models[0].context_window, Some(128));
    assert!(stats.usage_complete);
}

fn model_of(response: EvaluateResponse) -> String {
    match ok(response) {
        EvaluateResponse::Ok { model, .. } => model,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn routing_follows_explicit_model_then_workflow_then_state_language() {
    let client = tiny_client_named(&["laya", "laya-multilingual", "laya-typed-decisions"]);
    let routed = client.with_routing(judge_laya::Routing {
        auto_route: true,
        auto_task_detection: true,
        shortlist_k: None,
    });
    let noul = json!({"q": {"type": "noul", "instructions": "Refund?"}});
    let portuguese = "O cliente diz que a fatura foi cobrada duas vezes e pede o reembolso até sexta, não dá para esperar.";
    let english =
        "The customer says the invoice was billed twice and asks for a refund before Friday.";
    let evaluation = |id: &str, state: &str, questions: &Value| json!({"id": id, "state": state, "questions": questions});
    let one = |state: &str, questions: &Value| {
        request(json!({"evaluations": [evaluation("e", state, questions)]}))
    };
    assert_eq!(
        model_of(routed.evaluate(one(portuguese, &noul)).await),
        "laya-multilingual"
    );
    assert_eq!(model_of(routed.evaluate(one(english, &noul)).await), "laya");
    assert_eq!(model_of(routed.evaluate(one("12345", &noul)).await), "laya");
    // Routing off: everything answers on the default checkpoint.
    assert_eq!(
        model_of(client.evaluate(one(portuguese, &noul)).await),
        "laya"
    );
    // An explicit model wins over detection.
    let mut explicit = one(portuguese, &noul);
    explicit.model = Some("laya-typed-decisions".into());
    assert_eq!(
        model_of(routed.evaluate(explicit).await),
        "laya-typed-decisions"
    );
    // The customer-service signature routes to the fine-tuned checkpoint.
    let workflow: Value = ["action", "category", "churn_risk", "needs_human", "urgency"]
        .iter()
        .map(|id| (id.to_string(), json!({"type": "noul", "instructions": id})))
        .collect::<serde_json::Map<_, _>>()
        .into();
    assert_eq!(
        model_of(routed.evaluate(one(english, &workflow)).await),
        "laya-typed-decisions"
    );
    // Mixed languages: each evaluation answers on its own checkpoint, the
    // response names the default, and usage counts both evaluations.
    let mixed = request(json!({"evaluations": [
        evaluation("pt", portuguese, &noul),
        evaluation("en", english, &noul)
    ]}));
    let EvaluateResponse::Ok {
        model,
        results,
        stats,
    } = ok(routed.evaluate(mixed).await)
    else {
        unreachable!()
    };
    assert_eq!(model, "laya");
    assert_eq!(results.len(), 2);
    assert_eq!((stats.attempts, stats.requests, stats.questions), (2, 2, 2));
}

#[tokio::test]
async fn shortlist_keeps_k_options_and_answers_zero_for_the_rest() {
    let labels = [
        "billing",
        "sales",
        "technical",
        "legal",
        "shipping",
        "returns",
        "privacy",
        "other",
    ];
    let criteria: serde_json::Map<String, Value> = labels
        .iter()
        .map(|label| (label.to_string(), json!(format!("questions about {label}"))))
        .collect();
    let question =
        json!({"dept": {"type": "choice", "instructions": "Which team?", "criteria": criteria}});
    let build = || {
        request(
            json!({"evaluations": [{"id": "t", "state": "Billed twice, refund now.", "questions": question}]}),
        )
    };
    let shortlisted = tiny_client().with_routing(judge_laya::Routing {
        shortlist_k: Some(3),
        ..Default::default()
    });
    let EvaluateResponse::Ok { results, .. } = ok(shortlisted.evaluate(build()).await) else {
        unreachable!()
    };
    let Answer::Choice {
        choice,
        probabilities,
        ..
    } = &results["t"].answers["dept"]
    else {
        panic!("choice answer")
    };
    assert_eq!(probabilities.len(), labels.len());
    let kept: Vec<_> = probabilities
        .iter()
        .filter(|(_, p)| **p > 0.0)
        .map(|(k, _)| k)
        .collect();
    assert_eq!(kept.len(), 3, "{probabilities:?}");
    assert!(kept.contains(&choice));
    assert!((probabilities.values().sum::<f64>() - 1.0).abs() < 1e-6);
    // k at or above the option count leaves the question untouched.
    let relaxed = tiny_client().with_routing(judge_laya::Routing {
        shortlist_k: Some(8),
        ..Default::default()
    });
    let EvaluateResponse::Ok { results, .. } = ok(relaxed.evaluate(build()).await) else {
        unreachable!()
    };
    let Answer::Choice { probabilities, .. } = &results["t"].answers["dept"] else {
        panic!("choice answer")
    };
    assert!(probabilities.values().all(|p| *p > 0.0));
}
