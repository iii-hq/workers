//! `register_with_iii` — wires `run::start`, `run::start_and_wait`,
//! `turn::step`, and the subscription that drives the state machine.

use iii_sdk::{RegisterTriggerInput, III};
use serde_json::json;

use crate::run_start::{self, STEP_TOPIC};
use crate::subscriber::{self, FUNCTION_ID as STEP_FN_ID};

pub async fn register_with_iii(iii: &III) -> anyhow::Result<()> {
    run_start::register(iii);
    subscriber::register(iii);

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: STEP_FN_ID.into(),
        config: json!({ "topic": STEP_TOPIC }),
        metadata: None,
    })?;

    Ok(())
}
