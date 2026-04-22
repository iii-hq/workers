use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::{json, Value};

use crate::config::ShellConfig;
use crate::jobs;

pub fn build_handler(
    config: Arc<ShellConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let cfg = config.clone();
        Box::pin(async move {
            jobs::remove_old(cfg.job_retention_secs).await;
            let all = jobs::list_all().await;
            Ok(json!({
                "jobs": all,
                "count": all.len(),
            }))
        })
    }
}
