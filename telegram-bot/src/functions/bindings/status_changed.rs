use std::sync::Arc;

use iii_sdk::{IIIError, III};

use crate::deps::Deps;
use crate::telemetry;
use crate::types::StatusChangedEvent;

use super::message_added::BindingAck;

pub fn register(iii: &Arc<III>, deps: &Arc<Deps>) {
    super::super::register(
        iii,
        deps,
        super::super::ON_STATUS_CHANGED_ID,
        "Observe session status changes.",
        |d, evt| async move { handle(d, evt).await },
    );
}

async fn handle(deps: Arc<Deps>, evt: StatusChangedEvent) -> Result<BindingAck, IIIError> {
    telemetry::with_session_baggage(&deps, &evt.session_id, None, || async {
        tracing::debug!(
            status = %evt.status,
            reason = evt.status_reason.as_deref().unwrap_or(""),
            "session status changed"
        );
        Ok(BindingAck { ok: true })
    })
    .await
}
