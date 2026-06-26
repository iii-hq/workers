use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;

use crate::clients::telegram;
use crate::deps::Deps;
use crate::kv;
use crate::types::PendingResolvedEvent;

use super::message_added::BindingAck;

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::super::register(
        iii,
        deps,
        super::super::ON_PENDING_RESOLVED_ID,
        "Clear the approval prompt when a held call is resolved.",
        |d, evt| async move { handle(&d, evt).await },
    );
}

async fn handle(deps: &Deps, evt: PendingResolvedEvent) -> Result<BindingAck, Error> {
    let Some(chat_id) = kv::chat_id_for_session(deps, &evt.session_id).await else {
        return Ok(BindingAck { ok: true });
    };
    let Some(message_id) =
        kv::approval_message_id(deps, &evt.session_id, &evt.function_call_id).await
    else {
        return Ok(BindingAck { ok: true });
    };

    let label = match evt.outcome.as_str() {
        "allow" => "✅ approved",
        "deny" => "❌ rejected",
        "timeout" => "⏱ timed out",
        "aborted" => "⏹ aborted",
        other => other,
    };
    let _ = telegram::edit_message_text(deps, chat_id, message_id, label, None).await;
    let _ = telegram::edit_message_reply_markup(deps, chat_id, message_id).await;
    Ok(BindingAck { ok: true })
}
