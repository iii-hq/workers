//! The share listener: a loopback HTTP server that answers exactly one route,
//! `GET /s/<token>`, with the frozen version that share points at.
//!
//! It is the only thing the quick-tunnel target may point at, so it exposes
//! nothing else: no listing, no ids, no engine. Every response carries
//! `no-store`, `noindex`, `no-referrer` and `nosniff`; content responses add
//! the CSP sandbox chosen in [`crate::render`].

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::render::{self, CSP_STATIC};
use crate::store::Store;
use crate::util::{is_token_shaped, unix_now};

/// Build the share router over `store`.
pub fn router(store: Arc<Store>) -> Router {
    Router::new()
        .route("/s/{token}", get(serve_share))
        .fallback(not_found)
        .with_state(store)
}

async fn serve_share(State(store): State<Arc<Store>>, Path(token): Path<String>) -> Response {
    if !is_token_shaped(&token) {
        return not_found().await;
    }
    match lookup(&store, &token).await {
        Ok(Lookup::Found {
            format,
            title,
            content,
        }) => {
            let rendered = render::render(format, &title, &content);
            respond(
                StatusCode::OK,
                rendered.content_type,
                rendered.csp,
                rendered.body,
            )
        }
        Ok(Lookup::Missing) => not_found().await,
        Ok(Lookup::Expired) => respond(
            StatusCode::GONE,
            "text/html; charset=utf-8",
            CSP_STATIC,
            render::status_page(
                "Link expired",
                "This shared artifact is no longer available.",
            ),
        ),
        Err(e) => {
            tracing::error!(error = %e, "share lookup failed");
            respond(
                StatusCode::INTERNAL_SERVER_ERROR,
                "text/html; charset=utf-8",
                CSP_STATIC,
                render::status_page("Unavailable", "Try again in a moment."),
            )
        }
    }
}

enum Lookup {
    Found {
        format: crate::store::ArtifactFormat,
        title: String,
        content: String,
    },
    Missing,
    Expired,
}

async fn lookup(store: &Store, token: &str) -> Result<Lookup, String> {
    let Some(share) = store.load_share(token).await? else {
        return Ok(Lookup::Missing);
    };
    if !share.is_live(unix_now()) {
        return Ok(Lookup::Expired);
    }
    let Some(artifact) = store.load_artifact(&share.artifact_id).await? else {
        return Ok(Lookup::Missing);
    };
    let Some(content) = store.load_content(&artifact.id, share.version).await? else {
        return Ok(Lookup::Missing);
    };
    Ok(Lookup::Found {
        format: artifact.format,
        title: artifact.title,
        content,
    })
}

async fn not_found() -> Response {
    respond(
        StatusCode::NOT_FOUND,
        "text/html; charset=utf-8",
        CSP_STATIC,
        render::status_page("Not found", "There is nothing at this link."),
    )
}

fn respond(
    status: StatusCode,
    content_type: &'static str,
    csp: &'static str,
    body: String,
) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        "x-robots-tag",
        HeaderValue::from_static("noindex, nofollow"),
    );
    (status, headers, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ArtifactFormat, ArtifactRecord, ShareRecord, VersionMeta};
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    async fn seeded(expires_at: i64) -> Arc<Store> {
        let store = Arc::new(Store::in_memory());
        store
            .save_content("aaaaaaaa", 1, "<h1>v1</h1>")
            .await
            .expect("v1");
        store
            .save_content("aaaaaaaa", 2, "<h1>v2</h1>")
            .await
            .expect("v2");
        let meta = |version| VersionMeta {
            version,
            created_at: 0,
            bytes: 11,
            note: None,
        };
        store
            .save_artifact(&ArtifactRecord {
                id: "aaaaaaaa".into(),
                title: "proto".into(),
                kind: "prototype".into(),
                format: ArtifactFormat::Html,
                latest_version: 2,
                versions: vec![meta(1), meta(2)],
                created_at: 0,
                updated_at: 0,
            })
            .await
            .expect("artifact");
        store
            .save_share(&ShareRecord {
                token: TOKEN.into(),
                artifact_id: "aaaaaaaa".into(),
                version: 1,
                public: false,
                created_at: 0,
                expires_at,
            })
            .await
            .expect("share");
        store
    }

    async fn get(store: Arc<Store>, path: &str) -> (StatusCode, HeaderMap, String) {
        let response = router(store)
            .oneshot(Request::get(path).body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 1 << 20).await.expect("body");
        (
            status,
            headers,
            String::from_utf8(body.to_vec()).expect("utf8"),
        )
    }

    #[tokio::test]
    async fn a_live_share_serves_its_frozen_version_sandboxed() {
        let (status, headers, body) =
            get(seeded(unix_now() + 3600).await, &format!("/s/{TOKEN}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "<h1>v1</h1>", "the share is frozen at version 1");
        let csp = headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .expect("csp");
        assert!(csp.starts_with("sandbox"));
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers["x-robots-tag"], "noindex, nofollow");
    }

    #[tokio::test]
    async fn an_expired_share_is_gone() {
        let (status, _, body) = get(seeded(unix_now() - 1).await, &format!("/s/{TOKEN}")).await;
        assert_eq!(status, StatusCode::GONE);
        assert!(!body.contains("v1"));
    }

    #[tokio::test]
    async fn unknown_or_malformed_tokens_and_other_paths_are_not_found() {
        let store = seeded(unix_now() + 3600).await;
        let other = "f".repeat(64);
        for path in [
            format!("/s/{other}"),
            "/s/short".to_string(),
            "/".to_string(),
            "/aaaaaaaa".to_string(),
        ] {
            let (status, _, _) = get(store.clone(), &path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
    }
}
