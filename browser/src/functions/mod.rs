//! Function registration: the `browser::*` wire surface. One module per
//! function (or small family) holds the typed request/response structs;
//! registration lives here so `register_all` reads as the product surface.

pub mod act;
pub mod console;
pub mod dom;
pub mod evaluate;
pub mod frame;
pub mod hint;
pub mod history;
pub mod navigate;
pub mod network;
pub mod pick;
pub mod screenshot;
pub mod sessions;
pub mod snapshot;
pub mod styles;

use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chromiumoxide::cdp::browser_protocol::accessibility as cdp_ax;
use chromiumoxide::cdp::browser_protocol::css as cdp_css;
use chromiumoxide::cdp::browser_protocol::dom as cdp_dom;
use chromiumoxide::cdp::browser_protocol::input;
use chromiumoxide::cdp::browser_protocol::overlay;
use chromiumoxide::cdp::browser_protocol::page as cdp_page;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::cdp::js_protocol::runtime as cdp_rt;
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
    "Interact with the page: click (left/right/middle, single or double), hover, type, press, \
     or scroll. Address elements with a [ref=eN] handle from browser::snapshot (or a pick), or \
     raw viewport coordinates.";
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
pub const HISTORY_ID: &str = "browser::history";
pub const HISTORY_DESC: &str =
    "Go back, go forward, or reload the session's page. Back/forward at the history edge is a \
     no-op with moved=false.";
pub const DOM_READ_ID: &str = "browser::dom::read";
pub const DOM_READ_DESC: &str =
    "Read the DOM as a tree of tags with id/class and refs. Structure-oriented complement to \
     browser::snapshot; read deep subtrees by passing a ref.";
pub const STYLES_READ_ID: &str = "browser::styles::read";
pub const STYLES_READ_DESC: &str =
    "Read an element's computed styles (curated design set by default, or named properties) \
     plus its inline style attribute.";
pub const STYLES_WRITE_ID: &str = "browser::styles::write";
pub const STYLES_WRITE_DESC: &str =
    "Set one inline CSS property on an element, live in the page. Visual experiment only: the \
     page's source files are untouched, and the edit dies with the next navigation.";
pub const SCREENCAST_START_ID: &str = "browser::screencast::start";
pub const SCREENCAST_START_DESC: &str =
    "Internal: start pushing live viewport frames for browser::frame. Console-UI plumbing; \
     agents use browser::screenshot. Not an agent function.";
pub const SCREENCAST_STOP_ID: &str = "browser::screencast::stop";
pub const SCREENCAST_STOP_DESC: &str =
    "Internal: stop the live frame push. Idempotent. Not an agent function.";
pub const FRAME_ID: &str = "browser::frame";
pub const FRAME_DESC: &str =
    "Internal: newest screencast frame, or nothing when since_frame is still current. No \
     capture round-trip; poll fast. Not an agent function.";
pub const PICK_HINT_ID: &str = "browser::pick::hint";
pub const PICK_HINT_DESC: &str =
    "Internal: element preview at a viewport point (tag, id, classes, bounds) so the console \
     UI can draw a hover highlight in pick mode. Not an agent function.";
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
        spec::<history::HistoryInput, history::HistoryOutput>(HISTORY_ID, HISTORY_DESC),
        spec::<dom::DomReadInput, dom::DomReadOutput>(DOM_READ_ID, DOM_READ_DESC),
        spec::<styles::StylesReadInput, styles::StylesReadOutput>(STYLES_READ_ID, STYLES_READ_DESC),
        spec::<styles::StylesWriteInput, styles::StylesWriteOutput>(
            STYLES_WRITE_ID,
            STYLES_WRITE_DESC,
        ),
        spec::<frame::ScreencastStartInput, pick::PickOutput>(
            SCREENCAST_START_ID,
            SCREENCAST_START_DESC,
        ),
        spec::<frame::ScreencastStopInput, pick::PickOutput>(
            SCREENCAST_STOP_ID,
            SCREENCAST_STOP_DESC,
        ),
        spec::<frame::FrameInput, frame::FrameOutput>(FRAME_ID, FRAME_DESC),
        spec::<hint::PickHintInput, hint::PickHintOutput>(PICK_HINT_ID, PICK_HINT_DESC),
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
    register_history(iii, sessions);
    register_dom_read(iii, sessions);
    register_styles_read(iii, sessions);
    register_styles_write(iii, sessions);
    register_screencast_start(iii, sessions);
    register_screencast_stop(iii, sessions);
    register_frame(iii, sessions);
    register_pick_hint(iii, sessions);
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
                // Each session's url + title are two independent CDP
                // round-trips, and sessions are independent of each other:
                // join both axes so the whole list costs one round-trip time
                // instead of 2N. This is re-read on every lifecycle trigger
                // plus a poll fallback.
                let out =
                    futures::future::join_all(sx.list().into_iter().map(|session| async move {
                        let (url, title) =
                            futures::join!(session.page.url(), session.page.get_title());
                        let console_entries = {
                            let buf = session.console.lock().unwrap_or_else(|p| p.into_inner());
                            buf.len() as u64
                        };
                        sessions::SessionInfo {
                            session_id: session.id.clone(),
                            url: url.ok().flatten().unwrap_or_default(),
                            title: title.ok().flatten(),
                            headless: session.headless,
                            created_ms: session.created_ms,
                            last_used_ms: session.last_used_ms(),
                            console_entries,
                        }
                    }))
                    .await;
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
                        width: session.viewport_width,
                        height: session.viewport_height,
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
        let backend_id = session.resolve_ref_or_err(r)?;
        let model = session
            .page
            .execute(
                cdp_dom::GetBoxModelParams::builder()
                    .backend_node_id(cdp_dom::BackendNodeId::new(backend_id))
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

async fn dispatch_click(
    session: &Session,
    x: f64,
    y: f64,
    button: &str,
    click_count: i64,
) -> Result<(), Error> {
    use input::{DispatchMouseEventParams, DispatchMouseEventType, MouseButton};
    let button = match button {
        "left" => MouseButton::Left,
        "right" => MouseButton::Right,
        "middle" => MouseButton::Middle,
        other => return Err(handler_err(format!("unknown button '{other}'"))),
    };
    let moved = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MouseMoved)
        .x(x)
        .y(y)
        .build()
        .map_err(handler_err)?;
    session
        .page
        .execute(moved)
        .await
        .map_err(|e| handler_err(format!("mouse event failed: {e}")))?;
    for count in 1..=click_count.max(1) {
        let pressed = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MousePressed)
            .x(x)
            .y(y)
            .button(button.clone())
            .click_count(count)
            .build()
            .map_err(handler_err)?;
        let released = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseReleased)
            .x(x)
            .y(y)
            .button(button.clone())
            .click_count(count)
            .build()
            .map_err(handler_err)?;
        for params in [pressed, released] {
            session
                .page
                .execute(params)
                .await
                .map_err(|e| handler_err(format!("mouse event failed: {e}")))?;
        }
    }
    Ok(())
}

async fn dispatch_hover(session: &Session, x: f64, y: f64) -> Result<(), Error> {
    use input::{DispatchMouseEventParams, DispatchMouseEventType};
    let moved = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MouseMoved)
        .x(x)
        .y(y)
        .build()
        .map_err(handler_err)?;
    session
        .page
        .execute(moved)
        .await
        .map_err(|e| handler_err(format!("mouse event failed: {e}")))?;
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
                        let button = req.button.as_deref().unwrap_or("left");
                        let clicks = i64::from(req.click_count.unwrap_or(1));
                        dispatch_click(&session, x, y, button, clicks).await?;
                        format!("clicked {button} x{clicks} at ({x:.0}, {y:.0})")
                    }
                    "hover" => {
                        let (x, y) = action_point(&session, &req).await?;
                        dispatch_hover(&session, x, y).await?;
                        format!("hovering at ({x:.0}, {y:.0})")
                    }
                    "type" => {
                        let text = req
                            .text
                            .clone()
                            .ok_or_else(|| handler_err("type needs text"))?;
                        if let Some(r) = &req.r#ref {
                            let backend_id = session.resolve_ref_or_err(r)?;
                            session
                                .page
                                .execute(
                                    cdp_dom::FocusParams::builder()
                                        .backend_node_id(cdp_dom::BackendNodeId::new(backend_id))
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
                            (
                                session.viewport_width as f64 / 2.0,
                                session.viewport_height as f64 / 2.0,
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
                            "unknown action '{other}' (click, hover, type, press, scroll)"
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
                let level = req.level.clone();

                let (entries, last_seq, dropped) = crate::session::read_ring(
                    &session.console,
                    since,
                    limit,
                    |e| {
                        level
                            .as_deref()
                            .map(|want| console::level_matches(want, &e.level))
                            .unwrap_or(true)
                            && matcher
                                .as_ref()
                                .map(|m| m.is_match(&e.text))
                                .unwrap_or(true)
                    },
                    |e| e.seq,
                );
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

                let (entries, last_seq, dropped) = crate::session::read_ring(
                    &session.network,
                    since,
                    limit,
                    |e| {
                        (!failed_only || e.failed)
                            && matcher.as_ref().map(|m| m.is_match(&e.url)).unwrap_or(true)
                    },
                    |e| e.seq,
                );
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
                let _ = session.page.execute(cdp_dom::EnableParams::default()).await;
                let _ = session.page.execute(overlay::EnableParams::default()).await;
                let highlight = overlay::HighlightConfig {
                    content_color: Some(cdp_dom::Rgba {
                        r: 111,
                        g: 168,
                        b: 220,
                        a: Some(0.5),
                    }),
                    padding_color: Some(cdp_dom::Rgba {
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

fn register_history(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        HISTORY_ID,
        RegisterFunction::new_async(move |req: history::HistoryInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();

                let moved = match req.action.as_str() {
                    "reload" => {
                        session
                            .page
                            .execute(cdp_page::ReloadParams::default())
                            .await
                            .map_err(|e| handler_err(format!("reload failed: {e}")))?;
                        true
                    }
                    dir @ ("back" | "forward") => {
                        let history = session
                            .page
                            .execute(cdp_page::GetNavigationHistoryParams::default())
                            .await
                            .map_err(|e| handler_err(format!("history read failed: {e}")))?;
                        let target = if dir == "back" {
                            history.current_index - 1
                        } else {
                            history.current_index + 1
                        };
                        match usize::try_from(target)
                            .ok()
                            .and_then(|i| history.entries.get(i))
                        {
                            Some(entry) => {
                                session
                                    .page
                                    .execute(cdp_page::NavigateToHistoryEntryParams::new(entry.id))
                                    .await
                                    .map_err(|e| {
                                        handler_err(format!("history navigation failed: {e}"))
                                    })?;
                                true
                            }
                            None => false,
                        }
                    }
                    other => {
                        return Err(handler_err(format!(
                            "unknown action '{other}' (back, forward, reload)"
                        )))
                    }
                };

                let wait = Duration::from_millis(sx.config.load().default_timeout_ms);
                if moved {
                    let _ = timeout(wait, session.page.wait_for_navigation()).await;
                }
                let url = session.page.url().await.ok().flatten().unwrap_or_default();
                session.touch();
                Ok::<_, Error>(history::HistoryOutput {
                    ok: true,
                    url,
                    moved,
                })
            }
        })
        .description(HISTORY_DESC),
    );
}

/// Convert a CDP node subtree into the wire outline, registering a `d<id>`
/// ref per element so the tree is actionable. Whitespace-only text nodes are
/// skipped. `budget` caps total emitted nodes.
fn convert_dom_node(
    session: &Session,
    node: &cdp_dom::Node,
    budget: &mut usize,
) -> Option<dom::DomNode> {
    if *budget == 0 {
        return None;
    }
    const TEXT_NODE: i64 = 3;
    let mut text = None;
    if node.node_type == TEXT_NODE {
        let trimmed = node.node_value.trim();
        if trimmed.is_empty() {
            return None;
        }
        text = Some(crate::session::truncate(trimmed, 120));
    }
    *budget -= 1;

    let (id, classes) =
        crate::session::id_and_classes(node.attributes.as_deref().unwrap_or(&[]), 200);

    let backend_id = *node.backend_node_id.inner();
    let r#ref = format!("d{backend_id}");
    session.add_ref(r#ref.clone(), backend_id);

    let children = node
        .children
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|child| convert_dom_node(session, child, budget))
        .collect();

    Some(dom::DomNode {
        r#ref,
        tag: node.node_name.to_lowercase(),
        id,
        classes,
        text,
        child_count: node.child_node_count.unwrap_or(0).max(0) as u32,
        children,
    })
}

fn register_dom_read(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        DOM_READ_ID,
        RegisterFunction::new_async(move |req: dom::DomReadInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let depth = i64::from(req.depth.unwrap_or(dom::DEFAULT_DEPTH).clamp(1, 20));

                let node = match &req.r#ref {
                    Some(r) => {
                        let backend_id = session.resolve_ref_or_err(r)?;
                        session
                            .page
                            .execute(
                                cdp_dom::DescribeNodeParams::builder()
                                    .backend_node_id(cdp_dom::BackendNodeId::new(backend_id))
                                    .depth(depth)
                                    .pierce(true)
                                    .build(),
                            )
                            .await
                            .map_err(|e| handler_err(format!("describe node failed: {e}")))?
                            .node
                            .clone()
                    }
                    None => session
                        .page
                        .execute(
                            cdp_dom::GetDocumentParams::builder()
                                .depth(depth)
                                .pierce(true)
                                .build(),
                        )
                        .await
                        .map_err(|e| handler_err(format!("document read failed: {e}")))?
                        .root
                        .clone(),
                };

                let mut budget = dom::MAX_DOM_NODES;
                let root = convert_dom_node(&session, &node, &mut budget)
                    .ok_or_else(|| handler_err("document root is empty"))?;
                Ok::<_, Error>(dom::DomReadOutput {
                    root,
                    truncated: budget == 0,
                })
            }
        })
        .description(DOM_READ_DESC),
    );
}

/// Push a backend node id into the CSS agent's id space. CSS.* functions
/// take frontend node ids, not backend ids.
async fn frontend_node_id(session: &Session, backend_id: i64) -> Result<cdp_dom::NodeId, Error> {
    let _ = session.page.execute(cdp_dom::EnableParams::default()).await;
    let _ = session.page.execute(cdp_css::EnableParams::default()).await;
    let pushed = session
        .page
        .execute(cdp_dom::PushNodesByBackendIdsToFrontendParams::new(vec![
            cdp_dom::BackendNodeId::new(backend_id),
        ]))
        .await
        .map_err(|e| handler_err(format!("node push failed: {e}")))?;
    pushed
        .node_ids
        .first()
        .cloned()
        .ok_or_else(|| handler_err("element no longer exists"))
}

fn register_styles_read(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        STYLES_READ_ID,
        RegisterFunction::new_async(move |req: styles::StylesReadInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let backend_id = session.resolve_ref_or_err(&req.r#ref)?;
                let node_id = frontend_node_id(&session, backend_id).await?;

                let computed = session
                    .page
                    .execute(cdp_css::GetComputedStyleForNodeParams::new(node_id))
                    .await
                    .map_err(|e| handler_err(format!("computed styles failed: {e}")))?;

                let wanted: Option<Vec<&str>> = match &req.properties {
                    None => Some(styles::DEFAULT_PROPERTIES.to_vec()),
                    Some(list) if list.iter().any(|p| p == "*") => None,
                    Some(list) => Some(list.iter().map(String::as_str).collect()),
                };
                let properties = computed
                    .computed_style
                    .iter()
                    .filter(|p| {
                        wanted
                            .as_ref()
                            .map(|w| w.contains(&p.name.as_str()))
                            .unwrap_or(true)
                    })
                    .map(|p| styles::StyleProperty {
                        name: p.name.clone(),
                        value: p.value.clone(),
                    })
                    .collect();

                let inline_style = session
                    .page
                    .execute(cdp_css::GetInlineStylesForNodeParams::new(node_id))
                    .await
                    .ok()
                    .and_then(|r| r.inline_style.clone())
                    .and_then(|s| s.css_text)
                    .filter(|t| !t.is_empty());

                Ok::<_, Error>(styles::StylesReadOutput {
                    r#ref: req.r#ref,
                    properties,
                    inline_style,
                })
            }
        })
        .description(STYLES_READ_DESC),
    );
}

fn register_styles_write(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        STYLES_WRITE_ID,
        RegisterFunction::new_async(move |req: styles::StylesWriteInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let backend_id = session.resolve_ref_or_err(&req.r#ref)?;

                let resolved = session
                    .page
                    .execute(
                        cdp_dom::ResolveNodeParams::builder()
                            .backend_node_id(cdp_dom::BackendNodeId::new(backend_id))
                            .build(),
                    )
                    .await
                    .map_err(|e| handler_err(format!("node resolve failed: {e}")))?;
                let object_id = resolved
                    .object
                    .object_id
                    .clone()
                    .ok_or_else(|| handler_err("element has no JS object"))?;

                let priority = if req.important.unwrap_or(false) {
                    "important"
                } else {
                    ""
                };
                let call = cdp_rt::CallFunctionOnParams::builder()
                    .function_declaration(
                        "function(p, v, imp) { if (v === '') { this.style.removeProperty(p); } \
                         else { this.style.setProperty(p, v, imp); } \
                         return this.getAttribute('style') || ''; }",
                    )
                    .object_id(object_id)
                    .argument(cdp_rt::CallArgument {
                        value: Some(serde_json::Value::String(req.property.clone())),
                        unserializable_value: None,
                        object_id: None,
                    })
                    .argument(cdp_rt::CallArgument {
                        value: Some(serde_json::Value::String(req.value.clone())),
                        unserializable_value: None,
                        object_id: None,
                    })
                    .argument(cdp_rt::CallArgument {
                        value: Some(serde_json::Value::String(priority.to_string())),
                        unserializable_value: None,
                        object_id: None,
                    })
                    .return_by_value(true)
                    .build()
                    .map_err(handler_err)?;
                let result = session
                    .page
                    .execute(call)
                    .await
                    .map_err(|e| handler_err(format!("style write failed: {e}")))?;
                if let Some(details) = &result.exception_details {
                    return Err(handler_err(format!("style write threw: {}", details.text)));
                }
                let inline_style = result
                    .result
                    .result
                    .value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                session.touch();
                Ok::<_, Error>(styles::StylesWriteOutput {
                    ok: true,
                    inline_style,
                })
            }
        })
        .description(STYLES_WRITE_DESC),
    );
}

fn register_pick_hint(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        PICK_HINT_ID,
        RegisterFunction::new_async(move |req: hint::PickHintInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;

                let miss = hint::PickHintOutput {
                    hit: false,
                    tag: None,
                    id: None,
                    classes: None,
                    bounds: None,
                };
                let located = match session
                    .page
                    .execute(cdp_dom::GetNodeForLocationParams::new(
                        req.x as i64,
                        req.y as i64,
                    ))
                    .await
                {
                    Ok(node) => node,
                    Err(_) => return Ok::<_, Error>(miss),
                };
                let backend_id = *located.backend_node_id.inner();

                // describeNode and getBoxModel both depend only on
                // backend_id and not on each other; join them so a hint at
                // the 120ms cursor cadence pays one round-trip, not two.
                let (described, box_model) = futures::join!(
                    session.page.execute(
                        cdp_dom::DescribeNodeParams::builder()
                            .backend_node_id(cdp_dom::BackendNodeId::new(backend_id))
                            .build(),
                    ),
                    session.page.execute(
                        cdp_dom::GetBoxModelParams::builder()
                            .backend_node_id(cdp_dom::BackendNodeId::new(backend_id))
                            .build(),
                    ),
                );
                let described =
                    described.map_err(|e| handler_err(format!("describe node failed: {e}")))?;
                let node = &described.node;
                let (id, classes) =
                    crate::session::id_and_classes(node.attributes.as_deref().unwrap_or(&[]), 120);
                let bounds = box_model
                    .ok()
                    .map(|r| crate::session::quad_bounds(&r.model.content));

                Ok::<_, Error>(hint::PickHintOutput {
                    hit: true,
                    tag: Some(node.node_name.to_lowercase()),
                    id,
                    classes,
                    bounds,
                })
            }
        })
        .description(PICK_HINT_DESC)
        .metadata(json!({ "internal": true })),
    );
}

fn register_screencast_start(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SCREENCAST_START_ID,
        RegisterFunction::new_async(move |req: frame::ScreencastStartInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let cfg = sx.config.load();
                // Cap the push rate: the only consumer is the console's
                // ~150ms poll (~6.7fps), so pushing a JPEG per compositor
                // frame (up to ~60fps) would decode and discard most frames
                // unread. Every 4th frame keeps a ~15fps ceiling, invisible
                // at the poll cadence.
                let params = cdp_page::StartScreencastParams::builder()
                    .format(cdp_page::StartScreencastFormat::Jpeg)
                    .quality(cfg.screenshot_quality as i64)
                    .every_nth_frame(4)
                    .build();
                session
                    .page
                    .execute(params)
                    .await
                    .map_err(|e| handler_err(format!("screencast start failed: {e}")))?;
                session
                    .screencast_active
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                Ok::<_, Error>(pick::PickOutput { ok: true })
            }
        })
        .description(SCREENCAST_START_DESC)
        .metadata(json!({ "internal": true })),
    );
}

fn register_screencast_stop(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SCREENCAST_STOP_ID,
        RegisterFunction::new_async(move |req: frame::ScreencastStopInput| {
            let sx = sx.clone();
            async move {
                if let Some(session) = sx.get(&req.session_id) {
                    let _ = session
                        .page
                        .execute(cdp_page::StopScreencastParams::default())
                        .await;
                    session
                        .screencast_active
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                }
                Ok::<_, Error>(pick::PickOutput { ok: true })
            }
        })
        .description(SCREENCAST_STOP_DESC)
        .metadata(json!({ "internal": true })),
    );
}

fn register_frame(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        FRAME_ID,
        RegisterFunction::new_async(move |req: frame::FrameInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                let active = session
                    .screencast_active
                    .load(std::sync::atomic::Ordering::Relaxed);
                // Clone the Arc under the lock and release it before copying
                // the base64 payload, so the push-rate pump never waits
                // behind this poll-rate copy.
                let latest = {
                    let slot = session
                        .latest_frame
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    slot.as_ref()
                        .map(|l| (l.frame.clone(), l.seq, l.timestamp, l.width(), l.height()))
                };
                let output = match latest {
                    Some((frame, seq, timestamp, width, height)) => {
                        let unchanged = req.since_frame == Some(seq);
                        frame::FrameOutput {
                            frame: if unchanged {
                                None
                            } else {
                                let data: &str = frame.data.as_ref();
                                Some(data.to_string())
                            },
                            width,
                            height,
                            frame_seq: seq,
                            timestamp,
                            active,
                        }
                    }
                    None => frame::FrameOutput {
                        frame: None,
                        width: 0,
                        height: 0,
                        frame_seq: 0,
                        timestamp: 0,
                        active,
                    },
                };
                Ok::<_, Error>(output)
            }
        })
        .description(FRAME_DESC)
        .metadata(json!({ "internal": true })),
    );
}
