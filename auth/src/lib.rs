pub mod config;
pub mod io;
pub mod store;

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::Utc;
use jsonwebtoken::{
    decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use rand::rngs::OsRng;
use rand::RngCore;
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::AuthConfig;
use crate::store::AuthStore;

pub const SKILL_ID: &str = "auth";
pub const SKILL_MD: &str = include_str!("../skill.md");

pub const SUB_SKILLS: &[(&str, &str)] = &[
    ("auth/validate", include_str!("../skills/validate.md")),
    (
        "auth/server_metadata",
        include_str!("../skills/server_metadata.md"),
    ),
    (
        "auth/resource_metadata",
        include_str!("../skills/resource_metadata.md"),
    ),
    ("auth/register", include_str!("../skills/register.md")),
    ("auth/jwks", include_str!("../skills/jwks.md")),
    ("auth/jwks_rotate", include_str!("../skills/jwks_rotate.md")),
    ("auth/token", include_str!("../skills/token.md")),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientRecord {
    pub client_id: String,
    pub client_name: String,
    pub client_secret_sha256: Option<String>,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub scopes: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicJwk {
    pub kty: String,
    pub use_: String,
    pub kid: String,
    pub alg: String,
    pub n: String,
    pub e: String,
}

impl PublicJwk {
    fn to_json(&self) -> Value {
        json!({
            "kty": self.kty,
            "use": self.use_,
            "kid": self.kid,
            "alg": self.alg,
            "n": self.n,
            "e": self.e,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRecord {
    pub kid: String,
    pub private_pem: String,
    pub public_jwk: PublicJwk,
    pub created_at: i64,
    pub retire_after: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySet {
    pub current_kid: String,
    pub keys: Vec<KeyRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshTokenRecord {
    pub token_id: String,
    pub client_id: String,
    pub subject: String,
    pub scopes: Vec<String>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: String,
    exp: usize,
    iat: usize,
    nbf: usize,
    scope: String,
    client_id: String,
    jti: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthDecision {
    #[serde(default)]
    pub allowed_functions: Vec<String>,
    #[serde(default)]
    pub forbidden_functions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_trigger_types: Option<Vec<String>>,
    #[serde(default)]
    pub allow_trigger_type_registration: bool,
    #[serde(default = "default_true")]
    pub allow_function_registration: bool,
    #[serde(default)]
    pub trusted_internal: bool,
    #[serde(default)]
    pub context: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_registration_prefix: Option<String>,
}

fn default_true() -> bool {
    true
}

fn now() -> i64 {
    Utc::now().timestamp()
}

fn random_url_token(bytes: usize) -> String {
    let mut buf = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn sha256_url(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

fn split_scope(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|scope| !scope.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn normalize_body(payload: &Value) -> Value {
    payload
        .get("body")
        .filter(|body| body.is_object())
        .cloned()
        .unwrap_or_else(|| payload.clone())
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn scope_field(value: &Value, key: &str) -> Vec<String> {
    if let Some(raw) = string_field(value, key) {
        split_scope(raw)
    } else {
        string_array_field(value, key)
    }
}

fn endpoint(issuer: &str, path: &str) -> String {
    format!("{}{}", issuer.trim_end_matches('/'), path)
}

pub fn idp_capability_matrix() -> Vec<Value> {
    vec![
        json!({"idp": "keycloak", "dynamic_client_registration": true, "authorization_server_metadata": true, "pkce": "required", "notes": "reference bridge target"}),
        json!({"idp": "okta", "dynamic_client_registration": true, "authorization_server_metadata": true, "pkce": "required", "notes": "bridge config straightforward"}),
        json!({"idp": "auth0", "dynamic_client_registration": true, "authorization_server_metadata": true, "pkce": "required", "notes": "bridge config straightforward"}),
        json!({"idp": "entra", "dynamic_client_registration": false, "authorization_server_metadata": true, "pkce": "required", "notes": "pre-register clients"}),
        json!({"idp": "google", "dynamic_client_registration": false, "authorization_server_metadata": true, "pkce": "required", "notes": "pre-register clients"}),
        json!({"idp": "ping", "dynamic_client_registration": true, "authorization_server_metadata": true, "pkce": "required", "notes": "bridge config straightforward"}),
        json!({"idp": "forgerock", "dynamic_client_registration": true, "authorization_server_metadata": true, "pkce": "required", "notes": "bridge config straightforward"}),
    ]
}

pub fn server_metadata_document(cfg: &AuthConfig) -> Value {
    json!({
        "issuer": cfg.issuer,
        "authorization_endpoint": endpoint(&cfg.issuer, "/authorize"),
        "token_endpoint": endpoint(&cfg.issuer, "/token"),
        "registration_endpoint": endpoint(&cfg.issuer, "/register"),
        "jwks_uri": endpoint(&cfg.issuer, "/.well-known/jwks.json"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["client_credentials", "refresh_token"],
        "token_endpoint_auth_methods_supported": cfg.token_endpoint_auth_methods_supported,
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": cfg.supported_scopes,
        "idp_mode": cfg.idp_mode,
        "idp_capabilities": idp_capability_matrix(),
    })
}

pub fn resource_metadata_document(cfg: &AuthConfig) -> Value {
    json!({
        "resource": cfg.audience,
        "authorization_servers": [cfg.issuer],
        "jwks_uri": endpoint(&cfg.issuer, "/.well-known/jwks.json"),
        "scopes_supported": cfg.supported_scopes,
        "bearer_methods_supported": ["header"],
    })
}

fn http_json(body: Value) -> Value {
    json!({
        "status_code": 200,
        "headers": { "content-type": "application/json" },
        "body": body,
    })
}

pub fn server_metadata_response(cfg: &AuthConfig) -> Value {
    http_json(server_metadata_document(cfg))
}

pub fn resource_metadata_response(cfg: &AuthConfig) -> Value {
    http_json(resource_metadata_document(cfg))
}

fn allowed_scopes(
    requested: &[String],
    client: Option<&ClientRecord>,
    cfg: &AuthConfig,
) -> Vec<String> {
    let requested = if requested.is_empty() {
        cfg.default_scopes.clone()
    } else {
        requested.to_vec()
    };
    let supported: BTreeSet<_> = cfg.supported_scopes.iter().cloned().collect();
    let client_scopes: Option<BTreeSet<String>> =
        client.map(|c| c.scopes.iter().cloned().collect());
    requested
        .into_iter()
        .filter(|scope| {
            supported.contains(scope)
                || scope.starts_with("function:")
                || scope.starts_with("trigger:")
        })
        .filter(|scope| {
            client_scopes
                .as_ref()
                .is_none_or(|allowed| allowed.contains(scope) || allowed.contains("function:*"))
        })
        .collect()
}

fn generate_key_record() -> anyhow::Result<KeyRecord> {
    let mut rng = OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048)?;
    let public = RsaPublicKey::from(&private);
    let kid = Uuid::new_v4().to_string();
    let private_pem = private.to_pkcs8_pem(LineEnding::LF)?.to_string();
    Ok(KeyRecord {
        kid: kid.clone(),
        private_pem,
        public_jwk: PublicJwk {
            kty: "RSA".to_string(),
            use_: "sig".to_string(),
            kid,
            alg: "RS256".to_string(),
            n: URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            e: URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
        },
        created_at: now(),
        retire_after: None,
    })
}

async fn ensure_keyset(store: &dyn AuthStore) -> anyhow::Result<KeySet> {
    if let Some(keyset) = store.get_keyset().await? {
        return Ok(keyset);
    }
    let key = generate_key_record()?;
    let keyset = KeySet {
        current_kid: key.kid.clone(),
        keys: vec![key],
    };
    store.set_keyset(keyset.clone()).await?;
    Ok(keyset)
}

pub async fn rotate_jwks(store: &dyn AuthStore, cfg: &AuthConfig) -> anyhow::Result<Value> {
    let mut keyset = ensure_keyset(store).await?;
    let current_time = now();
    for key in &mut keyset.keys {
        if key.kid == keyset.current_kid && key.retire_after.is_none() {
            key.retire_after = Some(current_time + cfg.rotation_overlap_seconds);
        }
    }
    keyset
        .keys
        .retain(|key| key.retire_after.is_none_or(|retire| retire > current_time));
    let new_key = generate_key_record()?;
    keyset.current_kid = new_key.kid.clone();
    keyset.keys.push(new_key);
    store.set_keyset(keyset.clone()).await?;
    Ok(json!({
        "ok": true,
        "current_kid": keyset.current_kid,
        "active_keys": keyset.keys.len(),
    }))
}

pub async fn jwks_document(store: &dyn AuthStore) -> anyhow::Result<Value> {
    let keyset = ensure_keyset(store).await?;
    let current_time = now();
    let keys: Vec<Value> = keyset
        .keys
        .iter()
        .filter(|key| key.retire_after.is_none_or(|retire| retire > current_time))
        .map(|key| key.public_jwk.to_json())
        .collect();
    Ok(json!({ "keys": keys }))
}

pub async fn register_client(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    payload: Value,
) -> anyhow::Result<Value> {
    let body = normalize_body(&payload);
    let client_id = random_url_token(18);
    let client_secret = random_url_token(32);
    let method = string_field(&body, "token_endpoint_auth_method")
        .unwrap_or("client_secret_post")
        .to_string();
    let requested_scopes = scope_field(&body, "scope");
    let scopes = allowed_scopes(&requested_scopes, None, cfg);
    let grant_types = {
        let values = string_array_field(&body, "grant_types");
        if values.is_empty() {
            vec![
                "client_credentials".to_string(),
                "refresh_token".to_string(),
            ]
        } else {
            values
        }
    };
    let redirect_uris = string_array_field(&body, "redirect_uris");
    let client_name = string_field(&body, "client_name")
        .unwrap_or("iii client")
        .to_string();
    let stored_secret = if method == "none" {
        None
    } else {
        Some(sha256_url(&client_secret))
    };
    let record = ClientRecord {
        client_id: client_id.clone(),
        client_name,
        client_secret_sha256: stored_secret,
        redirect_uris: redirect_uris.clone(),
        grant_types: grant_types.clone(),
        scopes: scopes.clone(),
        token_endpoint_auth_method: method.clone(),
        created_at: now(),
    };
    store.set_client(record).await?;
    let mut out = json!({
        "client_id": client_id,
        "client_id_issued_at": now(),
        "client_name": string_field(&body, "client_name").unwrap_or("iii client"),
        "redirect_uris": redirect_uris,
        "grant_types": grant_types,
        "scope": scopes.join(" "),
        "token_endpoint_auth_method": method,
    });
    if method != "none" {
        out.as_object_mut()
            .expect("registration response is object")
            .insert("client_secret".to_string(), Value::String(client_secret));
    }
    Ok(out)
}

fn client_secret_matches(client: &ClientRecord, secret: Option<&str>) -> bool {
    match (&client.client_secret_sha256, secret) {
        (None, _) => true,
        (Some(expected), Some(actual)) => sha256_url(actual) == *expected,
        _ => false,
    }
}

fn issue_access_token(
    cfg: &AuthConfig,
    keyset: &KeySet,
    client: &ClientRecord,
    subject: &str,
    scopes: &[String],
) -> anyhow::Result<(String, Claims)> {
    let current = keyset
        .keys
        .iter()
        .find(|key| key.kid == keyset.current_kid)
        .ok_or_else(|| anyhow::anyhow!("current signing key missing"))?;
    let issued_at = now();
    let expires_at = issued_at + cfg.access_token_ttl_seconds;
    let claims = Claims {
        iss: cfg.issuer.clone(),
        sub: subject.to_string(),
        aud: cfg.audience.clone(),
        exp: expires_at as usize,
        iat: issued_at as usize,
        nbf: issued_at as usize,
        scope: scopes.join(" "),
        client_id: client.client_id.clone(),
        jti: Uuid::new_v4().to_string(),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(current.kid.clone());
    let token = encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(current.private_pem.as_bytes())?,
    )?;
    Ok((token, claims))
}

fn decode_token(cfg: &AuthConfig, keyset: &KeySet, token: &str) -> anyhow::Result<Claims> {
    let header = decode_header(token)?;
    let kid = header
        .kid
        .ok_or_else(|| anyhow::anyhow!("token missing kid"))?;
    let key = keyset
        .keys
        .iter()
        .find(|key| key.kid == kid)
        .ok_or_else(|| anyhow::anyhow!("unknown kid"))?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(std::slice::from_ref(&cfg.audience));
    validation.set_issuer(std::slice::from_ref(&cfg.issuer));
    let decoded = decode::<Claims>(
        token,
        &DecodingKey::from_rsa_components(&key.public_jwk.n, &key.public_jwk.e)?,
        &validation,
    )?;
    Ok(decoded.claims)
}

fn auth_header(headers: &Map<String, Value>) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, value)| value.as_str())
        .map(str::to_string)
}

fn basic_credentials(value: &str) -> Option<(String, String)> {
    let encoded = value.strip_prefix("Basic ")?;
    let decoded = STANDARD.decode(encoded).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (client_id, secret) = decoded.split_once(':')?;
    Some((client_id.to_string(), secret.to_string()))
}

fn bearer_token_from_payload(payload: &Value) -> Option<String> {
    if let Some(token) = string_field(payload, "token") {
        return Some(token.to_string());
    }
    let headers = payload.get("headers").and_then(Value::as_object)?;
    let raw = auth_header(headers)?;
    raw.strip_prefix("Bearer ").map(str::to_string)
}

fn client_credentials(body: &Value) -> (Option<String>, Option<String>) {
    if let Some(headers) = body.get("headers").and_then(Value::as_object) {
        if let Some(raw) = auth_header(headers) {
            if let Some((client_id, secret)) = basic_credentials(&raw) {
                return (Some(client_id), Some(secret));
            }
        }
    }
    (
        string_field(body, "client_id").map(str::to_string),
        string_field(body, "client_secret").map(str::to_string),
    )
}

async fn issue_for_client_credentials(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    body: &Value,
) -> anyhow::Result<Value> {
    let (client_id, secret) = client_credentials(body);
    let client_id = client_id.ok_or_else(|| anyhow::anyhow!("missing client_id"))?;
    let client = store
        .get_client(&client_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("unknown client_id"))?;
    if !client
        .grant_types
        .iter()
        .any(|grant| grant == "client_credentials")
    {
        anyhow::bail!("client_credentials grant not allowed for client");
    }
    if !client_secret_matches(&client, secret.as_deref()) {
        anyhow::bail!("invalid client_secret");
    }
    let requested = scope_field(body, "scope");
    let scopes = allowed_scopes(&requested, Some(&client), cfg);
    let keyset = ensure_keyset(store).await?;
    let (access_token, claims) =
        issue_access_token(cfg, &keyset, &client, &client.client_id, &scopes)?;
    let refresh_token = random_url_token(32);
    let refresh = RefreshTokenRecord {
        token_id: refresh_token.clone(),
        client_id: client.client_id.clone(),
        subject: claims.sub,
        scopes: scopes.clone(),
        expires_at: now() + cfg.refresh_token_ttl_seconds,
    };
    store.set_refresh_token(refresh).await?;
    Ok(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": cfg.access_token_ttl_seconds,
        "refresh_token": refresh_token,
        "scope": scopes.join(" "),
    }))
}

async fn issue_for_refresh_token(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    body: &Value,
) -> anyhow::Result<Value> {
    let refresh_token = string_field(body, "refresh_token")
        .ok_or_else(|| anyhow::anyhow!("missing refresh_token"))?;
    let refresh = store
        .get_refresh_token(refresh_token)
        .await?
        .ok_or_else(|| anyhow::anyhow!("unknown refresh_token"))?;
    if refresh.expires_at <= now() || store.is_revoked(refresh_token).await? {
        anyhow::bail!("refresh_token expired or revoked");
    }
    let client = store
        .get_client(&refresh.client_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("refresh token client missing"))?;
    let keyset = ensure_keyset(store).await?;
    let (access_token, _) =
        issue_access_token(cfg, &keyset, &client, &refresh.subject, &refresh.scopes)?;
    Ok(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": cfg.access_token_ttl_seconds,
        "scope": refresh.scopes.join(" "),
    }))
}

pub async fn introspect_token(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    token: &str,
) -> anyhow::Result<Value> {
    let keyset = ensure_keyset(store).await?;
    let claims = match decode_token(cfg, &keyset, token) {
        Ok(claims) => claims,
        Err(_) => return Ok(json!({ "active": false })),
    };
    if store.is_revoked(&claims.jti).await? {
        return Ok(json!({ "active": false }));
    }
    Ok(json!({
        "active": true,
        "client_id": claims.client_id,
        "sub": claims.sub,
        "aud": claims.aud,
        "iss": claims.iss,
        "exp": claims.exp,
        "iat": claims.iat,
        "scope": claims.scope,
        "jti": claims.jti,
    }))
}

pub async fn token_endpoint(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    payload: Value,
) -> anyhow::Result<Value> {
    let body = normalize_body(&payload);
    let action = string_field(&body, "action");
    let grant_type = string_field(&body, "grant_type");
    if action == Some("introspect") || grant_type == Some("introspection") {
        let token = string_field(&body, "token").ok_or_else(|| anyhow::anyhow!("missing token"))?;
        return introspect_token(store, cfg, token).await;
    }
    match grant_type.unwrap_or("client_credentials") {
        "client_credentials" => issue_for_client_credentials(store, cfg, &body).await,
        "refresh_token" => issue_for_refresh_token(store, cfg, &body).await,
        other => anyhow::bail!("unsupported grant_type: {other}"),
    }
}

fn scopes_to_decision(claims: &Claims) -> AuthDecision {
    let scopes = split_scope(&claims.scope);
    let allowed_functions: Vec<String> = scopes
        .iter()
        .filter_map(|scope| scope.strip_prefix("function:").map(str::to_string))
        .filter(|function_id| function_id != "*")
        .collect();
    let trigger_types: Vec<String> = scopes
        .iter()
        .filter_map(|scope| scope.strip_prefix("trigger:").map(str::to_string))
        .collect();
    AuthDecision {
        allowed_functions,
        forbidden_functions: vec![],
        allowed_trigger_types: if trigger_types.is_empty() {
            None
        } else {
            Some(trigger_types)
        },
        allow_trigger_type_registration: scopes
            .iter()
            .any(|scope| scope == "iii:trigger_type_registration"),
        allow_function_registration: scopes
            .iter()
            .any(|scope| scope == "iii:function_registration"),
        trusted_internal: scopes.iter().any(|scope| scope == "iii:trusted_internal"),
        context: json!({
            "client_id": claims.client_id,
            "subject": claims.sub,
            "scopes": scopes,
            "token_id": claims.jti,
        }),
        function_registration_prefix: None,
    }
}

pub async fn validate_session(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    payload: Value,
) -> anyhow::Result<AuthDecision> {
    let token = bearer_token_from_payload(&payload)
        .ok_or_else(|| anyhow::anyhow!("missing bearer token"))?;
    let keyset = ensure_keyset(store).await?;
    let claims = decode_token(cfg, &keyset, &token)?;
    if store.is_revoked(&claims.jti).await? {
        anyhow::bail!("token revoked");
    }
    Ok(scopes_to_decision(&claims))
}

pub async fn register_with_iii(
    iii: &iii_sdk::III,
    store: Arc<dyn AuthStore>,
    cfg: Arc<AuthConfig>,
) -> anyhow::Result<AuthFunctionRefs> {
    use iii_sdk::{IIIError, RegisterFunctionMessage};

    let validate_store = store.clone();
    let validate_cfg = cfg.clone();
    let validate = iii.register_function((
        RegisterFunctionMessage::with_id("auth::validate".to_string()).with_description(
            "Validate a Bearer token and return an iii RBAC session decision.".into(),
        ),
        move |payload: Value| {
            let store = validate_store.clone();
            let cfg = validate_cfg.clone();
            async move {
                validate_session(&*store, &cfg, payload)
                    .await
                    .and_then(|decision| serde_json::to_value(decision).map_err(Into::into))
                    .map_err(|e: anyhow::Error| IIIError::Handler(e.to_string()))
            }
        },
    ));

    let server_cfg = cfg.clone();
    let server_metadata = iii.register_function((
        RegisterFunctionMessage::with_id("auth::server_metadata".to_string())
            .with_description("Return RFC 8414 authorization server metadata.".into()),
        move |_payload: Value| {
            let cfg = server_cfg.clone();
            async move { Ok(server_metadata_response(&cfg)) }
        },
    ));

    let resource_cfg = cfg.clone();
    let resource_metadata = iii.register_function((
        RegisterFunctionMessage::with_id("auth::resource_metadata".to_string())
            .with_description("Return RFC 9728 protected resource metadata.".into()),
        move |_payload: Value| {
            let cfg = resource_cfg.clone();
            async move { Ok(resource_metadata_response(&cfg)) }
        },
    ));

    let register_store = store.clone();
    let register_cfg = cfg.clone();
    let register = iii.register_function((
        RegisterFunctionMessage::with_id("auth::register".to_string())
            .with_description("Register an OAuth client at runtime.".into()),
        move |payload: Value| {
            let store = register_store.clone();
            let cfg = register_cfg.clone();
            async move {
                register_client(&*store, &cfg, payload)
                    .await
                    .map(http_json)
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

    let jwks_store = store.clone();
    let jwks = iii.register_function((
        RegisterFunctionMessage::with_id("auth::jwks".to_string())
            .with_description("Return the active public JSON Web Key Set.".into()),
        move |_payload: Value| {
            let store = jwks_store.clone();
            async move {
                jwks_document(&*store)
                    .await
                    .map(http_json)
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

    let rotate_store = store.clone();
    let rotate_cfg = cfg.clone();
    let jwks_rotate = iii.register_function((
        RegisterFunctionMessage::with_id("auth::jwks_rotate".to_string())
            .with_description("Rotate the active local signing key.".into()),
        move |_payload: Value| {
            let store = rotate_store.clone();
            let cfg = rotate_cfg.clone();
            async move {
                rotate_jwks(&*store, &cfg)
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

    let token_store = store;
    let token_cfg = cfg;
    let token = iii.register_function((
        RegisterFunctionMessage::with_id("auth::token".to_string())
            .with_description("Issue, refresh, or introspect OAuth tokens.".into()),
        move |payload: Value| {
            let store = token_store.clone();
            let cfg = token_cfg.clone();
            async move {
                token_endpoint(&*store, &cfg, payload)
                    .await
                    .map(http_json)
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

    Ok(AuthFunctionRefs {
        validate,
        server_metadata,
        resource_metadata,
        register,
        jwks,
        jwks_rotate,
        token,
    })
}

pub struct AuthFunctionRefs {
    pub validate: iii_sdk::FunctionRef,
    pub server_metadata: iii_sdk::FunctionRef,
    pub resource_metadata: iii_sdk::FunctionRef,
    pub register: iii_sdk::FunctionRef,
    pub jwks: iii_sdk::FunctionRef,
    pub jwks_rotate: iii_sdk::FunctionRef,
    pub token: iii_sdk::FunctionRef,
}

impl AuthFunctionRefs {
    pub fn unregister_all(self) {
        for reference in [
            self.validate,
            self.server_metadata,
            self.resource_metadata,
            self.register,
            self.jwks,
            self.jwks_rotate,
            self.token,
        ] {
            reference.unregister();
        }
    }
}

pub fn extract_response_body(value: &Value) -> Option<&Value> {
    value.get("body").or(Some(value))
}

pub fn public_token_payload(value: &Value) -> HashMap<String, Value> {
    value
        .as_object()
        .map(|object| object.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryAuthStore;

    fn cfg() -> AuthConfig {
        AuthConfig {
            issuer: "https://auth.test".to_string(),
            audience: "iii-test".to_string(),
            store: crate::config::StoreBackend::Memory,
            supported_scopes: vec![
                "mcp:tools".to_string(),
                "function:demo::read".to_string(),
                "iii:function_registration".to_string(),
                "iii:trusted_internal".to_string(),
            ],
            default_scopes: vec!["mcp:tools".to_string()],
            ..AuthConfig::default()
        }
    }

    #[tokio::test]
    async fn client_credentials_roundtrip_validates_session() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg();
        let registration = register_client(
            &store,
            &cfg,
            json!({
                "client_name": "test",
                "scope": "function:demo::read iii:function_registration iii:trusted_internal"
            }),
        )
        .await?;
        let client_id = registration["client_id"].as_str().unwrap();
        let client_secret = registration["client_secret"].as_str().unwrap();
        let token = token_endpoint(
            &store,
            &cfg,
            json!({
                "grant_type": "client_credentials",
                "client_id": client_id,
                "client_secret": client_secret,
                "scope": "function:demo::read iii:function_registration iii:trusted_internal"
            }),
        )
        .await?;
        let access_token = token["access_token"].as_str().unwrap();
        let decision = validate_session(
            &store,
            &cfg,
            json!({ "headers": { "authorization": format!("Bearer {access_token}") } }),
        )
        .await?;
        assert_eq!(decision.allowed_functions, vec!["demo::read"]);
        assert!(decision.allow_function_registration);
        assert!(decision.trusted_internal);
        assert_eq!(decision.context["client_id"], client_id);
        Ok(())
    }

    #[tokio::test]
    async fn refresh_token_issues_new_access_token() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg();
        let registration = register_client(&store, &cfg, json!({ "client_name": "test" })).await?;
        let token = token_endpoint(
            &store,
            &cfg,
            json!({
                "grant_type": "client_credentials",
                "client_id": registration["client_id"],
                "client_secret": registration["client_secret"],
            }),
        )
        .await?;
        let refreshed = token_endpoint(
            &store,
            &cfg,
            json!({
                "grant_type": "refresh_token",
                "refresh_token": token["refresh_token"],
            }),
        )
        .await?;
        assert_eq!(refreshed["token_type"], "Bearer");
        assert!(refreshed["access_token"].as_str().unwrap().len() > 100);
        Ok(())
    }

    #[tokio::test]
    async fn jwks_rotate_keeps_previous_key() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg();
        let first = jwks_document(&store).await?;
        assert_eq!(first["keys"].as_array().unwrap().len(), 1);
        rotate_jwks(&store, &cfg).await?;
        let second = jwks_document(&store).await?;
        assert_eq!(second["keys"].as_array().unwrap().len(), 2);
        Ok(())
    }

    #[test]
    fn metadata_contains_required_endpoints() {
        let cfg = cfg();
        let metadata = server_metadata_document(&cfg);
        assert_eq!(metadata["issuer"], "https://auth.test");
        assert_eq!(
            metadata["registration_endpoint"],
            "https://auth.test/register"
        );
        assert_eq!(
            metadata["jwks_uri"],
            "https://auth.test/.well-known/jwks.json"
        );
    }
}
