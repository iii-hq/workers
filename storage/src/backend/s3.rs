//! AWS S3 backend.
//!
//! Used directly by `provider: s3`; also reused by R2 (`backend/r2.rs`) and
//! local rustfs (`backend/local.rs`) by varying endpoint URL + credentials.
//! All three providers speak the S3 protocol; this module is the one
//! implementation.

use super::*;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::SharedCredentialsProvider;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use chrono::TimeZone;
use std::time::Duration;

pub struct S3Backend {
    client: Client,
    /// Underlying cloud bucket name.
    bucket: String,
    /// Provider tag for error envelopes — `"s3"`, `"r2"`, or `"local"`.
    provider_tag: &'static str,
}

pub struct S3BackendOpts<'a> {
    pub region: &'a str,
    /// `None` → defer to the AWS credential chain. `Some` → static.
    pub credentials: Option<S3StaticCreds>,
    /// `None` for AWS S3; `Some(url)` for R2/local/custom endpoints.
    pub endpoint_url: Option<String>,
    /// `true` for path-style addressing (rustfs, MinIO). `false` for virtual-host style (AWS, R2).
    pub force_path_style: bool,
    /// What to call this backend in error envelopes.
    pub provider_tag: &'static str,
}

pub struct S3StaticCreds {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

impl S3Backend {
    pub async fn build(opts: S3BackendOpts<'_>, bucket: String) -> Result<Self, BackendError> {
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(opts.region.to_string()));
        if let Some(creds) = opts.credentials {
            let static_creds = Credentials::new(
                creds.access_key_id,
                creds.secret_access_key,
                creds.session_token,
                None,
                "storage-static",
            );
            loader = loader.credentials_provider(SharedCredentialsProvider::new(static_creds));
        }
        let sdk_config = loader.load().await;
        let mut s3_config = aws_sdk_s3::config::Builder::from(&sdk_config);
        if let Some(url) = opts.endpoint_url {
            s3_config = s3_config.endpoint_url(url);
        }
        if opts.force_path_style {
            s3_config = s3_config.force_path_style(true);
        }
        let client = Client::from_conf(s3_config.build());
        Ok(Self {
            client,
            bucket,
            provider_tag: opts.provider_tag,
        })
    }
}

#[async_trait]
impl Backend for S3Backend {
    async fn put(&self, req: PutReq) -> Result<PutResp, BackendError> {
        let body = ByteStream::from(req.body.clone());
        let size = req.body.len() as u64;
        let mut put = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(&req.key)
            .body(body)
            .content_type(req.content_type);
        if let Some(cc) = req.cache_control {
            put = put.cache_control(cc);
        }
        for (k, v) in req.metadata {
            put = put.metadata(k, v);
        }
        let resp = put.send().await.map_err(map_s3_error)?;
        Ok(PutResp {
            etag: resp.e_tag.unwrap_or_default(),
            size,
            version_id: resp.version_id,
        })
    }

    async fn get(&self, req: GetReq) -> Result<GetResp, BackendError> {
        let mut g = self.client.get_object().bucket(&self.bucket).key(&req.key);
        if let Some(v) = req.version_id {
            g = g.version_id(v);
        }
        let resp = g.send().await.map_err(map_s3_error)?;
        let last_modified = resp
            .last_modified
            .and_then(|dt| {
                chrono::Utc
                    .timestamp_opt(dt.secs(), dt.subsec_nanos())
                    .single()
            })
            .unwrap_or_else(Utc::now);
        let content_type = resp
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let etag = resp.e_tag.clone().unwrap_or_default();
        // Reject BEFORE consuming the body stream — content_length comes back in
        // headers, so this avoids buffering an oversized object into worker memory.
        // When max_inline_bytes is set we MUST know the size up front; some
        // S3-compatible servers omit Content-Length on chunked transfers, and
        // letting the cap be 0 there would silently bypass the limit.
        let size = match resp.content_length {
            Some(n) if n >= 0 => n as u64,
            Some(_) | None => {
                if let Some(cap) = req.max_inline_bytes {
                    return Err(BackendError::Provider {
                        inner_code: None,
                        message: format!(
                            "GET response missing Content-Length; cannot enforce {cap}-byte cap. Use presignUrl for objects of unknown size.",
                        ),
                    });
                }
                0
            }
        };
        if let Some(cap) = req.max_inline_bytes {
            if size > cap {
                return Err(BackendError::ObjectTooLarge {
                    actual_size: size,
                    cap,
                });
            }
        }
        let body = resp
            .body
            .collect()
            .await
            .map_err(|e| BackendError::Provider {
                inner_code: None,
                message: format!("read body: {e}"),
            })?
            .into_bytes()
            .to_vec();
        Ok(GetResp {
            body,
            content_type,
            etag,
            last_modified,
            size,
        })
    }

    async fn delete(&self, req: DeleteReq) -> Result<DeleteResp, BackendError> {
        let mut d = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(&req.key);
        if let Some(v) = req.version_id {
            d = d.version_id(v);
        }
        match d.send().await {
            Ok(_) => Ok(DeleteResp { deleted: true }),
            Err(e) => match map_s3_error(e) {
                BackendError::NotFound => Ok(DeleteResp { deleted: false }),
                other => Err(other),
            },
        }
    }

    async fn presign(&self, req: PresignReq) -> Result<PresignResp, BackendError> {
        let cfg = PresigningConfig::expires_in(Duration::from_secs(req.expires_in_seconds))
            .map_err(|e| BackendError::Provider {
                inner_code: None,
                message: format!("presign config: {e}"),
            })?;
        let presigned = match req.method {
            PresignMethod::Get => self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(&req.key)
                .presigned(cfg)
                .await
                .map_err(map_s3_error)?,
            PresignMethod::Put => {
                let mut put = self.client.put_object().bucket(&self.bucket).key(&req.key);
                if let Some(ct) = &req.content_type {
                    put = put.content_type(ct);
                }
                put.presigned(cfg).await.map_err(map_s3_error)?
            }
        };
        let url = presigned.uri().to_string();
        let expires_at = Utc::now() + chrono::Duration::seconds(req.expires_in_seconds as i64);
        Ok(PresignResp { url, expires_at })
    }

    fn provider(&self) -> &'static str {
        self.provider_tag
    }
}

/// Map an aws-sdk-s3 SdkError into a BackendError. NoSuchKey/NotFound responses
/// fold into BackendError::NotFound; auth issues into AuthFailed; everything
/// else surfaces with a stable code in `inner_code`.
fn map_s3_error<E>(
    err: aws_sdk_s3::error::SdkError<E, aws_smithy_runtime_api::http::Response>,
) -> BackendError
where
    E: std::error::Error + ProvideErrorMetadata + Send + Sync + 'static,
{
    use aws_sdk_s3::error::SdkError;
    match &err {
        SdkError::ServiceError(svc) => {
            let raw = svc.raw();
            let status = raw.status().as_u16();
            // Use the SDK's own ProvideErrorMetadata::code() rather than parsing
            // a Debug print — the latter would silently drift if the SDK ever
            // changes its derive(Debug) output.
            let inner_code = svc.err().code().map(|s| s.to_string());
            match status {
                404 => BackendError::NotFound,
                401 | 403 => BackendError::AuthFailed(format!("{}", err)),
                _ => BackendError::Provider {
                    inner_code,
                    message: format!("{err}"),
                },
            }
        }
        SdkError::TimeoutError(_) => BackendError::Provider {
            inner_code: Some("Timeout".into()),
            message: format!("{err}"),
        },
        _ => BackendError::Provider {
            inner_code: None,
            message: format!("{err}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_with_static_creds_does_not_panic() {
        // Construction-only smoke; no real network call.
        let r = S3Backend::build(
            S3BackendOpts {
                region: "us-east-1",
                credentials: Some(S3StaticCreds {
                    access_key_id: "AKIATEST".into(),
                    secret_access_key: "secret".into(),
                    session_token: None,
                }),
                endpoint_url: None,
                force_path_style: false,
                provider_tag: "s3",
            },
            "test-bucket".into(),
        )
        .await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn build_with_endpoint_url_does_not_panic() {
        // Used by R2 / local backends.
        let r = S3Backend::build(
            S3BackendOpts {
                region: "auto",
                credentials: Some(S3StaticCreds {
                    access_key_id: "k".into(),
                    secret_access_key: "s".into(),
                    session_token: None,
                }),
                endpoint_url: Some("http://127.0.0.1:9000".into()),
                force_path_style: true,
                provider_tag: "local",
            },
            "scratch".into(),
        )
        .await;
        assert!(r.is_ok());
    }
}
