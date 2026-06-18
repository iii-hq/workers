//! Typing indicator refresh while a session is `working`.
//!
//! Telegram `sendChatAction(typing)` lasts ~5s on clients and has no cancel API.
//! We defer the first ping when draft streaming will show progress, guard refresh
//! ticks with a generation counter, and abort in-flight HTTP on suppress.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::IIIError;
use tokio_util::sync::CancellationToken;

use crate::clients::telegram;
use crate::config::StreamTransport;
use crate::deps::Deps;

/// Delay before the first `sendChatAction` so thinking/answer drafts can appear first.
const TYPING_START_DELAY: Duration = Duration::from_millis(400);
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);

/// Clear typing suppression so a new turn may show the indicator.
pub fn allow_typing(deps: &Deps, session_id: &str) {
    deps.runtime.typing_suppressed.remove(session_id);
    deps.runtime.typing_output_seen.remove(session_id);
    deps.runtime.bump_typing_generation(session_id);
}

/// Stop typing and block restarts until [`allow_typing`] (e.g. next user send).
pub fn suppress_typing(deps: &Deps, session_id: &str) {
    deps.runtime
        .typing_suppressed
        .insert(session_id.to_string(), ());
    stop_typing(deps, session_id);
}

/// Send typing once and start a refresh loop (re-pings every ~4s until stopped).
pub async fn start_typing(
    deps: Arc<Deps>,
    session_id: String,
    chat_id: i64,
) -> Result<(), IIIError> {
    start_typing_if_allowed(deps, session_id, chat_id).await
}

/// Start typing only when the session is not suppressed and output has not
/// already appeared (guards stale `working` events).
pub async fn start_typing_if_allowed(
    deps: Arc<Deps>,
    session_id: String,
    chat_id: i64,
) -> Result<(), IIIError> {
    if deps.runtime.typing_suppressed.contains_key(&session_id) {
        return Ok(());
    }
    if deps.runtime.typing_output_seen.contains_key(&session_id) {
        return Ok(());
    }
    if draft_feedback_available(&deps, chat_id).await {
        return Ok(());
    }

    stop_typing_refresh(&deps, &session_id);

    let generation = deps.runtime.typing_generation(&session_id);
    let cancel = CancellationToken::new();
    deps.runtime
        .typing_tasks
        .insert(session_id.clone(), cancel.clone());

    let deps_for_task = deps.clone();
    tokio::spawn(async move {
        run_typing_loop(deps_for_task, session_id, chat_id, generation, cancel).await;
    });

    Ok(())
}

/// Stop the refresh loop for a session. Idempotent.
pub fn stop_typing(deps: &Deps, session_id: &str) {
    stop_typing_refresh(deps, session_id);
}

/// Stop typing once visible assistant output is on screen (turn may still be active).
pub fn stop_typing_on_output(deps: &Deps, session_id: &str) {
    deps.runtime
        .typing_output_seen
        .insert(session_id.to_string(), ());
    stop_typing(deps, session_id);
}

async fn draft_feedback_available(deps: &Deps, chat_id: i64) -> bool {
    if deps.runtime.draft_disabled_chats.contains_key(&chat_id) {
        return false;
    }
    let cfg = deps.cfg().await;
    matches!(
        cfg.streaming.transport,
        StreamTransport::Draft | StreamTransport::Auto
    )
}

fn typing_still_allowed(deps: &Deps, session_id: &str, generation: u64) -> bool {
    deps.runtime.typing_generation(session_id) == generation
        && !deps.runtime.typing_suppressed.contains_key(session_id)
        && !deps.runtime.typing_output_seen.contains_key(session_id)
}

async fn run_typing_loop(
    deps: Arc<Deps>,
    session_id: String,
    chat_id: i64,
    generation: u64,
    cancel: CancellationToken,
) {
    tokio::select! {
        _ = cancel.cancelled() => {
            deps.runtime.typing_tasks.remove(&session_id);
            return;
        }
        _ = tokio::time::sleep(TYPING_START_DELAY) => {}
    }

    if !typing_still_allowed(&deps, &session_id, generation) {
        deps.runtime.typing_tasks.remove(&session_id);
        return;
    }

    if send_typing_ping(&deps, chat_id, &session_id, generation, &cancel)
        .await
        .is_err()
    {
        deps.runtime.typing_tasks.remove(&session_id);
        return;
    }

    let mut interval = tokio::time::interval(TYPING_REFRESH_INTERVAL);
    interval.tick().await; // skip immediate tick; initial typing already sent

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = interval.tick() => {
                if cancel.is_cancelled() {
                    break;
                }
                if !typing_still_allowed(&deps, &session_id, generation) {
                    break;
                }
                if send_typing_ping(&deps, chat_id, &session_id, generation, &cancel)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }

    deps.runtime.typing_tasks.remove(&session_id);
}

async fn send_typing_ping(
    deps: &Deps,
    chat_id: i64,
    session_id: &str,
    generation: u64,
    cancel: &CancellationToken,
) -> Result<(), IIIError> {
    if !typing_still_allowed(deps, session_id, generation) {
        return Err(IIIError::Handler("typing stale".into()));
    }

    match telegram::send_chat_action_cancellable(deps, chat_id, "typing", cancel).await {
        Ok(()) => {}
        Err(e) if format!("{e}").contains("cancelled") => {
            return Err(e);
        }
        Err(e) => {
            tracing::debug!(error = %e, "sendChatAction failed");
        }
    }

    if !typing_still_allowed(deps, session_id, generation) {
        return Err(IIIError::Handler("typing stale after send".into()));
    }

    Ok(())
}

fn stop_typing_refresh(deps: &Deps, session_id: &str) {
    deps.runtime.bump_typing_generation(session_id);
    if let Some((_, token)) = deps.runtime.typing_tasks.remove(session_id) {
        token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allow_typing, start_typing_if_allowed, stop_typing_on_output, stop_typing_refresh,
        suppress_typing, typing_still_allowed, TYPING_START_DELAY,
    };
    use crate::config::{StreamTransport, StreamingConfig, WorkerConfig};
    use crate::deps::{Deps, RuntimeState};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;
    use tokio_util::sync::CancellationToken;

    fn deps_with(runtime: Arc<RuntimeState>) -> Deps {
        Deps {
            iii: Arc::new(iii_sdk::III::new("ws://127.0.0.1:1")),
            config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
            runtime,
        }
    }

    async fn deps_with_transport(transport: StreamTransport) -> Arc<Deps> {
        let runtime = Arc::new(RuntimeState::new());
        let mut cfg = WorkerConfig::default();
        cfg.streaming = StreamingConfig {
            transport,
            ..StreamingConfig::default()
        };
        Arc::new(Deps {
            iii: Arc::new(iii_sdk::III::new("ws://127.0.0.1:1")),
            config: Arc::new(RwLock::new(Arc::new(cfg))),
            runtime,
        })
    }

    #[test]
    fn stop_typing_is_idempotent() {
        let runtime = Arc::new(RuntimeState::new());
        let token = CancellationToken::new();
        runtime.typing_tasks.insert("s1".into(), token.clone());

        let deps = deps_with(runtime.clone());
        let gen_before = runtime.typing_generation("s1");
        stop_typing_refresh(&deps, "s1");
        assert!(!runtime.typing_tasks.contains_key("s1"));
        assert!(token.is_cancelled());
        assert_eq!(runtime.typing_generation("s1"), gen_before + 1);

        stop_typing_refresh(&deps, "s1");
    }

    #[test]
    fn suppress_bumps_generation() {
        let runtime = Arc::new(RuntimeState::new());
        runtime.typing_generation.insert("s1".into(), 2);
        let deps = deps_with(runtime.clone());
        suppress_typing(&deps, "s1");
        assert_eq!(runtime.typing_generation("s1"), 3);
        assert!(runtime.typing_suppressed.contains_key("s1"));
    }

    #[test]
    fn stale_generation_fails_still_allowed() {
        let runtime = Arc::new(RuntimeState::new());
        runtime.typing_generation.insert("s1".into(), 2);
        let deps = deps_with(runtime);
        assert!(typing_still_allowed(&deps, "s1", 2));
        assert!(!typing_still_allowed(&deps, "s1", 1));
    }

    #[tokio::test]
    async fn suppressed_session_skips_typing_start() {
        let runtime = Arc::new(RuntimeState::new());
        let deps = Arc::new(deps_with(runtime.clone()));
        suppress_typing(&deps, "s1");

        start_typing_if_allowed(deps.clone(), "s1".into(), 42)
            .await
            .unwrap();

        assert!(!runtime.typing_tasks.contains_key("s1"));
    }

    #[tokio::test]
    async fn output_seen_blocks_stale_working_restart() {
        let runtime = Arc::new(RuntimeState::new());
        let deps = Arc::new(deps_with(runtime.clone()));
        stop_typing_on_output(&deps, "s1");

        start_typing_if_allowed(deps.clone(), "s1".into(), 42)
            .await
            .unwrap();

        assert!(!runtime.typing_tasks.contains_key("s1"));
        assert!(runtime.typing_output_seen.contains_key("s1"));
    }

    #[tokio::test]
    async fn draft_transport_skips_typing_start() {
        let deps = deps_with_transport(StreamTransport::Draft).await;
        start_typing_if_allowed(deps.clone(), "s1".into(), 42)
            .await
            .unwrap();
        assert!(!deps.runtime.typing_tasks.contains_key("s1"));
    }

    #[tokio::test]
    async fn edit_transport_starts_deferred_typing_task() {
        let deps = deps_with_transport(StreamTransport::Edit).await;
        start_typing_if_allowed(deps.clone(), "s1".into(), 42)
            .await
            .unwrap();
        assert!(deps.runtime.typing_tasks.contains_key("s1"));
    }

    #[tokio::test]
    async fn deferred_start_skips_when_output_seen_before_delay() {
        let deps = deps_with_transport(StreamTransport::Edit).await;
        start_typing_if_allowed(deps.clone(), "s1".into(), 42)
            .await
            .unwrap();
        assert!(deps.runtime.typing_tasks.contains_key("s1"));

        stop_typing_on_output(&deps, "s1");
        tokio::time::sleep(TYPING_START_DELAY + Duration::from_millis(50)).await;
        tokio::task::yield_now().await;

        assert!(!deps.runtime.typing_tasks.contains_key("s1"));
    }

    #[test]
    fn allow_typing_clears_suppress_and_output_seen() {
        let runtime = Arc::new(RuntimeState::new());
        let deps = deps_with(runtime.clone());
        suppress_typing(&deps, "s1");
        stop_typing_on_output(&deps, "s1");
        let gen = runtime.typing_generation("s1");
        allow_typing(&deps, "s1");

        assert!(!runtime.typing_suppressed.contains_key("s1"));
        assert!(!runtime.typing_output_seen.contains_key("s1"));
        assert_eq!(runtime.typing_generation("s1"), gen + 1);
    }
}
