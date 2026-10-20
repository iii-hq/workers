//! `storage::testBucket` — probe a CANDIDATE bucket entry.
//!
//! Exists for bucket editors: the operator fills in an entry, presses "Test
//! bucket", and learns whether it works *before* saving. The probe builds a
//! throwaway backend outside the live ones — with every `secret://` /
//! `env://` reference resolved fresh, never from the cache — lists at most
//! one object at the root, and reports the outcome as data (`ok: false` is a
//! normal response, not a handler error) with the `stage` that failed.
//!
//! `"__redacted__"` values (what `getBucket` shows for stored literals) are
//! taken from the stored bucket `name`, and only toward that bucket's
//! endpoint / account; a reference a stored bucket uses is only sent where
//! that bucket sends it. Error texts are returned with every credential the
//! probe used scrubbed.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

use super::buckets::{prepare_candidate, stored_entry_both};
use super::AppState;
use crate::backend::factory::{self, Purpose};
use crate::config::{parse_bucket, BucketConfig, WorkerConfig};
use crate::configuration::StorageRuntime;
use crate::material::{self, BucketError, Material, Stage};

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TestBucketReq {
    /// The candidate entry (`provider` and its fields), as `saveBucket`
    /// takes it. Absent: test the stored bucket `name` as it is.
    #[serde(default)]
    pub bucket: Option<Value>,
    /// The stored bucket `"__redacted__"` values come from (and the default
    /// provider-side bucket when the entry has no `bucket`).
    #[serde(default)]
    pub name: Option<String>,
    /// Sensitive values apart from `bucket` (`access_key_id`,
    /// `secret_access_key`, `session_token`, `credentials_json`,
    /// `notifications_api_token`), merged over it.
    #[serde(default)]
    pub credentials: Option<Value>,
    /// Overall budget. Default 10000, capped at 30000.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TestBucketResp {
    /// Whether the bucket was reached and listed.
    pub ok: bool,
    /// `s3` | `r2` | `gcs` | `local` | `unknown`.
    pub provider: String,
    /// Wall time of the whole attempt.
    pub latency_ms: u64,
    /// Why the probe failed (credentials scrubbed). Absent on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `config` | `secrets` | `endpoint` | `auth` | `bucket` | `list`.
    /// Absent on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    /// The provider said the bucket does not exist (`saveBucket` with
    /// `create_bucket: true` creates it, for s3 and r2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket_missing: Option<bool>,
    /// Whether presigned GETs from this bucket are fetchable by clients.
    /// Present once the backend was built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_downloads: Option<bool>,
}

impl TestBucketResp {
    fn failed(provider: &str, stage: Stage, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            provider: provider.to_string(),
            latency_ms: 0,
            message: Some(message.into()),
            stage: Some(stage),
            bucket_missing: None,
            direct_downloads: None,
        }
    }
}

pub async fn handle(
    state: &AppState,
    runtime: &StorageRuntime,
    req: TestBucketReq,
) -> Result<TestBucketResp, String> {
    let started = Instant::now();
    let budget = Duration::from_millis(
        req.timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1, MAX_TIMEOUT_MS),
    );
    let mut resp = run(state, runtime, &req, budget).await;
    resp.latency_ms = started.elapsed().as_millis() as u64;
    Ok(resp)
}

async fn run(
    state: &AppState,
    runtime: &StorageRuntime,
    req: &TestBucketReq,
    budget: Duration,
) -> TestBucketResp {
    let (stored, stored_expanded) = match &req.name {
        Some(name) => stored_entry_both(runtime, name).await.unwrap_or_default(),
        None => (None, None),
    };
    let candidate = match (&req.bucket, &stored) {
        (Some(bucket), _) => bucket.clone(),
        // The stored bucket as the worker runs it: templates expanded.
        (None, Some(stored)) => stored_expanded.clone().unwrap_or_else(|| stored.clone()),
        (None, None) => {
            return TestBucketResp::failed(
                "unknown",
                Stage::Config,
                match &req.name {
                    Some(name) => format!("no stored bucket named `{name}`; send `bucket`"),
                    None => {
                        "send `bucket` (the entry to test) or the `name` of a stored bucket".into()
                    }
                },
            )
        }
    };
    let provider_hint = candidate
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    // A stored entry tested as it is may hold legacy literals: they are not
    // in a request, so only a request's `bucket` is held to the rules.
    let (entry, restored) = if req.bucket.is_some() {
        match prepare_candidate(candidate, req.credentials.as_ref(), stored.as_ref()) {
            Ok(prepared) => prepared,
            Err(e) => {
                let hint = if req.name.is_none() && e.contains("no stored value") {
                    " (pass `name` to use the stored bucket's values)"
                } else {
                    ""
                };
                return TestBucketResp::failed(&provider_hint, Stage::Config, format!("{e}{hint}"));
            }
        }
    } else {
        (candidate, Vec::new())
    };
    // `${VAR}` templates kept from the stored entry take the value the
    // configuration worker expanded for it; a new template cannot be
    // expanded here (the engine's environment is not the worker's).
    let mut entry = entry;
    let mut restored = restored;
    if req.bucket.is_some() {
        restored.extend(expand_stored_templates(
            &mut entry,
            stored.as_ref(),
            stored_expanded.as_ref(),
        ));
    }
    if let Some(path) = template_paths(&entry, "").first() {
        return TestBucketResp::failed(
            &provider_hint,
            Stage::Config,
            format!(
                "`bucket.{path}` is a ${{VAR}} template, which the configuration worker expands \
                 when the bucket is saved; save it first, or use env://VAR"
            ),
        );
    }

    let label = req.name.clone().unwrap_or_else(|| "candidate".into());
    if req.name.is_none()
        && entry
            .get("bucket")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return TestBucketResp::failed(
            &provider_hint,
            Stage::Config,
            "`bucket.bucket` (the provider-side bucket) is required when testing without a `name`",
        );
    }
    let bucket = match parse_bucket(&label, &entry) {
        Ok(bucket) => bucket,
        Err(e) => return TestBucketResp::failed(&provider_hint, Stage::Config, e),
    };
    let provider = bucket.provider();

    // A stored value only ever goes where its bucket goes.
    if !restored.is_empty() {
        let (Some(name), Some(stored)) = (&req.name, &stored) else {
            return TestBucketResp::failed(provider, Stage::Config, "nothing stored to restore");
        };
        let stored_bucket = match parse_bucket(name, stored) {
            Ok(b) => b,
            Err(_) => {
                return TestBucketResp::failed(
                    provider,
                    Stage::Config,
                    format!(
                        "the stored bucket `{name}` could not be read to compare endpoints; \
                         re-enter the redacted values"
                    ),
                )
            }
        };
        if let Err(e) = restored_targets_match(&bucket, &stored_bucket, name, &restored) {
            return TestBucketResp::failed(provider, Stage::Config, e);
        }
    }
    let snapshot = state.catalog.read().await.config.clone();
    if let Err(e) = references_stay_bound(&bucket, &snapshot) {
        return TestBucketResp::failed(provider, Stage::Config, e);
    }

    match tokio::time::timeout(budget, probe(state, &label, &bucket, &snapshot)).await {
        Ok(resp) => resp,
        Err(_) => TestBucketResp::failed(
            provider,
            Stage::Endpoint,
            format!("timed out after {}ms", budget.as_millis()),
        ),
    }
}

async fn probe(
    state: &AppState,
    label: &str,
    bucket: &BucketConfig,
    snapshot: &WorkerConfig,
) -> TestBucketResp {
    let provider = bucket.provider();
    // Fresh, never the live buckets' cache: a person who just fixed the
    // secret and taps Test again must see the fix.
    let fresh = state
        .secrets
        .resolve_fresh(material::references(bucket))
        .await;
    let material = match Material::resolve(bucket, |key| fresh.get(key).cloned()) {
        Ok(material) => material,
        Err(e) => return TestBucketResp::failed(provider, e.stage, e.message),
    };
    if let BucketConfig::Local(_) = bucket {
        return probe_local(label, bucket, snapshot).await;
    }
    let backend = match factory::build(
        label,
        bucket,
        &snapshot.providers,
        None,
        &material,
        Purpose::Probe,
    )
    .await
    {
        Ok(backend) => backend,
        Err(e) => {
            let e = BucketError::from_build(e, &material);
            return TestBucketResp::failed(provider, e.stage, e.message);
        }
    };
    let direct_downloads = Some(backend.direct_downloads());
    match backend.probe().await {
        Ok(()) => TestBucketResp {
            ok: true,
            provider: provider.to_string(),
            latency_ms: 0,
            message: None,
            stage: None,
            bucket_missing: None,
            direct_downloads,
        },
        Err(failure) => TestBucketResp {
            ok: false,
            provider: provider.to_string(),
            latency_ms: 0,
            message: Some(material.scrub(&failure.message)),
            stage: Some(failure.stage),
            bucket_missing: failure.bucket_missing.then_some(true),
            direct_downloads,
        },
    }
}

/// A local bucket works when its storage directory can be written: the
/// bucket's directory when it exists, else the nearest existing parent
/// (the save creates the rest). Nothing is left behind.
async fn probe_local(
    label: &str,
    bucket: &BucketConfig,
    snapshot: &WorkerConfig,
) -> TestBucketResp {
    let data_dir = snapshot
        .providers
        .local
        .as_ref()
        .map(|local| local.data_dir.clone())
        .unwrap_or_else(|| "data/storage".into());
    let root = iii_worker_paths::resolve_path(&data_dir);
    let target = root.join("buckets").join(bucket.underlying(label));
    let result = tokio::task::spawn_blocking(move || {
        let mut dir = target.as_path();
        while !dir.exists() {
            dir = dir
                .parent()
                .ok_or_else(|| "no existing parent directory".to_string())?;
        }
        let probe = dir.join(format!(".iii-storage-probe-{:016x}", rand::random::<u64>()));
        std::fs::write(&probe, b"probe")
            .map_err(|e| format!("{} is not writable: {e}", dir.display()))?;
        let _ = std::fs::remove_file(&probe);
        Ok::<_, String>(())
    })
    .await
    .unwrap_or_else(|e| Err(format!("probe task failed: {e}")));
    match result {
        Ok(()) => TestBucketResp {
            ok: true,
            provider: "local".into(),
            latency_ms: 0,
            message: None,
            stage: None,
            bucket_missing: None,
            direct_downloads: Some(true),
        },
        Err(message) => TestBucketResp::failed("local", Stage::Bucket, message),
    }
}

/// Where a string holds a `${…}` template, as dotted paths.
fn template_paths(value: &Value, prefix: &str) -> Vec<String> {
    match value {
        Value::String(s) if s.contains("${") && s.contains('}') => vec![prefix.to_string()],
        Value::Object(map) => map
            .iter()
            .flat_map(|(k, v)| {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                template_paths(v, &path)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn at<'a>(value: Option<&'a Value>, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value?, |v, k| v.get(k))
}

/// Replace every template the candidate kept from the stored entry with its
/// expanded stored value; returns the fields replaced.
fn expand_stored_templates(
    candidate: &mut Value,
    raw: Option<&Value>,
    expanded: Option<&Value>,
) -> Vec<String> {
    let mut replaced = Vec::new();
    for path in template_paths(candidate, "") {
        let kept = at(Some(candidate), &path) == at(raw, &path);
        let Some(value) = at(expanded, &path).filter(|v| v.is_string()).cloned() else {
            continue;
        };
        if !kept {
            continue;
        }
        let slot = path
            .split('.')
            .try_fold(&mut *candidate, |v, k| v.get_mut(k));
        if let Some(slot) = slot {
            *slot = value;
            replaced.push(path);
        }
    }
    replaced
}

/// Credential fields bound to where they are sent (an access key id and an
/// account id are identifiers, sent in clear anyway).
const BOUND_FIELDS: [&str; 3] = ["secret_access_key", "session_token", "credentials_json"];

/// Where a bucket's credentials are sent: its endpoint (scheme, host and
/// port, lowercased), the R2 account's endpoint, or the provider's default.
fn destination(bucket: &BucketConfig) -> String {
    let normalize = |raw: &str| match url::Url::parse(raw) {
        Ok(url) => format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str().unwrap_or("").to_ascii_lowercase(),
            url.port_or_known_default().unwrap_or(0)
        ),
        Err(_) => raw.trim().to_ascii_lowercase(),
    };
    match bucket {
        BucketConfig::S3(v) => v
            .endpoint_url
            .as_deref()
            .map(normalize)
            .unwrap_or_else(|| "aws".into()),
        BucketConfig::R2(v) => v
            .endpoint_url
            .as_deref()
            .map(normalize)
            .unwrap_or_else(|| format!("r2-account:{}", v.account_id.trim())),
        BucketConfig::Gcs(v) => v
            .endpoint_url
            .as_deref()
            .map(normalize)
            .unwrap_or_else(|| "google".into()),
        BucketConfig::Local(_) => "local".into(),
    }
}

/// `"__redacted__"` values restored from bucket `name` may only be sent to
/// that bucket's endpoint (same provider, same scheme, host and port).
fn restored_targets_match(
    candidate: &BucketConfig,
    stored: &BucketConfig,
    name: &str,
    restored: &[String],
) -> Result<(), String> {
    for field in restored {
        if !BOUND_FIELDS.contains(&field.as_str()) {
            continue;
        }
        if candidate.provider() != stored.provider()
            || destination(candidate) != destination(stored)
        {
            return Err(format!(
                "`{field}` keeps bucket `{name}`'s stored value (\"__redacted__\" or its ${{VAR}} \
                 template), but this test targets a different endpoint or account; re-enter the \
                 value to test against a different endpoint"
            ));
        }
    }
    Ok(())
}

/// A `secret://` / `env://` reference a configured bucket already uses in a
/// bound field may only be sent to that bucket's endpoint. References no
/// configured bucket uses (a new bucket's fresh secret) are free.
fn references_stay_bound(candidate: &BucketConfig, live: &WorkerConfig) -> Result<(), String> {
    for (field, value) in candidate.credential_fields() {
        if !BOUND_FIELDS.contains(&field) {
            continue;
        }
        let Some(Ok(reference)) = crate::secrets::parse_ref(value) else {
            continue;
        };
        let key = reference.key();
        let mut users: Vec<(&str, String, &str)> = Vec::new();
        for (name, bucket) in &live.buckets {
            for (stored_field, stored_value) in bucket.credential_fields() {
                if BOUND_FIELDS.contains(&stored_field)
                    && crate::secrets::parse_ref(stored_value)
                        .and_then(Result::ok)
                        .is_some_and(|r| r.key() == key)
                {
                    users.push((name, destination(bucket), bucket.provider()));
                }
            }
        }
        if users.is_empty() {
            continue;
        }
        let target = destination(candidate);
        if !users
            .iter()
            .any(|(_, d, p)| *d == target && *p == candidate.provider())
        {
            let mut names: Vec<&str> = users.iter().map(|(n, _, _)| *n).collect();
            names.sort_unstable();
            names.dedup();
            return Err(format!(
                "`{field}`: {key} belongs to bucket `{}` and is only sent to that bucket's \
                 endpoint; enter the value itself to test against a different endpoint",
                names.join("`, `")
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::buckets::tests::fixture;
    use super::*;
    use serde_json::json;

    fn req(v: Value) -> TestBucketReq {
        serde_json::from_value(v).unwrap()
    }

    /// An S3-ish server that records every request line and answers 403.
    async fn recording_server() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = got.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let record = record.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16384];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    record
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf[..n]).to_string());
                    let body = "<?xml version=\"1.0\"?><Error><Code>InvalidAccessKeyId</Code><Message>nope</Message></Error>";
                    let _ = sock
                        .write_all(
                            format!(
                                "HTTP/1.1 403 Forbidden\r\ncontent-type: application/xml\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await;
                });
            }
        });
        (port, got)
    }

    #[tokio::test]
    async fn literals_unknown_keys_and_missing_values_fail_as_config() {
        let f = fixture(json!({ "buckets": {} })).await;
        let resp = handle(
            &f.state,
            &f.runtime,
            req(json!({ "bucket": {
            "provider": "s3", "region": "r", "bucket": "b", "secret_access_key": "hunter2" } })),
        )
        .await
        .unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.stage, Some(Stage::Config));
        assert!(resp
            .message
            .as_deref()
            .unwrap()
            .contains("literal credential"));
        let resp = handle(
            &f.state,
            &f.runtime,
            req(json!({ "bucket": { "provider": "s3", "region": "r", "bucket": "b", "nope": 1 } })),
        )
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config));
        let resp = handle(
            &f.state,
            &f.runtime,
            req(json!({ "bucket": { "provider": "s3", "region": "r" } })),
        )
        .await
        .unwrap();
        assert!(resp.message.unwrap().contains("`bucket.bucket`"));
        let resp = handle(&f.state, &f.runtime, req(json!({}))).await.unwrap();
        assert_eq!(resp.stage, Some(Stage::Config));
    }

    #[tokio::test]
    async fn an_unresolvable_reference_is_the_secrets_stage_and_the_cache_is_never_used() {
        let f = fixture(json!({ "buckets": {} })).await;
        let probe = || {
            req(json!({ "bucket": {
            "provider": "s3", "region": "us-east-1", "bucket": "b", "endpoint_url": "http://127.0.0.1:1",
            "access_key_id": "AKIA", "secret_access_key": "secret://NEW_SK" }, "timeout_ms": 3000 }))
        };
        let resp = handle(&f.state, &f.runtime, probe()).await.unwrap();
        assert_eq!(resp.stage, Some(Stage::Secrets), "{:?}", resp.message);
        assert!(resp.message.unwrap().contains("secret://NEW_SK"));
        // Stored a second later: the next test sees it (no cached failure).
        f.secrets.set("secret://NEW_SK", Ok("sk"));
        let resp = handle(&f.state, &f.runtime, probe()).await.unwrap();
        assert_eq!(resp.stage, Some(Stage::Endpoint), "{:?}", resp.message);
        assert_eq!(resp.direct_downloads, Some(true));
    }

    #[tokio::test]
    async fn rejected_credentials_are_auth_and_scrubbed() {
        let (port, got) = recording_server().await;
        let f = fixture(json!({ "buckets": {} })).await;
        let resp = handle(&f.state, &f.runtime, req(json!({
            "bucket": { "provider": "s3", "region": "us-east-1", "bucket": "b", "force_path_style": true,
                        "endpoint_url": format!("http://127.0.0.1:{port}") },
            "credentials": { "access_key_id": "AKIATEST", "secret_access_key": "sk-typed", "session_token": "tok-typed" },
            "timeout_ms": 5000 })))
        .await
        .unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.stage, Some(Stage::Auth), "{:?}", resp.message);
        assert!(resp
            .message
            .as_deref()
            .unwrap()
            .contains("InvalidAccessKeyId"));
        assert!(!resp.message.unwrap().contains("tok-typed"));
        assert_eq!(got.lock().unwrap().len(), 1, "one attempt, no retries");
    }

    #[tokio::test]
    async fn a_stored_buckets_secret_is_never_sent_to_another_endpoint() {
        let (port, got) = recording_server().await;
        let f = fixture(json!({ "buckets": { "prod": {
            "provider": "s3", "region": "us-east-1", "endpoint_url": "https://s3.prod.internal",
            "access_key_id": "AKIA", "secret_access_key": "secret://PROD_SK", "session_token": "prod-literal-token" } } }))
        .await;
        f.secrets.set("secret://PROD_SK", Ok("prod-secret"));
        let elsewhere = format!("http://127.0.0.1:{port}");
        // The stored reference, aimed elsewhere.
        let resp = handle(&f.state, &f.runtime, req(json!({ "bucket": {
            "provider": "s3", "region": "us-east-1", "bucket": "b", "endpoint_url": elsewhere,
            "access_key_id": "AKIA", "secret_access_key": "secret://PROD_SK" }, "timeout_ms": 3000 })))
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config));
        assert!(resp.message.unwrap().contains("belongs to bucket `prod`"));
        // The stored literal (redacted), aimed elsewhere.
        let resp = handle(&f.state, &f.runtime, req(json!({ "name": "prod", "bucket": {
            "provider": "s3", "region": "us-east-1", "endpoint_url": elsewhere,
            "access_key_id": "AKIA", "secret_access_key": "secret://PROD_SK", "session_token": "__redacted__" }, "timeout_ms": 3000 })))
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config));
        assert!(
            got.lock().unwrap().is_empty(),
            "nothing reached the other endpoint"
        );
        // The same endpoint is fine (it fails to connect, but it is tried).
        let resp = handle(&f.state, &f.runtime, req(json!({ "name": "prod", "bucket": {
            "provider": "s3", "region": "us-east-1", "endpoint_url": "https://S3.prod.internal:443",
            "access_key_id": "AKIA", "secret_access_key": "secret://PROD_SK", "session_token": "__redacted__" }, "timeout_ms": 3000 })))
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Endpoint), "{:?}", resp.message);
        // A downgrade to http is another endpoint.
        let stored: BucketConfig = serde_json::from_value(
            json!({ "provider": "s3", "region": "r", "endpoint_url": "https://h" }),
        )
        .unwrap();
        let http: BucketConfig = serde_json::from_value(
            json!({ "provider": "s3", "region": "r", "endpoint_url": "http://h" }),
        )
        .unwrap();
        assert!(restored_targets_match(&http, &stored, "x", &["session_token".into()]).is_err());
        assert!(restored_targets_match(&stored, &stored, "x", &["session_token".into()]).is_ok());
    }

    #[tokio::test]
    async fn a_local_bucket_checks_its_directory_and_a_stored_bucket_tests_by_name() {
        let f = fixture(json!({ "buckets": { "scratch": { "provider": "local" } } })).await;
        let resp = handle(&f.state, &f.runtime, req(json!({ "name": "scratch" })))
            .await
            .unwrap();
        assert!(resp.ok, "{:?}", resp.message);
        assert_eq!(resp.provider, "local");
        let resp = handle(
            &f.state,
            &f.runtime,
            req(json!({ "bucket": { "provider": "local", "bucket": "fresh" } })),
        )
        .await
        .unwrap();
        assert!(resp.ok, "{:?}", resp.message);
        let resp = handle(&f.state, &f.runtime, req(json!({ "name": "nope" })))
            .await
            .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config));
    }

    #[test]
    fn kept_templates_take_the_stored_expanded_values() {
        let raw = json!({ "provider": "s3", "access_key_id": "${AK}", "secret_access_key": "${SK}", "region": "r" });
        let expanded = json!({ "provider": "s3", "access_key_id": "ak-value", "secret_access_key": "sk-value", "region": "r" });
        let mut candidate = json!({ "provider": "s3", "access_key_id": "${AK}", "secret_access_key": "${OTHER}", "region": "r" });
        let replaced = expand_stored_templates(&mut candidate, Some(&raw), Some(&expanded));
        assert_eq!(replaced, vec!["access_key_id"]);
        assert_eq!(candidate["access_key_id"], "ak-value");
        assert_eq!(template_paths(&candidate, ""), vec!["secret_access_key"]);
    }

    #[tokio::test]
    async fn a_new_template_cannot_be_tested_before_it_is_saved() {
        let f = fixture(json!({ "buckets": {} })).await;
        let resp = handle(
            &f.state,
            &f.runtime,
            req(json!({ "bucket": {
            "provider": "s3", "region": "r", "bucket": "b", "endpoint_url": "http://127.0.0.1:1",
            "access_key_id": "${AK}", "secret_access_key": "env://SK" } })),
        )
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config));
        assert!(resp
            .message
            .unwrap()
            .contains("`bucket.access_key_id` is a ${VAR} template"));
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_the_endpoint_stage_within_the_budget() {
        let f = fixture(json!({ "buckets": {} })).await;
        let started = Instant::now();
        let resp = handle(&f.state, &f.runtime, req(json!({ "bucket": {
            "provider": "r2", "account_id": "acc", "bucket": "b", "endpoint_url": "http://127.0.0.1:1" },
            "credentials": { "access_key_id": "a", "secret_access_key": "s" }, "timeout_ms": 4000 })))
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Endpoint), "{:?}", resp.message);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
