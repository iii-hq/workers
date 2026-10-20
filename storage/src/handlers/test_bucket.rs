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

use super::buckets::{prepare_candidate, stored_buckets_both, Candidate};
use super::AppState;
use crate::backend::factory::{self, Purpose};
use crate::binding;
use crate::config::{parse_bucket, BucketConfig, WorkerConfig};
use crate::configuration::StorageRuntime;
use crate::material::{self, BucketError, Material, Stage};
use crate::redaction::{at, template_paths};

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
    if req.bucket.is_none() && req.credentials.as_ref().is_some_and(|c| !c.is_null()) {
        return TestBucketResp::failed(
            "unknown",
            Stage::Config,
            "`credentials` need a `bucket` to test; to test a stored bucket as it is, send only \
             its `name`",
        );
    }
    // Every stored entry, as written and as the configuration worker expands
    // it (parsed or not: an entry that does not parse still binds its
    // references).
    let (raw_buckets, expanded_buckets) = stored_buckets_both(runtime).await;
    let stored = req.name.as_ref().and_then(|n| raw_buckets.get(n).cloned());
    let stored_expanded = req
        .name
        .as_ref()
        .and_then(|n| expanded_buckets.as_ref()?.get(n).cloned());
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

    // A stored entry tested as it is may hold legacy literals and templates:
    // they are not in a request, so only a request's `bucket` is held to the
    // rules.
    let (entry, kept) = if req.bucket.is_some() {
        match prepare_candidate(candidate, req.credentials.as_ref(), stored.as_ref()) {
            Ok(Candidate {
                mut entry,
                kept,
                templates,
            }) => {
                // A template kept from the stored entry takes the value the
                // configuration worker expanded for it.
                for path in &templates {
                    if let Some(value) =
                        at(stored_expanded.as_ref(), path).filter(|v| v.is_string())
                    {
                        set_at(&mut entry, path, value.clone());
                    }
                }
                (entry, kept)
            }
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
    if let Some(path) = template_paths(&entry, "").first() {
        return TestBucketResp::failed(
            &provider_hint,
            Stage::Config,
            format!(
                "`bucket.{path}` is a ${{VAR}} template the configuration worker could not \
                 expand; fix the stored value, or use env://NAME"
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

    // Credentials only go where they belong (see `crate::binding`): kept
    // values where the stored entry sends them, bound references and
    // credentials files where their stored buckets send them — this bucket's
    // own included, so testing a moved endpoint needs the values re-entered.
    if req.bucket.is_some() {
        if let (Some(name), false) = (&req.name, kept.is_empty()) {
            let Some(stored) = stored_expanded.as_ref().or(stored.as_ref()) else {
                return TestBucketResp::failed(provider, Stage::Config, "nothing stored to keep");
            };
            if let Err(e) = binding::check_kept(&entry, stored, name, &kept) {
                return TestBucketResp::failed(provider, Stage::Config, e);
            }
        }
        let all = expanded_buckets.as_ref().unwrap_or(&raw_buckets);
        if let Err(e) = binding::check_bound(&entry, &binding::uses(all, None)) {
            return TestBucketResp::failed(provider, Stage::Config, e);
        }
    }

    let snapshot = state.catalog.read().await.config.clone();
    match tokio::time::timeout(budget, probe(state, &label, &bucket, &snapshot)).await {
        Ok(resp) => resp,
        Err(_) => TestBucketResp::failed(
            provider,
            Stage::Endpoint,
            format!("timed out after {}ms", budget.as_millis()),
        ),
    }
}

/// Set the value at a dotted path that exists.
fn set_at(entry: &mut Value, path: &str, value: Value) {
    if let Some(slot) = path.split('.').try_fold(entry, |v, k| v.get_mut(k)) {
        *slot = value;
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

    #[tokio::test]
    async fn kept_templates_take_the_stored_expanded_values_and_stay_with_their_endpoint() {
        let f = fixture(json!({ "buckets": { "tpl": {
            "provider": "s3", "region": "r", "endpoint_url": "http://127.0.0.1:1",
            "access_key_id": "${AK}", "secret_access_key": "${SK}" } } }))
        .await;
        let mut expanded = f.store.value.lock().unwrap().clone();
        expanded["buckets"]["tpl"]["access_key_id"] = json!("ak-value");
        expanded["buckets"]["tpl"]["secret_access_key"] = json!("sk-value");
        *f.store.expanded.lock().unwrap() = Some(expanded);
        let entry = |endpoint: &str, sk: &str| {
            json!({ "name": "tpl", "timeout_ms": 3000, "bucket": {
                "provider": "s3", "region": "r", "endpoint_url": endpoint,
                "access_key_id": "${AK}", "secret_access_key": sk } })
        };
        // Kept: expanded, and the probe goes out (and fails to connect).
        let resp = handle(
            &f.state,
            &f.runtime,
            req(entry("http://127.0.0.1:1", "${SK}")),
        )
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Endpoint), "{:?}", resp.message);
        // Changed: refused.
        let resp = handle(
            &f.state,
            &f.runtime,
            req(entry("http://127.0.0.1:1", "${HOME}")),
        )
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config));
        assert!(resp.message.unwrap().contains("env://NAME"));
        // Kept, toward another endpoint: refused, nothing sent.
        let (port, got) = recording_server().await;
        let resp = handle(
            &f.state,
            &f.runtime,
            req(entry(&format!("http://127.0.0.1:{port}"), "${SK}")),
        )
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config), "{:?}", resp.message);
        assert!(resp
            .message
            .unwrap()
            .contains("keeps bucket `tpl`'s stored value"));
        assert!(got.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn any_field_binds_a_reference_and_broken_entries_and_files_bind_too() {
        let (port, got) = recording_server().await;
        let evil = format!("http://127.0.0.1:{port}");
        let dir = tempfile::tempdir().unwrap();
        let sa = dir.path().join("sa.json");
        std::fs::write(&sa, b"{}").unwrap();
        let f = fixture(json!({ "buckets": {
            "prod": { "provider": "s3", "region": "r", "endpoint_url": "https://s3.prod.internal",
                      "access_key_id": "AKIA", "secret_access_key": "secret://PROD_SK" },
            "broken": { "provider": "s3", "endpoint_url": "ftp://x", "secret_access_key": "secret://BROKEN_SK" },
            "docs": { "provider": "gcs", "credentials_file": sa.to_string_lossy() } } }))
        .await;
        f.secrets.set("secret://PROD_SK", Ok("prod-secret"));
        f.secrets.set("secret://BROKEN_SK", Ok("broken-secret"));
        let s3 = |field: &str, value: &str| {
            let mut b = json!({ "provider": "s3", "region": "r", "bucket": "b", "endpoint_url": evil,
                                "access_key_id": "a" });
            b[field] = json!(value);
            req(
                json!({ "bucket": b, "credentials": { "secret_access_key": "typed" }, "timeout_ms": 3000 }),
            )
        };
        for (field, value) in [
            ("access_key_id", "secret://PROD_SK"),
            ("session_token", "secret://PROD_SK"),
            ("session_token", "secret://BROKEN_SK"),
        ] {
            let resp = handle(&f.state, &f.runtime, s3(field, value))
                .await
                .unwrap();
            assert_eq!(
                resp.stage,
                Some(Stage::Config),
                "{field}: {:?}",
                resp.message
            );
        }
        let other_spelling = dir.path().join(".").join("sa.json");
        let resp = handle(
            &f.state,
            &f.runtime,
            req(json!({ "bucket": {
            "provider": "gcs", "bucket": "docs", "endpoint_url": evil,
            "credentials_file": other_spelling.to_string_lossy() }, "timeout_ms": 3000 })),
        )
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config), "{:?}", resp.message);
        // The host's own credentials do not go to an unknown endpoint either.
        let resp = handle(&f.state, &f.runtime, req(json!({ "bucket": {
            "provider": "s3", "region": "r", "bucket": "b", "endpoint_url": evil }, "timeout_ms": 3000 })))
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config), "{:?}", resp.message);
        assert!(
            got.lock().unwrap().is_empty(),
            "nothing reached the other endpoint"
        );
    }

    #[tokio::test]
    async fn credentials_without_a_bucket_and_unknown_keys_are_refused() {
        let f = fixture(json!({ "buckets": { "scratch": { "provider": "local" } } })).await;
        let resp = handle(
            &f.state,
            &f.runtime,
            req(json!({ "name": "scratch", "credentials": { "secret_access_key": "x" } })),
        )
        .await
        .unwrap();
        assert_eq!(resp.stage, Some(Stage::Config));
        assert!(resp.message.unwrap().contains("need a `bucket`"));
        let err = crate::handlers::parse_request::<TestBucketReq>(
            json!({ "name": "scratch", "timeout": 5 }),
        )
        .unwrap_err();
        assert!(
            err.contains("CONFIG_ERROR") && err.contains("unknown field `timeout`"),
            "{err}"
        );
        let err = crate::handlers::parse_request::<TestBucketReq>(json!({ "timeout_ms": "5" }))
            .unwrap_err();
        assert!(err.contains("`timeout_ms`"), "{err}");
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
