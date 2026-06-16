//! The `router::abort` iii function (spec § router::abort).
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::IIIError;

use crate::types::router::{AbortRequest, AbortResponse};

use super::inflight::InflightMap;

pub fn make_abort(
    inflight: Arc<InflightMap>,
) -> impl Fn(AbortRequest) -> BoxFuture<'static, Result<AbortResponse, IIIError>> + Send + Sync + 'static
{
    move |req: AbortRequest| {
        let inflight = inflight.clone();
        Box::pin(async move {
            let aborted = inflight.abort(&req.request_id);
            Ok(AbortResponse { aborted })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::inflight::InflightMap;
    use std::sync::Arc;

    fn req(id: &str) -> AbortRequest {
        AbortRequest {
            request_id: id.to_string(),
        }
    }

    #[tokio::test]
    async fn aborts_known_requests_once_and_ignores_unknown() {
        let inflight = Arc::new(InflightMap::default());
        inflight.insert("r1");
        let abort = make_abort(inflight);
        // First abort of a known request succeeds, the second is a no-op
        // (idempotent), and an unknown id reports `aborted: false`.
        assert!(abort(req("r1")).await.unwrap().aborted);
        assert!(!abort(req("r1")).await.unwrap().aborted);
        assert!(!abort(req("unknown")).await.unwrap().aborted);
    }
}
