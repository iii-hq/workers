//! A cancel that lands between a choice tournament's rounds, on the tiny
//! checkpoint. Alone in its binary: the hook is a thread-local tracing
//! subscriber, and another test hitting the same event on its own thread can
//! leave the event's cached interest without it.
mod support;

use judge_contract::{CancelRequest, ErrorCode, EvaluateResponse};
use judge_laya::LayaClient;
use serde_json::{json, Map, Value};
use support::tiny_client;
use tracing_subscriber::layer::SubscriberExt;

/// Cancels `request_id` on its client as the tournament's next round is cut,
/// after the first round's passes and before the final's.
struct CancelAtNextRound {
    client: LayaClient,
    request_id: &'static str,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CancelAtNextRound {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        if message.0 == "choice tournament round" {
            self.client.cancel(CancelRequest {
                request_id: self.request_id.into(),
            });
        }
    }
}

#[tokio::test]
async fn a_cancel_between_the_rounds_never_answers_the_final() {
    let client = tiny_client().with_routing(judge_laya::Routing {
        choice_tournament: true,
        ..Default::default()
    });
    let _cancels =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(CancelAtNextRound {
            client: client.clone(),
            request_id: "wide",
        }));
    // 40 options: three groups in the first pass, the final in a second.
    let criteria: Map<String, Value> = (0..40)
        .map(|i| {
            (
                format!("option-{i:03}"),
                json!("a long description ".repeat(6)),
            )
        })
        .collect();
    let request = json!({
        "timeout_ms": 30000, "request_id": "wide",
        "evaluations": [{"id": "e", "state": "x", "questions": {
            "pick": {"type": "choice", "instructions": "Which option fits?", "criteria": criteria}
        }}]
    });
    let response = client
        .evaluate(serde_json::from_value(request).unwrap())
        .await;
    let EvaluateResponse::Error {
        code: ErrorCode::Cancelled,
        stats,
        ..
    } = response
    else {
        panic!("{response:?}")
    };
    // The first round's pass ran; the final's stopped before an answer.
    assert_eq!((stats.attempts, stats.requests), (2, 0));
    // The engine still answers. Its passes run in order, so this also waits
    // out the cancelled one: exiting with a pass in flight can crash llama.cpp.
    let next = json!({"timeout_ms": 30000, "evaluations": [
        {"id": "e", "state": "x", "questions": {"ok": {"type": "noul"}}}
    ]});
    let response = client.evaluate(serde_json::from_value(next).unwrap()).await;
    assert!(
        matches!(response, EvaluateResponse::Ok { .. }),
        "{response:?}"
    );
}
