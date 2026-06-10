//! The `router::abort` iii function (spec § router::abort).
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::IIIError;
use serde_json::{json, Value};

use super::inflight::InflightMap;

pub fn make_abort(
    inflight: Arc<InflightMap>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let inflight = inflight.clone();
        Box::pin(async move {
            let aborted = raw
                .get("request_id")
                .and_then(Value::as_str)
                .map(|id| inflight.abort(id))
                .unwrap_or(false);
            Ok(json!({ "aborted": aborted }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::inflight::InflightMap;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn aborts_known_requests_once_and_tolerates_garbage() {
        let inflight = Arc::new(InflightMap::default());
        inflight.insert("r1");
        let abort = make_abort(inflight);
        assert_eq!(
            abort(json!({ "request_id": "r1" })).await.unwrap(),
            json!({ "aborted": true })
        );
        assert_eq!(
            abort(json!({ "request_id": "r1" })).await.unwrap(),
            json!({ "aborted": false })
        );
        assert_eq!(abort(json!({})).await.unwrap(), json!({ "aborted": false }));
        assert_eq!(
            abort(json!(null)).await.unwrap(),
            json!({ "aborted": false })
        );
    }
}
