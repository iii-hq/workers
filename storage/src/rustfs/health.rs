//! Readiness probe for the rustfs sidecar.

use crate::error::StorageError;
use std::time::Duration;
use tokio::time::{sleep, Instant};

/// Poll the rustfs S3 endpoint until it returns any HTTP response (any status
/// < 500). rustfs responds to a bare GET on the S3 endpoint with either an XML
/// ListAllMyBuckets-style body when the request signs correctly, or a 403
/// when it doesn't — either is proof the process is up.
pub async fn wait_for_healthy(port: u16, max_wait: Duration) -> Result<(), StorageError> {
    let deadline = Instant::now() + max_wait;
    let url = format!("http://127.0.0.1:{port}/");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| StorageError::LocalBackendBootFailed {
            reason: format!("reqwest build: {e}"),
        })?;
    while Instant::now() < deadline {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().as_u16() < 500 {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(StorageError::LocalBackendBootFailed {
        reason: format!("rustfs did not become healthy within {max_wait:?}"),
    })
}
