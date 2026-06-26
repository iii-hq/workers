use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;

use crate::deps::Deps;
use crate::render::stream;
use crate::telemetry;
use crate::types::MessageUpdatedEvent;

use super::message_added::BindingAck;

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::super::register(
        iii,
        deps,
        super::super::ON_MESSAGE_UPDATED_ID,
        "Stream assistant edits into Telegram, throttled by revision.",
        |d, evt| async move { handle(&d, evt).await },
    );
}

async fn handle(deps: &Deps, evt: MessageUpdatedEvent) -> Result<BindingAck, Error> {
    let session_id = evt.session_id.clone();
    let entry_id = evt.entry_id.clone();
    telemetry::with_session_baggage(deps, &session_id, Some(&entry_id), || async {
        stream::on_message_updated(deps, evt).await
    })
    .await?;
    Ok(BindingAck { ok: true })
}
