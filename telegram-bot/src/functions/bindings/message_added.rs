use std::sync::Arc;

use iii_sdk::{IIIError, III};

use crate::deps::Deps;
use crate::render::stream;
use crate::telemetry;
use crate::types::MessageAddedEvent;

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct BindingAck {
    pub ok: bool,
}

pub fn register(iii: &Arc<III>, deps: &Arc<Deps>) {
    super::super::register(
        iii,
        deps,
        super::super::ON_MESSAGE_ADDED_ID,
        "Create a Telegram message for each new assistant or function_result entry.",
        |d, evt| async move { handle(&d, evt).await },
    );
}

async fn handle(deps: &Deps, evt: MessageAddedEvent) -> Result<BindingAck, IIIError> {
    let message = match evt.message {
        Some(m) => m,
        None => return Ok(BindingAck { ok: true }),
    };

    let session_id = evt.session_id.clone();
    let entry_id = evt.entry_id.clone();
    let timestamp = evt.timestamp;
    telemetry::with_session_baggage(deps, &session_id, Some(&entry_id), || async {
        stream::on_message_added(deps, &session_id, &entry_id, &message, timestamp).await
    })
    .await?;
    Ok(BindingAck { ok: true })
}
