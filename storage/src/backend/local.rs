//! Local rustfs-managed backend — S3 protocol against the loopback rustfs.

use super::s3::{S3Backend, S3BackendOpts, S3StaticCreds};
use super::{Backend, BackendError};
use std::sync::Arc;

pub struct LocalBackendOpts {
    pub port: u16,
    pub access_key_id: String,
    pub secret_access_key: String,
}

pub async fn build(
    opts: LocalBackendOpts,
    bucket: String,
) -> Result<Arc<dyn Backend>, BackendError> {
    let endpoint = format!("http://127.0.0.1:{}", opts.port);
    let b = S3Backend::build(
        S3BackendOpts {
            region: "us-east-1",
            credentials: Some(S3StaticCreds {
                access_key_id: opts.access_key_id,
                secret_access_key: opts.secret_access_key,
                session_token: None,
            }),
            endpoint_url: Some(endpoint),
            force_path_style: true,
            provider_tag: "local",
        },
        bucket,
    )
    .await?;
    Ok(Arc::new(b))
}

/// Idempotent bucket bootstrap. `head_bucket` first; on 404, `create_bucket`.
/// Used at startup once rustfs is healthy.
pub async fn ensure_bucket(
    port: u16,
    access_key_id: &str,
    secret_access_key: &str,
    bucket: &str,
) -> Result<(), BackendError> {
    let client = build_admin_client(port, access_key_id, secret_access_key).await;

    if client.head_bucket().bucket(bucket).send().await.is_ok() {
        return Ok(());
    }
    client
        .create_bucket()
        .bucket(bucket)
        .send()
        .await
        .map(|_| ())
        .map_err(|e| BackendError::Provider {
            inner_code: None,
            message: format!("create_bucket {bucket}: {e}"),
        })
}

/// Webhook target id we register at startup via the rustfs admin API.
/// Pairs with the ARN below.
const WEBHOOK_TARGET_ID: &str = "iii";

/// rustfs ARN for the in-process webhook notify target. Format:
/// `arn:rustfs:sqs:{region}:{id}:{name}` — see `rustfs/crates/targets/src/arn.rs::ARN::parse`,
/// which strictly requires the `arn:rustfs:sqs:` prefix and 6 colon-separated
/// tokens. The id matches `WEBHOOK_TARGET_ID`; region matches the value our
/// admin client uses (us-east-1); `webhook` is the rustfs target-type name.
const WEBHOOK_ARN: &str = "arn:rustfs:sqs:us-east-1:iii:webhook";

/// Register the webhook target in rustfs's notification subsystem via the
/// admin API. rustfs's env-var loader for `RUSTFS_NOTIFY_WEBHOOK_*_<id>` does
/// not reliably surface targets at startup (we observed `Configuration error:
/// No targets configured` at put_bucket_notification_configuration time even
/// with `RUSTFS_NOTIFY_ENABLE=true` and the per-id env vars set). Calling
/// `PUT /rustfs/admin/v3/target/webhook/<id>` directly is the canonical path
/// — same one rustfs's own admin handlers expose.
pub async fn register_webhook_target(
    port: u16,
    access_key_id: &str,
    secret_access_key: &str,
    webhook_url: &str,
    queue_dir: &std::path::Path,
) -> Result<(), BackendError> {
    // The webhook target requires a writable queue_dir for buffering events
    // when the receiver is briefly unreachable. Without it, the target's
    // `Store::new` call falls back to a default path it can't write to and
    // fails with `Permission denied (os error 13)` — at which point the
    // target is silently dropped and put_bucket_notification_configuration
    // returns `Configuration error: No targets configured`.
    let _ = std::fs::create_dir_all(queue_dir);
    let body = serde_json::json!({
        "key_values": [
            {"key": "enable",    "value": "on"},
            {"key": "endpoint",  "value": webhook_url},
            {"key": "queue_dir", "value": queue_dir.to_string_lossy()},
        ],
    })
    .to_string();

    // The admin route's `target_type` segment is matched against the
    // SUBSYSTEM constant (NOTIFY_WEBHOOK_SUB_SYS = "notify_webhook"), not
    // the service display name "webhook". See rustfs/src/admin/handlers/
    // target_descriptor.rs::target_spec.
    let path = format!("/rustfs/admin/v3/target/notify_webhook/{WEBHOOK_TARGET_ID}");
    let (status, text) = signed_admin_request(
        port,
        access_key_id,
        secret_access_key,
        "PUT",
        &path,
        body.as_bytes(),
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(BackendError::Provider {
            inner_code: Some(status.to_string()),
            message: format!("admin PUT {path} returned {status}: {text}"),
        });
    }

    // Diagnostic: poll /target/arns until it's non-empty (or 3s elapses).
    // The admin PUT returns once the config is persisted, but rustfs
    // initializes targets asynchronously — there's a window where the
    // target is in /target/list (config view) but not yet in /target/arns
    // (live notifier view), which is exactly the window where
    // put_bucket_notification_configuration would fail with "No targets
    // configured".
    let mut last_list = String::new();
    let mut last_arns = String::new();
    for attempt in 0..15 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let (_, ls_body) = signed_admin_request(
            port,
            access_key_id,
            secret_access_key,
            "GET",
            "/rustfs/admin/v3/target/list",
            b"",
        )
        .await?;
        let (_, arns_body) = signed_admin_request(
            port,
            access_key_id,
            secret_access_key,
            "GET",
            "/rustfs/admin/v3/target/arns",
            b"",
        )
        .await?;
        last_list = ls_body;
        last_arns = arns_body;
        if !last_arns.trim().starts_with("[]") {
            tracing::info!(
                target = "rustfs_admin_diag",
                attempt,
                arns = %last_arns,
                "webhook target appeared in /target/arns"
            );
            return Ok(());
        }
    }
    tracing::warn!(
        target = "rustfs_admin_diag",
        list = %last_list,
        arns = %last_arns,
        "webhook target never appeared in /target/arns after 3s"
    );
    Ok(())
}

async fn signed_admin_request(
    port: u16,
    access_key_id: &str,
    secret_access_key: &str,
    method: &str,
    path: &str,
    body_bytes: &[u8],
) -> Result<(u16, String), BackendError> {
    use aws_credential_types::Credentials;
    use aws_sigv4::http_request::{
        sign, PayloadChecksumKind, SignableBody, SignableRequest, SigningSettings,
    };
    use aws_sigv4::sign::v4;
    use std::time::SystemTime;

    let url = format!("http://127.0.0.1:{port}{path}");
    let identity = Credentials::new(
        access_key_id,
        secret_access_key,
        None,
        None,
        "storage-admin",
    )
    .into();

    let mut settings = SigningSettings::default();
    // rustfs's S3 server enforces x-amz-content-sha256 on every request.
    settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region("us-east-1")
        .name("s3")
        .time(SystemTime::now())
        .settings(settings)
        .build()
        .map_err(|e| BackendError::Provider {
            inner_code: None,
            message: format!("sigv4 params: {e}"),
        })?
        .into();

    let header_iter = std::iter::empty::<(&str, &str)>();
    let signable = SignableRequest::new(method, &url, header_iter, SignableBody::Bytes(body_bytes))
        .map_err(|e| BackendError::Provider {
            inner_code: None,
            message: format!("sigv4 signable: {e}"),
        })?;
    let (signing_instructions, _signature) = sign(signable, &signing_params)
        .map_err(|e| BackendError::Provider {
            inner_code: None,
            message: format!("sigv4 sign: {e}"),
        })?
        .into_parts();

    let client = reqwest::Client::new();
    let mut req = match method {
        "PUT" => client.put(&url),
        "GET" => client.get(&url),
        "DELETE" => client.delete(&url),
        m => {
            return Err(BackendError::Provider {
                inner_code: None,
                message: format!("unsupported admin method: {m}"),
            })
        }
    };
    if !body_bytes.is_empty() {
        req = req
            .header("content-type", "application/json")
            .body(body_bytes.to_vec());
    }
    for (name, value) in signing_instructions.headers() {
        req = req.header(name, value);
    }

    let resp = req.send().await.map_err(|e| BackendError::Provider {
        inner_code: None,
        message: format!("admin {method} {path}: {e}"),
    })?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((status, text))
}

/// Subscribe a bucket to the webhook target so rustfs actually POSTs object-
/// created / object-deleted events to the worker's `/notify` receiver.
///
/// Configuring the webhook target via env vars (in `rustfs::spawn::spawn`)
/// only DEFINES the target; rustfs does not deliver events for any bucket
/// until that bucket has an explicit `NotificationConfiguration` referencing
/// the target's ARN. This is the same two-step model MinIO uses (define
/// target with `mc admin config`, then subscribe with `mc event add`); we
/// just collapse step 2 into worker startup so users don't need an external
/// admin tool. Idempotent: rustfs replaces the existing config on each call.
pub async fn subscribe_bucket_notifications(
    port: u16,
    access_key_id: &str,
    secret_access_key: &str,
    bucket: &str,
) -> Result<(), BackendError> {
    use aws_sdk_s3::types::{Event, NotificationConfiguration, QueueConfiguration};

    let client = build_admin_client(port, access_key_id, secret_access_key).await;

    // Subscribe to all object-created and object-removed events. The
    // wildcard variants (`s3:ObjectCreated:*` / `s3:ObjectRemoved:*`) cover
    // PUT, POST, COPY, CompleteMultipartUpload, Delete, and
    // DeleteMarkerCreated — matching what `triggers::normalize::normalize_rustfs`
    // already maps into `EventKind::Created` / `EventKind::Deleted`.
    let queue_cfg = QueueConfiguration::builder()
        .id("storage-events")
        .queue_arn(WEBHOOK_ARN)
        .events(Event::S3ObjectCreated)
        .events(Event::S3ObjectRemoved)
        .build()
        .map_err(|e| BackendError::Provider {
            inner_code: None,
            message: format!("queue_configuration build: {e}"),
        })?;

    let notif = NotificationConfiguration::builder()
        .queue_configurations(queue_cfg)
        .build();

    client
        .put_bucket_notification_configuration()
        .bucket(bucket)
        .notification_configuration(notif)
        .send()
        .await
        .map(|_| ())
        .map_err(|e| {
            // Pull the raw response body if available; the AWS SDK's
            // top-level Display only says "service error", but the inner
            // error has the rustfs/MinIO error code + message.
            let detail = e
                .raw_response()
                .and_then(|r| std::str::from_utf8(r.body().bytes().unwrap_or_default()).ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{e:?}"));
            BackendError::Provider {
                inner_code: None,
                message: format!("put_bucket_notification_configuration {bucket}: {detail}"),
            }
        })
}

async fn build_admin_client(
    port: u16,
    access_key_id: &str,
    secret_access_key: &str,
) -> aws_sdk_s3::Client {
    use aws_config::{BehaviorVersion, Region};
    use aws_credential_types::Credentials;
    use aws_sdk_s3::config::SharedCredentialsProvider;

    let static_creds = Credentials::new(
        access_key_id,
        secret_access_key,
        None,
        None,
        "storage-local-bootstrap",
    );
    let endpoint = format!("http://127.0.0.1:{port}");
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new("us-east-1".to_string()))
        .credentials_provider(SharedCredentialsProvider::new(static_creds))
        .load()
        .await;
    let s3_config = aws_sdk_s3::config::Builder::from(&sdk_config)
        .endpoint_url(endpoint)
        .force_path_style(true)
        .build();
    aws_sdk_s3::Client::from_conf(s3_config)
}
