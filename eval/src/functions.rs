use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::json;

use crate::contract::{
    ConfigRequestV1, ConfigureRequestV1, CycleIdRequestV1, CycleListRequestV1, CycleStartRequestV1,
    ExecutionChangedEventV1, ExecutionsRequestV1, StepRequestV1, SweepEventV1,
};
use crate::runtime::Deps;

/// The E2E dashboard's change event; its payload is only a hint to re-read
/// `execution-get`.
pub const E2E_CHANGED: &str = "e2e::dashboard::changed";

pub const CONFIGURE_ID: &str = "eval::configure";
pub const CONFIG_ID: &str = "eval::config";
pub const START_ID: &str = "eval::start";
pub const LIST_ID: &str = "eval::list";
pub const STATUS_ID: &str = "eval::status";
pub const RESULT_ID: &str = "eval::result";
pub const CANCEL_ID: &str = "eval::cancel";
pub const EXECUTIONS_ID: &str = "eval::executions";
pub const STEP_ID: &str = "eval::step";
pub const SWEEP_ID: &str = "eval::sweep";
pub const EXECUTION_CHANGED_ID: &str = "eval::execution-changed";

pub fn register_all(iii: &Arc<IIIClient>, deps: &Deps) {
    let current = deps.clone();
    iii.register_function(
        CONFIGURE_ID,
        RegisterFunction::new_async(move |request: ConfigureRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::configure(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Set the loop configuration every new plan freezes (its revision is the \
             policy_revision): the E2E subject provider/model (and agent), where it runs \
             (harness, or docker/github with a stack {name, yaml}, e.g. package://harness \
             pinned to commit:<sha>), runs per side, technical retries, max scenarios, the \
             per-run cost estimate reserved before dispatch, the minimum effects and the \
             per-cycle budget. Nothing has a default; credentials stay in the providers.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        CONFIG_ID,
        RegisterFunction::new_async(move |request: ConfigRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::config(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Read the loop configuration (null until configured) and the rules version."),
    );

    let current = deps.clone();
    iii.register_function(
        START_ID,
        RegisterFunction::new_async(move |request: CycleStartRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_start(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Start one loop cycle from exactly one input: a free `objective` (mapped onto the \
             E2E catalog; no coverage spends nothing; a covered objective freezes its plan and \
             starts a budgeted baseline) or the `execution_id` of an existing E2E execution \
             (its reports are imported and analyzed without a model). Both converge on the same \
             cycle record; follow it with eval::status and eval::result, stop it with \
             eval::cancel.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        LIST_ID,
        RegisterFunction::new_async(move |request: CycleListRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_list(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "List loop cycles, newest first, whichever input started them, each with its \
             history.",
        ),
    );

    let current = deps.clone();
    iii.register_function(
        STATUS_ID,
        RegisterFunction::new_async(move |request: CycleIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_status(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Read one loop cycle record or null."),
    );

    let current = deps.clone();
    iii.register_function(
        RESULT_ID,
        RegisterFunction::new_async(move |request: CycleIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_result(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Read one loop cycle with what it drew on its budget, or null."),
    );

    let current = deps.clone();
    iii.register_function(
        CANCEL_ID,
        RegisterFunction::new_async(move |request: CycleIdRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::cycle_cancel(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description("Cancel a loop cycle that is not finished and stop the E2E executions it dispatched; a finished cycle is left as is."),
    );

    let current = deps.clone();
    iii.register_function(
        EXECUTIONS_ID,
        RegisterFunction::new_async(move |request: ExecutionsRequestV1| {
            let deps = current.clone();
            async move {
                crate::runtime::executions(&deps, request)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "List recent finished E2E executions to start a cycle from, newest first: each \
             with its scenarios' mean score and what a cycle would restore, or the cycle \
             that already imported it. Executions a cycle dispatched are left out.",
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
        .description("Internal durable cycle step.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        SWEEP_ID,
        RegisterFunction::new_async(move |_event: SweepEventV1| {
            let deps = current.clone();
            async move { crate::runtime::sweep(&deps).await.map_err(Error::from) }
        })
        .description("Internal recovery, reconciliation, budget and admission sweep.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        EXECUTION_CHANGED_ID,
        RegisterFunction::new_async(move |event: ExecutionChangedEventV1| {
            let deps = current.clone();
            async move {
                crate::runtime::execution_changed(&deps, event)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(
            "Internal: on e2e::dashboard::changed, re-read the execution with \
             e2e::dashboard::execution-get and admit a complete terminal execution into one \
             cycle per execution/run/attempt. An execution a cycle dispatched (label \
             eval:<cycle_id>:<role>) returns to that cycle.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
    // The handle is not kept: dropping it leaves the binding in place.
    if let Err(error) = iii.register_trigger(RegisterTriggerInput::new(
        E2E_CHANGED,
        EXECUTION_CHANGED_ID,
        json!({}),
    )) {
        tracing::warn!(%error, "binding {E2E_CHANGED} failed; the sweep still admits executions");
    }
}
