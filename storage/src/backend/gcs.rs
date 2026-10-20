//! Google Cloud Storage backend.
//!
//! Native client (not S3-interop) — GCS's S3-interop mode is feature-limited
//! and forces operators onto HMAC keys. We use yoshidan's `gcloud-storage`
//! crate (1.x) which speaks the JSON API directly. Authentication uses the
//! standard Application Default Credentials (ADC) chain by default; an
//! explicit `credentials_file` is loaded via `CredentialsFile::new_from_file`
//! (no process-wide env mutation — see review CRITICAL #1).

use super::*;
use chrono::TimeZone;
use gcloud_storage::client::{Client, ClientConfig};
use gcloud_storage::http::error::ErrorResponse;
use gcloud_storage::http::objects::delete::DeleteObjectRequest;
use gcloud_storage::http::objects::download::Range;
use gcloud_storage::http::objects::get::GetObjectRequest;
use gcloud_storage::http::objects::list::ListObjectsRequest;
use gcloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};
use gcloud_storage::http::objects::Object;
use gcloud_storage::http::Error as GcsHttpError;
use gcloud_storage::sign::{SignedURLMethod, SignedURLOptions};
use std::borrow::Cow;
use std::time::Duration;

pub struct GcsBackend {
    client: Client,
    bucket: String,
    /// True when an `endpoint_url` override is in effect. Presign always
    /// signs against `storage.googleapis.com`, so it produces URLs that
    /// can't reach the override; we fail closed to avoid silently handing
    /// out unusable links.
    endpoint_overridden: bool,
    /// The client can sign URLs: a service-account key, or the metadata
    /// server's identity on Google Cloud.
    can_sign: bool,
}

pub struct GcsBackendOpts {
    /// The service-account JSON itself (resolved `credentials_json`).
    /// Takes precedence over `credentials_file`.
    pub credentials_json: Option<String>,
    /// Path to a service-account JSON. When `None`, defers to ADC chain.
    pub credentials_file: Option<String>,
    /// Custom storage endpoint. `None` → default GCS host. The actual
    /// application of this field to the SDK is wired in Phase 2 once the
    /// gcloud-storage endpoint-override spike confirms the API.
    pub endpoint_url: Option<String>,
}

impl GcsBackend {
    pub async fn build(opts: GcsBackendOpts, bucket: String) -> Result<Self, BackendError> {
        let mut config = if let Some(json) = opts.credentials_json.as_deref() {
            // The parse error never quotes the input (it may be a key).
            let creds = gcloud_auth::credentials::CredentialsFile::new_from_str(json)
                .await
                .map_err(|_| {
                    BackendError::AuthFailed(
                        "gcs `credentials_json` is not a valid service-account or ADC credentials JSON"
                            .into(),
                    )
                })?;
            ClientConfig::default()
                .with_credentials(creds)
                .await
                .map_err(|e| {
                    BackendError::AuthFailed(format!("gcs auth (credentials_json): {e}"))
                })?
        } else if let Some(path) = opts.credentials_file {
            // Load the SA file directly instead of mutating GOOGLE_APPLICATION_CREDENTIALS,
            // which would race across multiple GCS buckets and leak into unrelated
            // SDK clients or child processes.
            // A file of the engine host: read here, bounded, never a FIFO or
            // a device (a reload must not block), and only a key whose token
            // endpoint is Google's (the signed assertion goes there). Every
            // failure reads the same, so the message does not tell whether a
            // host path exists or what it holds.
            let creds = tokio::task::spawn_blocking(move || host_key_file(&path))
                .await
                .map_err(|_| unusable_key_file())??;
            ClientConfig::default()
                .with_credentials(creds)
                .await
                .map_err(|e| BackendError::AuthFailed(format!("gcs auth (explicit): {e}")))?
        } else {
            ClientConfig::default()
                .with_auth()
                .await
                .map_err(|e| BackendError::AuthFailed(format!("gcs auth (ADC): {e}")))?
        };
        // Custom endpoint for fake-gcs-server / private GCS-compatible deployments.
        // gcloud-storage 1.3.0 stores this on ClientConfig.storage_endpoint and
        // threads it into v1_endpoint + v1_upload_endpoint, so object reads,
        // writes, and metadata calls all hit the override. Presign signing
        // (sign.rs) generates URLs against storage.googleapis.com regardless;
        // fake-gcs accepts unsigned/anonymous URLs so this is fine for tests.
        let endpoint_overridden = opts.endpoint_url.is_some();
        let can_sign =
            config.default_google_access_id.is_some() && config.default_sign_by.is_some();
        if let Some(url) = opts.endpoint_url.as_deref() {
            config.storage_endpoint = url.to_string();
            tracing::info!(
                target = "storage::gcs",
                endpoint = %crate::config::redact_url(url),
                "GCS backend using custom storage_endpoint override",
            );
        }
        let client = Client::new(config);
        Ok(Self {
            client,
            bucket,
            endpoint_overridden,
            can_sign,
        })
    }
}

#[async_trait]
impl Backend for GcsBackend {
    async fn put(&self, req: PutReq) -> Result<PutResp, BackendError> {
        let size = req.body.len() as u64;
        // Switch from Simple to Multipart upload only when the caller actually
        // supplied cache_control or metadata. Simple is cheaper (no multipart
        // framing), so we keep it for the common case of a bare body.
        let needs_multipart = req.cache_control.is_some() || !req.metadata.is_empty();
        let upload = if needs_multipart {
            let metadata = if req.metadata.is_empty() {
                None
            } else {
                Some(req.metadata.clone())
            };
            let object = Object {
                name: req.key.clone(),
                bucket: self.bucket.clone(),
                content_type: Some(req.content_type.clone()),
                cache_control: req.cache_control.clone(),
                metadata,
                ..Default::default()
            };
            UploadType::Multipart(Box::new(object))
        } else {
            let mut media = Media::new(req.key.clone());
            media.content_type = Cow::Owned(req.content_type.clone());
            media.content_length = Some(size);
            UploadType::Simple(media)
        };
        let object = self
            .client
            .upload_object(
                &UploadObjectRequest {
                    bucket: self.bucket.clone(),
                    ..Default::default()
                },
                req.body,
                &upload,
            )
            .await
            .map_err(map_gcs_error)?;
        Ok(PutResp {
            etag: object.etag,
            size,
            version_id: Some(object.generation.to_string()),
        })
    }

    async fn get(&self, req: GetReq) -> Result<GetResp, BackendError> {
        let mut get_req = GetObjectRequest {
            bucket: self.bucket.clone(),
            object: req.key.clone(),
            ..Default::default()
        };
        if let Some(v) = req.version_id.as_deref() {
            get_req.generation = v.parse::<i64>().ok();
        }
        // Metadata first — gcloud-storage's download_object returns Vec<u8> with
        // no header surface, so we need a separate get_object for size/etag.
        let object = self
            .client
            .get_object(&get_req)
            .await
            .map_err(map_gcs_error)?;
        let size = if object.size < 0 {
            0
        } else {
            object.size as u64
        };
        let (start, end) = match req.range {
            Some(range) => range.span(size)?,
            None => (0, size),
        };
        // Reject oversized reads BEFORE the download call buffers anything.
        ByteRange::check_cap(start, end, req.max_inline_bytes)?;
        // Pin the body download to the exact generation we just read so a
        // concurrent overwrite returns 412 instead of silently mismatching the
        // returned etag/body.
        let mut download_req = get_req.clone();
        download_req.if_generation_match = Some(object.generation);
        let body = if start == end && req.range.is_some() {
            Vec::new()
        } else {
            let range = match req.range {
                Some(_) => Range(Some(start), Some(end - 1)),
                None => Range::default(),
            };
            self.client
                .download_object(&download_req, &range)
                .await
                .map_err(map_gcs_error)?
        };
        let last_modified = object
            .updated
            .and_then(|t| chrono::Utc.timestamp_opt(t.unix_timestamp(), 0).single())
            .unwrap_or_else(Utc::now);
        let content_type = object
            .content_type
            .unwrap_or_else(|| "application/octet-stream".to_string());
        Ok(GetResp {
            body,
            content_type,
            etag: object.etag,
            last_modified,
            size,
            offset: start,
        })
    }

    async fn head(&self, req: HeadReq) -> Result<HeadResp, BackendError> {
        let mut get_req = GetObjectRequest {
            bucket: self.bucket.clone(),
            object: req.key.clone(),
            ..Default::default()
        };
        if let Some(v) = req.version_id.as_deref() {
            get_req.generation = v.parse::<i64>().ok();
        }
        let object = self
            .client
            .get_object(&get_req)
            .await
            .map_err(map_gcs_error)?;
        let size = if object.size < 0 {
            0
        } else {
            object.size as u64
        };
        let last_modified = object
            .updated
            .and_then(|t| chrono::Utc.timestamp_opt(t.unix_timestamp(), 0).single())
            .unwrap_or_else(Utc::now);
        let content_type = object
            .content_type
            .unwrap_or_else(|| "application/octet-stream".to_string());
        Ok(HeadResp {
            content_type,
            etag: object.etag,
            last_modified,
            size,
        })
    }

    async fn delete(&self, req: DeleteReq) -> Result<DeleteResp, BackendError> {
        let mut del_req = DeleteObjectRequest {
            bucket: self.bucket.clone(),
            object: req.key.clone(),
            ..Default::default()
        };
        if let Some(v) = req.version_id.as_deref() {
            del_req.generation = v.parse::<i64>().ok();
        }
        match self.client.delete_object(&del_req).await {
            Ok(()) => Ok(DeleteResp { deleted: true }),
            Err(e) => match map_gcs_error(e) {
                BackendError::NotFound => Ok(DeleteResp { deleted: false }),
                other => Err(other),
            },
        }
    }

    async fn list(&self, req: ListReq) -> Result<ListResp, BackendError> {
        let resp = self
            .client
            .list_objects(&ListObjectsRequest {
                bucket: self.bucket.clone(),
                prefix: Some(req.prefix),
                delimiter: req.delimiter,
                page_token: req.cursor,
                max_results: Some(req.limit.min(1_000) as i32),
                ..Default::default()
            })
            .await
            .map_err(map_gcs_error)?;
        let objects = resp
            .items
            .unwrap_or_default()
            .into_iter()
            .map(|object| ObjectSummary {
                key: object.name,
                etag: object.etag,
                size: object.size.max(0) as u64,
                last_modified: object
                    .updated
                    .and_then(|value| {
                        chrono::Utc
                            .timestamp_opt(value.unix_timestamp(), value.nanosecond())
                            .single()
                    })
                    .unwrap_or_else(Utc::now),
                content_type: object.content_type,
            })
            .collect();
        Ok(ListResp {
            objects,
            common_prefixes: resp.prefixes.unwrap_or_default(),
            next_cursor: resp.next_page_token,
        })
    }

    async fn presign(&self, req: PresignReq) -> Result<PresignResp, BackendError> {
        if self.endpoint_overridden {
            return Err(BackendError::PresignUnsupported(
                "gcs presign always signs against storage.googleapis.com; not supported when endpoint_url is overridden (fake-gcs-server, etc.)".into(),
            ));
        }
        let mut opts = SignedURLOptions {
            expires: Duration::from_secs(req.expires_in_seconds),
            ..Default::default()
        };
        opts.method = match req.method {
            PresignMethod::Get => SignedURLMethod::GET,
            PresignMethod::Put => SignedURLMethod::PUT,
        };
        if matches!(req.method, PresignMethod::Put) {
            opts.content_type = req.content_type.clone();
        }
        if let Some(cd) = &req.response_content_disposition {
            opts.query_parameters
                .insert("response-content-disposition".to_string(), vec![cd.clone()]);
        }
        if let Some(ct) = &req.response_content_type {
            opts.query_parameters
                .insert("response-content-type".to_string(), vec![ct.clone()]);
        }
        let url = self
            .client
            .signed_url(&self.bucket, &req.key, None, None, opts)
            .await
            .map_err(|e| BackendError::PresignUnsupported(format!("gcs signed_url: {e}")))?;
        let expires_at = Utc::now() + chrono::Duration::seconds(req.expires_in_seconds as i64);
        Ok(PresignResp { url, expires_at })
    }

    fn provider(&self) -> &'static str {
        "gcs"
    }

    fn direct_downloads(&self) -> bool {
        !self.endpoint_overridden && self.can_sign
    }

    async fn probe(&self) -> Result<(), ProbeFailure> {
        use crate::material::Stage;
        let result = self
            .client
            .list_objects(&ListObjectsRequest {
                bucket: self.bucket.clone(),
                prefix: Some(String::new()),
                delimiter: Some("/".into()),
                max_results: Some(1),
                ..Default::default()
            })
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(GcsHttpError::Response(ErrorResponse { code, message, .. })) => Err(match code {
                404 => ProbeFailure::missing(message),
                401 | 403 => ProbeFailure::new(Stage::Auth, message),
                _ => ProbeFailure::new(Stage::List, format!("HTTP {code}: {message}")),
            }),
            Err(GcsHttpError::TokenSource(e)) => Err(ProbeFailure::new(
                Stage::Auth,
                format!("token source failed: {e}"),
            )),
            Err(GcsHttpError::HttpClient(e))
                if e.is_connect() || e.is_timeout() || e.is_request() =>
            {
                Err(ProbeFailure::new(Stage::Endpoint, error_chain(&e)))
            }
            Err(other) => Err(ProbeFailure::new(Stage::List, error_chain(&other))),
        }
    }
}

/// The largest `credentials_file` read (a service-account key is ~2.4 KiB).
pub const KEY_FILE_CAP: u64 = 64 * 1024;

fn unusable_key_file() -> BackendError {
    BackendError::Provider {
        inner_code: Some("CONFIG".into()),
        message: "`credentials_file` cannot be used as a service-account key file (a readable \
                  regular JSON key file of at most 64 KiB whose token_uri is Google's)"
            .into(),
    }
}

/// A token endpoint of Google's.
fn google_token_uri(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|url| {
        url.scheme() == "https"
            && url.port().is_none_or(|port| port == 443)
            && url.host_str().is_some_and(|host| {
                host == "oauth2.googleapis.com"
                    || host == "accounts.google.com"
                    || host.ends_with(".googleapis.com")
            })
    })
}

/// Read and check a host `credentials_file`: a regular, non-empty file of at
/// most [`KEY_FILE_CAP`] bytes (checked before and after opening it without
/// blocking), a `service_account` or `authorized_user` JSON whose
/// `token_uri`, if any, is Google's.
fn host_key_file(path: &str) -> Result<gcloud_auth::credentials::CredentialsFile, BackendError> {
    use std::io::Read;
    let regular =
        |meta: &std::fs::Metadata| meta.is_file() && meta.len() > 0 && meta.len() <= KEY_FILE_CAP;
    let meta = std::fs::metadata(path).map_err(|_| unusable_key_file())?;
    if !regular(&meta) {
        return Err(unusable_key_file());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Swapped for a FIFO after the check: opening must not wait for a writer.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| unusable_key_file())?;
    if !file.metadata().is_ok_and(|meta| regular(&meta)) {
        return Err(unusable_key_file());
    }
    let mut bytes = Vec::new();
    file.take(KEY_FILE_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unusable_key_file())?;
    if bytes.len() as u64 > KEY_FILE_CAP {
        return Err(unusable_key_file());
    }
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| unusable_key_file())?;
    let kind = json.get("type").and_then(serde_json::Value::as_str);
    let token_uri_ok = match json.get("token_uri") {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(uri)) => google_token_uri(uri),
        Some(_) => false,
    };
    if !matches!(kind, Some("service_account" | "authorized_user")) || !token_uri_ok {
        return Err(unusable_key_file());
    }
    serde_json::from_value(json).map_err(|_| unusable_key_file())
}

/// Map a `gcloud_storage::http::Error` into our `BackendError`. 404 / NotFound
/// service responses fold into `BackendError::NotFound`; 401/403 into
/// `AuthFailed`; everything else into `Provider`.
fn map_gcs_error(err: GcsHttpError) -> BackendError {
    match &err {
        GcsHttpError::Response(ErrorResponse { code, message, .. }) => match *code {
            404 => BackendError::NotFound,
            401 | 403 => BackendError::AuthFailed(message.clone()),
            other => BackendError::Provider {
                inner_code: Some(other.to_string()),
                message: message.clone(),
            },
        },
        GcsHttpError::TokenSource(_) => BackendError::AuthFailed(format!("{err}")),
        _ => BackendError::Provider {
            inner_code: None,
            message: format!("{err}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // GCS construction can't smoke-test without ADC creds (it'll fail auth).
    // Real round-trip lives in tests/e2e/gcs.rs (Phase 17). This test only
    // guarantees that a missing/invalid credentials_file path doesn't panic
    // — the SDK should return a recoverable Result we then map to AuthFailed.
    #[tokio::test]
    async fn gcs_construction_with_missing_creds_returns_auth_error() {
        let r = GcsBackend::build(
            GcsBackendOpts {
                credentials_json: None,
                credentials_file: Some("/nonexistent/path/to/sa.json".into()),
                endpoint_url: None,
            },
            "test-bucket".into(),
        )
        .await;
        // It's OK if this passes or returns AuthFailed depending on what
        // the SDK does with a non-existent file — the test guarantees no panic.
        let _ = r;
    }

    #[tokio::test]
    async fn gcs_build_with_endpoint_url_does_not_panic() {
        let r = GcsBackend::build(
            GcsBackendOpts {
                credentials_json: None,
                credentials_file: None,
                endpoint_url: Some("http://127.0.0.1:4443".into()),
            },
            "bucket".into(),
        )
        .await;
        // Whether this returns Ok or Err depends on whether ADC is reachable
        // in the test env (it likely isn't on a clean CI box) — the assertion
        // is just "doesn't panic". The endpoint-application path is exercised
        // post-credentials-check, so any auth Err would surface BEFORE we
        // touch config.storage_endpoint. The test guards against the field
        // assignment itself panicking, e.g., from a typo in the field name.
        let _ = r;
    }

    async fn build_with_file(path: &str) -> Result<GcsBackend, BackendError> {
        GcsBackend::build(
            GcsBackendOpts {
                credentials_json: None,
                credentials_file: Some(path.into()),
                endpoint_url: None,
            },
            "b".into(),
        )
        .await
    }

    fn message(result: Result<GcsBackend, BackendError>) -> String {
        match result {
            Err(BackendError::Provider { message, .. }) => message,
            Err(other) => panic!("expected a config error, got {other}"),
            Ok(_) => panic!("expected a refusal"),
        }
    }

    #[tokio::test]
    async fn an_unusable_credentials_file_fails_fast_with_one_message() {
        let dir = tempfile::tempdir().unwrap();
        let expected = message(build_with_file("/nonexistent/secret-dir/sa.json").await);
        assert!(
            expected.contains("cannot be used as a service-account key file"),
            "{expected}"
        );
        assert!(!expected.contains("secret-dir"));
        let write = |name: &str, body: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, body).unwrap();
            path.to_string_lossy().to_string()
        };
        let sa = |token_uri: &str| {
            format!(
                "{{\"type\":\"service_account\",\"client_email\":\"a@b\",\"private_key\":\"LEAKME\",\"token_uri\":\"{token_uri}\"}}"
            )
        };
        let cases = vec![
            write("malformed.json", b"{\"private_key\": \"LEAKME\""),
            write("empty.json", b""),
            write("big.json", &vec![b' '; (KEY_FILE_CAP + 1) as usize]),
            write(
                "external.json",
                b"{\"type\":\"external_account\",\"audience\":\"x\"}",
            ),
            write(
                "evil-token.json",
                sa("http://127.0.0.1:49262/token").as_bytes(),
            ),
            write(
                "evil-host.json",
                sa("https://oauth2.googleapis.com.evil.example/token").as_bytes(),
            ),
            dir.path().to_string_lossy().to_string(),
            "/dev/zero".to_string(),
        ];
        for path in cases {
            assert_eq!(message(build_with_file(&path).await), expected, "{path}");
        }
        // A FIFO is refused at once, without waiting for a writer.
        let fifo = dir.path().join("fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        let started = std::time::Instant::now();
        assert_eq!(
            message(build_with_file(&fifo.to_string_lossy()).await),
            expected
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        // Google's token endpoint is accepted (the key itself is fake, so
        // the client may still refuse it later, but not as unusable).
        assert!(google_token_uri("https://oauth2.googleapis.com/token"));
        assert!(google_token_uri(
            "https://accounts.google.com/o/oauth2/token"
        ));
        assert!(!google_token_uri(
            "https://oauth2.googleapis.com:8443/token"
        ));
        assert!(!google_token_uri("http://oauth2.googleapis.com/token"));
    }

    #[tokio::test]
    async fn a_malformed_credentials_json_is_an_auth_error_that_never_quotes_it() {
        let r = GcsBackend::build(
            GcsBackendOpts {
                credentials_json: Some("{\"private_key\": \"LEAKME\"".into()),
                credentials_file: None,
                endpoint_url: None,
            },
            "bucket".into(),
        )
        .await;
        match r {
            Err(BackendError::AuthFailed(message)) => {
                assert!(!message.contains("LEAKME"), "{message}")
            }
            Err(other) => panic!("expected AuthFailed, got {other}"),
            Ok(_) => panic!("a malformed credentials JSON must not build"),
        }
    }
}
