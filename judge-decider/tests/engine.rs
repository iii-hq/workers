//! The provider over a real engine, with the tiny GGUF. Run with
//! .github/scripts/judge-e2e.sh.
#[path = "../../judge-typesafe/tests/support/mod.rs"]
mod engine_support;
mod support;

use engine_support::{connect, invoke, wait_for, Engine};
use judge_contract::EvaluateResponse;
use judge_decider::{CANCEL_ID, EVALUATE_ID, MODELS_ID};
use serde_json::json;

#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine, no network"]
async fn tiny_gguf_answers_through_a_real_engine() {
    let engine = Engine::start("judge_decider").await;
    let provider = connect(&engine.url, "judge-decider").await;
    let consumer = connect(&engine.url, "ticket-consumer").await;
    judge_decider::register(
        &provider,
        judge_decider::configuration::new_cell(judge_decider::DeciderConfig::default()),
        support::tiny_client(),
    );
    for id in [EVALUATE_ID, MODELS_ID, CANCEL_ID] {
        wait_for(&consumer, id).await;
    }
    let evaluated = invoke(
        &consumer,
        EVALUATE_ID,
        json!({"timeout_ms": 30000, "evaluations": [{
            "id": "ticket", "state": {"message": "Sign-in is blocked"},
            "questions": {
                "urgent": {"type": "noul", "instructions": "Is this urgent?"},
                "department": {"type": "choice", "criteria": {"billing": null, "support": "bugs"}},
                "severity": {"type": "score", "criteria": ["routine", {"impact": "blocking"}]}
            }
        }]}),
    )
    .await
    .unwrap();
    assert_eq!(evaluated["status"], "ok", "{evaluated}");
    assert_eq!(evaluated["model"], "decider-4b-v2");
    assert_eq!(evaluated["stats"]["questions"], 3);
    assert_eq!(evaluated["stats"]["usage_complete"], true);
    assert_eq!(
        evaluated["results"]["ticket"]["answers"]["severity"]["legend"]["1"],
        json!({"impact": "blocking"})
    );
    serde_json::from_value::<EvaluateResponse>(evaluated).unwrap();
    let listed = invoke(&consumer, MODELS_ID, json!({"timeout_ms": 3000}))
        .await
        .unwrap();
    assert_eq!(listed["models"][0]["name"], "decider-4b-v2", "{listed}");
    let denied = invoke(&consumer, CANCEL_ID, json!({"request_id": "none"}))
        .await
        .unwrap();
    assert_eq!(denied, json!({"status": "ok", "cancelled": false}));
    provider.shutdown_async().await;
    consumer.shutdown_async().await;
}
