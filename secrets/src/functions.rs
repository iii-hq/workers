//! `secrets::*` registration and the bus-facing half of each handler.
//!
//! Payloads are parsed here rather than by the SDK so a malformed request is
//! answered with a fixed message: serde's own errors quote the offending
//! input, which for `value` (or a key pasted into `name`) is a credential.
use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::access::CallerDirectory;
use crate::api::{
    ids, AccessRequest, DeleteResponse, DetectRequest, DetectResponse, EmptyRequest, ImportRequest,
    ListResponse, NameRequest, ResolveRequest, ResolveResponse, SecretMeta, SetRequest,
    StatusResponse, StoreKind,
};
use crate::detect::{Detector, SourceKind};
use crate::envwatch::EnvWatch;
use crate::error::{codes, SecretsError};
use crate::events::{self, ChangedEvent, Subscribers};
use crate::names::{parse_reference, validate_name};
use crate::store::{validate_detect_names, Store};

/// Everything a handler needs.
pub struct Ctx {
    pub iii: Arc<IIIClient>,
    pub store: Arc<Store>,
    pub callers: CallerDirectory,
    pub subscribers: Subscribers,
    pub detector: Detector,
    /// Follows the env file the store reads; replaced when the configuration
    /// points it elsewhere ([`crate::envwatch::follow`]).
    pub env_watch: std::sync::Mutex<Option<EnvWatch>>,
}

impl Ctx {
    /// The detector, reading the env file the configuration names now.
    async fn detector(&self) -> Detector {
        Detector {
            dotenv: self.store.dotenv_path().await,
            ..self.detector.clone()
        }
    }

    /// Log and deliver one `secrets::changed` event.
    pub fn changed(&self, event: &ChangedEvent) {
        tracing::info!(name = event.name, action = ?event.action, "secret changed");
        events::emit(&self.iii, &self.subscribers, event);
    }
}

/// Register the `secrets::changed` trigger type and every function.
pub fn register(ctx: &Arc<Ctx>) {
    events::register_trigger_type(&ctx.iii, &ctx.subscribers);
    register_fn::<SetRequest, SecretMeta, _, _>(
        ctx,
        ids::SET,
        "Store a credential under NAME (create, or rotate an existing one) and return its metadata. store \"vault\" (default) encrypts it, referenced as secret://NAME; store \"env\" writes NAME=value to the project's .env, referenced as env://NAME. The value is never returned by this or any metadata function. Omitted consumers keep the current allowlist ([] for a new secret).",
        set,
    );
    register_fn::<AccessRequest, SecretMeta, _, _>(
        ctx,
        ids::ACCESS,
        "Replace the list of worker names allowed to resolve a secret; [] revokes everyone. With store \"env\", shares the environment variable NAME (env://NAME), whether or not it is set yet.",
        access,
    );
    register_fn::<NameRequest, DeleteResponse, _, _>(
        ctx,
        ids::DELETE,
        "Delete a secret and revoke every consumer's access to it. With store \"env\", stops sharing the variable; its value stays in .env.",
        delete,
    );
    register_fn::<NameRequest, Option<SecretMeta>, _, _>(
        ctx,
        ids::GET,
        "One secret's metadata (masked hint, fingerprint, consumers, timestamps; for store \"env\", where the variable is read from), or null. Never the value.",
        get,
    );
    register_fn::<EmptyRequest, ListResponse, _, _>(
        ctx,
        ids::LIST,
        "Every secret's metadata: vault secrets, then shared environment variables (masked hint, consumers, timestamps). Never a value.",
        list,
    );
    register_fn::<ResolveRequest, ResolveResponse, _, _>(
        ctx,
        ids::RESOLVE,
        "Return the value behind secret://NAME (the vault) or env://NAME (the project's .env, else this worker's environment) to a worker listed in its consumers; anyone else gets SECRET_FORBIDDEN. The caller is identified by the engine, not by the request.",
        resolve,
    );
    register_fn::<DetectRequest, DetectResponse, _, _>(
        ctx,
        ids::DETECT,
        "Look for existing values of the given env var names in this worker's environment, the project's .env and the user's login shell. Reports masked hints and whether each equals the stored value; never a value.",
        detect,
    );
    register_fn::<ImportRequest, SecretMeta, _, _>(
        ctx,
        ids::IMPORT,
        "Store a value found by secrets::detect: the worker re-reads it from the named source itself, so it never passes through the caller. With store \"env\", a value found in .env or this worker's environment is shared as it is, and one from the login shell is copied into .env.",
        import,
    );
    register_fn::<EmptyRequest, StatusResponse, _, _>(
        ctx,
        ids::STATUS,
        "Where the vault lives, where the master key comes from (III_SECRETS_KEY or a key file outside the project), how many secrets are stored, and the .env the env store uses.",
        status,
    );
}

fn register_fn<Req, Resp, F, Fut>(
    ctx: &Arc<Ctx>,
    id: &'static str,
    description: &'static str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Ctx>, Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Resp, SecretsError>> + Send + 'static,
{
    let shared = ctx.clone();
    let handler = Arc::new(handler);
    let registration = RegisterFunction::new_async(move |payload: Value| {
        let ctx = shared.clone();
        let handler = handler.clone();
        async move {
            let request = parse_request::<Req>(payload)?;
            match handler(ctx, request).await {
                Ok(response) => Ok::<Resp, Error>(response),
                Err(error) => {
                    tracing::debug!(function = id, code = error.code, "request refused");
                    Err(error.into())
                }
            }
        }
    })
    .description(description)
    .request_format(schema::<Req>())
    .response_format(schema::<Resp>());
    ctx.iii.register_function(id, registration);
}

pub fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

/// Deserialize without quoting the input back: only a missing field is
/// named, every other mismatch gets the same fixed message.
pub fn parse_request<T: DeserializeOwned>(payload: Value) -> Result<T, SecretsError> {
    let payload = match payload {
        Value::Null => Value::Object(Default::default()),
        payload => payload,
    };
    serde_json::from_value(payload).map_err(|error| {
        let text = error.to_string();
        let missing = text
            .strip_prefix("missing field `")
            .and_then(|rest| rest.split('`').next())
            .filter(|field| field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        SecretsError::invalid_request(match missing {
            Some(field) => format!("missing field `{field}`"),
            None => "request does not match the function's request schema".to_owned(),
        })
    })
}

async fn set(ctx: Arc<Ctx>, request: SetRequest) -> Result<SecretMeta, SecretsError> {
    let (meta, event) = match request.store {
        StoreKind::Vault => {
            ctx.store
                .set(
                    &request.name,
                    &request.value,
                    request.consumers,
                    request.description,
                )
                .await?
        }
        StoreKind::Env => {
            ctx.store
                .set_env(
                    &request.name,
                    Some(&request.value),
                    request.consumers,
                    request.description,
                )
                .await?
        }
    };
    ctx.changed(&event);
    Ok(meta)
}

async fn access(ctx: Arc<Ctx>, request: AccessRequest) -> Result<SecretMeta, SecretsError> {
    let (meta, event) = match request.store {
        StoreKind::Vault => ctx.store.access(&request.name, request.consumers).await?,
        StoreKind::Env => {
            ctx.store
                .set_env(&request.name, None, Some(request.consumers), None)
                .await?
        }
    };
    ctx.changed(&event);
    Ok(meta)
}

async fn delete(ctx: Arc<Ctx>, request: NameRequest) -> Result<DeleteResponse, SecretsError> {
    let event = match request.store {
        StoreKind::Vault => ctx.store.delete(&request.name).await?,
        StoreKind::Env => ctx.store.delete_env(&request.name).await?,
    };
    if let Some(event) = &event {
        ctx.changed(event);
    }
    Ok(DeleteResponse {
        deleted: event.is_some(),
    })
}

async fn get(ctx: Arc<Ctx>, request: NameRequest) -> Result<Option<SecretMeta>, SecretsError> {
    match request.store {
        StoreKind::Vault => ctx.store.get(&request.name).await,
        StoreKind::Env => ctx.store.get_env(&request.name).await,
    }
}

async fn list(ctx: Arc<Ctx>, _: EmptyRequest) -> Result<ListResponse, SecretsError> {
    Ok(ListResponse {
        secrets: ctx.store.list_all().await?,
    })
}

async fn resolve(ctx: Arc<Ctx>, request: ResolveRequest) -> Result<ResolveResponse, SecretsError> {
    let (store, name) = parse_reference(&request.reference)?;
    // A bare NAME may be a raw credential a caller forgot to wrap: name it in
    // errors only once it is known to be a stored secret's name.
    let consumers = match store {
        StoreKind::Vault => ctx.store.consumers_of(name).await?.ok_or_else(|| {
            SecretsError::new(
                codes::SECRET_NOT_FOUND,
                "no secret is stored under that reference",
            )
        })?,
        StoreKind::Env => ctx.store.env_consumers_of(name).await?.unwrap_or_default(),
    };
    let caller = if consumers.is_empty() {
        None
    } else {
        ctx.callers
            .name_of(request.caller_worker_id.as_deref())
            .await?
    };
    let value = match store {
        StoreKind::Vault => ctx.store.resolve(name, caller.as_deref()).await?,
        StoreKind::Env => ctx.store.resolve_env(name, caller.as_deref()).await?,
    };
    tracing::info!(
        name,
        store = ?store,
        caller = caller.as_deref().unwrap_or_default(),
        "secret resolved"
    );
    Ok(ResolveResponse {
        name: name.to_owned(),
        value,
    })
}

async fn detect(ctx: Arc<Ctx>, request: DetectRequest) -> Result<DetectResponse, SecretsError> {
    let names = validate_detect_names(request.names)?;
    let found = ctx.detector().await.scan(&names).await;
    ctx.store.compare(&names, &found).await
}

async fn import(ctx: Arc<Ctx>, request: ImportRequest) -> Result<SecretMeta, SecretsError> {
    validate_name(&request.name)?;
    let names = [request.name.clone()];
    let found = ctx
        .detector()
        .await
        .read(request.source, &names)
        .await
        .into_iter()
        .find(|found| found.name == request.name)
        .ok_or_else(|| {
            SecretsError::new(
                codes::SOURCE_NOT_FOUND,
                format!(
                    "`{}` has no value in {}",
                    request.name,
                    serde_json::to_value(request.source)
                        .ok()
                        .and_then(|kind| kind.as_str().map(str::to_owned))
                        .unwrap_or_default()
                ),
            )
        })?;
    let (meta, event) = match request.store {
        StoreKind::Vault => {
            ctx.store
                .set(
                    &request.name,
                    &found.value,
                    request.consumers,
                    request.description,
                )
                .await?
        }
        // Already in `.env` or this worker's environment: share it as it is.
        StoreKind::Env if request.source != SourceKind::LoginShell => {
            ctx.store
                .set_env(&request.name, None, request.consumers, request.description)
                .await?
        }
        StoreKind::Env => {
            ctx.store
                .set_env(
                    &request.name,
                    Some(&found.value),
                    request.consumers,
                    request.description,
                )
                .await?
        }
    };
    ctx.changed(&event);
    Ok(meta)
}

async fn status(ctx: Arc<Ctx>, _: EmptyRequest) -> Result<StatusResponse, SecretsError> {
    ctx.store.status().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_errors_never_quote_the_payload() {
        let error =
            parse_request::<SetRequest>(json!({"name":"A","value": 1234567890123u64})).unwrap_err();
        assert_eq!(error.code, codes::INVALID_REQUEST);
        assert!(!error.message.contains("1234567890123"), "{error}");
        let error = parse_request::<SetRequest>(json!({"name":"A"})).unwrap_err();
        assert_eq!(error.message, "missing field `value`");
        let error = parse_request::<ImportRequest>(json!({"name":"A","source":"sk-live-pasted"}))
            .unwrap_err();
        assert!(!error.message.contains("sk-live-pasted"), "{error}");
        parse_request::<EmptyRequest>(Value::Null).unwrap();
    }
}
