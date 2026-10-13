//! Contract behaviour of the in-process client on the tiny checkpoint.
mod support;

use judge_contract::{Answer, EvaluateRequest, EvaluateResponse, ModelsRequest, ModelsResponse};
use serde_json::{json, Map, Value};
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

/// `n` options whose long descriptions do not fit one row's head budget,
/// keyed so the contract's sorted order is their numeric order.
fn wide(n: usize) -> Map<String, Value> {
    (0..n)
        .map(|i| {
            (
                format!("option-{i:03}"),
                json!("a fairly long description of this option ".repeat(3)),
            )
        })
        .collect()
}

fn choice(criteria: &Map<String, Value>) -> Value {
    json!({"type": "choice", "instructions": "Which option fits?", "criteria": criteria})
}

/// One evaluation over the state "x".
fn single(questions: Value) -> EvaluateRequest {
    request(json!({"evaluations": [{"id": "e", "state": "x", "questions": questions}]}))
}

fn choice_of(answer: &Answer) -> &str {
    match answer {
        Answer::Choice { choice, .. } => choice,
        other => panic!("{other:?}"),
    }
}

/// `client` with `choice_tournament` on.
fn tournament(client: judge_laya::LayaClient) -> judge_laya::LayaClient {
    client.with_routing(judge_laya::Routing {
        choice_tournament: true,
        ..Default::default()
    })
}

#[tokio::test]
async fn a_choice_is_one_row_unless_the_tournament_is_on_and_it_is_wide() {
    // 16 options never play; 17 only when the operator turned it on.
    for (client, n) in [(tournament(tiny_client()), 16), (tiny_client(), 17)] {
        let criteria = wide(n);
        let EvaluateResponse::Ok { results, stats, .. } = ok(client
            .evaluate(single(json!({"pick": choice(&criteria)})))
            .await)
        else {
            unreachable!()
        };
        assert_eq!(stats.attempts, 1, "{n} options");
        let Answer::Choice { probabilities, .. } = &results["e"].answers["pick"] else {
            panic!("{results:?}")
        };
        assert_eq!(probabilities.len(), n);
        assert!(
            probabilities.values().all(|&p| p > 0.0),
            "{probabilities:?}"
        );
    }
}

#[tokio::test]
async fn a_wide_choice_plays_groups_then_a_final_over_their_winners() {
    let client = tournament(tiny_client());
    for n in [17, 40, 255] {
        let criteria = wide(n);
        let keys: Vec<&String> = criteria.keys().collect();
        let fine = json!({"type": "noul", "instructions": "Fine?"});
        let EvaluateResponse::Ok { results, stats, .. } = ok(client
            .evaluate(single(json!({"pick": choice(&criteria), "fine": fine})))
            .await)
        else {
            unreachable!()
        };
        // The groups share the first round's passes with the other question
        // (16 rows a pass on the tiny checkpoint); the final takes one more.
        let parts = n.div_ceil(16);
        assert_eq!(stats.attempts, (1 + parts).div_ceil(16) + 1, "{n} options");
        assert_eq!((stats.requests, stats.questions), (1, 2));
        let wide_answer = &results["e"].answers["pick"];
        let Answer::Choice {
            choice: picked,
            probabilities,
            confidence,
        } = wide_answer
        else {
            panic!("{wide_answer:?}")
        };
        // Every option is answered and the distribution is whole; the
        // finalists hold all of it.
        assert_eq!(probabilities.keys().collect::<Vec<_>>(), keys);
        assert!((probabilities.values().sum::<f64>() - 1.0).abs() < 1e-9);
        let finalists: Map<String, Value> = probabilities
            .iter()
            .filter(|(_, &p)| p > 0.0)
            .map(|(key, _)| (key.clone(), criteria[key].clone()))
            .collect();
        assert_eq!(finalists.len(), parts, "{probabilities:?}");
        assert!(finalists.contains_key(picked));

        // Played by hand like laya's `predict_tournament`: each group as its
        // own question beside the rest of the request, then the winners as
        // one question. Same rows in the same passes, so the same numbers.
        let mut round: Map<String, Value> = (0..parts)
            .map(|i| {
                let group: Map<String, Value> = keys[i * n / parts..(i + 1) * n / parts]
                    .iter()
                    .map(|&key| (key.clone(), criteria[key].clone()))
                    .collect();
                (format!("group-{i:02}"), choice(&group))
            })
            .collect();
        round.insert("fine".into(), fine.clone());
        let EvaluateResponse::Ok {
            results: played, ..
        } = ok(client.evaluate(single(round.into())).await)
        else {
            unreachable!()
        };
        let winners: Vec<&str> = played["e"]
            .answers
            .iter()
            .filter(|(qid, _)| qid.starts_with("group-"))
            .map(|(_, answer)| choice_of(answer))
            .collect();
        assert_eq!(winners, finalists.keys().collect::<Vec<_>>());
        assert_eq!(
            format!("{:?}", played["e"].answers["fine"]),
            format!("{:?}", results["e"].answers["fine"])
        );
        let EvaluateResponse::Ok { results: last, .. } = ok(client
            .evaluate(single(json!({"pick": choice(&finalists)})))
            .await)
        else {
            unreachable!()
        };
        let Answer::Choice {
            choice: final_choice,
            probabilities: final_probabilities,
            confidence: final_confidence,
        } = &last["e"].answers["pick"]
        else {
            panic!("{last:?}")
        };
        assert_eq!(final_choice, picked);
        assert_eq!(final_confidence, confidence);
        for (key, p) in final_probabilities {
            assert_eq!(probabilities[key], *p, "{key}");
        }
        // Usage counts every row the encoder read, in both rounds.
        let tokens = |results: &std::collections::BTreeMap<
            String,
            judge_contract::EvaluationResult,
        >| { results["e"].usage.as_ref().unwrap().input_tokens.unwrap() };
        assert_eq!(tokens(&results), tokens(&played) + tokens(&last));
        assert_eq!(stats.input_tokens, tokens(&results));
    }
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
    // No option count limit: the window bounds a row, and with
    // `choice_tournament` wide choices fit in any case.
    assert_eq!(models[0].max_options, None);
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
        ..Default::default()
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

    // Latin text no word list identifies is no evidence of English: it takes
    // the default checkpoint like a state without letters (laya #203).
    let multilingual =
        tiny_client_named(&["laya-multilingual", "laya"]).with_routing(judge_laya::Routing {
            auto_route: true,
            ..Default::default()
        });
    for undecided in ["Quero cancelar", "ok thanks", "12345"] {
        assert_eq!(
            model_of(multilingual.evaluate(one(undecided, &noul)).await),
            "laya-multilingual",
            "{undecided}"
        );
        assert_eq!(
            model_of(routed.evaluate(one(undecided, &noul)).await),
            "laya",
            "{undecided}"
        );
    }
    assert_eq!(
        model_of(multilingual.evaluate(one(english, &noul)).await),
        "laya"
    );
}
