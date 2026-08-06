use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::json;

use crate::contract::{
    EvalListRequestV1, EvalRerunRequestV1, EvalStartRequestV1, EvaluationIdRequestV1,
    EvaluatorInputV1, StepRequestV1, SweepEventV1, WakeEventV1,
};
use crate::runtime::Deps;

pub const START_ID: &str = "eval::start";
pub const LIST_ID: &str = "eval::list";
pub const RERUN_ID: &str = "eval::rerun";
pub const STATUS_ID: &str = "eval::status";
pub const RESULT_ID: &str = "eval::result";
pub const CANCEL_ID: &str = "eval::cancel";
pub const DELETE_ID: &str = "eval::delete";
pub const EXACT_ID: &str = "eval::assert::exact";
pub const NORMALIZED_TEXT_ID: &str = "eval::assert::normalized_text";
pub const STEP_ID: &str = "eval::step";
pub const WAKE_ID: &str = "eval::on-turn-completed";
pub const SWEEP_ID: &str = "eval::sweep";

pub fn register_all(iii: &Arc<IIIClient>, deps: &Deps) {
    let current = deps.clone();
    iii.register_function(
        START_ID,
        RegisterFunction::new_async(move |request: EvalStartRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::start(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Start a durable same-model A/B evaluation. Exactly one dimension may change: \
             `prompt` or `system_prompt`. Returns evaluation_id immediately; use eval::status, \
             eval::result, or the eval::completed trigger instead of polling in a tight loop.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        RERUN_ID,
        RegisterFunction::new_async(move |request: EvalRerunRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::rerun(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Repeat a terminal evaluation from its persisted request. Set reverse_order to invert \
             the balanced A/B order. The rerun receives a new evaluation ID and fresh sessions.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        LIST_ID,
        RegisterFunction::new_async(move |request: EvalListRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::list(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("List recent evaluations as lightweight summaries, newest first."),
    );

    let current = deps.clone();
    iii.register_function(
        STATUS_ID,
        RegisterFunction::new_async(move |request: EvaluationIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::status(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Read evaluation progress without loading the complete report."),
    );

    let current = deps.clone();
    iii.register_function(
        RESULT_ID,
        RegisterFunction::new_async(move |request: EvaluationIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::result(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Read an evaluation result. The report is present only after terminal completion.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        CANCEL_ID,
        RegisterFunction::new_async(move |request: EvaluationIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cancel(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Cancel an evaluation and its active harness turn."),
    );

    let current = deps.clone();
    iii.register_function(
        DELETE_ID,
        RegisterFunction::new_async(move |request: EvaluationIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::delete(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Delete a terminal evaluation report and its session indexes."),
    );

    iii.register_function(
        EXACT_ID,
        RegisterFunction::new_async(move |input: EvaluatorInputV1| async move {
            crate::runtime::exact(input).map_err(Error::from)
        })
        .description(
            "Built-in deterministic evaluator: deep-compare output with arguments.expected.",
        ),
    );

    iii.register_function(
        NORMALIZED_TEXT_ID,
        RegisterFunction::new_async(move |input: EvaluatorInputV1| async move {
            crate::runtime::normalized_text(input).map_err(Error::from)
        })
        .description(
            "Built-in deterministic evaluator: compare text after normalizing case, \
             whitespace, and surrounding punctuation.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        STEP_ID,
        RegisterFunction::new_async(move |request: StepRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::step(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Internal durable evaluation step.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        WAKE_ID,
        RegisterFunction::new_async(move |event: WakeEventV1| {
            let deps = current.clone();
            async move {
                crate::runtime::wake(&deps, event)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Internal harness turn-completed wake-up.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        SWEEP_ID,
        RegisterFunction::new_async(move |_event: SweepEventV1| {
            let deps = current.clone();
            async move { crate::runtime::sweep(&deps).await.map_err(Error::from) }
        })
        .description("Internal recovery and timeout sweep.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}
