//! Function registration: the `browser::*` wire surface. One module per
//! function (or small family) holds the typed request/response structs;
//! registration lives here so `register_all` reads as the product surface.

pub mod act;
pub mod attach;
pub mod console;
pub mod doctor;
pub mod dom;
pub mod evaluate;
pub mod execute;
pub mod frame;
pub mod handoff;
pub mod hint;
pub mod history;
pub mod navigate;
pub mod network;
pub mod overlays;
pub mod pick;
pub mod recording;
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

use crate::events::{EventKind, HandoffRequestedEvent, SessionStartedEvent};
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
pub const SESSIONS_ATTACH_ID: &str = "browser::sessions::attach";
pub const SESSIONS_ATTACH_DESC: &str =
    "Attach a session to an already-running browser over CDP (start Chrome with \
     --remote-debugging-port). Opens a fresh tab the session owns, or adopts an existing user \
     tab by URL substring and releases it untouched on stop. Reaches the real profile with its \
     logins; disabled unless allow_attach is set in config.";
pub const TABS_LIST_ID: &str = "browser::tabs::list";
pub const TABS_LIST_DESC: &str =
    "List the open tabs of a running browser reachable at a CDP endpoint (url, title, and \
     whether a session already adopted each). Read-only; adopt one with \
     browser::sessions::attach.";
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
pub const EXECUTE_ID: &str = "browser::execute";
pub const EXECUTE_DESC: &str =
    "Run a multi-step async JavaScript script in the page: top-level await and return work, \
     with log(...), sleep(ms), waitFor(selector), and a state object that persists across \
     execute calls for the session. One call replaces a chain of act/evaluate round-trips; \
     returns { result, logs, state }.";
pub const DOCTOR_ID: &str = "browser::doctor";
pub const DOCTOR_DESC: &str =
    "Read-only environment diagnostics: which Chromium the worker would launch, its version, \
     session capacity, and any degraded capability with how to enable it. Never starts a \
     browser.";
pub const HANDOFF_ID: &str = "browser::handoff";
pub const HANDOFF_DESC: &str =
    "Pause a session for a step only a human can do (CAPTCHA, 2FA, payment): show an in-page \
     continue banner and block until the human clicks it, a browser::handoff::confirm call \
     resolves it, or the timeout elapses. Human acknowledgment is not proof — verify the \
     expected page state after it returns.";
pub const HANDOFF_CONFIRM_ID: &str = "browser::handoff::confirm";
pub const HANDOFF_CONFIRM_DESC: &str =
    "Resolve a paused browser::handoff by handoff_id, or the one pending handoff for a \
     session_id. The console calls this when the human confirms outside the page.";
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
pub const RECORDING_START_ID: &str = "browser::recording::start";
pub const RECORDING_START_DESC: &str =
    "Record a session's live viewport to a video file (webm or mp4) by piping the screencast \
     through ffmpeg. Turns screencast on if needed. Requires ffmpeg on PATH; browser::doctor \
     reports whether it is available.";
pub const RECORDING_STOP_ID: &str = "browser::recording::stop";
pub const RECORDING_STOP_DESC: &str =
    "Stop a session's recording, finalize the file, and return its path, duration, and frame \
     count. Idempotent: stopping when nothing is recording returns ok=false.";
pub const PICK_HINT_ID: &str = "browser::pick::hint";
pub const PICK_HINT_DESC: &str =
    "Internal: element preview at a viewport point (tag, id, classes, bounds) so the console \
     UI can draw a hover highlight in pick mode. Not an agent function.";
pub const PICK_START_ID: &str = "browser::pick::start";
pub const PICK_START_DESC: &str =
    "Internal: enter pick mode so the human can select an element in the console UI. Not an \
     agent function.";
pub const PICK_RESOLVE_ID: &str = "browser::pick::resolve";
pub const PICK_RESOLVE_DESC: &str =
    "Internal: resolve the element at a clicked viewport point and emit browser::picked. The \
     console calls this on a pick-mode click. Not an agent function.";
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
        spec::<attach::AttachInput, attach::AttachOutput>(SESSIONS_ATTACH_ID, SESSIONS_ATTACH_DESC),
        spec::<attach::TabsListInput, attach::TabsListOutput>(TABS_LIST_ID, TABS_LIST_DESC),
        spec::<doctor::DoctorInput, doctor::DoctorOutput>(DOCTOR_ID, DOCTOR_DESC),
        spec::<navigate::NavigateInput, navigate::NavigateOutput>(NAVIGATE_ID, NAVIGATE_DESC),
        spec::<snapshot::SnapshotInput, snapshot::SnapshotOutput>(SNAPSHOT_ID, SNAPSHOT_DESC),
        spec::<screenshot::ScreenshotInput, screenshot::ScreenshotOutput>(
            SCREENSHOT_ID,
            SCREENSHOT_DESC,
        ),
        spec::<act::ActInput, act::ActOutput>(ACT_ID, ACT_DESC),
        spec::<evaluate::EvaluateInput, evaluate::EvaluateOutput>(EVALUATE_ID, EVALUATE_DESC),
        spec::<execute::ExecuteInput, execute::ExecuteOutput>(EXECUTE_ID, EXECUTE_DESC),
        spec::<handoff::HandoffInput, handoff::HandoffOutput>(HANDOFF_ID, HANDOFF_DESC),
        spec::<handoff::HandoffConfirmInput, handoff::HandoffConfirmOutput>(
            HANDOFF_CONFIRM_ID,
            HANDOFF_CONFIRM_DESC,
        ),
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
        spec::<frame::ScreencastStartInput, pick::AckOutput>(
            SCREENCAST_START_ID,
            SCREENCAST_START_DESC,
        ),
        spec::<frame::ScreencastStopInput, pick::AckOutput>(
            SCREENCAST_STOP_ID,
            SCREENCAST_STOP_DESC,
        ),
        spec::<frame::FrameInput, frame::FrameOutput>(FRAME_ID, FRAME_DESC),
        spec::<recording::RecordingStartInput, recording::RecordingStartOutput>(
            RECORDING_START_ID,
            RECORDING_START_DESC,
        ),
        spec::<recording::RecordingStopInput, recording::RecordingStopOutput>(
            RECORDING_STOP_ID,
            RECORDING_STOP_DESC,
        ),
        spec::<hint::PickHintInput, hint::PickHintOutput>(PICK_HINT_ID, PICK_HINT_DESC),
        spec::<pick::PickStartInput, pick::AckOutput>(PICK_START_ID, PICK_START_DESC),
        spec::<pick::PickResolveInput, pick::AckOutput>(PICK_RESOLVE_ID, PICK_RESOLVE_DESC),
        spec::<pick::PickStopInput, pick::AckOutput>(PICK_STOP_ID, PICK_STOP_DESC),
    ]
}

pub fn register_all(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    register_sessions_start(iii, sessions);
    register_sessions_list(iii, sessions);
    register_sessions_stop(iii, sessions);
    register_sessions_attach(iii, sessions);
    register_tabs_list(iii, sessions);
    register_doctor(iii, sessions);
    register_navigate(iii, sessions);
    register_snapshot(iii, sessions);
    register_screenshot(iii, sessions);
    register_act(iii, sessions);
    register_evaluate(iii, sessions);
    register_execute(iii, sessions);
    register_handoff(iii, sessions);
    register_handoff_confirm(iii, sessions);
    register_console_read(iii, sessions);
    register_network_read(iii, sessions);
    register_history(iii, sessions);
    register_dom_read(iii, sessions);
    register_styles_read(iii, sessions);
    register_styles_write(iii, sessions);
    register_screencast_start(iii, sessions);
    register_screencast_stop(iii, sessions);
    register_frame(iii, sessions);
    register_recording_start(iii, sessions);
    register_recording_stop(iii, sessions);
    register_pick_hint(iii, sessions);
    register_pick_start(iii, sessions);
    register_pick_resolve(iii, sessions);
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

/// Read-only guard shared by every interaction function. Inspection and
/// navigation stay available on a read-only session; anything that dispatches
/// input or runs script is rejected here.
fn ensure_writable(session: &Session, what: &str) -> Result<(), Error> {
    if session.read_only {
        return Err(handler_err(format!(
            "session '{}' is read-only: {what} is rejected. Navigation, snapshot, dom/styles \
             reads, console/network reads, and screenshots still work; start a writable session \
             with browser::sessions::start.",
            session.id
        )));
    }
    Ok(())
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
                let session = sx
                    .start(req.url, req.headful, req.read_only.unwrap_or(false))
                    .await
                    .map_err(handler_err)?;
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
                    read_only: session.read_only,
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
                            read_only: session.read_only,
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

fn register_sessions_attach(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SESSIONS_ATTACH_ID,
        RegisterFunction::new_async(move |req: attach::AttachInput| {
            let sx = sx.clone();
            async move {
                let cfg = sx.config.load_full();
                if !cfg.allow_attach {
                    return Err(handler_err(
                        "attach mode is disabled; set `allow_attach: true` in the browser worker \
                         config to connect to a running browser. It reaches the real profile with \
                         its logins, so it is opt-in.",
                    ));
                }
                if let Some(url) = &req.url {
                    sessions::check_scheme(&cfg, url).map_err(handler_err)?;
                }
                let session = sx
                    .attach(
                        req.cdp_url,
                        req.url,
                        req.adopt_url_substring,
                        req.read_only.unwrap_or(false),
                    )
                    .await
                    .map_err(handler_err)?;
                let adopted = matches!(
                    session.kind,
                    crate::session::SessionKind::Attached { owns_page: false }
                );
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
                Ok::<_, Error>(attach::AttachOutput {
                    session_id: session.id.clone(),
                    url,
                    read_only: session.read_only,
                    adopted,
                })
            }
        })
        .description(SESSIONS_ATTACH_DESC),
    );
}

fn register_tabs_list(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        TABS_LIST_ID,
        RegisterFunction::new_async(move |req: attach::TabsListInput| {
            let sx = sx.clone();
            async move {
                if !sx.config.load().allow_attach {
                    return Err(handler_err(
                        "attach mode is disabled; set `allow_attach: true` in the browser worker \
                         config to inspect a running browser's tabs.",
                    ));
                }
                let tabs = sx.list_tabs(&req.cdp_url).await.map_err(handler_err)?;
                Ok::<_, Error>(attach::TabsListOutput { tabs })
            }
        })
        .description(TABS_LIST_DESC),
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

                let result = crate::snapshot::serialize(
                    &tree.nodes,
                    cfg.max_snapshot_nodes as usize,
                    &session.ref_counter,
                );
                session.append_refs(result.refs);

                // Swap in this snapshot's keys as the next diff baseline,
                // taking the previous baseline out in the same lock.
                let previous_keys = {
                    let mut baseline = session
                        .snapshot_keys
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    baseline.replace(result.keys.clone())
                };

                let diff = match (req.diff.unwrap_or(false), previous_keys) {
                    (true, Some(previous)) => {
                        let d = crate::snapshot::diff_keys(&previous, &result.keys);
                        Some(snapshot::SnapshotDiff {
                            added: d
                                .added
                                .iter()
                                .filter_map(|&i| result.lines.get(i).cloned())
                                .collect(),
                            removed: d.removed,
                            unchanged: d.unchanged,
                        })
                    }
                    _ => None,
                };

                let url = session.page.url().await.ok().flatten().unwrap_or_default();
                let title = session.page.get_title().await.ok().flatten();
                Ok::<_, Error>(snapshot::SnapshotOutput {
                    url,
                    title,
                    tree: if diff.is_some() {
                        String::new()
                    } else {
                        result.tree
                    },
                    truncated: result.truncated,
                    generation: session.generation(),
                    diff,
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

/// Best-effort: draw the ghost cursor at the acted point so a human watching
/// the streamed viewport sees where the agent acted. Only injected while
/// screencast is active (someone is watching); a failed injection is ignored.
async fn move_ghost_cursor(session: &Session, x: f64, y: f64, click: bool) {
    if session
        .screencast_active
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        let _ = session
            .page
            .evaluate(overlays::ghost_cursor_script(x, y, click))
            .await;
    }
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
                ensure_writable(&session, "browser::act")?;
                session.touch();

                let detail = match req.action.as_str() {
                    "click" => {
                        let (x, y) = action_point(&session, &req).await?;
                        let button = req.button.as_deref().unwrap_or("left");
                        let clicks = i64::from(req.click_count.unwrap_or(1));
                        dispatch_click(&session, x, y, button, clicks).await?;
                        move_ghost_cursor(&session, x, y, true).await;
                        format!("clicked {button} x{clicks} at ({x:.0}, {y:.0})")
                    }
                    "hover" => {
                        let (x, y) = action_point(&session, &req).await?;
                        dispatch_hover(&session, x, y).await?;
                        move_ghost_cursor(&session, x, y, false).await;
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
                ensure_writable(&session, "browser::evaluate")?;
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

fn register_execute(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        EXECUTE_ID,
        RegisterFunction::new_async(move |req: execute::ExecuteInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                ensure_writable(&session, "browser::execute")?;
                session.touch();
                let cfg = sx.config.load();
                let wait = Duration::from_millis(cfg.clamp_timeout(req.timeout_ms));

                let state_json = {
                    let state = session.exec_state.lock().unwrap_or_else(|p| p.into_inner());
                    serde_json::to_string(&*state).unwrap_or_else(|_| "{}".to_string())
                };
                let wrapped = execute::wrap_code(&req.code, &state_json);

                let params = cdp_rt::EvaluateParams::builder()
                    .expression(wrapped)
                    .await_promise(true)
                    .return_by_value(true)
                    .build()
                    .map_err(handler_err)?;
                let evaluated = timeout(wait, session.page.execute(params)).await;
                session.touch();

                let current_state = || {
                    session
                        .exec_state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone()
                };
                let response = match evaluated {
                    Err(_) => {
                        return Ok::<_, Error>(execute::ExecuteOutput {
                            ok: false,
                            result: None,
                            error: Some(format!(
                                "script timed out after {}ms; state was not updated",
                                wait.as_millis()
                            )),
                            logs: Vec::new(),
                            state: current_state(),
                        })
                    }
                    Ok(Err(e)) => {
                        let text = e.to_string();
                        let hint = if text.contains("context") {
                            "; the execution context was destroyed, which usually means the \
                             script navigated — split the script at the navigation boundary"
                        } else {
                            ""
                        };
                        return Ok(execute::ExecuteOutput {
                            ok: false,
                            result: None,
                            error: Some(format!("script failed: {text}{hint}")),
                            logs: Vec::new(),
                            state: current_state(),
                        });
                    }
                    Ok(Ok(r)) => r,
                };

                if let Some(details) = &response.exception_details {
                    return Ok(execute::ExecuteOutput {
                        ok: false,
                        result: None,
                        error: Some(format!("script threw: {}", details.text)),
                        logs: Vec::new(),
                        state: current_state(),
                    });
                }
                let raw = response
                    .result
                    .result
                    .value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| handler_err("script returned no envelope"))?;
                if raw.len() > execute::MAX_ENVELOPE_BYTES {
                    return Ok(execute::ExecuteOutput {
                        ok: false,
                        result: None,
                        error: Some(format!(
                            "script result is {} bytes (cap {}); return a summary, not a dump",
                            raw.len(),
                            execute::MAX_ENVELOPE_BYTES
                        )),
                        logs: Vec::new(),
                        state: current_state(),
                    });
                }
                let envelope: execute::Envelope = serde_json::from_str(raw)
                    .map_err(|e| handler_err(format!("script envelope did not parse: {e}")))?;

                if envelope.state.is_object() {
                    *session.exec_state.lock().unwrap_or_else(|p| p.into_inner()) =
                        envelope.state.clone();
                }
                Ok(execute::ExecuteOutput {
                    ok: envelope.ok,
                    result: envelope.result,
                    error: envelope.error,
                    logs: envelope.logs,
                    state: envelope.state,
                })
            }
        })
        .description(EXECUTE_DESC),
    );
}

fn register_handoff(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        HANDOFF_ID,
        RegisterFunction::new_async(move |req: handoff::HandoffInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                let cfg = sx.config.load_full();
                let wait = Duration::from_millis(cfg.clamp_timeout(req.timeout_ms));

                let (handoff_id, mut confirm_rx) = sx.register_handoff(&req.session_id);
                // Mount the in-page continue banner carrying the poll flag.
                // Best-effort: on a page that rejects injection the
                // confirm-call path still resolves the same handoff.
                let _ = session
                    .page
                    .evaluate(handoff::banner_script(&handoff_id, &req.instructions))
                    .await;

                sx.emitter
                    .emit(
                        EventKind::HandoffRequested,
                        &req.session_id,
                        &HandoffRequestedEvent {
                            session_id: req.session_id.clone(),
                            handoff_id: handoff_id.clone(),
                            instructions: req.instructions.clone(),
                            timestamp: now_ms(),
                        },
                    )
                    .await;

                // Park until: a confirm call fires the oneshot, the human
                // clicks the in-page control (polled), or the timeout fires.
                let poll = handoff::poll_script(&handoff_id);
                let deadline = tokio::time::Instant::now() + wait;
                let via = loop {
                    let tick = tokio::time::sleep(Duration::from_millis(handoff::POLL_INTERVAL_MS));
                    tokio::select! {
                        // Ok = a confirm call fired the sender; Err = the
                        // sender was dropped (session stopped mid-handoff).
                        res = &mut confirm_rx => break if res.is_ok() { "confirm_call" } else { "cancelled" },
                        _ = tokio::time::sleep_until(deadline) => break "timeout",
                        _ = tick => {
                            if let Ok(v) = session.page.evaluate(poll.clone()).await {
                                if v.value().and_then(|x| x.as_bool()).unwrap_or(false) {
                                    break "in_page";
                                }
                            }
                        }
                    }
                };

                sx.drop_handoff(&handoff_id);
                let _ = session
                    .page
                    .evaluate(handoff::remove_script(&handoff_id))
                    .await;
                session.touch();
                let url = session.page.url().await.ok().flatten().unwrap_or_default();
                Ok::<_, Error>(handoff::HandoffOutput {
                    confirmed: via == "confirm_call" || via == "in_page",
                    handoff_id,
                    via: via.to_string(),
                    url,
                })
            }
        })
        .description(HANDOFF_DESC),
    );
}

fn register_handoff_confirm(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        HANDOFF_CONFIRM_ID,
        RegisterFunction::new_async(move |req: handoff::HandoffConfirmInput| {
            let sx = sx.clone();
            async move {
                if req.handoff_id.is_none() && req.session_id.is_none() {
                    return Err(handler_err("pass handoff_id or session_id"));
                }
                let resolved =
                    sx.confirm_handoff(req.handoff_id.as_deref(), req.session_id.as_deref());
                Ok::<_, Error>(handoff::HandoffConfirmOutput {
                    ok: resolved.is_some(),
                    handoff_id: resolved,
                })
            }
        })
        .description(HANDOFF_CONFIRM_DESC),
    );
}

fn register_doctor(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        DOCTOR_ID,
        RegisterFunction::new_async(move |_req: doctor::DoctorInput| {
            let sx = sx.clone();
            async move {
                let cfg = sx.config.load_full();
                let mut issues = Vec::new();

                let chromium_path = doctor::detect_chromium(&cfg);
                let chromium_version = match &chromium_path {
                    Some(path) => {
                        let path = path.clone();
                        tokio::task::spawn_blocking(move || doctor::chromium_version(&path))
                            .await
                            .ok()
                            .flatten()
                    }
                    None => None,
                };
                if chromium_path.is_none() {
                    issues.push(doctor::DoctorIssue {
                        what: if cfg.executable.is_empty() {
                            "no Chromium/Chrome install found".to_string()
                        } else {
                            format!("configured executable '{}' does not exist", cfg.executable)
                        },
                        enable_how: "install Google Chrome or Chromium, or point the worker \
                                     config `executable` at a browser binary"
                            .to_string(),
                    });
                }

                let active_sessions = sx.count() as u64;
                if active_sessions >= cfg.max_sessions {
                    issues.push(doctor::DoctorIssue {
                        what: format!(
                            "session capacity reached ({active_sessions}/{})",
                            cfg.max_sessions
                        ),
                        enable_how: "stop a session with browser::sessions::stop, or raise \
                                     `max_sessions` in the worker config"
                            .to_string(),
                    });
                }

                let recording_available = tokio::task::spawn_blocking(doctor::ffmpeg_available)
                    .await
                    .unwrap_or(false);
                if !recording_available {
                    issues.push(doctor::DoctorIssue {
                        what: "ffmpeg not found; browser::recording is unavailable".to_string(),
                        enable_how: "install ffmpeg and put it on PATH".to_string(),
                    });
                }

                Ok::<_, Error>(doctor::DoctorOutput {
                    ok: chromium_path.is_some() && active_sessions < cfg.max_sessions,
                    chromium_path: chromium_path.map(|p| p.display().to_string()),
                    chromium_version,
                    headless_default: cfg.headless,
                    max_sessions: cfg.max_sessions,
                    active_sessions,
                    allowed_schemes: cfg.allowed_schemes.clone(),
                    attach_enabled: cfg.allow_attach,
                    recording_available,
                    issues,
                })
            }
        })
        .description(DOCTOR_DESC),
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
                // Enable the DOM domain so getNodeForLocation can hit-test.
                // The pick itself is resolved by browser::pick::resolve from
                // the click coordinates, so no DevTools inspect mode is
                // needed (and none is wanted: it would draw a second overlay
                // the console already draws itself, and its synthesized-click
                // hit-test is unreliable in headless).
                let _ = session.page.execute(cdp_dom::EnableParams::default()).await;
                Ok::<_, Error>(pick::AckOutput { ok: true })
            }
        })
        .description(PICK_START_DESC)
        .metadata(json!({ "internal": true })),
    );
}

fn register_pick_resolve(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        PICK_RESOLVE_ID,
        RegisterFunction::new_async(move |req: pick::PickResolveInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id)?;
                session.touch();
                crate::session::resolve_pick_at(&sx, &session, req.x, req.y)
                    .await
                    .map_err(handler_err)?;
                Ok::<_, Error>(pick::AckOutput { ok: true })
            }
        })
        .description(PICK_RESOLVE_DESC)
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
                Ok::<_, Error>(pick::AckOutput { ok: true })
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
                ensure_writable(&session, "browser::styles::write")?;
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
                // every_nth_frame counts COMPOSITOR frames, not wall-clock, so
                // a value > 1 starves a static page (which may never produce
                // that many repaints and would then emit no frame at all). Take
                // every frame and cap the push rate by time in the pump
                // instead (see FRAME_MIN_INTERVAL_MS).
                let params = cdp_page::StartScreencastParams::builder()
                    .format(cdp_page::StartScreencastFormat::Jpeg)
                    .quality(cfg.screenshot_quality as i64)
                    .every_nth_frame(1)
                    .build();
                session
                    .page
                    .execute(params)
                    .await
                    .map_err(|e| handler_err(format!("screencast start failed: {e}")))?;
                session
                    .screencast_active
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                // A human is now watching: show the session-status badge.
                let mode = if session.read_only {
                    "read-only"
                } else {
                    "active"
                };
                let _ = session
                    .page
                    .evaluate(overlays::badge_script(&session.id, mode))
                    .await;
                Ok::<_, Error>(pick::AckOutput { ok: true })
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
                    // Nobody is watching now: remove the status badge and
                    // ghost cursor so they do not linger in the page.
                    let _ = session.page.evaluate(overlays::remove_badge_script()).await;
                    let _ = session
                        .page
                        .evaluate(overlays::remove_ghost_cursor_script())
                        .await;
                    // Drop the last frame from the stream so a later
                    // subscriber does not see a stale image.
                    let _ = sx
                        .iii
                        .trigger(iii_sdk::protocol::TriggerRequest {
                            function_id: "stream::delete".to_string(),
                            payload: json!({
                                "stream_name": crate::session::FRAMES_STREAM,
                                "group_id": req.session_id,
                            }),
                            action: None,
                            timeout_ms: Some(5_000),
                        })
                        .await;
                }
                Ok::<_, Error>(pick::AckOutput { ok: true })
            }
        })
        .description(SCREENCAST_STOP_DESC)
        .metadata(json!({ "internal": true })),
    );
}

fn register_recording_start(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        RECORDING_START_ID,
        RegisterFunction::new_async(move |req: recording::RecordingStartInput| {
            let sx = sx.clone();
            async move {
                get_session(&sx, &req.session_id)?;
                let (format, codec) =
                    recording::resolve_format(req.format.as_deref()).map_err(handler_err)?;
                sx.start_recording(&req.session_id, &req.path, codec)
                    .await
                    .map_err(handler_err)?;
                Ok::<_, Error>(recording::RecordingStartOutput {
                    ok: true,
                    path: req.path,
                    format: format.to_string(),
                })
            }
        })
        .description(RECORDING_START_DESC),
    );
}

fn register_recording_stop(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        RECORDING_STOP_ID,
        RegisterFunction::new_async(move |req: recording::RecordingStopInput| {
            let sx = sx.clone();
            async move {
                let output = match sx.stop_recording(&req.session_id).await {
                    Some((path, duration_ms, frames)) => recording::RecordingStopOutput {
                        ok: true,
                        path: Some(path),
                        duration_ms,
                        frames,
                    },
                    None => recording::RecordingStopOutput {
                        ok: false,
                        path: None,
                        duration_ms: 0,
                        frames: 0,
                    },
                };
                Ok::<_, Error>(output)
            }
        })
        .description(RECORDING_STOP_DESC),
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
