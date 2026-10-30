//! Minimal worker event function that an A2UI live binding can observe. Run
//! it next to an engine (no iii-stream needed):
//!
//! ```bash
//! cargo run --example counter_provider -- ws://127.0.0.1:49134
//! ```
//!
//! Then bind a surface to it with `a2ui::binding::set`:
//!
//! ```json
//! {
//!   "surface_id": "clicks",
//!   "binding": {
//!     "id": "clicks-count",
//!     "function_id": "demo::on-counter-changed",
//!     "args": { "counter": "clicks" },
//!     "target_path": "/count",
//!     "query": {
//!       "function_id": "demo::counter::get",
//!       "payload": { "counter": "clicks" },
//!       "result_path": "/value"
//!     }
//!   }
//! }
//! ```
//!
//! `demo::counter::bump { "counter": "clicks" }` commits a new value and then
//! calls `demo::on-counter-changed` with `{ counter, revision }`; the observers
//! whose args name that counter receive it, and the A2UI page re-reads through
//! the read-only `demo::counter::get` query.

mod provider;

use std::sync::Arc;

use iii_sdk::register_worker;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::InitOptions;

#[tokio::main]
async fn main() {
    let url = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("III_URL").ok())
        .unwrap_or_else(|| "ws://127.0.0.1:49134".into());
    let iii = Arc::new(register_worker(
        &url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                name: "a2ui-demo-provider".into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    provider::register(&iii);

    println!("a2ui-demo-provider ready on {url}");
    let _ = tokio::signal::ctrl_c().await;
    iii.shutdown_async().await;
}
