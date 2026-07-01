//! Shared e2e harness: connect-or-skip engine handle, worker boot, and the
//! echo backend.

use std::sync::Arc;
use std::time::Duration;

use iii_http::trigger::RouteTable;
use tokio::sync::RwLock;

pub mod backend;
pub mod engine;
pub mod worker;

/// Give the engine a moment to deliver trigger-type / trigger registrations
/// (and the resulting route inserts) before issuing HTTP requests. Prefer
/// [`wait_for_route`] for route-dependent assertions; this remains for coarse
/// settling needs.
#[allow(dead_code)]
pub async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Poll the worker's route table until `method`+`path` is GONE (after an
/// unregister). Mirrors [`wait_for_route`] for the removal direction.
pub async fn wait_for_no_route(routes: &Arc<RwLock<RouteTable>>, method: &str, path: &str) {
    for _ in 0..50 {
        if routes.read().await.match_route(method, path).is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("route {method} {path} was never unregistered");
}

/// Poll the worker's route table until `method`+`path` is registered. Trigger
/// registration propagates asynchronously (engine -> handler -> table), so a
/// fixed sleep is racy; this waits on the actual observable state instead.
pub async fn wait_for_route(routes: &Arc<RwLock<RouteTable>>, method: &str, path: &str) {
    for _ in 0..50 {
        if routes.read().await.match_route(method, path).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("route {method} {path} was never registered");
}
