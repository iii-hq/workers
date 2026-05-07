//! Cloudflare R2 backend — S3 protocol against R2's endpoint.

use super::s3::{S3Backend, S3BackendOpts, S3StaticCreds};
use super::{Backend, BackendError};
use std::sync::Arc;

pub struct R2BackendOpts {
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// When set, overrides the account-id-derived CF endpoint.
    pub endpoint_url: Option<String>,
}

pub async fn build(opts: R2BackendOpts, bucket: String) -> Result<Arc<dyn Backend>, BackendError> {
    let endpoint = opts
        .endpoint_url
        .clone()
        .unwrap_or_else(|| format!("https://{}.r2.cloudflarestorage.com", opts.account_id));
    if opts.endpoint_url.is_some() {
        tracing::warn!(
            target = "storage::r2",
            endpoint = %endpoint,
            "R2 backend is using a custom endpoint_url override; ensure this is intentional",
        );
    }
    let b = S3Backend::build(
        S3BackendOpts {
            region: "auto",
            credentials: Some(S3StaticCreds {
                access_key_id: opts.access_key_id,
                secret_access_key: opts.secret_access_key,
                session_token: None,
            }),
            endpoint_url: Some(endpoint),
            force_path_style: false,
            provider_tag: "r2",
        },
        bucket,
    )
    .await?;
    Ok(Arc::new(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn r2_build_smoke() {
        let r = build(
            R2BackendOpts {
                account_id: "abcd1234".into(),
                access_key_id: "k".into(),
                secret_access_key: "s".into(),
                endpoint_url: None,
            },
            "bucket".into(),
        )
        .await;
        assert!(r.is_ok());
    }
}
