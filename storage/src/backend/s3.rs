//! AWS S3 backend.
//!
//! Used directly by `provider: s3` and reused by R2 (`backend/r2.rs`) by
//! varying endpoint URL and credentials.

use super::*;
use crate::material::Stage;
use aws_config::retry::RetryConfig;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::SharedCredentialsProvider;
use aws_sdk_s3::error::{DisplayErrorContext, SdkError};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{BucketLocationConstraint, CreateBucketConfiguration};
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
    /// The `LocationConstraint` a CreateBucket sends: the region, except
    /// AWS's default `us-east-1` (which must be sent without one) and R2's
    /// `auto`.
    location_constraint: Option<String>,
}

pub struct S3BackendOpts<'a> {
    pub region: &'a str,
    /// `None` → defer to the AWS credential chain. `Some` → static.
    pub credentials: Option<S3StaticCreds>,
    /// `None` for AWS S3; `Some(url)` for R2/local/custom endpoints.
    pub endpoint_url: Option<String>,
    /// `true` for path-style addressing (MinIO, Ceph). `false` for virtual-host style (AWS, R2).
    pub force_path_style: bool,
    /// What to call this backend in error envelopes.
    pub provider_tag: &'static str,
    /// One attempt per call, no retries: for `storage::testBucket`, which
    /// must answer within its budget with the first failure.
    pub fast_fail: bool,
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
        if opts.fast_fail {
            loader = loader.retry_config(RetryConfig::standard().with_max_attempts(1));
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
        let region = opts.region.trim();
        let location_constraint = (opts.provider_tag == "s3"
            && !region.is_empty()
            && !region.eq_ignore_ascii_case("us-east-1")
            && !region.eq_ignore_ascii_case("auto"))
        .then(|| region.to_string());
        Ok(Self {
            client,
            bucket,
            provider_tag: opts.provider_tag,
            location_constraint,
        })
    }

    /// A ranged GET. The total size comes from `Content-Range`; a range
    /// that starts at (or past) the end answers 416, and a HEAD then tells
    /// an empty read at the end from an offset past it.
    async fn get_range(
        &self,
        key: String,
        version_id: Option<String>,
        range: ByteRange,
        cap: Option<u64>,
    ) -> Result<GetResp, BackendError> {
        if range.length == Some(0) {
            return self.empty_read(key, version_id, range).await;
        }
        let header = match range.length {
            Some(length) => format!(
                "bytes={}-{}",
                range.offset,
                range.offset.saturating_add(length - 1)
            ),
            None => format!("bytes={}-", range.offset),
        };
        let mut g = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .range(header);
        if let Some(v) = version_id.clone() {
            g = g.version_id(v);
        }
        let resp = match g.send().await {
            Ok(resp) => resp,
            Err(error) if status_of(&error) == Some(416) => {
                return self.empty_read(key, version_id, range).await;
            }
            Err(error) => return Err(map_s3_error(error)),
        };
        let content_length = resp.content_length.filter(|n| *n >= 0).map(|n| n as u64);
        // A server that ignored the Range header answers 200 with the whole
        // object (no Content-Range): its size is the content length.
        let partial = resp.content_range.as_deref().and_then(parse_content_range);
        let (served_start, size) = match partial {
            Some((start, total)) => (start, total),
            None => (
                0,
                content_length.ok_or_else(|| BackendError::Provider {
                    inner_code: None,
                    message: "GET response missing Content-Length and Content-Range".into(),
                })?,
            ),
        };
        let (start, end) = range.span(size)?;
        ByteRange::check_cap(start, end, cap)?;
        if partial.is_none() {
            // The whole object comes back; refuse to buffer more than the cap.
            ByteRange::check_cap(0, size, cap)?;
        }
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
        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| BackendError::Provider {
                inner_code: None,
                message: format!("read body: {e}"),
            })?
            .into_bytes();
        let from = (start - served_start.min(start)) as usize;
        let to = (from + (end - start) as usize).min(bytes.len());
        Ok(GetResp {
            body: bytes[from.min(to)..to].to_vec(),
            content_type,
            etag,
            last_modified,
            size,
            offset: start,
        })
    }

    /// No bytes to read: the metadata, after checking the offset.
    async fn empty_read(
        &self,
        key: String,
        version_id: Option<String>,
        range: ByteRange,
    ) -> Result<GetResp, BackendError> {
        let head = self.head(HeadReq { key, version_id }).await?;
        let (start, _) = range.span(head.size)?;
        Ok(GetResp {
            body: Vec::new(),
            content_type: head.content_type,
            etag: head.etag,
            last_modified: head.last_modified,
            size: head.size,
            offset: start,
        })
    }
}

/// `bytes <start>-<end>/<total>` → `(start, total)`.
fn parse_content_range(value: &str) -> Option<(u64, u64)> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (span, total) = rest.split_once('/')?;
    let (start, _) = span.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()?))
}

fn status_of<E>(error: &SdkError<E, aws_smithy_runtime_api::http::Response>) -> Option<u16> {
    match error {
        SdkError::ServiceError(svc) => Some(svc.raw().status().as_u16()),
        _ => None,
    }
}

/// A short, value-free description of an SDK failure: the service's code
/// and message, or the transport error's cause chain.
fn describe_sdk_error<E>(error: &SdkError<E, aws_smithy_runtime_api::http::Response>) -> String
where
    E: std::error::Error + ProvideErrorMetadata + Send + Sync + 'static,
{
    match error {
        SdkError::ServiceError(svc) => {
            let status = svc.raw().status().as_u16();
            let code = svc.err().code().unwrap_or("");
            let message = svc.err().message().unwrap_or("");
            match (code.is_empty(), message.is_empty()) {
                (false, false) => format!("HTTP {status} {code}: {message}"),
                (false, true) => format!("HTTP {status} {code}"),
                _ => format!("HTTP {status}"),
            }
        }
        other => DisplayErrorContext(other).to_string(),
    }
}

/// Classify a failed probe request for `storage::testBucket`.
fn probe_failure<E>(error: SdkError<E, aws_smithy_runtime_api::http::Response>) -> ProbeFailure
where
    E: std::error::Error + ProvideErrorMetadata + Send + Sync + 'static,
{
    let message = describe_sdk_error(&error);
    match &error {
        SdkError::DispatchFailure(_) | SdkError::TimeoutError(_) | SdkError::ResponseError(_) => {
            ProbeFailure::new(Stage::Endpoint, message)
        }
        SdkError::ConstructionFailure(_) => ProbeFailure::new(Stage::Config, message),
        SdkError::ServiceError(svc) => {
            let status = svc.raw().status().as_u16();
            match svc.err().code().unwrap_or("") {
                "NoSuchBucket" => ProbeFailure::missing(message),
                "PermanentRedirect"
                | "AuthorizationHeaderMalformed"
                | "IllegalLocationConstraintException" => ProbeFailure::new(
                    Stage::Bucket,
                    format!(
                        "{message} (the bucket lives in another region or needs another endpoint)"
                    ),
                ),
                "InvalidAccessKeyId"
                | "SignatureDoesNotMatch"
                | "AccessDenied"
                | "ExpiredToken"
                | "InvalidToken"
                | "TokenRefreshRequired"
                | "InvalidSecurity"
                | "Unauthorized" => ProbeFailure::new(Stage::Auth, message),
                _ if status == 404 => ProbeFailure::missing(message),
                _ if status == 401 || status == 403 => ProbeFailure::new(Stage::Auth, message),
                _ if status == 301 => ProbeFailure::new(
                    Stage::Bucket,
                    format!(
                        "{message} (the bucket lives in another region or needs another endpoint)"
                    ),
                ),
                _ => ProbeFailure::new(Stage::List, message),
            }
        }
        _ => ProbeFailure::new(Stage::List, message),
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
        if let Some(range) = req.range {
            return self
                .get_range(req.key, req.version_id, range, req.max_inline_bytes)
                .await;
        }
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
            offset: 0,
        })
    }

    async fn head(&self, req: HeadReq) -> Result<HeadResp, BackendError> {
        let mut h = self.client.head_object().bucket(&self.bucket).key(&req.key);
        if let Some(v) = req.version_id {
            h = h.version_id(v);
        }
        let resp = h.send().await.map_err(map_s3_error)?;
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
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let etag = resp.e_tag.unwrap_or_default();
        let size = match resp.content_length {
            Some(n) if n >= 0 => n as u64,
            _ => 0,
        };
        Ok(HeadResp {
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

    async fn list(&self, req: ListReq) -> Result<ListResp, BackendError> {
        let mut list = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(req.prefix)
            .max_keys(req.limit.min(1_000) as i32);
        if let Some(delimiter) = req.delimiter {
            list = list.delimiter(delimiter);
        }
        if let Some(cursor) = req.cursor {
            list = list.continuation_token(cursor);
        }
        let resp = list.send().await.map_err(map_s3_error)?;
        let objects = resp
            .contents()
            .iter()
            .filter_map(|object| {
                let key = object.key()?.to_string();
                let last_modified = object
                    .last_modified()
                    .and_then(|dt| {
                        chrono::Utc
                            .timestamp_opt(dt.secs(), dt.subsec_nanos())
                            .single()
                    })
                    .unwrap_or_else(Utc::now);
                Some(ObjectSummary {
                    key,
                    etag: object.e_tag().unwrap_or_default().to_string(),
                    size: object.size().unwrap_or_default().max(0) as u64,
                    last_modified,
                    // S3 list_objects_v2 does not include Content-Type.
                    content_type: None,
                })
            })
            .collect();
        let common_prefixes = resp
            .common_prefixes()
            .iter()
            .filter_map(|prefix| prefix.prefix().map(str::to_string))
            .collect();
        Ok(ListResp {
            objects,
            common_prefixes,
            next_cursor: resp.next_continuation_token,
        })
    }

    async fn presign(&self, req: PresignReq) -> Result<PresignResp, BackendError> {
        let cfg = PresigningConfig::expires_in(Duration::from_secs(req.expires_in_seconds))
            .map_err(|e| BackendError::Provider {
                inner_code: None,
                message: format!("presign config: {e}"),
            })?;
        let presigned = match req.method {
            PresignMethod::Get => {
                let mut get = self.client.get_object().bucket(&self.bucket).key(&req.key);
                if let Some(cd) = req.response_content_disposition {
                    get = get.response_content_disposition(cd);
                }
                if let Some(ct) = req.response_content_type {
                    get = get.response_content_type(ct);
                }
                get.presigned(cfg).await.map_err(map_s3_error)?
            }
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

    async fn probe(&self) -> Result<(), ProbeFailure> {
        self.client
            .list_objects_v2()
            .bucket(&self.bucket)
            .delimiter("/")
            .max_keys(1)
            .send()
            .await
            .map(|_| ())
            .map_err(probe_failure)
    }

    async fn ensure_bucket(&self) -> Result<bool, BackendError> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => return Ok(false),
            Err(error) => match map_s3_error(error) {
                BackendError::NotFound => {}
                other => return Err(other),
            },
        }
        let mut create = self.client.create_bucket().bucket(&self.bucket);
        if let Some(region) = &self.location_constraint {
            create = create.create_bucket_configuration(
                CreateBucketConfiguration::builder()
                    .location_constraint(BucketLocationConstraint::from(region.as_str()))
                    .build(),
            );
        }
        match create.send().await {
            Ok(_) => Ok(true),
            Err(error)
                if error
                    .as_service_error()
                    .and_then(ProvideErrorMetadata::code)
                    .is_some_and(|code| code == "BucketAlreadyOwnedByYou") =>
            {
                Ok(false)
            }
            Err(error) => Err(map_s3_error(error)),
        }
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
                401 | 403 => BackendError::AuthFailed(describe_sdk_error(&err)),
                _ => BackendError::Provider {
                    inner_code,
                    message: describe_sdk_error(&err),
                },
            }
        }
        SdkError::TimeoutError(_) => BackendError::Provider {
            inner_code: Some("Timeout".into()),
            message: describe_sdk_error(&err),
        },
        _ => BackendError::Provider {
            inner_code: None,
            message: describe_sdk_error(&err),
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
                fast_fail: false,
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
                fast_fail: false,
            },
            "scratch".into(),
        )
        .await;
        assert!(r.is_ok());
    }

    #[test]
    fn content_range_gives_start_and_total() {
        assert_eq!(parse_content_range("bytes 0-99/1234"), Some((0, 1234)));
        assert_eq!(parse_content_range("bytes 100-199/200"), Some((100, 200)));
        assert_eq!(parse_content_range("bytes */1234"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
    }

    #[tokio::test]
    async fn the_location_constraint_follows_the_aws_rule() {
        let build = |region: &'static str, tag: &'static str| async move {
            S3Backend::build(
                S3BackendOpts {
                    region,
                    credentials: None,
                    endpoint_url: None,
                    force_path_style: false,
                    provider_tag: tag,
                    fast_fail: true,
                },
                "b".into(),
            )
            .await
            .unwrap()
            .location_constraint
        };
        assert_eq!(build("us-east-1", "s3").await, None);
        assert_eq!(build("eu-west-1", "s3").await.as_deref(), Some("eu-west-1"));
        assert_eq!(build("auto", "r2").await, None);
        assert_eq!(build("eu-west-1", "r2").await, None);
    }
}
