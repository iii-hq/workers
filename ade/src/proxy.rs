//! Transparent WebSocket proxy at `/ws`.
//!
//! For every incoming `/ws` upgrade we open a fresh outbound WebSocket
//! to the iii engine (`engine_url`) and pump frames between the two
//! sockets until either side closes. Frame types are translated 1:1
//! between [`axum::extract::ws::Message`] and
//! [`tokio_tungstenite::tungstenite::Message`].
//!
//! The proxy is intentionally dumb — no buffering, no auth — with THREE
//! exceptions on the browser→engine leg:
//!
//! 1. The browser worker registration gets the Console worker's namespace
//!    stamped on (see [`enforce_worker_namespace`]). This is authoritative at
//!    the proxy boundary, so stale/cached bundles cannot reconnect in
//!    `default` while the Console worker runs in a project namespace.
//! 2. `registerfunction` messages get `metadata.internal = true` stamped
//!    on, and `private`, `args_format`, `observable` and the 0.25 `observe`
//!    descriptor stripped (see [`stamp_internal_registration`]). Everything
//!    a console page registers is a live-update target for that page's own
//!    observers, never a discoverable API or an event source, and stamping
//!    here (not just in the SPA) means stale/cached bundles can't pollute
//!    `engine::functions::list` either.
//! 3. A `registerfunction` for one of the functions this worker hosts
//!    (`console::ui::*`, `console::workspace::on-changed`,
//!    `console::compose::on-changed`) is dropped (see
//!    [`is_provider_registration`]). A function has one host per namespace
//!    and pages share this worker's: a page that registered
//!    `console::ui::asset` first would receive every worker's UI asset
//!    observer. The SPA never registers these ids, so dropping breaks
//!    nothing.

use std::sync::Arc;

use axum::extract::ws::{CloseFrame as AxumCloseFrame, Message as AxumMessage, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{header, uri::Authority, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame as TungCloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message as TungMessage;

/// Connection settings required by the `/ws` proxy.
#[derive(Clone)]
pub struct ProxyConfig {
    pub engine_url: Arc<String>,
    pub namespace: Option<String>,
}

/// Axum handler. Splits the upgraded socket into a sender + receiver
/// and spawns a single `handle_ws` future for the lifetime of the
/// connection.
///
/// Both legs run without message/frame size limits: the proxy must not
/// impose caps the endpoints themselves don't have. With the default
/// tungstenite limits (16 MiB frame / 64 MiB message) a single large
/// engine response (e.g. a traces read on a long-running stack) killed
/// the connection, and the browser SDK then reconnected, refetched the
/// same payload, and looped forever.
pub async fn ws_proxy(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(config): State<ProxyConfig>,
) -> Response {
    if !browser_origin_allowed(&headers) {
        tracing::warn!("rejected cross-origin browser WebSocket upgrade");
        return (StatusCode::FORBIDDEN, "cross-origin WebSocket denied").into_response();
    }
    ws.max_message_size(usize::MAX)
        .max_frame_size(usize::MAX)
        .on_upgrade(move |socket| handle_ws(socket, config))
}

/// Browsers always send `Origin` on a WebSocket handshake. Require its
/// authority to match the request `Host`, preventing an unrelated page from
/// using this unauthenticated proxy as an engine client. Non-browser clients
/// that omit `Origin` remain compatible.
fn browser_origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(host) = headers.get(header::HOST) else {
        return false;
    };
    let (Ok(origin), Ok(host)) = (origin.to_str(), host.to_str()) else {
        return false;
    };
    origin_matches_host(origin, host)
}

fn origin_matches_host(origin: &str, host: &str) -> bool {
    let Ok(origin) = url::Url::parse(origin) else {
        return false;
    };
    if !matches!(origin.scheme(), "http" | "https") {
        return false;
    }
    let Some(origin_host) = origin.host_str() else {
        return false;
    };
    let Ok(authority) = host.parse::<Authority>() else {
        return false;
    };
    let origin_host = origin_host.trim_matches(['[', ']']);
    let request_host = authority.host().trim_matches(['[', ']']);
    let origin_port = origin.port_or_known_default();
    let request_port = authority.port_u16().or_else(|| match origin.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    });
    origin_host.eq_ignore_ascii_case(request_host) && origin_port == request_port
}

async fn handle_ws(client: WebSocket, config: ProxyConfig) {
    let engine_ws_config = WebSocketConfig {
        max_message_size: None,
        max_frame_size: None,
        ..Default::default()
    };
    let (engine, _resp) = match tokio_tungstenite::connect_async_with_config(
        config.engine_url.as_str(),
        Some(engine_ws_config),
        false,
    )
    .await
    {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(
                error = %e,
                engine_url = %config.engine_url,
                "failed to dial iii engine WebSocket; closing browser WS"
            );
            // Best-effort close; ignore the result. axum will drop the
            // socket once `client` goes out of scope anyway.
            let mut client = client;
            let _ = client
                .send(AxumMessage::Close(Some(AxumCloseFrame {
                    code: 1011, // 1011 = "internal error"
                    reason: "engine WS dial failed".into(),
                })))
                .await;
            return;
        }
    };

    let (mut client_tx, mut client_rx) = client.split();
    let (mut engine_tx, mut engine_rx) = engine.split();

    let client_to_engine = async {
        while let Some(msg) = client_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "browser -> engine: client read error; closing proxied WS");
                    break;
                }
            };
            let is_close = matches!(msg, AxumMessage::Close(_));
            let msg = match msg {
                AxumMessage::Text(t) => {
                    if is_provider_registration(&t) {
                        tracing::warn!(
                            "dropped a browser registerfunction for a function the console \
                             hosts (the console's own functions cannot be registered through \
                             the /ws proxy)"
                        );
                        continue;
                    }
                    let t = enforce_worker_namespace(&t, config.namespace.as_deref()).unwrap_or(t);
                    match stamp_internal_registration(&t) {
                        Some(stamped) => AxumMessage::Text(stamped),
                        None => AxumMessage::Text(t),
                    }
                }
                other => other,
            };
            if let Some(out) = axum_to_tungstenite(msg) {
                if let Err(e) = engine_tx.send(out).await {
                    tracing::debug!(error = %e, "browser -> engine: engine send error");
                    break;
                }
            }
            if is_close {
                break;
            }
        }
        let _ = engine_tx.close().await;
    };

    let engine_to_client = async {
        while let Some(msg) = engine_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "engine -> browser: engine read error; closing proxied WS");
                    break;
                }
            };
            let is_close = matches!(msg, TungMessage::Close(_));
            if let Some(out) = tungstenite_to_axum(msg) {
                if let Err(e) = client_tx.send(out).await {
                    tracing::debug!(error = %e, "engine -> browser: client send error");
                    break;
                }
            }
            if is_close {
                break;
            }
        }
        let _ = client_tx.close().await;
    };

    tokio::select! {
        _ = client_to_engine => {}
        _ = engine_to_client => {}
    }
}

/// If `text` is the browser SDK's `engine::workers::register` invocation,
/// return a copy whose `data.namespace` matches this Console worker.
///
/// `None` as the configured namespace means the engine's `default` namespace,
/// so an explicit namespace from a stale bundle is removed. A `None` return
/// means the original frame should pass through unchanged.
pub(crate) fn enforce_worker_namespace(text: &str, namespace: Option<&str>) -> Option<String> {
    const REGISTER_WORKER: &str = "engine::workers::register";

    // Fast path: only the SDK's worker-registration invocation needs a parse.
    if !text.contains(REGISTER_WORKER) {
        return None;
    }
    let mut msg: serde_json::Value = serde_json::from_str(text).ok()?;
    if msg.get("type").and_then(|value| value.as_str()) != Some("invokefunction")
        || msg.get("function_id").and_then(|value| value.as_str()) != Some(REGISTER_WORKER)
    {
        return None;
    }
    let data = msg.get_mut("data")?.as_object_mut()?;
    match namespace {
        Some(namespace) => {
            data.insert(
                "namespace".into(),
                serde_json::Value::String(namespace.to_owned()),
            );
        }
        None => {
            data.remove("namespace");
        }
    }
    serde_json::to_string(&msg).ok()
}

/// The functions this worker hosts, which a page must not register first.
const PROVIDER_FUNCTIONS: &[&str] = &[
    crate::ui_assets::ASSET_FN,
    crate::ui_assets::ON_ASSETS_CHANGED_FN,
    crate::functions::workspace::ON_CHANGED_FN,
    crate::compose::ON_CHANGED_FN,
];
/// Every id under it is the console's injected-UI surface.
const PROVIDER_PREFIX: &str = "console::ui::";

/// `true` if `text` is a wire `registerfunction` for one of the functions
/// this worker hosts — the one frame the proxy refuses to forward
/// (tab-originated takeover; a worker registering them first remains an
/// RBAC concern).
pub(crate) fn is_provider_registration(text: &str) -> bool {
    if !text.contains("\"registerfunction\"") {
        return false;
    }
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    if msg.get("type").and_then(|t| t.as_str()) != Some("registerfunction") {
        return false;
    }
    msg.get("id")
        .and_then(|id| id.as_str())
        .is_some_and(|id| id.starts_with(PROVIDER_PREFIX) || PROVIDER_FUNCTIONS.contains(&id))
}

/// What a page's registration may not declare: an event source's settings
/// (`private`, `args_format`), `observable`, and the 0.25 `observe`
/// descriptor that carries the same.
const STRIPPED_REGISTRATION_KEYS: &[&str] = &["private", "args_format", "observable", "observe"];

/// If `text` is a wire `registerfunction` message, return a copy with
/// `metadata.internal = true` merged in and [`STRIPPED_REGISTRATION_KEYS`]
/// removed; `None` means "forward the original untouched" (not a
/// registration, unparseable, or nothing to change — a metadata shape we
/// don't understand is left as it is).
pub(crate) fn stamp_internal_registration(text: &str) -> Option<String> {
    // Fast path: skip the JSON parse for the overwhelming majority of
    // frames (invocations, results, stream sends).
    if !text.contains("\"registerfunction\"") {
        return None;
    }
    let mut msg: serde_json::Value = serde_json::from_str(text).ok()?;
    if msg.get("type").and_then(|t| t.as_str()) != Some("registerfunction") {
        return None;
    }
    let obj = msg.as_object_mut()?;
    let mut changed = false;
    match obj.get_mut("metadata") {
        Some(serde_json::Value::Object(meta)) => {
            meta.insert("internal".into(), serde_json::Value::Bool(true));
            changed = true;
        }
        // Unexpected metadata shape — don't rewrite what we don't understand.
        Some(_) => {}
        None => {
            obj.insert("metadata".into(), serde_json::json!({ "internal": true }));
            changed = true;
        }
    }
    // A page's function is a target for the page's own observers: plain,
    // callable, observable. A private one could not be a target at all.
    for key in STRIPPED_REGISTRATION_KEYS {
        changed |= obj.remove(*key).is_some();
    }
    if !changed {
        return None;
    }
    serde_json::to_string(&msg).ok()
}

/// Convert an axum `Message` into a tungstenite `Message`. Returns
/// `None` when the variant has no useful tungstenite equivalent.
pub(crate) fn axum_to_tungstenite(msg: AxumMessage) -> Option<TungMessage> {
    Some(match msg {
        AxumMessage::Text(t) => TungMessage::Text(t),
        AxumMessage::Binary(b) => TungMessage::Binary(b),
        AxumMessage::Ping(p) => TungMessage::Ping(p),
        AxumMessage::Pong(p) => TungMessage::Pong(p),
        AxumMessage::Close(frame) => TungMessage::Close(frame.map(|f| TungCloseFrame {
            code: f.code.into(),
            reason: f.reason.into_owned().into(),
        })),
    })
}

/// Convert a tungstenite `Message` into an axum `Message`. Returns
/// `None` for `Frame` (raw frames are an internal tungstenite escape
/// hatch the proxy never produces or forwards).
pub(crate) fn tungstenite_to_axum(msg: TungMessage) -> Option<AxumMessage> {
    Some(match msg {
        TungMessage::Text(t) => AxumMessage::Text(t),
        TungMessage::Binary(b) => AxumMessage::Binary(b),
        TungMessage::Ping(p) => AxumMessage::Ping(p),
        TungMessage::Pong(p) => AxumMessage::Pong(p),
        TungMessage::Close(frame) => AxumMessage::Close(frame.map(|f| AxumCloseFrame {
            code: u16::from(f.code),
            reason: f.reason.into_owned().into(),
        })),
        TungMessage::Frame(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrips_both_directions() {
        let original = AxumMessage::Text("hello".into());
        let tung = axum_to_tungstenite(original.clone()).unwrap();
        assert!(matches!(&tung, TungMessage::Text(s) if s == "hello"));
        let back = tungstenite_to_axum(tung).unwrap();
        assert!(matches!(&back, AxumMessage::Text(s) if s == "hello"));
    }

    #[test]
    fn browser_websocket_origin_must_match_the_request_host() {
        assert!(origin_matches_host(
            "http://127.0.0.1:3113",
            "127.0.0.1:3113"
        ));
        assert!(origin_matches_host(
            "https://console.example",
            "console.example"
        ));
        assert!(origin_matches_host("http://[::1]:3113", "[::1]:3113"));
        assert!(!origin_matches_host(
            "http://127.0.0.1:8766",
            "127.0.0.1:3113"
        ));
        assert!(!origin_matches_host(
            "https://hostile.invalid",
            "console.example"
        ));
        assert!(!origin_matches_host("null", "127.0.0.1:3113"));
        assert!(!origin_matches_host(
            "file:///tmp/attack.html",
            "127.0.0.1:3113"
        ));
    }

    #[test]
    fn non_browser_clients_may_omit_origin_but_browser_requests_need_host() {
        assert!(browser_origin_allowed(&HeaderMap::new()));

        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "http://127.0.0.1:3113".parse().unwrap());
        assert!(!browser_origin_allowed(&headers));
        headers.insert(header::HOST, "127.0.0.1:3113".parse().unwrap());
        assert!(browser_origin_allowed(&headers));
    }

    #[test]
    fn binary_roundtrips_both_directions() {
        let original = AxumMessage::Binary(vec![1, 2, 3, 4]);
        let tung = axum_to_tungstenite(original.clone()).unwrap();
        assert!(matches!(&tung, TungMessage::Binary(b) if b == &vec![1, 2, 3, 4]));
        let back = tungstenite_to_axum(tung).unwrap();
        assert!(matches!(&back, AxumMessage::Binary(b) if b == &vec![1, 2, 3, 4]));
    }

    #[test]
    fn ping_pong_pass_through() {
        let p = axum_to_tungstenite(AxumMessage::Ping(vec![9])).unwrap();
        assert!(matches!(&p, TungMessage::Ping(b) if b == &vec![9]));
        let p = axum_to_tungstenite(AxumMessage::Pong(vec![10])).unwrap();
        assert!(matches!(&p, TungMessage::Pong(b) if b == &vec![10]));
    }

    #[test]
    fn close_preserves_code_and_reason() {
        let original = AxumMessage::Close(Some(AxumCloseFrame {
            code: 1001,
            reason: "going away".into(),
        }));
        let tung = axum_to_tungstenite(original).unwrap();
        let frame = match tung {
            TungMessage::Close(f) => f.unwrap(),
            _ => panic!("expected close"),
        };
        assert_eq!(u16::from(frame.code), 1001);
        assert_eq!(frame.reason, "going away");

        let back = tungstenite_to_axum(TungMessage::Close(Some(frame))).unwrap();
        match back {
            AxumMessage::Close(Some(f)) => {
                assert_eq!(f.code, 1001);
                assert_eq!(f.reason, "going away");
            }
            _ => panic!("expected close"),
        }
    }

    #[test]
    fn stamp_adds_internal_metadata_when_absent() {
        let wire = r#"{"type":"registerfunction","id":"console::harness-watch::r0::console-abc"}"#;
        let out = stamp_internal_registration(wire).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["metadata"]["internal"], serde_json::json!(true));
        assert_eq!(v["id"], "console::harness-watch::r0::console-abc");
    }

    #[test]
    fn worker_registration_gets_the_console_namespace() {
        let wire = r#"{"type":"invokefunction","function_id":"engine::workers::register","data":{"runtime":"browser","name":"console-old"},"action":{"type":"void"}}"#;
        let out = enforce_worker_namespace(wire, Some("project-a")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();

        assert_eq!(value["data"]["namespace"], "project-a");
        assert_eq!(value["data"]["runtime"], "browser");
    }

    #[test]
    fn worker_registration_cannot_override_the_console_namespace() {
        let wire = r#"{"type":"invokefunction","function_id":"engine::workers::register","data":{"runtime":"browser","namespace":"default"}}"#;
        let out = enforce_worker_namespace(wire, Some("project-a")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();

        assert_eq!(value["data"]["namespace"], "project-a");
    }

    #[test]
    fn default_console_removes_a_stale_explicit_namespace() {
        let wire = r#"{"type":"invokefunction","function_id":"engine::workers::register","data":{"runtime":"browser","namespace":"project-a"}}"#;
        let out = enforce_worker_namespace(wire, None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();

        assert!(value["data"].get("namespace").is_none());
    }

    #[test]
    fn namespace_enforcement_ignores_other_messages_and_bad_input() {
        assert!(enforce_worker_namespace(
            r#"{"type":"invokefunction","function_id":"session::messages","data":{"text":"engine::workers::register"}}"#,
            Some("project-a")
        )
        .is_none());
        assert!(
            enforce_worker_namespace("engine::workers::register{", Some("project-a")).is_none()
        );
        assert!(enforce_worker_namespace(
            r#"{"type":"invokefunction","function_id":"engine::workers::register","data":"weird"}"#,
            Some("project-a")
        )
        .is_none());
    }

    #[test]
    fn stamp_merges_into_existing_metadata() {
        let wire = r#"{"type":"registerfunction","id":"x","metadata":{"tenant":"acme"}}"#;
        let out = stamp_internal_registration(wire).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["metadata"]["internal"], serde_json::json!(true));
        assert_eq!(v["metadata"]["tenant"], "acme");
    }

    #[test]
    fn stamp_ignores_other_messages_and_bad_input() {
        // Different message type — even one that mentions registerfunction in a payload.
        assert!(stamp_internal_registration(
            r#"{"type":"invokefunction","payload":"\"registerfunction\""}"#
        )
        .is_none());
        // Not JSON.
        assert!(stamp_internal_registration("registerfunction{").is_none());
        // Metadata of an unexpected shape is left alone.
        assert!(stamp_internal_registration(
            r#"{"type":"registerfunction","id":"x","metadata":"weird"}"#
        )
        .is_none());
    }

    #[test]
    fn stamp_strips_event_source_settings() {
        let wire = r#"{"type":"registerfunction","id":"x","private":true,"args_format":{"type":"object"},"observable":false,"observe":{"delivery":"owner","allow":false},"description":"d"}"#;
        let out = stamp_internal_registration(wire).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        for key in ["private", "args_format", "observable", "observe"] {
            assert!(v.get(key).is_none(), "`{key}` survived");
        }
        assert_eq!(v["description"], "d");
        assert_eq!(v["metadata"]["internal"], serde_json::json!(true));

        // Stripped even when the metadata is a shape left alone.
        let weird = r#"{"type":"registerfunction","id":"x","metadata":"weird","private":true}"#;
        let out = stamp_internal_registration(weird).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["metadata"], "weird");
        assert!(v.get("private").is_none());
    }

    #[test]
    fn provider_registrations_are_detected() {
        for id in [
            "console::ui::asset",
            "console::ui::on-assets-changed",
            "console::ui::anything-else",
            "console::workspace::on-changed",
            "console::compose::on-changed",
        ] {
            let wire = format!(r#"{{"type":"registerfunction","id":"{id}"}}"#);
            assert!(is_provider_registration(&wire), "{id} was forwarded");
        }
        // A page's own handlers pass.
        assert!(!is_provider_registration(
            r#"{"type":"registerfunction","id":"iii::console::ui-assets::console-1"}"#
        ));
        assert!(!is_provider_registration(
            r#"{"type":"registerfunction","id":"console::workspace::list"}"#
        ));
        // Payload mentions it, but the frame is a different type — forward.
        assert!(!is_provider_registration(
            r#"{"type":"invokefunction","function_id":"console::ui::asset","payload":"\"registerfunction\""}"#
        ));
        assert!(!is_provider_registration("registerfunction{"));
    }

    #[test]
    fn raw_frame_is_dropped() {
        // Raw frames are an internal tungstenite construct; we don't
        // forward them. Use a real Frame ctor — `default()` panics.
        let frame = tokio_tungstenite::tungstenite::protocol::frame::Frame::ping(vec![]);
        assert!(tungstenite_to_axum(TungMessage::Frame(frame)).is_none());
    }
}
