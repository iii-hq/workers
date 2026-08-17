//! `storage::listBuckets` — enumerate worker-facing buckets for explorers.

use super::AppState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListBucketsReq {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BucketSummary {
    pub name: String,
    pub provider: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListBucketsResp {
    pub buckets: Vec<BucketSummary>,
}

pub async fn handle(state: &AppState, _req: ListBucketsReq) -> Result<ListBucketsResp, String> {
    let backends = state.backends_snapshot().await;
    let mut buckets: Vec<_> = backends
        .iter()
        .map(|(name, backend)| BucketSummary {
            name: name.clone(),
            provider: backend.provider().to_string(),
        })
        .collect();
    buckets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ListBucketsResp { buckets })
}
