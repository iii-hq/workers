#![allow(dead_code)]
//! Connect-or-skip wrapper around the iii SDK.
//!
//! These are e2e tests: they only make sense with a running engine. By
//! default, when the engine is unreachable, `get_or_init`/`connect_fresh`
//! return `None` and the caller skips the test. Set `III_E2E_REQUIRE` to any
//! value to fail loudly instead.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::json;
use tokio::sync::OnceCell;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

static ENGINE: OnceCell<Option<Arc<IIIClient>>> = OnceCell::const_new();

pub fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

fn require_or_skip(connected: Option<Arc<IIIClient>>) -> Option<Arc<IIIClient>> {
    if connected.is_some() {
        return connected;
    }

    if std::env::var("III_E2E_REQUIRE").is_ok() {
        panic!(
            "e2e requires a running iii engine at {} - start `iii` or set \
             III_ENGINE_WS_URL to point at one",
            ws_url()
        );
    }

    eprintln!(
        "[skip] no iii engine reachable at {} - skipping e2e test (set III_E2E_REQUIRE=1 to \
         fail instead)",
        ws_url()
    );
    None
}

async fn try_connect_raw(namespace: Option<&str>, name: Option<&str>) -> Option<Arc<IIIClient>> {
    let url = ws_url();
    let iii = Arc::new(register_worker(
        &url,
        InitOptions {
            namespace: namespace.map(str::to_owned),
            // 0.25 engines refuse a second live worker with the same name in
            // a namespace; the default name is per process.
            metadata: name.map(|name| WorkerMetadata {
                name: name.to_string(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let probe = iii
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        match probe {
            Ok(_) => return Some(iii),
            Err(Error::NotConnected) => continue,
            Err(_) => continue,
        }
    }

    iii.shutdown_async().await;
    None
}

pub async fn connect_fresh() -> Option<Arc<IIIClient>> {
    require_or_skip(try_connect_raw(None, None).await)
}

/// A fresh connection under its own worker name, for tests that connect
/// several clients from one process.
pub async fn connect_fresh_named(name: &str) -> Option<Arc<IIIClient>> {
    require_or_skip(try_connect_raw(None, Some(name)).await)
}

pub async fn connect_fresh_in_namespace(namespace: &str) -> Option<Arc<IIIClient>> {
    require_or_skip(try_connect_raw(Some(namespace), None).await)
}

pub async fn get_or_init() -> Option<Arc<IIIClient>> {
    ENGINE
        .get_or_init(|| async { require_or_skip(try_connect_raw(None, None).await) })
        .await
        .clone()
}
