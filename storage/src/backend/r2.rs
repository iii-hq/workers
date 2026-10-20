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
    /// One attempt per call (`storage::testBucket`).
    pub fast_fail: bool,
}

pub async fn build(opts: R2BackendOpts, bucket: String) -> Result<Arc<dyn Backend>, BackendError> {
    let endpoint = match opts.endpoint_url.clone() {
        Some(endpoint) => endpoint,
        // The account id becomes a hostname: anything but an account id
        // (a resolved reference pointing at another secret) would leak
        // through DNS and SNI.
        None if crate::config::valid_cf_account_id(&opts.account_id) => {
            format!("https://{}.r2.cloudflarestorage.com", opts.account_id)
        }
        None => {
            return Err(BackendError::Provider {
                inner_code: Some("CONFIG".into()),
                message: "`account_id` is not a Cloudflare account id (32 hex characters)".into(),
            })
        }
    };
    if opts.endpoint_url.is_some() && !opts.fast_fail {
        tracing::warn!(
            target = "storage::r2",
            endpoint = %crate::config::redact_url(&endpoint),
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
            fast_fail: opts.fast_fail,
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
    async fn an_account_id_that_is_not_one_never_becomes_a_hostname() {
        let r = build(
            R2BackendOpts {
                account_id: "SECRET-VALUE.attacker.example".into(),
                access_key_id: "k".into(),
                secret_access_key: "s".into(),
                endpoint_url: None,
                fast_fail: true,
            },
            "bucket".into(),
        )
        .await;
        match r {
            Err(BackendError::Provider { message, .. }) => {
                assert!(
                    message.contains("32 hex") && !message.contains("SECRET"),
                    "{message}"
                )
            }
            _ => panic!("expected a refusal"),
        }
    }

    #[tokio::test]
    async fn r2_build_smoke() {
        let r = build(
            R2BackendOpts {
                account_id: "0123456789abcdef0123456789ABCDEF".into(),
                access_key_id: "k".into(),
                secret_access_key: "s".into(),
                endpoint_url: None,
                fast_fail: false,
            },
            "bucket".into(),
        )
        .await;
        assert!(r.is_ok());
    }
}
