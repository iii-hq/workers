//! Build concrete `Backend`s from `BucketConfig`. GCS/R2/local arms return
//! errors until their phases land — only S3 is fully wired in Phase 6.

use super::s3::{S3Backend, S3BackendOpts, S3StaticCreds};
use super::{Backend, BackendError};
use crate::config::{BucketConfig, ProvidersConfig};
use std::sync::Arc;

/// Context for one prepared native-local service generation. A live
/// configuration update creates a new generation before publishing it.
pub use super::local::LocalBackendCtx;

pub async fn build(
    name: &str,
    cfg: &BucketConfig,
    _providers: &ProvidersConfig,
    local_ctx: Option<&LocalBackendCtx>,
) -> Result<Arc<dyn Backend>, BackendError> {
    match cfg {
        BucketConfig::S3(s) => {
            let creds = match (
                s.access_key_id.clone(),
                s.secret_access_key.clone(),
                s.session_token.clone(),
            ) {
                (Some(ak), Some(sk), st) => Some(S3StaticCreds {
                    access_key_id: ak,
                    secret_access_key: sk,
                    session_token: st,
                }),
                _ => None,
            };
            let bucket = s.bucket.clone().unwrap_or_else(|| name.to_string());
            let b = S3Backend::build(
                S3BackendOpts {
                    region: &s.region,
                    credentials: creds,
                    endpoint_url: s.endpoint_url.clone(),
                    force_path_style: s.force_path_style.unwrap_or(false),
                    provider_tag: "s3",
                },
                bucket,
            )
            .await?;
            Ok(Arc::new(b))
        }
        BucketConfig::Gcs(g) => {
            let bucket = g.bucket.clone().unwrap_or_else(|| name.to_string());
            let b = super::gcs::GcsBackend::build(
                super::gcs::GcsBackendOpts {
                    credentials_file: g.credentials_file.clone(),
                    endpoint_url: g.endpoint_url.clone(),
                },
                bucket,
            )
            .await?;
            Ok(Arc::new(b))
        }
        BucketConfig::R2(r) => {
            let bucket = r.bucket.clone().unwrap_or_else(|| name.to_string());
            super::r2::build(
                super::r2::R2BackendOpts {
                    account_id: r.account_id.clone(),
                    access_key_id: r.access_key_id.clone(),
                    secret_access_key: r.secret_access_key.clone(),
                    endpoint_url: r.endpoint_url.clone(),
                },
                bucket,
            )
            .await
        }
        BucketConfig::Local(l) => {
            let ctx = local_ctx.ok_or_else(|| BackendError::Provider {
                inner_code: Some("LOCAL_NO_CTX".into()),
                message: "local backend requested but no native local context".into(),
            })?;
            let bucket = l.bucket.clone().unwrap_or_else(|| name.to_string());
            super::local::build(ctx, name.to_string(), bucket)
        }
    }
}
