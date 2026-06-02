//! Real-time IDLE listener. Server pushes `EXISTS` → we fetch the new UIDs and
//! dispatch via [`crate::triggers::dispatcher`]. NO polling: the only timer
//! present is the RFC 2177 IDLE refresh (29 minutes), which is per-spec, not
//! application-level polling.

use anyhow::{anyhow, Result};
use async_imap::extensions::idle::IdleResponse;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use super::Session;
use crate::triggers::dispatcher::EventDispatcher;
use crate::triggers::Event;

/// Park on IDLE until the server pushes data or shuts down. Returns the
/// underlying connection error so the supervisor (`connection::run_until_shutdown`)
/// can decide whether to reconnect.
pub async fn run(
    mut session: Session,
    dispatcher: Arc<dyn EventDispatcher>,
    account: String,
    folder: String,
) -> Result<()> {
    // Track highest UID we've already announced so EXISTS pushes only fetch the
    // new tail. On first IDLE, prime with the current mailbox UIDNEXT - 1.
    let mut high_water: u32 = current_uidnext(&mut session, &folder)
        .await?
        .saturating_sub(1);

    loop {
        let mut idle = session.idle();
        idle.init()
            .await
            .map_err(|e| anyhow!("idle init failed: {e}"))?;

        // RFC 2177: re-issue IDLE every < 30 min. Return value resolves the
        // INSTANT the server pushes data; the timeout only fires if the server
        // is silent.
        let (idle_wait, _interrupt) = idle.wait_with_timeout(Duration::from_secs(29 * 60));
        let outcome = idle_wait.await;

        session = idle
            .done()
            .await
            .map_err(|e| anyhow!("idle done failed: {e}"))?;

        match outcome {
            Ok(IdleResponse::NewData(_)) => {
                let new_uids =
                    crate::provider::imap::fetch::uids_above(&mut session, high_water).await?;
                tracing::info!(
                    account = %account,
                    folder = %folder,
                    new_uid_count = new_uids.len(),
                    high_water,
                    "imap IDLE: server pushed data; new uids fetched"
                );
                for uid in new_uids {
                    match crate::provider::imap::fetch::header_summary(&mut session, uid).await {
                        Ok(summary) => {
                            high_water = high_water.max(uid);
                            tracing::info!(
                                account = %account,
                                folder = %folder,
                                uid,
                                from = summary.from.as_deref().unwrap_or(""),
                                subject = summary.subject.as_deref().unwrap_or(""),
                                "imap IDLE: dispatching email::new-mail event"
                            );
                            dispatcher
                                .dispatch(Event {
                                    account: account.clone(),
                                    folder: folder.clone(),
                                    uid,
                                    message_id: summary.message_id,
                                    from: summary.from,
                                    subject: summary.subject,
                                    snippet: summary.snippet,
                                    ts: summary.ts,
                                })
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                account = %account,
                                folder = %folder,
                                uid,
                                error = %e,
                                "header_summary failed; skipping event for this uid"
                            );
                        }
                    }
                }
            }
            Ok(IdleResponse::Timeout) => {
                // Server was silent for 29min — re-IDLE. Not a poll: the wait
                // would have returned earlier if anything arrived.
                tracing::info!(account = %account, folder = %folder, "imap IDLE: 29min refresh");
            }
            Ok(IdleResponse::ManualInterrupt) => {
                tracing::info!(account = %account, folder = %folder, "idle interrupted; exiting");
                return Ok(());
            }
            Err(e) => return Err(anyhow!("idle wait failed: {e}")),
        }
    }
}

async fn current_uidnext(session: &mut Session, folder: &str) -> Result<u32> {
    let mailbox = session
        .select(folder)
        .await
        .map_err(|e| anyhow!("imap SELECT for uidnext failed: {e}"))?;
    Ok(mailbox.uid_next.unwrap_or(1))
}

#[allow(dead_code)]
fn _suppress_unused(_: TlsStream<TcpStream>, _: serde_json::Value) {
    let _ = json!({});
}
