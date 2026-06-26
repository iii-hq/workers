use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;

use crate::config::SteeringMode;
use crate::deps::Deps;
use crate::kv;
use crate::render::{stream, verbosity};
use crate::telemetry;
use crate::types::TurnCompletedEvent;

use super::message_added::BindingAck;

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::super::register(
        iii,
        deps,
        super::super::ON_TURN_COMPLETED_ID,
        "Finalize streaming, drain FIFO queue, and post turn outcome toasts.",
        |d, evt| async move { handle(&d, evt).await },
    );
}

async fn handle(deps: &Deps, evt: TurnCompletedEvent) -> Result<BindingAck, Error> {
    telemetry::with_baggage(&evt.session_id, &evt.turn_id, || async {
        let cfg = deps.cfg().await;

        stream::finalize_session(deps, &evt.session_id).await?;

        deps.runtime.active_turns.remove(&evt.session_id);

        let Some(chat_id) = kv::chat_id_for_session(deps, &evt.session_id).await else {
            return Ok(BindingAck { ok: true });
        };

        let status_msg = verbosity::turn_status_message(
            &evt.status,
            evt.reason.as_deref(),
            evt.result_error.as_deref(),
        );
        if !status_msg.is_empty() {
            let order_key = deps
                .runtime
                .last_created_order
                .get(&chat_id)
                .map(|r| *r.value() + 1)
                .unwrap_or(i64::MAX);
            let entry_id = format!("_turn_status_{}", evt.turn_id);
            let _ = stream::send_chat_message_in_order(
                deps,
                &cfg,
                stream::OrderedChatMessage {
                    chat_id,
                    order_key,
                    entry_id: &entry_id,
                    chunk_idx: 0,
                    text: &status_msg,
                    reply_markup: None,
                },
            )
            .await;
        }

        if cfg.steering_mode == SteeringMode::Fifo {
            drain_fifo(deps, chat_id, &evt.session_id, &cfg).await?;
        }

        Ok(BindingAck { ok: true })
    })
    .await
}

async fn drain_fifo(
    deps: &Deps,
    chat_id: i64,
    session_id: &str,
    cfg: &crate::config::WorkerConfig,
) -> Result<(), Error> {
    let text = deps
        .runtime
        .fifo_queues
        .get(&chat_id)
        .and_then(|q| q.first().cloned());

    let Some(text) = text else {
        return Ok(());
    };

    let Some(model) = kv::chat_model(deps, chat_id).await else {
        tracing::warn!(chat_id, "FIFO drain skipped: no model selected");
        return Ok(());
    };

    crate::functions::webhook::send_user_message(
        deps,
        chat_id,
        Some(session_id),
        &text,
        None,
        cfg,
        &model,
    )
    .await?;

    if let Some(mut q) = deps.runtime.fifo_queues.get_mut(&chat_id) {
        if !q.is_empty() {
            q.remove(0);
        }
    }
    Ok(())
}
