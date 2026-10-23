//! Build concrete `Backend`s from a `BucketConfig` and its resolved
//! credentials ([`Material`]).

use super::s3::{S3Backend, S3BackendOpts, S3StaticCreds};
use super::{Backend, BackendError};
use crate::config::{BucketConfig, ProvidersConfig};
use crate::material::Material;
use std::sync::Arc;

/// Context for one prepared native-local service generation. A live
/// configuration update creates a new generation before publishing it.
pub use super::local::LocalBackendCtx;

/// How a backend is built: for the worker's live buckets, or as a
/// throwaway for `storage::testBucket` (one attempt per call, no retries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Purpose {
    #[default]
    Live,
    Probe,
}

/// Every credential field comes from `material` (its references already
/// resolved), never from `cfg`.
pub async fn build(
    name: &str,
    cfg: &BucketConfig,
    _providers: &ProvidersConfig,
    local_ctx: Option<&LocalBackendCtx>,
    material: &Material,
    purpose: Purpose,
) -> Result<Arc<dyn Backend>, BackendError> {
    let fast_fail = purpose == Purpose::Probe;
    let value = |field: &str| material.value(field).map(str::to_string);
    let bucket = cfg.underlying(name);
    match cfg {
        BucketConfig::S3(s) => {
            let creds = match (
                value("access_key_id"),
                value("secret_access_key"),
                value("session_token"),
            ) {
                (Some(ak), Some(sk), st) => Some(S3StaticCreds {
                    access_key_id: ak,
                    secret_access_key: sk,
                    session_token: st,
                }),
                _ => None,
            };
            let b = S3Backend::build(
                S3BackendOpts {
                    region: &s.region,
                    credentials: creds,
                    endpoint_url: s.endpoint_url.clone(),
                    force_path_style: s.force_path_style.unwrap_or(false),
                    provider_tag: "s3",
                    fast_fail,
                },
                bucket,
            )
            .await?;
            Ok(Arc::new(b))
        }
        BucketConfig::Gcs(g) => {
            let b = super::gcs::GcsBackend::build(
                super::gcs::GcsBackendOpts {
                    credentials_json: value("credentials_json"),
                    credentials_file: g.credentials_file.clone(),
                    endpoint_url: g.endpoint_url.clone(),
                },
                bucket,
            )
            .await?;
            Ok(Arc::new(b))
        }
        BucketConfig::R2(r) => {
            let missing = |field: &str| BackendError::Provider {
                inner_code: Some("CONFIG".into()),
                message: format!("`{field}` is required for r2 buckets"),
            };
            super::r2::build(
                super::r2::R2BackendOpts {
                    account_id: value("account_id").ok_or_else(|| missing("account_id"))?,
                    access_key_id: value("access_key_id")
                        .ok_or_else(|| missing("access_key_id"))?,
                    secret_access_key: value("secret_access_key")
                        .ok_or_else(|| missing("secret_access_key"))?,
                    endpoint_url: r.endpoint_url.clone(),
                    fast_fail,
                },
                bucket,
            )
            .await
        }
        BucketConfig::Local(_) => {
            let ctx = local_ctx.ok_or_else(|| BackendError::Provider {
                inner_code: Some("LOCAL_NO_CTX".into()),
                message: "local backend requested but no native local context".into(),
            })?;
            super::local::build(ctx, name.to_string(), bucket)
        }
    }
}
