//! `register_with_iii` — wires `run::start`, `run::start_and_wait`,
//! `turn::step`, and the subscription that drives the state machine.

use std::sync::Arc;

use iii_sdk::{RegisterTriggerInput, III};
use serde_json::json;

use crate::agent_call;
use crate::config::TurnOrchestratorConfig;
use crate::run_start::{self, STEP_TOPIC};
use crate::subscriber::{self, FUNCTION_ID as STEP_FN_ID};

pub async fn register_with_iii(
    iii: &Arc<III>,
    cfg: &Arc<TurnOrchestratorConfig>,
) -> anyhow::Result<()> {
    run_start::register(iii, cfg);
    agent_call::register(iii);
    subscriber::register(iii);

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "durable:subscriber".into(),
        function_id: STEP_FN_ID.into(),
        config: json!({ "topic": STEP_TOPIC }),
        metadata: None,
    })?;

    Ok(())
}
