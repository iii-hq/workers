//! Telegram update ingress supervisor — polling loop or webhook registration.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

use crate::clients::telegram;
use crate::config::{PollingConfig, UpdatesAdapter, WorkerConfig};
use crate::deps::Deps;
use crate::functions::webhook;

/// Start ingress for the initial config (called once at boot).
pub async fn start(deps: &Arc<Deps>) {
    let cfg = deps.cfg().await;
    apply_updates_adapter(deps, None, &cfg).await;
}

/// React to a config reload. `prev` is `None` on first boot (handled by [`start`]).
pub async fn apply_config_change(deps: &Arc<Deps>, prev: &WorkerConfig, next: &WorkerConfig) {
    if !ingress_changed(prev, next) {
        return;
    }
    apply_updates_adapter(deps, Some(prev), next).await;
}

fn ingress_changed(prev: &WorkerConfig, next: &WorkerConfig) -> bool {
    match (&prev.updates, &next.updates) {
        (UpdatesAdapter::Polling(_), UpdatesAdapter::Polling(_)) => false,
        (UpdatesAdapter::Webhook(a), UpdatesAdapter::Webhook(b)) => a != b,
        _ => true,
    }
}

async fn apply_updates_adapter(deps: &Arc<Deps>, _prev: Option<&WorkerConfig>, cfg: &WorkerConfig) {
    stop_poller(deps).await;

    match cfg.resolve_updates() {
        UpdatesAdapter::Polling(polling) => {
            if let Err(e) = telegram::delete_webhook(deps).await {
                tracing::warn!(error = %e, "deleteWebhook failed (may already be unset)");
            }
            start_poller(deps.clone(), polling).await;
            tracing::info!("updates adapter: polling");
        }
        UpdatesAdapter::Webhook(webhook) => {
            if webhook.url.trim().is_empty() {
                tracing::warn!("updates adapter: webhook (url not set; call set-webhook)");
                return;
            }
            match telegram::set_webhook(deps, &webhook.url, webhook.secret.as_deref()).await {
                Ok(()) => tracing::info!(url = %webhook.url, "updates adapter: webhook registered"),
                Err(e) => tracing::error!(error = %e, "setWebhook failed"),
            }
        }
    }
}

async fn start_poller(deps: Arc<Deps>, _polling: PollingConfig) {
    let cancel = CancellationToken::new();
    let child_cancel = cancel.clone();
    let deps_for_task = deps.clone();
    let handle = tokio::spawn(async move {
        run_poller(deps_for_task, child_cancel).await;
    });

    let mut guard = deps.runtime.poller_cancel.lock().await;
    *guard = Some(cancel);
    drop(guard);

    let mut handle_guard = deps.runtime.poller_handle.lock().await;
    *handle_guard = Some(handle);
}

/// Stop the background poller on shutdown.
pub async fn shutdown(deps: &Arc<Deps>) {
    stop_poller(deps).await;
}

async fn stop_poller(deps: &Arc<Deps>) {
    let cancel = deps.runtime.poller_cancel.lock().await.take();
    if let Some(token) = cancel {
        token.cancel();
    }
    let handle = deps.runtime.poller_handle.lock().await.take();
    if let Some(h) = handle {
        let _ = h.await;
    }
}

async fn run_poller(deps: Arc<Deps>, cancel: CancellationToken) {
    let mut backoff_secs = 1u64;
    loop {
        if cancel.is_cancelled() {
            return;
        }

        let cfg = deps.cfg().await;
        let UpdatesAdapter::Polling(polling) = cfg.resolve_updates() else {
            return;
        };

        let offset = deps.runtime.poll_offset.load(Ordering::Relaxed);
        let timeout = polling.timeout_seconds.min(50);

        match telegram::get_updates(&deps, offset, timeout).await {
            Ok(updates) => {
                backoff_secs = 1;
                if updates.is_empty() {
                    continue;
                }
                let mut last_id = offset;
                for update in updates {
                    last_id = update.update_id;
                    if let Err(e) = webhook::process_update_with_tracing(&deps, update).await {
                        tracing::warn!(error = %e, "failed to process polled update");
                    }
                }
                deps.runtime
                    .poll_offset
                    .store(last_id.saturating_add(1), Ordering::Relaxed);
            }
            Err(e) => {
                tracing::warn!(error = %e, backoff_secs, "getUpdates failed");
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = sleep(Duration::from_secs(backoff_secs)) => {}
                }
                backoff_secs = (backoff_secs * 2).min(30);
            }
        }
    }
}

/// Advance offset after processing a batch (unit-tested).
pub fn advance_offset(current: i64, last_update_id: i64) -> i64 {
    last_update_id.saturating_add(1).max(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PollingConfig, WebhookConfig};

    #[test]
    fn advance_offset_moves_past_last_id() {
        assert_eq!(advance_offset(0, 42), 43);
        assert_eq!(advance_offset(50, 42), 50);
    }

    #[test]
    fn polling_timeout_change_does_not_restart_ingress() {
        let prev = WorkerConfig {
            bot_token: "t".into(),
            updates: UpdatesAdapter::Polling(PollingConfig {
                timeout_seconds: 50,
            }),
            ..WorkerConfig::default()
        };
        let next = WorkerConfig {
            updates: UpdatesAdapter::Polling(PollingConfig {
                timeout_seconds: 30,
            }),
            ..prev.clone()
        };
        assert!(!ingress_changed(&prev, &next));
    }

    #[test]
    fn adapter_swap_restarts_ingress() {
        let prev = WorkerConfig {
            bot_token: "t".into(),
            updates: UpdatesAdapter::Polling(PollingConfig::default()),
            ..WorkerConfig::default()
        };
        let next = WorkerConfig {
            updates: UpdatesAdapter::Webhook(WebhookConfig {
                url: "https://x".into(),
                secret: None,
            }),
            ..prev.clone()
        };
        assert!(ingress_changed(&prev, &next));
    }
}
