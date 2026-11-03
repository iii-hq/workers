#![allow(dead_code)]
//! Shared e2e harness: connect-or-skip engine handle, worker boot, and the
//! echo backend.

use std::sync::Arc;
use std::time::Duration;

use iii_http::routes::RouteTable;
use tokio::sync::RwLock;

pub mod backend;
pub mod engine;
pub mod fake_engine;
pub mod worker;

/// Give the engine a moment to deliver observers of `http::on-request` (and
/// the resulting route inserts) before issuing HTTP requests. Prefer
/// [`wait_for_route`] for route-dependent assertions; this remains for coarse
/// settling needs.
#[allow(dead_code)]
pub async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Poll the worker's route table until `method`+`path` is GONE (after its
/// observer is disposed). Mirrors [`wait_for_route`] for the removal direction.
pub async fn wait_for_no_route(routes: &Arc<RwLock<RouteTable>>, method: &str, path: &str) {
    for _ in 0..50 {
        if routes.read().await.match_route(method, path).is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("route {method} {path} was never unregistered");
}

/// Poll the worker's route table until `method`+`path` is registered. An
/// observer reaches the table asynchronously (engine -> added hook -> table),
/// so a fixed sleep is racy; this waits on the actual state instead.
pub async fn wait_for_route(routes: &Arc<RwLock<RouteTable>>, method: &str, path: &str) {
    for _ in 0..50 {
        if routes.read().await.match_route(method, path).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("route {method} {path} was never registered");
}
