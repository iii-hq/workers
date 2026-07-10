//! `harness::turn` — the internal durable loop step (enqueued onto the
//! `default` queue). Consumers never call it directly. An unexpected
//! step error finalises the turn as failed rather than retrying forever.

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::turn_loop::{self, TurnStepPayload, TurnStepResult};
use crate::types::turn::TurnStatus;
use iii_helpers::observability::opentelemetry::trace::{Status, TraceContextExt as _};
use iii_helpers::observability::opentelemetry::{Context, KeyValue};

pub async fn handle(deps: &Deps, payload: TurnStepPayload) -> Result<TurnStepResult, HarnessError> {
    // Stamp this turn's identity into OTel baggage for the whole step. Every
    // span the step spawns — session::*, router::chat, and (via the
    // spawn-context propagation in the router/provider clients)
    // provider::*::stream — then carries `iii.session.id`/`iii.message.id`
    // (the BaggageSpanProcessor allowlist), so the turn's spans are
    // attributable and groupable by session/message. `iii.message.id` is the
    // turn id: one assistant turn is one grouped "message".
    //
    // Display metadata rides along for the console's grouped views: the
    // message preview as the `iii.tag.message` trace tag, and the session
    // title as `iii.session.name` (fetched best-effort — a missing title
    // must never fail or delay the step).
    //
    // `iii.tag.kind` / `iii.tag.display_name` are the console timeline's
    // relevant-span convention (see workers/console/docs/timeline-span-tags.md):
    // `harness.turn` vs `harness.subagent` classifies the step, and
    // sub-agent steps get a suggestive `Sub-agent · <task preview>` label so
    // they read as distinct segments from the parent turn's own steps in the
    // per-trace detail waterfall. Requires iii-sdk >= 0.21.2-next.1 (the
    // BaggageSpanProcessor allowlist that used to drop unrecognised keys was
    // removed there).
    let session_id = payload.session_id.clone();
    let turn_id = payload.turn_id.clone();
    let preview = payload
        .message_preview
        .clone()
        .filter(|p| !p.trim().is_empty());
    let session_name = deps.session().await.title(&session_id).await;
    let is_subagent = payload.depth > 0;
    let kind = if is_subagent {
        "harness.subagent"
    } else {
        "harness.turn"
    };
    let subagent_display_name = is_subagent
        .then(|| preview.as_deref().map(|p| format!("Sub-agent · {p}")))
        .flatten();

    let mut baggage: Vec<(&str, &str)> = vec![
        ("iii.session.id", session_id.as_str()),
        ("iii.message.id", turn_id.as_str()),
        ("iii.tag.kind", kind),
    ];
    if let Some(preview) = preview.as_deref() {
        baggage.push(("iii.tag.message", preview));
    }
    if let Some(name) = session_name.as_deref() {
        baggage.push(("iii.session.name", name));
    }
    if let Some(display_name) = subagent_display_name.as_deref() {
        baggage.push(("iii.tag.display_name", display_name));
    }
    // The explicit step span matters: the baggage only materializes as span
    // attributes when a span STARTS inside this scope, and downstream workers
    // may run older SDKs whose processors drop the newer keys. This span is
    // ours, so the turn's trace always carries the tags — and the session::*
    // / router client calls parent under it instead of dangling.
    iii_helpers::observability::run_with_baggage(&baggage, async {
        iii_helpers::observability::run_in_span("harness::turn step", None, || run(deps, payload))
            .await
    })
    .await
}

async fn run(deps: &Deps, payload: TurnStepPayload) -> Result<TurnStepResult, HarnessError> {
    let (session_id, turn_id) = (payload.session_id.clone(), payload.turn_id.clone());
    let result = match turn_loop::run_step(deps, payload).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(session_id = %session_id, turn_id = %turn_id, error = %e, "turn step failed; finalising turn as failed");
            turn_loop::fail_turn(deps, &session_id, &turn_id, &e.to_string()).await
        }
    };
    record_step_status(&result);
    Ok(result)
}

fn record_step_status(result: &TurnStepResult) {
    let status = match result.status {
        TurnStatus::Running => "running",
        TurnStatus::AwaitingFunctions => "awaiting_functions",
        TurnStatus::Completed => "completed",
        TurnStatus::Cancelled => "cancelled",
        TurnStatus::Failed => "failed",
    };
    let cx = Context::current();
    let span = cx.span();
    if !span.span_context().is_valid() {
        return;
    }
    span.set_attribute(KeyValue::new("iii.turn.status", status));
    if result.status == TurnStatus::Failed {
        span.set_status(Status::error("harness turn failed"));
    }
}
