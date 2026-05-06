//! Local → rustfs notify webhook receiver. Binds on 127.0.0.1:0 and
//! receives POSTed events from rustfs's notify subsystem.

use crate::triggers::normalize::{
    normalize_rustfs, EventKind, NormalizeError, ObjectEventNormalized,
};
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;

#[derive(Clone)]
pub struct WebhookState {
    pub bucket_reverse: Arc<HashMap<String, String>>,
    pub dispatch_timeout: Duration,
    /// Shared dispatcher — same trait the SQS/CF/Pub-Sub pollers use.
    pub dispatch: Arc<dyn crate::triggers::dispatcher::EventDispatcher>,
}

pub struct WebhookHandle {
    pub addr: SocketAddr,
    pub shutdown_tx: oneshot::Sender<()>,
}

/// Spawn the loopback HTTP receiver. Returns its bound `SocketAddr` and a
/// shutdown handle. Caller passes the resulting URL to rustfs's notify
/// webhook config at spawn time.
pub async fn spawn_receiver(state: WebhookState) -> std::io::Result<WebhookHandle> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let app = Router::new()
        .route("/notify", post(notify_handler))
        .with_state(state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok(WebhookHandle { addr, shutdown_tx })
}


/// Same as `spawn_receiver` but binds on a pre-allocated port. Used at worker
/// startup so we can pass the URL to the rustfs sidecar via env vars at spawn
/// time, before the receiver itself is bound.
pub async fn spawn_receiver_on(state: WebhookState, port: u16) -> std::io::Result<WebhookHandle> {
    let listener = TcpListener::bind(SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port))).await?;
    let addr = listener.local_addr()?;
    let app = Router::new()
        .route("/notify", post(notify_handler))
        .with_state(state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok(WebhookHandle { addr, shutdown_tx })
}

async fn notify_handler(
    State(state): State<WebhookState>,
    Json(event): Json<serde_json::Value>,
) -> StatusCode {
    let bucket_reverse = state.bucket_reverse.clone();
    let n = match normalize_rustfs(&event, &move |u| bucket_reverse.get(u).cloned()) {
        Ok(n) => n,
        Err(NormalizeError::Ignored(_)) | Err(NormalizeError::UnmappedBucket(_)) => {
            return StatusCode::OK; // drop silently
        }
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let dispatcher = state.dispatch.clone();
    match timeout(state.dispatch_timeout, async move {
        dispatcher.dispatch(n).await
    })
    .await
    {
        Ok(true) => StatusCode::OK,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub fn matches_direction(event: &ObjectEventNormalized, direction: EventKind) -> bool {
    event.event_kind == direction
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysAckDispatcher;
    #[async_trait::async_trait]
    impl crate::triggers::dispatcher::EventDispatcher for AlwaysAckDispatcher {
        async fn dispatch(&self, _: ObjectEventNormalized) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn webhook_receiver_starts_and_handles_event() {
        let mut br = HashMap::new();
        br.insert("scratch".to_string(), "scratch".to_string());
        let state = WebhookState {
            bucket_reverse: Arc::new(br),
            dispatch_timeout: Duration::from_secs(1),
            dispatch: Arc::new(AlwaysAckDispatcher),
        };
        let handle = spawn_receiver(state).await.unwrap();
        let path = format!(
            "{}/tests/fixtures/events/rustfs_created.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(path).unwrap();
        let url = format!("http://{}/notify", handle.addr);
        let resp = reqwest::Client::new()
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let _ = handle.shutdown_tx.send(());
    }
}
