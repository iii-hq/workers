//! `iii-cloud::api` (spec §11): any iii cloud route, as the person. `api` reads
//! (GET); `api::write` changes (POST, PUT, PATCH, DELETE) and is the page's own,
//! denied to agents (owner decision, 26/11).

use std::sync::Arc;

use serde_json::Value;

use crate::cloud::Req;
use crate::ctx::Ctx;
use crate::error::{CloudError, Res};
use crate::types::*;

/// Relative to `/api/v1`: starts with `/`, no `..`, `//` or scheme.
pub fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains("//")
        && !path.contains("://")
        && !path.contains('\\')
        && !path.split(['/', '?', '#']).any(|seg| seg == "..")
        && !path.contains(['?', '#'])
        // Percent-encoded dots, slashes and backslashes are how `..` gets past a plain check.
        && !["%2e", "%2f", "%5c", "%00"]
            .iter()
            .any(|e| path.to_lowercase().contains(e))
}

/// A write of a secret variable goes through the page, never through `api`.
pub fn refuses_secret(method: HttpMethod, path: &str, body: Option<&Value>) -> bool {
    // A resource's temporary credential is a Postgres URL with its password
    // (cloud-v1.yaml:5809-5834): refused for every method.
    path.trim_end_matches('/')
        .ends_with("/temporary-credential")
        || (!matches!(method, HttpMethod::Get | HttpMethod::Delete)
            && path.contains("/variables")
            && body.and_then(|b| b.get("type")).and_then(Value::as_str) == Some("secret"))
}

fn verb(m: HttpMethod) -> reqwest::Method {
    match m {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
    }
}

/// `iii-cloud::api`: reads only. A change goes through the page.
pub async fn api(ctx: &Arc<Ctx>, request: ApiRequest) -> Res<ApiResponse> {
    if request.method != HttpMethod::Get {
        return Err(CloudError::new(
            "PERSON_ONLY",
            "iii-cloud::api only reads (GET). Changes to iii cloud (deploys, variables, settings) are the person's: ask them to make it on the iii cloud page.",
        ));
    }
    write(ctx, request).await
}

/// `iii-cloud::api::write`: the page's call, any method. Denied to agents.
pub async fn write(ctx: &Arc<Ctx>, request: ApiRequest) -> Res<ApiResponse> {
    if !valid_path(&request.path) {
        return Err(CloudError::invalid(
            "path must start with / and have no .., //, scheme or query",
        ));
    }
    if refuses_secret(request.method, &request.path, request.body.as_ref()) {
        return Err(CloudError::new(
            "SECRET_VIA_API",
            "Secret values go through the page, not through the agent.",
        ));
    }
    let query = match request.query {
        Some(Value::Object(map)) => map
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect(),
        Some(Value::Null) | None => vec![],
        Some(_) => return Err(CloudError::invalid("query must be an object")),
    };
    let (status, body) = ctx
        .raw(&Req {
            verb: Some(verb(request.method)),
            path: request.path,
            query,
            body: request.body,
            if_match: request.if_match,
            idempotency_key: request.idempotency_key,
        })
        .await?;
    Ok(ApiResponse { status, body })
}
