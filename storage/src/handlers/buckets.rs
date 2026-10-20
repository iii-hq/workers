//! `storage::getBucket` / `saveBucket` / `deleteBucket` — the bucket
//! editor's read and write path.
//!
//! Writes are a read-modify-write of the WHOLE storage configuration value
//! through the configuration worker, read with `raw: true` so `providers`,
//! the other buckets, their `${VAR}` templates and every `secret://`
//! reference are written back exactly as they were. Saves are serialized
//! inside the worker; the response comes after the runtime reloaded,
//! carrying the saved bucket's build error when it has one (the entry is
//! saved either way: an unreachable provider is not a reason to refuse it).
//!
//! Literal sensitive values never leave the worker: `getBucket` returns them
//! as `"__redacted__"`, and a save keeps the stored value wherever the
//! request still holds the placeholder (see `crate::redaction`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{err_to_str, AppState};
use crate::backend::BackendError;
use crate::config::{
    entry_keys, parse_bucket, unknown_entry_keys, validate_bucket_name, BucketConfig,
};
use crate::configuration::{ConfigStore, StorageRuntime};
use crate::error::StorageError;
use crate::material::{self, Stage};
use crate::redaction;
use crate::secrets::Refetch;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBucketReq {
    /// Bucket name (the key under `buckets`).
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetBucketResp {
    pub name: String,
    /// The entry as stored: references and `${VAR}` templates intact,
    /// literal secret keys, session tokens, credentials JSON and API tokens
    /// as `"__redacted__"`.
    pub bucket: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveBucketReq {
    /// The name to save under: `[a-z0-9][a-z0-9_-]{0,62}`.
    pub name: String,
    /// The bucket's current name when editing an existing one (equal to
    /// `name` to update in place, different to rename). Absent creates a new
    /// bucket, refused when `name` is taken.
    #[serde(default)]
    pub previous_name: Option<String>,
    /// The entry (`provider` and its fields). `"__redacted__"` keeps the
    /// stored value of that field. Sensitive fields hold references here;
    /// literals go in `credentials`.
    pub bucket: Value,
    /// Sensitive values sent apart from `bucket` and merged over it:
    /// `access_key_id`, `secret_access_key`, `session_token`,
    /// `credentials_json`, `notifications_api_token`. Prefer `secret://`
    /// references in `bucket`, which are safe anywhere.
    #[serde(default)]
    pub credentials: Option<Value>,
    /// s3 / r2: create the provider-side bucket when it does not exist.
    /// Local buckets always ensure their storage.
    #[serde(default)]
    pub create_bucket: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SaveBucketResp {
    pub name: String,
    pub saved: bool,
    /// With `create_bucket`: whether this save created the provider-side
    /// bucket (false: it already existed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_bucket: Option<bool>,
    /// Why the saved bucket has no backend after the reload, or why its
    /// creation failed (it is saved regardless). Absent when it is usable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_stage: Option<Stage>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteBucketReq {
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteBucketResp {
    pub deleted: bool,
}

pub(crate) fn config_error(message: impl Into<String>) -> String {
    err_to_str(StorageError::ConfigError {
        message: message.into(),
    })
}

fn store(runtime: &StorageRuntime) -> Result<std::sync::Arc<dyn ConfigStore>, String> {
    runtime.store().cloned().ok_or_else(|| {
        config_error("no engine connection; the configuration worker is unreachable")
    })
}

/// The stored `buckets` map (raw), or an empty one.
fn stored_buckets(raw: &Value) -> Result<Map<String, Value>, String> {
    match raw.get("buckets") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Err("the stored `buckets` is not an object".into()),
    }
}

/// One stored entry as written (raw), if it exists.
pub(crate) async fn stored_entry(
    runtime: &StorageRuntime,
    name: &str,
) -> Result<Option<Value>, String> {
    let raw = store(runtime)?
        .get(true)
        .await
        .map_err(|e| config_error(format!("reading the stored configuration failed: {e}")))?;
    Ok(stored_buckets(&raw)
        .map_err(config_error)?
        .get(name)
        .cloned())
}

/// One stored entry as written (raw) and as applied (`${VAR}` templates
/// expanded by the configuration worker; `None` when the value cannot be
/// expanded).
pub(crate) async fn stored_entry_both(
    runtime: &StorageRuntime,
    name: &str,
) -> Result<(Option<Value>, Option<Value>), String> {
    let store = store(runtime)?;
    let raw = store
        .get(true)
        .await
        .map_err(|e| config_error(format!("reading the stored configuration failed: {e}")))?;
    let raw_entry = stored_buckets(&raw)
        .map_err(config_error)?
        .get(name)
        .cloned();
    if raw_entry.is_none() {
        return Ok((None, None));
    }
    let expanded = store
        .get(false)
        .await
        .ok()
        .and_then(|value| stored_buckets(&value).ok())
        .and_then(|buckets| buckets.get(name).cloned());
    Ok((raw_entry, expanded))
}

/// Refuse keys that are not fields of the entry's provider, naming them
/// (never their values).
pub(crate) fn refuse_unknown_keys(entry: &Value, prefix: &str) -> Result<(), String> {
    let Some(obj) = entry.as_object() else {
        return Err(format!("`{prefix}` must be an object"));
    };
    let provider = obj.get("provider").and_then(Value::as_str).unwrap_or("");
    if entry_keys(provider).is_none() {
        return Err(format!(
            "`{prefix}.provider` must be one of s3, r2, gcs, local"
        ));
    }
    let unknown = unknown_entry_keys(entry);
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "`{prefix}.{}` is not a {provider} bucket field (fields are case-sensitive; see the \
         storage configuration schema)",
        unknown.join(&format!("`, `{prefix}."))
    ))
}

/// A candidate entry made usable: unknown keys and literal credentials
/// refused, `credentials` merged, `"__redacted__"` restored from `stored`.
/// Returns the entry and the fields restored.
pub(crate) fn prepare_candidate(
    mut entry: Value,
    credentials: Option<&Value>,
    stored: Option<&Value>,
) -> Result<(Value, Vec<String>), String> {
    refuse_unknown_keys(&entry, "bucket")?;
    if let Some(field) = redaction::literal_sensitive_fields(&entry).first() {
        return Err(redaction::literal_refusal(field));
    }
    redaction::merge_credentials(&mut entry, credentials)?;
    // A credential the provider has no field for (`session_token` on r2).
    let provider = entry["provider"].as_str().unwrap_or("").to_string();
    if let Some(key) = unknown_entry_keys(&entry).first() {
        return Err(format!(
            "`credentials` sets `{key}`, which is not a {provider} bucket field"
        ));
    }
    let restored = redaction::restore_entry(&mut entry, stored)?;
    Ok((entry, restored))
}

/// What a save writes: the whole new configuration value and the parsed
/// entry.
#[derive(Debug)]
pub(crate) struct SavePlan {
    pub value: Value,
    pub bucket: BucketConfig,
}

/// Validate a save against the stored raw value and build the value to
/// write. Pure: every refusal happens here, before anything is written.
pub(crate) fn plan_save(raw: Value, req: &SaveBucketReq) -> Result<SavePlan, String> {
    validate_bucket_name(&req.name)?;
    let mut raw = if raw.is_null() {
        Value::Object(Map::new())
    } else {
        raw
    };
    if !raw.is_object() {
        return Err("the stored configuration is not an object".into());
    }
    let mut buckets = stored_buckets(&raw)?;

    let stored = match &req.previous_name {
        Some(previous) => {
            let Some(entry) = buckets.get(previous) else {
                return Err(format!(
                    "no bucket named `{previous}` to update; omit previous_name to create one"
                ));
            };
            if previous != &req.name && buckets.contains_key(&req.name) {
                return Err(format!(
                    "a bucket named `{}` already exists; pick another name",
                    req.name
                ));
            }
            Some(entry.clone())
        }
        None => {
            if buckets.contains_key(&req.name) {
                return Err(format!(
                    "a bucket named `{0}` already exists; pass previous_name: \"{0}\" to update it",
                    req.name
                ));
            }
            None
        }
    };

    let (mut entry, _) = prepare_candidate(
        req.bucket.clone(),
        req.credentials.as_ref(),
        stored.as_ref(),
    )?;

    // A rename keeps pointing at the same provider-side bucket: the
    // underlying bucket defaults to the name, so pin the old one.
    if let (Some(previous), Some(stored)) = (&req.previous_name, &stored) {
        let unnamed = entry
            .get("bucket")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty);
        let same_provider = stored.get("provider") == entry.get("provider");
        if previous != &req.name && unnamed && same_provider {
            let underlying = stored
                .get("bucket")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .unwrap_or(previous)
                .to_string();
            entry["bucket"] = Value::String(underlying);
        }
    }

    let bucket = parse_bucket(&req.name, &entry)?;
    if let Some(display) = bucket.display() {
        display.validate()?;
    }
    if req.create_bucket == Some(true) && matches!(bucket, BucketConfig::Gcs(_)) {
        return Err(
            "create_bucket is supported for s3, r2 and S3-compatible buckets; create the GCS \
             bucket in Google Cloud"
                .into(),
        );
    }

    if let Some(previous) = &req.previous_name {
        if previous != &req.name {
            buckets.remove(previous);
        }
    }
    buckets.insert(req.name.clone(), entry);
    raw["buckets"] = Value::Object(buckets);
    Ok(SavePlan { value: raw, bucket })
}

/// The value without bucket `name`; `None` when there is no such bucket.
pub(crate) fn plan_delete(raw: Value, name: &str) -> Result<Option<Value>, String> {
    if raw.is_null() {
        return Ok(None);
    }
    let mut raw = raw;
    let mut buckets = stored_buckets(&raw)?;
    if buckets.remove(name).is_none() {
        return Ok(None);
    }
    match raw.as_object_mut() {
        Some(obj) => {
            obj.insert("buckets".into(), Value::Object(buckets));
        }
        None => return Err("the stored configuration is not an object".into()),
    }
    Ok(Some(raw))
}

/// Reads the stored value (not the live snapshot), so an entry that failed
/// to parse can still be opened and fixed.
pub async fn get(
    _state: &AppState,
    runtime: &StorageRuntime,
    req: GetBucketReq,
) -> Result<GetBucketResp, String> {
    let Some(mut entry) = stored_entry(runtime, &req.name).await? else {
        return Err(err_to_str(StorageError::UnknownBucket { bucket: req.name }));
    };
    redaction::redact_entry(&mut entry);
    Ok(GetBucketResp {
        name: req.name,
        bucket: entry,
    })
}

pub async fn save(
    state: &AppState,
    runtime: &StorageRuntime,
    req: SaveBucketReq,
) -> Result<SaveBucketResp, String> {
    let store = store(runtime)?;
    let _saving = runtime.saves().lock().await;

    let raw = store
        .get(true)
        .await
        .map_err(|e| config_error(format!("reading the stored configuration failed: {e}")))?;
    let plan = plan_save(raw, &req).map_err(config_error)?;
    store
        .set(plan.value)
        .await
        .map_err(|e| config_error(format!("configuration::set refused the value: {e}")))?;

    // The saved entry's references are read again now: saving right after
    // correcting a secret must not build the bucket from a cached value.
    state
        .secrets
        .refresh(material::references(&plan.bucket), Refetch::Invalidate)
        .await;
    let (mut error, mut error_stage) = reload(state, runtime, &req.name).await;

    let mut created_bucket = None;
    if req.create_bucket == Some(true)
        && matches!(plan.bucket, BucketConfig::S3(_) | BucketConfig::R2(_))
        && error.is_none()
    {
        match state.backend(&req.name).await {
            Ok(backend) => match backend.ensure_bucket().await {
                Ok(created) => created_bucket = Some(created),
                Err(e) => {
                    let (stage, message) = creation_failure(e);
                    error = Some(format!("saved, but creating the bucket failed: {message}"));
                    error_stage = Some(stage);
                }
            },
            Err(e) => {
                error = Some(e.to_string());
                error_stage = Some(Stage::Config);
            }
        }
    }
    Ok(SaveBucketResp {
        name: req.name,
        saved: true,
        created_bucket,
        error,
        error_stage,
    })
}

fn creation_failure(error: BackendError) -> (Stage, String) {
    match error {
        BackendError::AuthFailed(message) => (Stage::Auth, message),
        BackendError::Provider {
            inner_code: None,
            message,
        } => (Stage::Endpoint, message),
        other => (Stage::Bucket, other.to_string()),
    }
}

pub async fn delete(
    state: &AppState,
    runtime: &StorageRuntime,
    req: DeleteBucketReq,
) -> Result<DeleteBucketResp, String> {
    let store = store(runtime)?;
    let _saving = runtime.saves().lock().await;
    let raw = store
        .get(true)
        .await
        .map_err(|e| config_error(format!("reading the stored configuration failed: {e}")))?;
    let Some(value) = plan_delete(raw, &req.name).map_err(config_error)? else {
        return Ok(DeleteBucketResp { deleted: false });
    };
    store
        .set(value)
        .await
        .map_err(|e| config_error(format!("configuration::set refused the value: {e}")))?;
    let _ = reload(state, runtime, &req.name).await;
    Ok(DeleteBucketResp { deleted: true })
}

/// Apply the just-written configuration now (the change trigger that follows
/// finds nothing left to rebuild) and report `name`'s build error, if any.
async fn reload(
    state: &AppState,
    runtime: &StorageRuntime,
    name: &str,
) -> (Option<String>, Option<Stage>) {
    match runtime.reload(state).await {
        Ok(()) => match state.catalog.read().await.errors.get(name) {
            Some(e) => (Some(e.message.clone()), Some(e.stage)),
            None => (None, None),
        },
        Err(e) => (
            Some(format!(
                "saved, but the configuration could not be applied: {e}"
            )),
            Some(Stage::Config),
        ),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::secrets::testing::FakeSecrets;
    use crate::triggers::dispatcher::EngineDispatcher;
    use crate::triggers::registry::TriggerRegistry;
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use tokio::sync::RwLock;

    /// An in-memory configuration worker: raw and expanded reads are the
    /// same value; every set is recorded.
    #[derive(Default)]
    pub(crate) struct MemoryStore {
        pub value: Mutex<Value>,
        pub sets: Mutex<Vec<Value>>,
    }

    #[async_trait]
    impl ConfigStore for MemoryStore {
        async fn get(&self, _raw: bool) -> Result<Value, String> {
            Ok(self.value.lock().unwrap().clone())
        }
        async fn set(&self, value: Value) -> Result<(), String> {
            *self.value.lock().unwrap() = value.clone();
            self.sets.lock().unwrap().push(value);
            Ok(())
        }
    }

    pub(crate) struct Fixture {
        pub state: AppState,
        pub runtime: Arc<StorageRuntime>,
        pub store: Arc<MemoryStore>,
        pub secrets: FakeSecrets,
        pub _data: tempfile::TempDir,
    }

    /// A runtime over an in-memory store holding `value`, with its local
    /// data in a temp dir (`providers.local` is filled in when absent).
    pub(crate) async fn fixture(mut value: Value) -> Fixture {
        let data = tempfile::tempdir().unwrap();
        if value.get("providers").is_none() {
            value["providers"] = json!({
                "local": {
                    "data_dir": data.path().to_string_lossy(),
                    "http": { "bind_address": "127.0.0.1:0" }
                }
            });
        }
        let secrets = FakeSecrets::default();
        let state = AppState::with_secrets(HashMap::new(), Arc::new(secrets.cache()));
        let store = Arc::new(MemoryStore {
            value: Mutex::new(value),
            sets: Mutex::new(Vec::new()),
        });
        let runtime = StorageRuntime::with_store(
            Arc::new(EngineDispatcher::new(
                iii_sdk::IIIClient::new("ws://127.0.0.1:1"),
                Arc::new(TriggerRegistry::new()),
            )),
            &state,
            Arc::new(RwLock::new(HashSet::new())),
            Some(store.clone() as Arc<dyn ConfigStore>),
        );
        runtime.reload(&state).await.unwrap();
        Fixture {
            state,
            runtime,
            store,
            secrets,
            _data: data,
        }
    }

    fn save_req(v: Value) -> SaveBucketReq {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn create_update_and_rename_follow_previous_name() {
        let raw = json!({
            "providers": { "local": { "data_dir": "/x" } },
            "buckets": {
                "photos": { "provider": "local", "bucket": "photos" },
                "docs": { "provider": "s3", "region": "us-east-1", "secret_access_key": "${SK}" }
            }
        });
        // Create: a taken name is refused.
        let err = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "photos", "bucket": { "provider": "local" } })),
        )
        .unwrap_err();
        assert!(
            err.contains("already exists") && err.contains("previous_name"),
            "{err}"
        );
        let plan = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "videos", "bucket": { "provider": "local" } })),
        )
        .unwrap();
        assert!(plan.value["buckets"]["videos"].is_object());
        // Everything else is written back verbatim.
        assert_eq!(plan.value["providers"], raw["providers"]);
        assert_eq!(plan.value["buckets"]["docs"], raw["buckets"]["docs"]);
        // Update in place.
        let plan = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "photos", "previous_name": "photos",
                              "bucket": { "provider": "local", "bucket": "pics" } })),
        )
        .unwrap();
        assert_eq!(plan.value["buckets"]["photos"]["bucket"], "pics");
        // Rename onto a taken name is refused; onto a free one moves it.
        let err = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "docs", "previous_name": "photos", "bucket": { "provider": "local" } })),
        )
        .unwrap_err();
        assert!(err.contains("`docs` already exists"), "{err}");
        let plan = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "pictures", "previous_name": "photos", "bucket": { "provider": "local" } })),
        )
        .unwrap();
        assert!(plan.value["buckets"].get("photos").is_none());
        assert_eq!(
            plan.value["buckets"]["pictures"]["bucket"], "photos",
            "a rename keeps the provider-side bucket"
        );
        // Updating a bucket that is not there.
        let err = plan_save(
            raw,
            &save_req(
                json!({ "name": "x", "previous_name": "gone", "bucket": { "provider": "local" } }),
            ),
        )
        .unwrap_err();
        assert!(err.contains("no bucket named `gone`"), "{err}");
    }

    #[test]
    fn unknown_keys_literals_and_bad_values_are_refused_by_name() {
        let raw = json!({ "buckets": {} });
        let err = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "a", "bucket": { "provider": "s3", "region": "r", "Region": "x" } })),
        )
        .unwrap_err();
        assert!(
            err.contains("`bucket.Region` is not a s3 bucket field"),
            "{err}"
        );
        let err = plan_save(
            raw.clone(),
            &save_req(
                json!({ "name": "a", "bucket": { "provider": "local", "display": { "size": 3 } } }),
            ),
        )
        .unwrap_err();
        assert!(err.contains("`bucket.display.size`"), "{err}");
        let err = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "a", "bucket": { "provider": "ftp" } })),
        )
        .unwrap_err();
        assert!(err.contains("`bucket.provider` must be one of"), "{err}");
        let err = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "a", "bucket": {
                "provider": "s3", "region": "r", "access_key_id": "AKIA", "secret_access_key": "hunter2" } })),
        )
        .unwrap_err();
        assert!(
            err.contains("`bucket.secret_access_key` holds a literal credential"),
            "{err}"
        );
        assert!(!err.contains("hunter2"));
        // The same literal in `credentials` is accepted.
        let plan = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "a",
                "bucket": { "provider": "s3", "region": "r", "access_key_id": "AKIA" },
                "credentials": { "secret_access_key": "hunter2" } })),
        )
        .unwrap();
        assert_eq!(plan.value["buckets"]["a"]["secret_access_key"], "hunter2");
        // A credential the provider has no field for.
        let err = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "a",
                "bucket": { "provider": "r2", "account_id": "x", "access_key_id": "a", "secret_access_key": "secret://S" },
                "credentials": { "session_token": "t" } })),
        )
        .unwrap_err();
        assert!(
            err.contains("`session_token`, which is not a r2 bucket field"),
            "{err}"
        );
        // A bad name, a bad color, a missing required field, create on gcs.
        assert!(plan_save(
            raw.clone(),
            &save_req(json!({ "name": "Bad", "bucket": { "provider": "local" } }))
        )
        .is_err());
        let err = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "a", "bucket": { "provider": "local", "display": { "color": "blue" } } })),
        )
        .unwrap_err();
        assert!(err.contains("`display.color`"), "{err}");
        let err = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "a", "bucket": { "provider": "s3" } })),
        )
        .unwrap_err();
        assert!(err.contains("region"), "{err}");
        let err = plan_save(
            raw,
            &save_req(
                json!({ "name": "a", "create_bucket": true, "bucket": { "provider": "gcs" } }),
            ),
        )
        .unwrap_err();
        assert!(err.contains("create_bucket"), "{err}");
    }

    #[test]
    fn redacted_values_keep_the_stored_ones() {
        let raw = json!({ "buckets": { "a": {
            "provider": "r2", "account_id": "acc", "access_key_id": "ak", "secret_access_key": "stored-sk",
            "notifications": { "queue_id": "q", "api_token": "stored-tok" } } } });
        let plan = plan_save(
            raw.clone(),
            &save_req(json!({ "name": "a", "previous_name": "a", "bucket": {
                "provider": "r2", "account_id": "acc2", "access_key_id": "ak",
                "secret_access_key": "__redacted__",
                "notifications": { "queue_id": "q", "api_token": "__redacted__" } } })),
        )
        .unwrap();
        let saved = &plan.value["buckets"]["a"];
        assert_eq!(saved["secret_access_key"], "stored-sk");
        assert_eq!(saved["notifications"]["api_token"], "stored-tok");
        assert_eq!(saved["account_id"], "acc2");
        // A create has nothing stored to keep.
        let err = plan_save(
            raw,
            &save_req(json!({ "name": "b", "bucket": {
                "provider": "r2", "account_id": "acc", "access_key_id": "ak", "secret_access_key": "__redacted__" } })),
        )
        .unwrap_err();
        assert!(err.contains("no stored value to keep"), "{err}");
    }

    #[test]
    fn delete_removes_only_the_entry_and_zero_buckets_is_fine() {
        let raw =
            json!({ "providers": { "local": null }, "buckets": { "a": { "provider": "local" } } });
        let value = plan_delete(raw.clone(), "a").unwrap().unwrap();
        assert_eq!(value["buckets"], json!({}));
        assert_eq!(value["providers"], raw["providers"]);
        assert!(plan_delete(raw, "nope").unwrap().is_none());
        assert!(plan_delete(Value::Null, "a").unwrap().is_none());
    }

    #[tokio::test]
    async fn save_writes_reloads_and_reports_the_saved_buckets_error() {
        let f = fixture(json!({ "buckets": { "keep": { "provider": "local" } } })).await;
        // A bucket whose secret does not resolve is saved, and says why.
        let resp = save(
            &f.state,
            &f.runtime,
            save_req(json!({ "name": "cloud", "bucket": {
                "provider": "s3", "region": "us-east-1", "endpoint_url": "http://127.0.0.1:1",
                "access_key_id": "AKIA", "secret_access_key": "secret://STORAGE_CLOUD_SECRET_ACCESS_KEY" } })),
        )
        .await
        .unwrap();
        assert!(resp.saved);
        assert_eq!(resp.error_stage, Some(Stage::Secrets));
        assert!(resp
            .error
            .as_deref()
            .unwrap()
            .contains("secret://STORAGE_CLOUD_SECRET_ACCESS_KEY"));
        let err = f
            .state
            .backend("cloud")
            .await
            .err()
            .unwrap()
            .to_wire_string();
        assert!(
            err.contains("BUCKET_UNAVAILABLE") && err.contains("\"stage\":\"secrets\""),
            "{err}"
        );
        // The other bucket kept working the whole time.
        assert!(f.state.backend("keep").await.is_ok());
        // Storing the secret and saving again brings it up (fresh read).
        f.secrets
            .set("secret://STORAGE_CLOUD_SECRET_ACCESS_KEY", Ok("sk"));
        let resp = save(
            &f.state,
            &f.runtime,
            save_req(json!({ "name": "cloud", "previous_name": "cloud", "bucket": {
                "provider": "s3", "region": "us-east-1", "endpoint_url": "http://127.0.0.1:1",
                "access_key_id": "AKIA", "secret_access_key": "secret://STORAGE_CLOUD_SECRET_ACCESS_KEY" } })),
        )
        .await
        .unwrap();
        assert!(resp.error.is_none(), "{:?}", resp.error);
        assert_eq!(f.state.backend("cloud").await.unwrap().provider(), "s3");
        // Delete leaves the rest; the last bucket can go too.
        assert!(
            delete(
                &f.state,
                &f.runtime,
                DeleteBucketReq {
                    name: "cloud".into()
                }
            )
            .await
            .unwrap()
            .deleted
        );
        assert!(
            !delete(
                &f.state,
                &f.runtime,
                DeleteBucketReq {
                    name: "cloud".into()
                }
            )
            .await
            .unwrap()
            .deleted
        );
        assert!(
            delete(
                &f.state,
                &f.runtime,
                DeleteBucketReq {
                    name: "keep".into()
                }
            )
            .await
            .unwrap()
            .deleted
        );
        assert!(f.state.backends_snapshot().await.is_empty());
        assert!(f.store.value.lock().unwrap()["providers"]["local"].is_object());
    }

    #[tokio::test]
    async fn get_bucket_redacts_literals_and_keeps_references() {
        let f = fixture(json!({ "buckets": { "s": {
            "provider": "s3", "region": "us-east-1", "access_key_id": "AKIA",
            "secret_access_key": "hunter2", "session_token": "secret://TOK" } } }))
        .await;
        let resp = get(&f.state, &f.runtime, GetBucketReq { name: "s".into() })
            .await
            .unwrap();
        assert_eq!(resp.bucket["secret_access_key"], "__redacted__");
        assert_eq!(resp.bucket["session_token"], "secret://TOK");
        assert_eq!(resp.bucket["access_key_id"], "AKIA");
        let err = get(
            &f.state,
            &f.runtime,
            GetBucketReq {
                name: "nope".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("UNKNOWN_BUCKET"), "{err}");
    }

    #[tokio::test]
    async fn a_reload_keeps_unchanged_cloud_backends() {
        let f = fixture(json!({ "buckets": {
            "a": { "provider": "s3", "region": "us-east-1", "endpoint_url": "http://127.0.0.1:1" },
            "b": { "provider": "s3", "region": "us-east-1", "endpoint_url": "http://127.0.0.1:2" } } }))
        .await;
        let before_a = f.state.backend("a").await.unwrap();
        let before_b = f.state.backend("b").await.unwrap();
        save(
            &f.state,
            &f.runtime,
            save_req(json!({ "name": "b", "previous_name": "b", "bucket": {
                "provider": "s3", "region": "eu-west-1", "endpoint_url": "http://127.0.0.1:2" } })),
        )
        .await
        .unwrap();
        let after_a: Arc<dyn Backend> = f.state.backend("a").await.unwrap();
        let after_b: Arc<dyn Backend> = f.state.backend("b").await.unwrap();
        assert!(
            Arc::ptr_eq(&before_a, &after_a),
            "an untouched bucket keeps its client"
        );
        assert!(
            !Arc::ptr_eq(&before_b, &after_b),
            "the edited bucket is rebuilt"
        );
    }
}
