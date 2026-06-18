use std::sync::Arc;

use iii_sdk::{IIIError, III};

use crate::deps::Deps;
use crate::kv;
use crate::render::typing;
use crate::telemetry;
use crate::types::StatusChangedEvent;

use super::message_added::BindingAck;

pub fn register(iii: &Arc<III>, deps: &Arc<Deps>) {
    super::super::register(
        iii,
        deps,
        super::super::ON_STATUS_CHANGED_ID,
        "Send typing indicator while the session is working.",
        |d, evt| async move { handle(d, evt).await },
    );
}

async fn handle(deps: Arc<Deps>, evt: StatusChangedEvent) -> Result<BindingAck, IIIError> {
    telemetry::with_session_baggage(&deps, &evt.session_id, None, || async {
        let Some(chat_id) = kv::chat_id_for_session(&deps, &evt.session_id).await else {
            return Ok(BindingAck { ok: true });
        };

        tracing::debug!(
            status = %evt.status,
            reason = evt.status_reason.as_deref().unwrap_or(""),
            "session status changed"
        );

        match evt.status.as_str() {
            "working" => {
                typing::start_typing_if_allowed(deps.clone(), evt.session_id.clone(), chat_id)
                    .await?;
            }
            "done" | "error" | "idle" => {
                typing::suppress_typing(&deps, &evt.session_id);
            }
            _ => {}
        }
        Ok(BindingAck { ok: true })
    })
    .await
}
