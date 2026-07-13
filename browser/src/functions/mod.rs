//! Function registration: the `browser::*` wire surface. One module per
//! function (or small family) holds the typed request/response structs;
//! registration lives here so `register_all` reads as the product surface.

pub mod act;
pub mod console;
pub mod evaluate;
pub mod navigate;
pub mod network;
pub mod pick;
pub mod screenshot;
pub mod sessions;
pub mod snapshot;

use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chromiumoxide::cdp::browser_protocol::accessibility as cdp_ax;
use chromiumoxide::cdp::browser_protocol::dom;
use chromiumoxide::cdp::browser_protocol::input;
use chromiumoxide::cdp::browser_protocol::overlay;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::json;
use tokio::time::timeout;

use crate::events::{EventKind, SessionStartedEvent};
use crate::session::{now_ms, Session, Sessions};

pub const SESSIONS_START_ID: &str = "browser::sessions::start";
pub const SESSIONS_START_DESC: &str =
    "Start an interactive Chromium session and return its session_id. Sessions keep console \
     and network history; stop them with browser::sessions::stop when done.";
pub const SESSIONS_LIST_ID: &str = "browser::sessions::list";
pub const SESSIONS_LIST_DESC: &str =
    "List live browser sessions with their current URL and activity.";
pub const SESSIONS_STOP_ID: &str = "browser::sessions::stop";
pub const SESSIONS_STOP_DESC: &str =
    "Stop a browser session and its Chromium process. Idempotent: stopping an unknown or \
     already-stopped session succeeds with was_running=false.";
pub const NAVIGATE_ID: &str = "browser::navigate";
pub const NAVIGATE_DESC: &str =
    "Navigate a session to a URL and wait for the page to load. Element refs from earlier \
     snapshots are invalidated by navigation.";
pub const SNAPSHOT_ID: &str = "browser::snapshot";
pub const SNAPSHOT_DESC: &str =
    "Read the page as an accessibility-tree outline. Lines carry [ref=eN] handles that \
     browser::act accepts; refs stay valid until the next navigation. Prefer this over \
     browser::screenshot; it is cheaper and machine-readable.";
pub const SCREENSHOT_ID: &str = "browser::screenshot";
pub const SCREENSHOT_DESC: &str =
    "Capture the session viewport as a viewable JPEG. Use browser::snapshot for \
     machine-readable structure; screenshot when layout or rendering matters.";
pub const ACT_ID: &str = "browser::act";
pub const ACT_DESC: &str =
    "Interact with the page: click, type, press, or scroll. Address elements with a [ref=eN] \
     handle from browser::snapshot (or a pick), or raw viewport coordinates.";
pub const EVALUATE_ID: &str = "browser::evaluate";
pub const EVALUATE_DESC: &str =
    "Evaluate a JavaScript expression in the page and return its completion value. Use for \
     reads the snapshot can't express; prefer browser::act for interactions.";
pub const CONSOLE_READ_ID: &str = "browser::console::read";
pub const CONSOLE_READ_DESC: &str =
    "Read the session's captured console: console.* calls, uncaught exceptions, and \
     browser-level log entries. Filter with pattern/level and page with since_seq instead of \
     dumping everything.";
pub const NETWORK_READ_ID: &str = "browser::network::read";
pub const NETWORK_READ_DESC: &str =
    "Read the session's captured network requests (method, URL, status, failures). \
     failed_only=true is the fast path for 'what broke'.";
pub const PICK_START_ID: &str = "browser::pick::start";
pub const PICK_START_DESC: &str =
    "Internal: enter DevTools inspect mode so the human can pick an element in the console \
     UI. The pick arrives as a browser::picked trigger event. Not an agent function.";
pub const PICK_STOP_ID: &str = "browser::pick::stop";
pub const PICK_STOP_DESC: &str =
    "Internal: leave DevTools inspect mode without picking. Idempotent. Not an agent \
     function.";

/// One wire-surface entry: everything the golden schema test pins.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Build a schema exactly the way iii-sdk does at registration, so the
/// catalog snapshot pins what registration emits.
fn spec<Req, Resp>(function_id: &'static str, description: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    let generator = || schemars::r#gen::SchemaSettings::draft07().into_generator();
    FunctionSpec {
        function_id,
        description,
        request_schema: generator().into_root_schema_for::<Req>(),
        response_schema: generator().into_root_schema_for::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Golden-tested in
/// `tests/schemas.rs`; keep in lockstep with `register_all`.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<sessions::StartInput, sessions::StartOutput>(SESSIONS_START_ID, SESSIONS_START_DESC),
        spec::<sessions::ListInput, sessions::ListOutput>(SESSIONS_LIST_ID, SESSIONS_LIST_DESC),
        spec::<sessions::StopInput, sessions::StopOutput>(SESSIONS_STOP_ID, SESSIONS_STOP_DESC),
        spec::<navigate::NavigateInput, navigate::NavigateOutput>(NAVIGATE_ID, NAVIGATE_DESC),
        spec::<snapshot::SnapshotInput, snapshot::SnapshotOutput>(SNAPSHOT_ID, SNAPSHOT_DESC),
        spec::<screenshot::ScreenshotInput, screenshot::ScreenshotOutput>(
            SCREENSHOT_ID,
            SCREENSHOT_DESC,
        ),
        spec::<act::ActInput, act::ActOutput>(ACT_ID, ACT_DESC),
        spec::<evaluate::EvaluateInput, evaluate::EvaluateOutput>(EVALUATE_ID, EVALUATE_DESC),
        spec::<console::ConsoleReadInput, console::ConsoleReadOutput>(
            CONSOLE_READ_ID,
            CONSOLE_READ_DESC,
        ),
        spec::<network::NetworkReadInput, network::NetworkReadOutput>(
            NETWORK_READ_ID,
            NETWORK_READ_DESC,
        ),
        spec::<pick::PickStartInput, pick::PickOutput>(PICK_START_ID, PICK_START_DESC),
        spec::<pick::PickStopInput, pick::PickOutput>(PICK_STOP_ID, PICK_STOP_DESC),
    ]
}

pub fn register_all(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    register_sessions_start(iii, sessions);
    register_sessions_list(iii, sessions);
    register_sessions_stop(iii, sessions);
    register_navigate(iii, sessions);
    register_snapshot(iii, sessions);
    register_screenshot(iii, sessions);
    register_act(iii, sessions);
    register_evaluate(iii, sessions);
    register_console_read(iii, sessions);
    register_network_read(iii, sessions);
    register_pick_start(iii, sessions);
    register_pick_stop(iii, sessions);
    tracing::info!("all functions registered");
}

fn handler_err(msg: impl Into<String>) -> Error {
    Error::Handler(msg.into())
}

fn get_session(sessions: &Sessions, id: &str) -> Result<Arc<Session>, Error> {
    sessions.get(id).ok_or_else(|| {
        handler_err(format!(
            "unknown session '{id}'; list live sessions with browser::sessions::list"
        ))
    })
}

fn register_sessions_start(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SESSIONS_START_ID,
        RegisterFunction::new_async(move |req: sessions::StartInput| {
            let sx = sx.clone();
            async move {
                if let Some(url) = &req.url {
                    sessions::check_scheme(&sx.config.load(), url).map_err(handler_err)?;
                }
                let session = sx.start(req.url, req.headful).await.map_err(handler_err)?;
                let url = session
                    .page
                    .url()
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "about:blank".to_string());
                sx.emitter
                    .emit(
                        EventKind::SessionStarted,
                        &session.id,
                        &SessionStartedEvent {
                            session_id: session.id.clone(),
                            url: url.clone(),
                            headless: session.headless,
                            timestamp: now_ms(),
                        },
                    )
                    .await;
                Ok::<_, Error>(sessions::StartOutput {
                    session_id: session.id.clone(),
                    url,
                    headless: session.headless,
                })
            }
        })
        .description(SESSIONS_START_DESC),
    );
}

fn register_sessions_list(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SESSIONS_LIST_ID,
        RegisterFunction::new_async(move |_req: sessions::ListInput| {
            let sx = sx.clone();
            async move {
                let mut out = Vec::new();
                for session in sx.list() {
                    let url = session.page.url().await.ok().flatten().unwrap_or_default();
                    let title = session.page.get_title().await.ok().flatten();
                    let console_entries = {
                        let buf = session.console.lock().unwrap_or_else(|p| p.into_inner());
                        buf.len() as u64
                    };
                    out.push(sessions::SessionInfo {
                        session_id: session.id.clone(),
                        url,
                        title,
                        headless: session.headless,
                        created_ms: session.created_ms,
                        last_used_ms: session.last_used_ms(),
                        console_entries,
                    });
                }
                Ok::<_, Error>(sessions::ListOutput { sessions: out })
            }
        })
        .description(SESSIONS_LIST_DESC),
    );
}

fn register_sessions_stop(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SESSIONS_STOP_ID,
        RegisterFunction::new_async(move |req: sessions::StopInput| {
            let sx = sx.clone();
            async move {
                let was_running = sx.stop(&req.session_id, "stopped").await;
                Ok::<_, Error>(sessions::StopOutput {
                    ok: true,
                    was_running,
                })
            }
        })
        .description(SESSIONS_STOP_DESC),
    );
}

fn register_navigate(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        NAVIGATE_ID,
        RegisterFunction::new_async(move |req: navigate::NavigateInput| {
            let sx = sx.clone();
            async move {
                let cfg = sx.config.load_full();
                sessions::check_scheme(&cfg, &req.url).map_err(handler_err)?;
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let wait = Duration::from_millis(cfg.clamp_timeout(req.timeout_ms));

                session
                    .page
                    .goto(req.url.as_str())
                    .await
                    .map_err(|e| handler_err(format!("navigation failed: {e}")))?;
                let timed_out = timeout(wait, session.page.wait_for_navigation())
                    .await
                    .is_err();

                let url = session.page.url().await.ok().flatten().unwrap_or(req.url);
                let title = session.page.get_title().await.ok().flatten();
                session.touch();
                Ok::<_, Error>(navigate::NavigateOutput {
                    ok: true,
                    url,
                    title,
                    timed_out,
                })
            }
        })
        .description(NAVIGATE_DESC),
    );
}

fn register_snapshot(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SNAPSHOT_ID,
        RegisterFunction::new_async(move |req: snapshot::SnapshotInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let cfg = sx.config.load_full();

                let _ = session.page.execute(cdp_ax::EnableParams::default()).await;
                let tree = session
                    .page
                    .execute(cdp_ax::GetFullAxTreeParams::default())
                    .await
                    .map_err(|e| handler_err(format!("accessibility tree failed: {e}")))?;

                let result =
                    crate::snapshot::serialize(&tree.nodes, cfg.max_snapshot_nodes as usize);
                session.store_refs(result.refs);

                let url = session.page.url().await.ok().flatten().unwrap_or_default();
                let title = session.page.get_title().await.ok().flatten();
                Ok::<_, Error>(snapshot::SnapshotOutput {
                    url,
                    title,
                    tree: result.tree,
                    truncated: result.truncated,
                })
            }
        })
        .description(SNAPSHOT_DESC),
    );
}

fn register_screenshot(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SCREENSHOT_ID,
        RegisterFunction::new_async(move |req: screenshot::ScreenshotInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let cfg = sx.config.load_full();

                let params = ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Jpeg)
                    .quality(cfg.screenshot_quality as i64)
                    .full_page(req.full_page.unwrap_or(false))
                    .build();
                let bytes = session
                    .page
                    .screenshot(params)
                    .await
                    .map_err(|e| handler_err(format!("screenshot failed: {e}")))?;

                let url = session.page.url().await.ok().flatten().unwrap_or_default();
                let size = bytes.len();
                Ok::<_, Error>(screenshot::ScreenshotOutput {
                    content: vec![
                        screenshot::ContentBlock {
                            r#type: "image".to_string(),
                            mime: Some("image/jpeg".to_string()),
                            data: Some(STANDARD.encode(&bytes)),
                            text: None,
                        },
                        screenshot::ContentBlock {
                            r#type: "text".to_string(),
                            mime: None,
                            data: None,
                            text: Some(format!("Screenshot of {url} ({size} bytes)")),
                        },
                    ],
                    details: screenshot::ScreenshotDetails {
                        session_id: session.id.clone(),
                        url,
                        width: cfg.viewport_width,
                        height: cfg.viewport_height,
                    },
                })
            }
        })
        .description(SCREENSHOT_DESC),
    );
}

/// Resolve the target point for a ref- or coordinate-addressed action.
async fn action_point(session: &Session, req: &act::ActInput) -> Result<(f64, f64), Error> {
    if let Some(r) = &req.r#ref {
        let backend_id = session.resolve_ref(r).ok_or_else(|| {
            handler_err(format!(
                "unknown ref '{r}'; refs come from browser::snapshot / browser::picked and die \
                 on navigation"
            ))
        })?;
        let model = session
            .page
            .execute(
                dom::GetBoxModelParams::builder()
                    .backend_node_id(dom::BackendNodeId::new(backend_id))
                    .build(),
            )
            .await
            .map_err(|e| handler_err(format!("element has no box model: {e}")))?;
        let quad = model.model.content.inner();
        let xs: Vec<f64> = quad.iter().step_by(2).copied().collect();
        let ys: Vec<f64> = quad.iter().skip(1).step_by(2).copied().collect();
        let cx = xs.iter().sum::<f64>() / xs.len().max(1) as f64;
        let cy = ys.iter().sum::<f64>() / ys.len().max(1) as f64;
        Ok((cx, cy))
    } else {
        match (req.x, req.y) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => Err(handler_err("pass either ref or both x and y")),
        }
    }
}

async fn dispatch_click(session: &Session, x: f64, y: f64) -> Result<(), Error> {
    use input::{DispatchMouseEventParams, DispatchMouseEventType, MouseButton};
    let moved = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MouseMoved)
        .x(x)
        .y(y)
        .build()
        .map_err(handler_err)?;
    let pressed = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MousePressed)
        .x(x)
        .y(y)
        .button(MouseButton::Left)
        .click_count(1)
        .build()
        .map_err(handler_err)?;
    let released = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MouseReleased)
        .x(x)
        .y(y)
        .button(MouseButton::Left)
        .click_count(1)
        .build()
        .map_err(handler_err)?;
    for params in [moved, pressed, released] {
        session
            .page
            .execute(params)
            .await
            .map_err(|e| handler_err(format!("mouse event failed: {e}")))?;
    }
    Ok(())
}

fn register_act(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        ACT_ID,
        RegisterFunction::new_async(move |req: act::ActInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();

                let detail = match req.action.as_str() {
                    "click" => {
                        let (x, y) = action_point(&session, &req).await?;
                        dispatch_click(&session, x, y).await?;
                        format!("clicked at ({x:.0}, {y:.0})")
                    }
                    "type" => {
                        let text = req
                            .text
                            .clone()
                            .ok_or_else(|| handler_err("type needs text"))?;
                        if let Some(r) = &req.r#ref {
                            let backend_id = session.resolve_ref(r).ok_or_else(|| {
                                handler_err(format!("unknown ref '{r}'; re-snapshot first"))
                            })?;
                            session
                                .page
                                .execute(
                                    dom::FocusParams::builder()
                                        .backend_node_id(dom::BackendNodeId::new(backend_id))
                                        .build(),
                                )
                                .await
                                .map_err(|e| handler_err(format!("focus failed: {e}")))?;
                        }
                        session
                            .page
                            .execute(
                                input::InsertTextParams::builder()
                                    .text(text.clone())
                                    .build()
                                    .map_err(handler_err)?,
                            )
                            .await
                            .map_err(|e| handler_err(format!("insert text failed: {e}")))?;
                        format!("typed {} chars", text.chars().count())
                    }
                    "press" => {
                        let name = req
                            .key
                            .clone()
                            .ok_or_else(|| handler_err("press needs key"))?;
                        let spec = act::key_spec(&name)
                            .ok_or_else(|| handler_err(format!("unsupported key '{name}'")))?;
                        use input::{DispatchKeyEventParams, DispatchKeyEventType};
                        let mut down = DispatchKeyEventParams::builder()
                            .r#type(DispatchKeyEventType::KeyDown)
                            .key(spec.key)
                            .code(spec.code)
                            .windows_virtual_key_code(spec.windows_virtual_key_code)
                            .native_virtual_key_code(spec.windows_virtual_key_code);
                        if let Some(text) = spec.text {
                            down = down.text(text);
                        }
                        let up = DispatchKeyEventParams::builder()
                            .r#type(DispatchKeyEventType::KeyUp)
                            .key(spec.key)
                            .code(spec.code)
                            .windows_virtual_key_code(spec.windows_virtual_key_code)
                            .native_virtual_key_code(spec.windows_virtual_key_code)
                            .build()
                            .map_err(handler_err)?;
                        session
                            .page
                            .execute(down.build().map_err(handler_err)?)
                            .await
                            .map_err(|e| handler_err(format!("key down failed: {e}")))?;
                        session
                            .page
                            .execute(up)
                            .await
                            .map_err(|e| handler_err(format!("key up failed: {e}")))?;
                        format!("pressed {name}")
                    }
                    "scroll" => {
                        let (x, y) = if req.r#ref.is_some() || (req.x.is_some() && req.y.is_some())
                        {
                            action_point(&session, &req).await?
                        } else {
                            let cfg = sx.config.load();
                            (
                                cfg.viewport_width as f64 / 2.0,
                                cfg.viewport_height as f64 / 2.0,
                            )
                        };
                        let delta_y = req.delta_y.unwrap_or(600.0);
                        use input::{DispatchMouseEventParams, DispatchMouseEventType};
                        let wheel = DispatchMouseEventParams::builder()
                            .r#type(DispatchMouseEventType::MouseWheel)
                            .x(x)
                            .y(y)
                            .delta_x(0.0)
                            .delta_y(delta_y)
                            .build()
                            .map_err(handler_err)?;
                        session
                            .page
                            .execute(wheel)
                            .await
                            .map_err(|e| handler_err(format!("scroll failed: {e}")))?;
                        format!("scrolled {delta_y:.0}px")
                    }
                    other => {
                        return Err(handler_err(format!(
                            "unknown action '{other}' (click, type, press, scroll)"
                        )))
                    }
                };
                session.touch();
                Ok::<_, Error>(act::ActOutput { ok: true, detail })
            }
        })
        .description(ACT_DESC),
    );
}

fn register_evaluate(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        EVALUATE_ID,
        RegisterFunction::new_async(move |req: evaluate::EvaluateInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let cfg = sx.config.load();
                let wait = Duration::from_millis(cfg.clamp_timeout(req.timeout_ms));

                let evaluated = timeout(wait, session.page.evaluate(req.expression.as_str())).await;
                let output = match evaluated {
                    Err(_) => evaluate::EvaluateOutput {
                        ok: false,
                        value: None,
                        error: Some("evaluation timed out".to_string()),
                    },
                    Ok(Err(e)) => evaluate::EvaluateOutput {
                        ok: false,
                        value: None,
                        error: Some(e.to_string()),
                    },
                    Ok(Ok(result)) => evaluate::EvaluateOutput {
                        ok: true,
                        value: result.value().cloned(),
                        error: None,
                    },
                };
                Ok::<_, Error>(output)
            }
        })
        .description(EVALUATE_DESC),
    );
}

fn register_console_read(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        CONSOLE_READ_ID,
        RegisterFunction::new_async(move |req: console::ConsoleReadInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let matcher = req
                    .pattern
                    .as_deref()
                    .map(regex::Regex::new)
                    .transpose()
                    .map_err(|e| handler_err(format!("invalid pattern: {e}")))?;
                let since = req.since_seq.unwrap_or(0);
                let limit = req.limit.unwrap_or(console::DEFAULT_LIMIT as u64) as usize;

                let (mut entries, dropped) = {
                    let buf = session.console.lock().unwrap_or_else(|p| p.into_inner());
                    let entries: Vec<_> = buf
                        .iter()
                        .filter(|e| e.seq > since)
                        .filter(|e| {
                            req.level
                                .as_deref()
                                .map(|want| console::level_matches(want, &e.level))
                                .unwrap_or(true)
                        })
                        .filter(|e| {
                            matcher
                                .as_ref()
                                .map(|m| m.is_match(&e.text))
                                .unwrap_or(true)
                        })
                        .cloned()
                        .collect();
                    (entries, buf.dropped())
                };
                let overflow = entries.len().saturating_sub(limit);
                entries.drain(..overflow);
                let last_seq = entries.last().map(|e| e.seq).unwrap_or(since);
                Ok::<_, Error>(console::ConsoleReadOutput {
                    entries,
                    last_seq,
                    dropped,
                })
            }
        })
        .description(CONSOLE_READ_DESC),
    );
}

fn register_network_read(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        NETWORK_READ_ID,
        RegisterFunction::new_async(move |req: network::NetworkReadInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let matcher = req
                    .pattern
                    .as_deref()
                    .map(regex::Regex::new)
                    .transpose()
                    .map_err(|e| handler_err(format!("invalid pattern: {e}")))?;
                let since = req.since_seq.unwrap_or(0);
                let limit = req.limit.unwrap_or(console::DEFAULT_LIMIT as u64) as usize;
                let failed_only = req.failed_only.unwrap_or(false);

                let (mut entries, dropped) = {
                    let buf = session.network.lock().unwrap_or_else(|p| p.into_inner());
                    let entries: Vec<_> = buf
                        .iter()
                        .filter(|e| e.seq > since)
                        .filter(|e| !failed_only || e.failed)
                        .filter(|e| matcher.as_ref().map(|m| m.is_match(&e.url)).unwrap_or(true))
                        .cloned()
                        .collect();
                    (entries, buf.dropped())
                };
                let overflow = entries.len().saturating_sub(limit);
                entries.drain(..overflow);
                let last_seq = entries.last().map(|e| e.seq).unwrap_or(since);
                Ok::<_, Error>(network::NetworkReadOutput {
                    entries,
                    last_seq,
                    dropped,
                })
            }
        })
        .description(NETWORK_READ_DESC),
    );
}

fn register_pick_start(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        PICK_START_ID,
        RegisterFunction::new_async(move |req: pick::PickStartInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let _ = session.page.execute(dom::EnableParams::default()).await;
                let _ = session.page.execute(overlay::EnableParams::default()).await;
                let highlight = overlay::HighlightConfig {
                    content_color: Some(dom::Rgba {
                        r: 111,
                        g: 168,
                        b: 220,
                        a: Some(0.5),
                    }),
                    padding_color: Some(dom::Rgba {
                        r: 147,
                        g: 196,
                        b: 125,
                        a: Some(0.5),
                    }),
                    ..overlay::HighlightConfig::default()
                };
                session
                    .page
                    .execute(
                        overlay::SetInspectModeParams::builder()
                            .mode(overlay::InspectMode::SearchForNode)
                            .highlight_config(highlight)
                            .build()
                            .map_err(handler_err)?,
                    )
                    .await
                    .map_err(|e| handler_err(format!("inspect mode failed: {e}")))?;
                Ok::<_, Error>(pick::PickOutput { ok: true })
            }
        })
        .description(PICK_START_DESC)
        .metadata(json!({ "internal": true })),
    );
}

fn register_pick_stop(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        PICK_STOP_ID,
        RegisterFunction::new_async(move |req: pick::PickStopInput| {
            let sx = sx.clone();
            async move {
                if let Some(session) = sx.get(&req.session_id) {
                    session.touch();
                    let _ = session
                        .page
                        .execute(
                            overlay::SetInspectModeParams::builder()
                                .mode(overlay::InspectMode::None)
                                .build()
                                .map_err(handler_err)?,
                        )
                        .await;
                }
                Ok::<_, Error>(pick::PickOutput { ok: true })
            }
        })
        .description(PICK_STOP_DESC)
        .metadata(json!({ "internal": true })),
    );
}
