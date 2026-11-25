//! The share listeners: loopback HTTP servers that answer exactly one path,
//! `/s/<token>`, with the frozen version that share points at.
//!
//! Two listeners run from the same router:
//!
//! - [`Exposure::Public`] is the quick-tunnel target. It serves only shares
//!   made with `public: true`; a local-only token answers 404 there, exactly
//!   like an unknown one. It also records the viewer's country from the
//!   `cf-ipcountry` header Cloudflare adds.
//! - [`Exposure::Local`] serves every share, for previews on this machine.
//!
//! Every request is rate-limited per token and overall. A passcode-protected
//! share first answers a prompt (`401`) and serves the content only to a POST
//! carrying the right passcode; wrong guesses are limited much harder. A
//! single-view share is consumed by its first successful view. Every response
//! carries `no-store`, `noindex`, `no-referrer` and `nosniff`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Form, Router};
use serde::Deserialize;

use crate::render::{self, CSP_PASSCODE, CSP_STATIC};
use crate::store::{ShareRecord, Store};
use crate::util::{is_token_shaped, unix_now, verify_passcode};

/// Which listener a router serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// The quick-tunnel target: public shares only.
    Public,
    /// This machine: every share.
    Local,
}

/// Requests one token may take per minute.
const PER_TOKEN_PER_MINUTE: u32 = 120;
/// Requests one listener takes per minute across all tokens.
const GLOBAL_PER_MINUTE: u32 = 1_200;
/// Wrong passcodes one token tolerates per ten minutes.
const PASSCODE_FAILURES_PER_10_MIN: u32 = 5;
/// Entries the limiter keeps before forgetting closed windows.
const LIMITER_MAX_KEYS: usize = 10_000;

/// Fixed-window counters, in memory: a restart forgets them, which only
/// ever loosens a limit for one window.
#[derive(Default)]
pub struct Limiter {
    windows: Mutex<HashMap<String, (i64, u32)>>,
}

impl Limiter {
    /// Count one hit against `key`; `false` once `limit` hits fall in the
    /// current `window` seconds.
    fn hit(&self, key: &str, limit: u32, window: i64, now: i64) -> bool {
        let mut windows = self.windows.lock().expect("limiter lock");
        if windows.len() > LIMITER_MAX_KEYS {
            windows.retain(|_, (start, _)| now - *start < 600);
        }
        let slot = windows.entry(key.to_string()).or_insert((now, 0));
        if now - slot.0 >= window {
            *slot = (now, 0);
        }
        if slot.1 >= limit {
            return false;
        }
        slot.1 += 1;
        true
    }

    /// Whether `key` has used up `limit` in its current window, without
    /// counting a hit.
    fn exhausted(&self, key: &str, limit: u32, window: i64, now: i64) -> bool {
        let windows = self.windows.lock().expect("limiter lock");
        windows
            .get(key)
            .is_some_and(|(start, count)| now - *start < window && *count >= limit)
    }
}

#[derive(Clone)]
struct AppState {
    store: Arc<Store>,
    exposure: Exposure,
    limiter: Arc<Limiter>,
}

#[derive(Deserialize)]
struct PasscodeForm {
    passcode: String,
}

/// Build the router of one listener over `store`.
pub fn router(store: Arc<Store>, exposure: Exposure) -> Router {
    Router::new()
        .route("/s/{token}", get(serve_get).post(serve_post))
        .fallback(not_found)
        .with_state(AppState {
            store,
            exposure,
            limiter: Arc::new(Limiter::default()),
        })
}

async fn serve_get(
    State(app): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    serve(&app, &token, &headers, None).await
}

async fn serve_post(
    State(app): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    Form(form): Form<PasscodeForm>,
) -> Response {
    serve(&app, &token, &headers, Some(form.passcode)).await
}

async fn serve(
    app: &AppState,
    token: &str,
    headers: &HeaderMap,
    passcode: Option<String>,
) -> Response {
    if !is_token_shaped(token) {
        return not_found().await;
    }
    let now = unix_now();
    let exposure = match app.exposure {
        Exposure::Public => "public",
        Exposure::Local => "local",
    };
    if !app
        .limiter
        .hit(&format!("all:{exposure}"), GLOBAL_PER_MINUTE, 60, now)
        || !app
            .limiter
            .hit(&format!("tok:{token}"), PER_TOKEN_PER_MINUTE, 60, now)
    {
        return too_many();
    }
    match handle(app, token, headers, passcode, now).await {
        Ok(response) => response,
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

async fn handle(
    app: &AppState,
    token: &str,
    headers: &HeaderMap,
    passcode: Option<String>,
    now: i64,
) -> Result<Response, String> {
    let Some(share) = app.store.load_share(token).await? else {
        return Ok(not_found().await);
    };
    if app.exposure == Exposure::Public && !share.public {
        // Indistinguishable from an unknown token: a local-only link must
        // not even be confirmed to exist through the tunnel.
        return Ok(not_found().await);
    }
    if !share.is_live(now) {
        return Ok(gone(&share));
    }
    let Some(artifact) = app.store.load_artifact(&share.artifact_id).await? else {
        return Ok(not_found().await);
    };

    if let Some(stored) = &share.passcode {
        let failures = format!("pass:{token}");
        if app
            .limiter
            .exhausted(&failures, PASSCODE_FAILURES_PER_10_MIN, 600, now)
        {
            return Ok(too_many());
        }
        let Some(attempt) = passcode else {
            return Ok(prompt(&artifact.title, None));
        };
        let stored = stored.clone();
        let ok = tokio::task::spawn_blocking(move || verify_passcode(&attempt, &stored))
            .await
            .map_err(|e| format!("passcode check failed: {e}"))?;
        if !ok {
            app.limiter
                .hit(&failures, PASSCODE_FAILURES_PER_10_MIN, 600, now);
            return Ok(prompt(&artifact.title, Some("That passcode is not right.")));
        }
    }

    let Some(content) = app.store.load_content(&artifact.id, share.version).await? else {
        return Ok(not_found().await);
    };
    if share.once && !app.store.consume_share(token, now).await? {
        return Ok(gone(&share));
    }
    let country = match app.exposure {
        Exposure::Public => headers
            .get("cf-ipcountry")
            .and_then(|v| v.to_str().ok())
            .filter(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_alphabetic()))
            .map(str::to_ascii_uppercase),
        Exposure::Local => None,
    };
    if let Err(e) = app.store.record_view(token, now, country).await {
        tracing::warn!(error = %e, "could not count a share view");
    }
    let rendered = render::render(artifact.format, &artifact.title, &content, share.expires_at);
    Ok(respond(
        StatusCode::OK,
        rendered.content_type,
        rendered.csp,
        rendered.body,
    ))
}

fn prompt(title: &str, error: Option<&str>) -> Response {
    respond(
        StatusCode::UNAUTHORIZED,
        "text/html; charset=utf-8",
        CSP_PASSCODE,
        render::passcode_page(title, error),
    )
}

fn gone(share: &ShareRecord) -> Response {
    let detail = if share.consumed_at.is_some() {
        "This was a single-view link and it has been opened already."
    } else {
        "This shared artifact is no longer available."
    };
    respond(
        StatusCode::GONE,
        "text/html; charset=utf-8",
        CSP_STATIC,
        render::status_page("Link expired", detail),
    )
}

fn too_many() -> Response {
    let mut response = respond(
        StatusCode::TOO_MANY_REQUESTS,
        "text/html; charset=utf-8",
        CSP_STATIC,
        render::status_page(
            "Slow down",
            "Too many requests for this link. Try again in a few minutes.",
        ),
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
    response
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
    use crate::store::{ArtifactFormat, ArtifactRecord, VersionMeta};
    use crate::util::hash_passcode;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn share(public: bool, expires_at: i64) -> ShareRecord {
        ShareRecord {
            token: TOKEN.into(),
            artifact_id: "aaaaaaaa".into(),
            version: 1,
            public,
            created_at: 0,
            expires_at,
            passcode: None,
            once: false,
            consumed_at: None,
        }
    }

    async fn seeded(record: ShareRecord) -> Arc<Store> {
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
        store.save_share(&record).await.expect("share");
        store
    }

    async fn call(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
        let response = app.oneshot(request).await.expect("response");
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 1 << 20).await.expect("body");
        (
            status,
            headers,
            String::from_utf8(body.to_vec()).expect("utf8"),
        )
    }

    fn get_req(path: &str) -> Request<Body> {
        Request::get(path).body(Body::empty()).expect("request")
    }

    fn post_req(passcode: &str) -> Request<Body> {
        Request::post(format!("/s/{TOKEN}"))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!("passcode={passcode}")))
            .expect("request")
    }

    #[tokio::test]
    async fn a_live_share_serves_its_frozen_version_inside_the_shell() {
        let store = seeded(share(true, unix_now() + 3600)).await;
        let (status, headers, body) = call(
            router(store.clone(), Exposure::Public),
            get_req(&format!("/s/{TOKEN}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("&lt;h1&gt;v1&lt;/h1&gt;"),
            "v1, escaped into srcdoc: {body}"
        );
        assert!(!body.contains("v2"));
        let csp = headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .expect("csp");
        assert!(csp.contains("frame-src 'none'"));
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(store.views(TOKEN).await.expect("views").count, 1);
    }

    #[tokio::test]
    async fn a_local_only_share_is_invisible_through_the_tunnel() {
        let store = seeded(share(false, unix_now() + 3600)).await;
        let (public, _, _) = call(
            router(store.clone(), Exposure::Public),
            get_req(&format!("/s/{TOKEN}")),
        )
        .await;
        assert_eq!(public, StatusCode::NOT_FOUND);
        let (local, _, _) = call(
            router(store, Exposure::Local),
            get_req(&format!("/s/{TOKEN}")),
        )
        .await;
        assert_eq!(local, StatusCode::OK);
    }

    #[tokio::test]
    async fn an_expired_share_is_gone() {
        let store = seeded(share(true, unix_now() - 1)).await;
        let (status, _, body) = call(
            router(store, Exposure::Local),
            get_req(&format!("/s/{TOKEN}")),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        assert!(!body.contains("v1"));
    }

    #[tokio::test]
    async fn a_single_view_share_is_gone_after_one_view() {
        let mut once = share(true, unix_now() + 3600);
        once.once = true;
        let store = seeded(once).await;
        let app = router(store, Exposure::Local);
        let (first, _, _) = call(app.clone(), get_req(&format!("/s/{TOKEN}"))).await;
        assert_eq!(first, StatusCode::OK);
        let (second, _, body) = call(app, get_req(&format!("/s/{TOKEN}"))).await;
        assert_eq!(second, StatusCode::GONE);
        assert!(body.contains("single-view"));
    }

    #[tokio::test]
    async fn a_passcode_share_prompts_then_serves_only_the_right_passcode() {
        let mut locked = share(true, unix_now() + 3600);
        locked.passcode = Some(hash_passcode("hunter22").expect("hash"));
        let store = seeded(locked).await;
        let app = router(store.clone(), Exposure::Local);

        let (prompt, _, body) = call(app.clone(), get_req(&format!("/s/{TOKEN}"))).await;
        assert_eq!(prompt, StatusCode::UNAUTHORIZED);
        assert!(body.contains("Protected link") && !body.contains("v1"));

        let (wrong, _, body) = call(app.clone(), post_req("nope")).await;
        assert_eq!(wrong, StatusCode::UNAUTHORIZED);
        assert!(body.contains("not right"));

        let (right, _, body) = call(app, post_req("hunter22")).await;
        assert_eq!(right, StatusCode::OK);
        assert!(body.contains("&lt;h1&gt;v1"));
        assert_eq!(
            store.views(TOKEN).await.expect("views").count,
            1,
            "prompts are not views"
        );
    }

    #[tokio::test]
    async fn passcode_guessing_is_cut_off() {
        let mut locked = share(true, unix_now() + 3600);
        locked.passcode = Some(hash_passcode("hunter22").expect("hash"));
        let app = router(seeded(locked).await, Exposure::Local);
        for _ in 0..PASSCODE_FAILURES_PER_10_MIN {
            let (status, _, _) = call(app.clone(), post_req("guess")).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (blocked, headers, _) = call(app, post_req("hunter22")).await;
        assert_eq!(
            blocked,
            StatusCode::TOO_MANY_REQUESTS,
            "even the right one, once cut off"
        );
        assert_eq!(headers[header::RETRY_AFTER], "60");
    }

    #[tokio::test]
    async fn one_token_cannot_flood_the_store() {
        let app = router(
            seeded(share(true, unix_now() + 3600)).await,
            Exposure::Local,
        );
        let path = format!("/s/{TOKEN}");
        for _ in 0..PER_TOKEN_PER_MINUTE {
            let (status, _, _) = call(app.clone(), get_req(&path)).await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, _, _) = call(app, get_req(&path)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn unknown_or_malformed_tokens_and_other_paths_are_not_found() {
        let store = seeded(share(true, unix_now() + 3600)).await;
        let other = "f".repeat(64);
        for path in [
            format!("/s/{other}"),
            "/s/short".to_string(),
            "/".to_string(),
            "/aaaaaaaa".to_string(),
        ] {
            let (status, _, _) = call(router(store.clone(), Exposure::Local), get_req(&path)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[test]
    fn the_limiter_opens_a_new_window() {
        let limiter = Limiter::default();
        assert!(limiter.hit("k", 1, 60, 0));
        assert!(!limiter.hit("k", 1, 60, 30));
        assert!(limiter.hit("k", 1, 60, 60));
    }
}
