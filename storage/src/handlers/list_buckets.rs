//! `storage::listBuckets` — enumerate worker-facing buckets for explorers and
//! bucket editors: every configured bucket, including the ones whose backend
//! could not be built (with their error).

use super::AppState;
use crate::backend::Backend;
use crate::config::{redact_url, BucketConfig, BucketDisplay, InvalidBucket};
use crate::material::Stage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListBucketsReq {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BucketSummary {
    /// The worker-facing name (the key under `buckets`).
    pub name: String,
    /// `s3` | `r2` | `gcs` | `local` (`unknown` for an entry that names none).
    pub provider: String,
    /// The provider-side bucket.
    pub bucket: String,
    /// s3 only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// The custom endpoint (S3-compatible stores, R2/GCS overrides), userinfo
    /// stripped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    /// r2 only: the literal account id, or the reference as stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// How clients draw the bucket (well-formed parts only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<BucketDisplay>,
    /// Some credential field is a `secret://` / `env://` reference.
    pub uses_secrets: bool,
    /// A presigned GET (`storage::presignUrl`) gives a URL a client can
    /// fetch: false for gcs without signing credentials or with an
    /// `endpoint_url` override (read through ranged `storage::getObject`).
    pub direct_downloads: bool,
    /// Why the bucket has no backend (its calls answer `BUCKET_UNAVAILABLE`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `config` | `secrets` | `auth`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_stage: Option<Stage>,
    /// Why the bucket's notifications source is not running (the bucket
    /// works; its object triggers do not fire).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notifications_error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListBucketsResp {
    pub buckets: Vec<BucketSummary>,
    pub count: usize,
    /// The configuration entry the buckets live in.
    pub config_id: String,
}

pub async fn handle(state: &AppState, _req: ListBucketsReq) -> Result<ListBucketsResp, String> {
    let (catalog, backends) = {
        let _gate = state.reconfigure_gate.read().await;
        (
            state.catalog.read().await.clone(),
            state.backends.read().await.clone(),
        )
    };
    let mut buckets: Vec<BucketSummary> = Vec::new();
    for (name, bucket) in &catalog.config.buckets {
        let mut summary = summarize(name, bucket, backends.get(name));
        if let Some(error) = catalog.errors.get(name) {
            summary.error = Some(error.message.clone());
            summary.error_stage = Some(error.stage);
        }
        summary.notifications_error = catalog.notification_errors.get(name).cloned();
        buckets.push(summary);
    }
    for (name, invalid) in &catalog.config.invalid {
        buckets.push(summarize_invalid(name, invalid));
    }
    // Backends installed without a configuration snapshot (tests, tools).
    for (name, backend) in &backends {
        if !catalog.config.buckets.contains_key(name) {
            buckets.push(BucketSummary {
                name: name.clone(),
                provider: backend.provider().to_string(),
                bucket: name.clone(),
                region: None,
                endpoint_url: None,
                account_id: None,
                display: None,
                uses_secrets: false,
                direct_downloads: backend.direct_downloads(),
                error: None,
                error_stage: None,
                notifications_error: None,
            });
        }
    }
    buckets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ListBucketsResp {
        count: buckets.len(),
        buckets,
        config_id: crate::configuration::config_id().to_string(),
    })
}

fn summarize(
    name: &str,
    bucket: &BucketConfig,
    backend: Option<&Arc<dyn Backend>>,
) -> BucketSummary {
    let (region, account_id) = match bucket {
        BucketConfig::S3(v) => (Some(v.region.clone()), None),
        BucketConfig::R2(v) => (None, Some(v.account_id.clone())),
        _ => (None, None),
    };
    BucketSummary {
        name: name.to_string(),
        provider: bucket.provider().to_string(),
        bucket: bucket.underlying(name),
        region: region.filter(|r| !r.is_empty()),
        endpoint_url: bucket.endpoint_url().map(display_url),
        account_id,
        display: bucket.display().and_then(BucketDisplay::sanitized),
        uses_secrets: bucket.uses_secrets(),
        direct_downloads: backend
            .map(|b| b.direct_downloads())
            .unwrap_or_else(|| static_direct_downloads(bucket)),
        error: None,
        error_stage: None,
        notifications_error: None,
    }
}

/// An endpoint as configured, minus any userinfo.
fn display_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(url) if !url.username().is_empty() || url.password().is_some() => redact_url(raw),
        _ => raw.to_string(),
    }
}

/// Without a live backend: what the entry promises.
fn static_direct_downloads(bucket: &BucketConfig) -> bool {
    match bucket {
        BucketConfig::Gcs(g) => {
            g.endpoint_url.is_none()
                && (g.credentials_json.is_some() || g.credentials_file.is_some())
        }
        _ => true,
    }
}

/// An entry that did not parse: whatever its raw value says.
fn summarize_invalid(name: &str, invalid: &InvalidBucket) -> BucketSummary {
    let text = |key: &str| {
        invalid
            .raw
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let provider = text("provider").unwrap_or_else(|| "unknown".into());
    let display = invalid
        .raw
        .get("display")
        .cloned()
        .and_then(|v| serde_json::from_value::<BucketDisplay>(v).ok())
        .and_then(|d| d.sanitized());
    let uses_secrets = [
        "access_key_id",
        "secret_access_key",
        "session_token",
        "account_id",
        "credentials_json",
    ]
    .iter()
    .filter_map(|k| text(k))
    .chain(
        invalid
            .raw
            .get("notifications")
            .and_then(|n| n.get("api_token"))
            .and_then(Value::as_str)
            .map(str::to_string),
    )
    .any(|v| crate::secrets::parse_ref(&v).is_some());
    BucketSummary {
        name: name.to_string(),
        bucket: text("bucket").unwrap_or_else(|| name.to_string()),
        region: (provider == "s3").then(|| text("region")).flatten(),
        endpoint_url: text("endpoint_url").map(|u| display_url(&u)),
        account_id: (provider == "r2").then(|| text("account_id")).flatten(),
        provider,
        display,
        uses_secrets,
        direct_downloads: false,
        error: Some(invalid.message.clone()),
        error_stage: Some(Stage::Config),
        notifications_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::config::WorkerConfig;
    use crate::handlers::Catalog;
    use crate::material::BucketError;
    use serde_json::json;
    use std::collections::HashMap;

    #[tokio::test]
    async fn lists_live_failed_and_invalid_buckets_with_their_details() {
        let cfg = WorkerConfig::from_json_isolating(&json!({
            "buckets": {
                "photos": {
                    "provider": "s3", "region": "us-east-1", "bucket": "my-photos",
                    "endpoint_url": "http://user:pw@minio.test:9000",
                    "access_key_id": "secret://AK", "secret_access_key": "secret://SK",
                    "display": { "icon": "image", "color": "#3B82F6" }
                },
                "docs": { "provider": "r2", "account_id": "acc", "access_key_id": "a", "secret_access_key": "secret://GONE" },
                "broken": { "provider": "s3", "bucket": "b", "display": { "color": "red" } },
                "gcs": { "provider": "gcs", "endpoint_url": "http://fake:4443" }
            }
        }))
        .unwrap();
        let mut backends: HashMap<String, Arc<dyn Backend>> = HashMap::new();
        backends.insert("photos".into(), Arc::new(MockBackend::default()));
        let st = AppState::new(backends);
        let mut errors = HashMap::new();
        errors.insert(
            "docs".to_string(),
            BucketError::new(
                Stage::Secrets,
                "`secret_access_key`: secret://GONE did not resolve",
            ),
        );
        errors.insert(
            "broken".to_string(),
            BucketError::new(Stage::Config, cfg.invalid["broken"].message.clone()),
        );
        *st.catalog.write().await = Catalog {
            config: cfg,
            errors,
            notification_errors: HashMap::new(),
        };
        let resp = handle(&st, ListBucketsReq {}).await.unwrap();
        assert_eq!(resp.count, 4);
        let by_name: HashMap<_, _> = resp.buckets.iter().map(|b| (b.name.as_str(), b)).collect();
        let photos = by_name["photos"];
        assert_eq!(photos.provider, "s3");
        assert_eq!(photos.bucket, "my-photos");
        assert_eq!(photos.region.as_deref(), Some("us-east-1"));
        assert_eq!(
            photos.endpoint_url.as_deref(),
            Some("http://minio.test:9000/")
        );
        assert!(photos.uses_secrets);
        assert!(photos.direct_downloads);
        assert_eq!(
            photos.display.as_ref().unwrap().icon.as_deref(),
            Some("image")
        );
        assert!(photos.error.is_none());
        let docs = by_name["docs"];
        assert_eq!(docs.error_stage, Some(Stage::Secrets));
        assert_eq!(docs.account_id.as_deref(), Some("acc"));
        assert_eq!(
            docs.bucket, "docs",
            "the underlying bucket defaults to the name"
        );
        let broken = by_name["broken"];
        assert_eq!(broken.provider, "s3");
        assert_eq!(broken.error_stage, Some(Stage::Config));
        assert!(
            broken.error.as_deref().unwrap().contains("region"),
            "{:?}",
            broken.error
        );
        assert!(broken.display.is_none(), "a malformed color is dropped");
        let gcs = by_name["gcs"];
        assert!(!gcs.direct_downloads, "an endpoint override cannot presign");
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["config_id"].is_string());
        assert!(
            json["buckets"][0].get("error").is_none() || json["buckets"][0]["error"].is_string()
        );
    }

    #[tokio::test]
    async fn a_state_without_a_snapshot_still_lists_its_backends() {
        let mut backends: HashMap<String, Arc<dyn Backend>> = HashMap::new();
        backends.insert("uploads".into(), Arc::new(MockBackend::default()));
        let resp = handle(&AppState::new(backends), ListBucketsReq {})
            .await
            .unwrap();
        assert_eq!(resp.count, 1);
        assert_eq!(resp.buckets[0].provider, "mock");
    }
}
