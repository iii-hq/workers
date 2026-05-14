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
pub const SKILL_MD: &str = include_str!("../skills/index.md");

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
    ("auth/introspect", include_str!("../skills/introspect.md")),
    ("auth/revoke", include_str!("../skills/revoke.md")),
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
    #[serde(rename = "use")]
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
    exp: u64,
    iat: u64,
    nbf: u64,
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
    #[serde(default)]
    pub allow_function_registration: bool,
    #[serde(default)]
    pub trusted_internal: bool,
    #[serde(default)]
    pub context: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_registration_prefix: Option<String>,
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

fn timestamp_claim(value: i64) -> anyhow::Result<u64> {
    u64::try_from(value).map_err(|_| anyhow::anyhow!("timestamp out of JWT claim range: {value}"))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for i in 0..max_len {
        let l = left.get(i).copied().unwrap_or(0);
        let r = right.get(i).copied().unwrap_or(0);
        diff |= usize::from(l ^ r);
    }
    diff == 0
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
        "token_endpoint": endpoint(&cfg.issuer, "/token"),
        "registration_endpoint": endpoint(&cfg.issuer, "/register"),
        "introspection_endpoint": endpoint(&cfg.issuer, "/introspect"),
        "revocation_endpoint": endpoint(&cfg.issuer, "/revoke"),
        "jwks_uri": endpoint(&cfg.issuer, "/.well-known/jwks.json"),
        "grant_types_supported": ["client_credentials", "refresh_token"],
        "token_endpoint_auth_methods_supported": cfg.token_endpoint_auth_methods_supported,
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

fn is_privileged_scope(scope: &str) -> bool {
    scope.starts_with("function:") || scope.starts_with("trigger:") || scope.starts_with("iii:")
}

fn scope_supported(scope: &str, cfg: &AuthConfig) -> bool {
    cfg.supported_scopes.iter().any(|supported| {
        supported == scope
            || (supported == "function:*" && scope.starts_with("function:"))
            || (supported == "trigger:*" && scope.starts_with("trigger:"))
    })
}

fn scope_allowed_by_client(scope: &str, client: &ClientRecord) -> bool {
    client.scopes.iter().any(|allowed| {
        allowed == scope
            || (allowed == "function:*" && scope.starts_with("function:") && scope != "function:*")
            || (allowed == "trigger:*" && scope.starts_with("trigger:") && scope != "trigger:*")
    })
}

fn requested_or_default(requested: &[String], cfg: &AuthConfig) -> Vec<String> {
    if requested.is_empty() {
        cfg.default_scopes.clone()
    } else {
        requested.to_vec()
    }
}

fn validate_registration_scopes(
    requested: &[String],
    cfg: &AuthConfig,
    admin_authorized: bool,
) -> anyhow::Result<Vec<String>> {
    let scopes = requested_or_default(requested, cfg);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for scope in scopes {
        if !scope_supported(&scope, cfg) {
            anyhow::bail!("unsupported scope: {scope}");
        }
        if is_privileged_scope(&scope) && !admin_authorized {
            anyhow::bail!("admin authorization required for privileged scope: {scope}");
        }
        if seen.insert(scope.clone()) {
            out.push(scope);
        }
    }
    Ok(out)
}

fn validate_token_scopes(
    requested: &[String],
    client: &ClientRecord,
    cfg: &AuthConfig,
) -> anyhow::Result<Vec<String>> {
    let scopes = requested_or_default(requested, cfg);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for scope in scopes {
        if !scope_supported(&scope, cfg) {
            anyhow::bail!("unsupported scope: {scope}");
        }
        if scope == "function:*" || scope == "trigger:*" {
            anyhow::bail!("wildcard scopes can be registered but cannot be issued: {scope}");
        }
        if !scope_allowed_by_client(&scope, client) {
            anyhow::bail!("scope not allowed for client: {scope}");
        }
        if seen.insert(scope.clone()) {
            out.push(scope);
        }
    }
    Ok(out)
}

fn supported_grant_type(value: &str) -> bool {
    matches!(value, "client_credentials" | "refresh_token")
}

fn requested_grant_types(body: &Value) -> anyhow::Result<Vec<String>> {
    let values = string_array_field(body, "grant_types");
    let grants = if values.is_empty() {
        vec![
            "client_credentials".to_string(),
            "refresh_token".to_string(),
        ]
    } else {
        values
    };
    for grant in &grants {
        if !supported_grant_type(grant) {
            anyhow::bail!("unsupported grant_type for local auth worker: {grant}");
        }
    }
    Ok(grants)
}

fn requested_auth_method(body: &Value, cfg: &AuthConfig) -> anyhow::Result<String> {
    let method = string_field(body, "token_endpoint_auth_method")
        .unwrap_or("client_secret_post")
        .to_string();
    if !cfg
        .token_endpoint_auth_methods_supported
        .iter()
        .any(|supported| supported == &method)
    {
        anyhow::bail!("unsupported token_endpoint_auth_method: {method}");
    }
    Ok(method)
}

fn admin_registration_token(cfg: &AuthConfig) -> Option<String> {
    if cfg.registration_admin_token_env.is_empty() {
        return None;
    }
    std::env::var(&cfg.registration_admin_token_env)
        .ok()
        .filter(|token| !token.is_empty())
}

fn registration_admin_authorized(payload: &Value, body: &Value, cfg: &AuthConfig) -> bool {
    let Some(expected) = admin_registration_token(cfg) else {
        return false;
    };
    let bearer = auth_header_from_payload(payload, body)
        .and_then(|raw| strip_auth_scheme(&raw, "Bearer").map(str::to_string));
    let body_token = string_field(body, "admin_token").map(str::to_string);
    [bearer, body_token]
        .into_iter()
        .flatten()
        .any(|token| constant_time_eq(&token, &expected))
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
    let method = requested_auth_method(&body, cfg)?;
    let requested_scopes = scope_field(&body, "scope");
    let admin_authorized = registration_admin_authorized(&payload, &body, cfg);
    let scopes = validate_registration_scopes(&requested_scopes, cfg, admin_authorized)?;
    let grant_types = requested_grant_types(&body)?;
    if method == "none"
        && grant_types
            .iter()
            .any(|grant| grant == "client_credentials" || grant == "refresh_token")
    {
        anyhow::bail!("token_endpoint_auth_method none cannot use local client_credentials or refresh_token grants");
    }
    let redirect_uris = string_array_field(&body, "redirect_uris");
    let client_name = string_field(&body, "client_name")
        .unwrap_or("iii client")
        .to_string();
    let response_client_name = client_name.clone();
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
        "client_name": response_client_name,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialSource {
    Basic,
    Post,
    None,
}

fn client_secret_matches(
    client: &ClientRecord,
    secret: Option<&str>,
    source: CredentialSource,
) -> bool {
    match (&client.client_secret_sha256, secret) {
        (None, None) => client.token_endpoint_auth_method == "none",
        (None, Some(_)) => false,
        (Some(expected), Some(actual)) => {
            let source_allowed = match client.token_endpoint_auth_method.as_str() {
                "client_secret_basic" => source == CredentialSource::Basic,
                "client_secret_post" => source == CredentialSource::Post,
                _ => false,
            };
            source_allowed && constant_time_eq(&sha256_url(actual), expected)
        }
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
        exp: timestamp_claim(expires_at)?,
        iat: timestamp_claim(issued_at)?,
        nbf: timestamp_claim(issued_at)?,
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

fn auth_header_from_payload(payload: &Value, body: &Value) -> Option<String> {
    payload
        .get("headers")
        .and_then(Value::as_object)
        .and_then(auth_header)
        .or_else(|| {
            body.get("headers")
                .and_then(Value::as_object)
                .and_then(auth_header)
        })
}

fn strip_auth_scheme<'a>(value: &'a str, scheme: &str) -> Option<&'a str> {
    let (actual, rest) = value.split_once(' ')?;
    actual.eq_ignore_ascii_case(scheme).then_some(rest)
}

fn basic_credentials(value: &str) -> Option<(String, String)> {
    let encoded = strip_auth_scheme(value, "Basic")?;
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
    strip_auth_scheme(&raw, "Bearer").map(str::to_string)
}

fn client_credentials(
    payload: &Value,
    body: &Value,
) -> (Option<String>, Option<String>, CredentialSource) {
    if let Some(raw) = auth_header_from_payload(payload, body) {
        if let Some((client_id, secret)) = basic_credentials(&raw) {
            return (Some(client_id), Some(secret), CredentialSource::Basic);
        }
    }
    (
        string_field(body, "client_id").map(str::to_string),
        string_field(body, "client_secret").map(str::to_string),
        if string_field(body, "client_secret").is_some() {
            CredentialSource::Post
        } else {
            CredentialSource::None
        },
    )
}

async fn issue_for_client_credentials(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    payload: &Value,
    body: &Value,
) -> anyhow::Result<Value> {
    let (client_id, secret, source) = client_credentials(payload, body);
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
    if !client_secret_matches(&client, secret.as_deref(), source) {
        anyhow::bail!("invalid client_secret");
    }
    let requested = scope_field(body, "scope");
    let scopes = validate_token_scopes(&requested, &client, cfg)?;
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
    payload: &Value,
    body: &Value,
) -> anyhow::Result<Value> {
    let refresh_token = string_field(body, "refresh_token")
        .ok_or_else(|| anyhow::anyhow!("missing refresh_token"))?;
    let (client_id, secret, source) = client_credentials(payload, body);
    let client_id = client_id.ok_or_else(|| anyhow::anyhow!("missing client_id"))?;
    let refresh = store
        .get_refresh_token(refresh_token)
        .await?
        .ok_or_else(|| anyhow::anyhow!("unknown refresh_token"))?;
    if refresh.expires_at <= now() || store.is_revoked(refresh_token).await? {
        anyhow::bail!("refresh_token expired or revoked");
    }
    if refresh.client_id != client_id {
        anyhow::bail!("refresh_token client mismatch");
    }
    let client = store
        .get_client(&refresh.client_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("refresh token client missing"))?;
    if !client_secret_matches(&client, secret.as_deref(), source) {
        anyhow::bail!("invalid client_secret");
    }
    let keyset = ensure_keyset(store).await?;
    let (access_token, _) =
        issue_access_token(cfg, &keyset, &client, &refresh.subject, &refresh.scopes)?;
    let new_refresh_token = random_url_token(32);
    store.revoke(refresh_token).await?;
    store
        .set_refresh_token(RefreshTokenRecord {
            token_id: new_refresh_token.clone(),
            client_id: client.client_id.clone(),
            subject: refresh.subject,
            scopes: refresh.scopes.clone(),
            expires_at: now() + cfg.refresh_token_ttl_seconds,
        })
        .await?;
    Ok(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": cfg.access_token_ttl_seconds,
        "refresh_token": new_refresh_token,
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
        return introspect_endpoint(store, cfg, payload).await;
    }
    match grant_type.unwrap_or("client_credentials") {
        "client_credentials" => issue_for_client_credentials(store, cfg, &payload, &body).await,
        "refresh_token" => issue_for_refresh_token(store, cfg, &payload, &body).await,
        other => anyhow::bail!("unsupported grant_type: {other}"),
    }
}

async fn authenticated_client(
    store: &dyn AuthStore,
    payload: &Value,
    body: &Value,
) -> anyhow::Result<ClientRecord> {
    let (client_id, secret, source) = client_credentials(payload, body);
    let client_id = client_id.ok_or_else(|| anyhow::anyhow!("missing client_id"))?;
    let client = store
        .get_client(&client_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("unknown client_id"))?;
    if !client_secret_matches(&client, secret.as_deref(), source) {
        anyhow::bail!("invalid client_secret");
    }
    Ok(client)
}

pub async fn introspect_endpoint(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    payload: Value,
) -> anyhow::Result<Value> {
    let body = normalize_body(&payload);
    let _client = authenticated_client(store, &payload, &body).await?;
    let token = string_field(&body, "token").ok_or_else(|| anyhow::anyhow!("missing token"))?;
    introspect_token(store, cfg, token).await
}

pub async fn revoke_endpoint(
    store: &dyn AuthStore,
    cfg: &AuthConfig,
    payload: Value,
) -> anyhow::Result<Value> {
    let body = normalize_body(&payload);
    let client = authenticated_client(store, &payload, &body).await?;
    let token = string_field(&body, "token").ok_or_else(|| anyhow::anyhow!("missing token"))?;
    let hint = string_field(&body, "token_type_hint");
    let refresh = store.get_refresh_token(token).await?;

    if hint == Some("refresh_token") || refresh.is_some() {
        if let Some(refresh) = refresh {
            if refresh.client_id == client.client_id {
                store.revoke(token).await?;
            }
        }
        return Ok(json!({ "ok": true }));
    }

    let keyset = ensure_keyset(store).await?;
    if let Ok(claims) = decode_token(cfg, &keyset, token) {
        if claims.client_id == client.client_id {
            store.revoke(&claims.jti).await?;
        }
    }
    Ok(json!({ "ok": true }))
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

    let token_store = store.clone();
    let token_cfg = cfg.clone();
    let token = iii.register_function((
        RegisterFunctionMessage::with_id("auth::token".to_string())
            .with_description("Issue or refresh OAuth tokens.".into()),
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

    let introspect_store = store.clone();
    let introspect_cfg = cfg.clone();
    let introspect = iii.register_function((
        RegisterFunctionMessage::with_id("auth::introspect".to_string())
            .with_description("Introspect an OAuth token for an authenticated client.".into()),
        move |payload: Value| {
            let store = introspect_store.clone();
            let cfg = introspect_cfg.clone();
            async move {
                introspect_endpoint(&*store, &cfg, payload)
                    .await
                    .map(http_json)
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

    let revoke_store = store;
    let revoke_cfg = cfg;
    let revoke = iii.register_function((
        RegisterFunctionMessage::with_id("auth::revoke".to_string())
            .with_description("Revoke an access token or refresh token.".into()),
        move |payload: Value| {
            let store = revoke_store.clone();
            let cfg = revoke_cfg.clone();
            async move {
                revoke_endpoint(&*store, &cfg, payload)
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
        introspect,
        revoke,
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
    pub introspect: iii_sdk::FunctionRef,
    pub revoke: iii_sdk::FunctionRef,
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
            self.introspect,
            self.revoke,
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
                "a2a:message".to_string(),
                "function:demo::read".to_string(),
                "function:*".to_string(),
                "trigger:http".to_string(),
                "iii:function_registration".to_string(),
                "iii:trusted_internal".to_string(),
            ],
            default_scopes: vec!["mcp:tools".to_string()],
            registration_admin_token_env: "III_AUTH_TEST_ADMIN_TOKEN".to_string(),
            ..AuthConfig::default()
        }
    }

    fn cfg_with_admin_env() -> AuthConfig {
        let env_name = format!("III_AUTH_TEST_ADMIN_TOKEN_{}", Uuid::new_v4().simple());
        std::env::set_var(&env_name, "admin-secret");
        AuthConfig {
            registration_admin_token_env: env_name,
            ..cfg()
        }
    }

    #[tokio::test]
    async fn client_credentials_roundtrip_validates_session() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg_with_admin_env();
        let registration = register_client(
            &store,
            &cfg,
            json!({
                "headers": { "authorization": "bearer admin-secret" },
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
            json!({ "headers": { "authorization": format!("beAREr {access_token}") } }),
        )
        .await?;
        assert_eq!(decision.allowed_functions, vec!["demo::read"]);
        assert!(decision.allow_function_registration);
        assert!(decision.trusted_internal);
        assert_eq!(decision.context["client_id"], client_id);
        Ok(())
    }

    #[tokio::test]
    async fn public_registration_rejects_privileged_scopes() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg_with_admin_env();
        let err = register_client(
            &store,
            &cfg,
            json!({
                "client_name": "test",
                "scope": "function:demo::read iii:trusted_internal"
            }),
        )
        .await
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("admin authorization required for privileged scope"));
        Ok(())
    }

    #[tokio::test]
    async fn public_registration_allows_public_scopes() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg();
        let registration = register_client(
            &store,
            &cfg,
            json!({
                "client_name": "test",
                "scope": "mcp:tools a2a:message"
            }),
        )
        .await?;
        assert_eq!(registration["scope"], "mcp:tools a2a:message");
        Ok(())
    }

    #[tokio::test]
    async fn client_cannot_escalate_scopes_at_token_time() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg_with_admin_env();
        let registration = register_client(
            &store,
            &cfg,
            json!({
                "headers": { "authorization": "Bearer admin-secret" },
                "client_name": "test",
                "scope": "function:demo::read"
            }),
        )
        .await?;
        let err = token_endpoint(
            &store,
            &cfg,
            json!({
                "grant_type": "client_credentials",
                "client_id": registration["client_id"],
                "client_secret": registration["client_secret"],
                "scope": "iii:trusted_internal"
            }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("scope not allowed for client"));
        Ok(())
    }

    #[tokio::test]
    async fn client_secret_basic_requires_basic_auth_header() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg();
        let registration = register_client(
            &store,
            &cfg,
            json!({
                "client_name": "basic-client",
                "token_endpoint_auth_method": "client_secret_basic"
            }),
        )
        .await?;
        let client_id = registration["client_id"].as_str().unwrap();
        let client_secret = registration["client_secret"].as_str().unwrap();
        let post_err = token_endpoint(
            &store,
            &cfg,
            json!({
                "grant_type": "client_credentials",
                "client_id": client_id,
                "client_secret": client_secret
            }),
        )
        .await
        .unwrap_err();
        assert!(post_err.to_string().contains("invalid client_secret"));
        let encoded = STANDARD.encode(format!("{client_id}:{client_secret}"));
        let token = token_endpoint(
            &store,
            &cfg,
            json!({
                "headers": { "authorization": format!("bAsIc {encoded}") },
                "grant_type": "client_credentials"
            }),
        )
        .await?;
        assert_eq!(token["token_type"], "Bearer");
        Ok(())
    }

    #[tokio::test]
    async fn wildcard_scope_is_not_issued_as_token_scope() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let cfg = cfg_with_admin_env();
        let registration = register_client(
            &store,
            &cfg,
            json!({
                "headers": { "authorization": "Bearer admin-secret" },
                "client_name": "wildcard-client",
                "scope": "function:*"
            }),
        )
        .await?;
        let err = token_endpoint(
            &store,
            &cfg,
            json!({
                "grant_type": "client_credentials",
                "client_id": registration["client_id"],
                "client_secret": registration["client_secret"],
                "scope": "function:*"
            }),
        )
        .await
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("wildcard scopes can be registered"));
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
                "client_id": registration["client_id"],
                "client_secret": registration["client_secret"],
            }),
        )
        .await?;
        assert_eq!(refreshed["token_type"], "Bearer");
        assert!(refreshed["access_token"].as_str().unwrap().len() > 100);
        assert!(refreshed["refresh_token"].as_str().unwrap().len() > 20);
        let old_refresh = token_endpoint(
            &store,
            &cfg,
            json!({
                "grant_type": "refresh_token",
                "refresh_token": token["refresh_token"],
                "client_id": registration["client_id"],
                "client_secret": registration["client_secret"],
            }),
        )
        .await
        .unwrap_err();
        assert!(old_refresh.to_string().contains("expired or revoked"));
        Ok(())
    }

    #[tokio::test]
    async fn revoke_invalidates_access_token() -> anyhow::Result<()> {
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
        let access_token = token["access_token"].as_str().unwrap();
        let active = introspect_endpoint(
            &store,
            &cfg,
            json!({
                "client_id": registration["client_id"],
                "client_secret": registration["client_secret"],
                "token": access_token
            }),
        )
        .await?;
        assert_eq!(active["active"], true);
        revoke_endpoint(
            &store,
            &cfg,
            json!({
                "client_id": registration["client_id"],
                "client_secret": registration["client_secret"],
                "token": access_token,
                "token_type_hint": "access_token"
            }),
        )
        .await?;
        let rejected = validate_session(
            &store,
            &cfg,
            json!({ "headers": { "authorization": format!("Bearer {access_token}") } }),
        )
        .await
        .unwrap_err();
        assert!(rejected.to_string().contains("token revoked"));
        let inactive = introspect_endpoint(
            &store,
            &cfg,
            json!({
                "client_id": registration["client_id"],
                "client_secret": registration["client_secret"],
                "token": access_token
            }),
        )
        .await?;
        assert_eq!(inactive["active"], false);
        Ok(())
    }

    #[tokio::test]
    async fn public_client_method_none_cannot_use_local_grants() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        let mut cfg = cfg();
        cfg.token_endpoint_auth_methods_supported
            .push("none".to_string());
        let err = register_client(
            &store,
            &cfg,
            json!({
                "client_name": "public",
                "token_endpoint_auth_method": "none",
                "grant_types": ["client_credentials"]
            }),
        )
        .await
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("cannot use local client_credentials"));
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
        assert_eq!(metadata["revocation_endpoint"], "https://auth.test/revoke");
        assert_eq!(
            metadata["introspection_endpoint"],
            "https://auth.test/introspect"
        );
        assert!(metadata.get("authorization_endpoint").is_none());
        assert!(metadata.get("code_challenge_methods_supported").is_none());
    }
}
