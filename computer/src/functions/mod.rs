//! Function registration: the `computer::*` wire surface. One module per
//! function (or small family) holds the typed request/response structs;
//! registration lives here so `register_all` reads as the product surface.

pub mod act;
pub mod displays;
pub mod frame;
pub mod observe;
pub mod screenshot;
pub mod sessions;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::driver::Screen;
use crate::session::{Session, Sessions};

pub const SESSIONS_START_ID: &str = "computer::sessions::start";
pub const SESSIONS_START_DESC: &str =
    "Start a computer-use session and return its session_id. Pass `image` to boot a fresh desktop \
     in an iii-sandbox microVM (fixed virtual display, no host setup), an `endpoint` to drive a \
     desktop through its guest executor, or omit both to drive the local machine. Sessions are \
     durable; stop them with computer::sessions::stop when done.";
pub const SESSIONS_LIST_ID: &str = "computer::sessions::list";
pub const SESSIONS_LIST_DESC: &str =
    "List live computer sessions with their endpoint, guest OS, and screen size.";
pub const SESSIONS_STOP_ID: &str = "computer::sessions::stop";
pub const SESSIONS_STOP_DESC: &str =
    "Stop a computer session and close its driver connection. Idempotent: stopping an unknown \
     or already-stopped session succeeds with was_running=false.";
pub const DISPLAYS_ID: &str = "computer::displays";
pub const DISPLAYS_DESC: &str =
    "List the local displays (index, name, primary, size). Pass the chosen index as `monitor` \
     to computer::sessions::start to drive that display; omit it to use the display under the \
     cursor. Native host only.";
pub const SCREENSHOT_ID: &str = "computer::screenshot";
pub const SCREENSHOT_DESC: &str =
    "Capture the desktop as a viewable image. This is how you see the screen before acting; the \
     coordinate space of computer::act is this image's pixels (top-left origin).";
pub const OBSERVE_ID: &str = "computer::observe";
pub const OBSERVE_DESC: &str =
    "Capture the desktop plus, optionally, the accessibility tree. Use include_a11y on macOS \
     guests for a machine-readable element tree; elsewhere prefer computer::screenshot.";
pub const ACT_ID: &str = "computer::act";
pub const ACT_DESC: &str =
    "Drive the desktop: click, right_click, double_click, move, drag, scroll, type, press, or \
     hotkey. Address by pixel coordinates read off the screenshot (top-left origin).";
pub const SCREENCAST_START_ID: &str = "computer::screencast::start";
pub const SCREENCAST_START_DESC: &str =
    "Internal: start pushing live desktop frames onto the computer:frames stream for the console \
     viewport. Console-UI plumbing; agents use computer::screenshot. Not an agent function.";
pub const SCREENCAST_STOP_ID: &str = "computer::screencast::stop";
pub const SCREENCAST_STOP_DESC: &str =
    "Internal: stop the live frame push. Idempotent. Not an agent function.";
pub const FRAME_ID: &str = "computer::frame";
pub const FRAME_DESC: &str =
    "Internal: newest screencast frame, or nothing when since_frame is still current. No capture \
     round-trip; poll fast. Not an agent function.";

/// One wire-surface entry: everything the golden schema test pins.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Build a schema exactly the way iii-sdk does at registration, so the catalog
/// snapshot pins what registration emits.
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
        spec::<displays::DisplaysInput, displays::DisplaysOutput>(DISPLAYS_ID, DISPLAYS_DESC),
        spec::<screenshot::ScreenshotInput, screenshot::ScreenshotOutput>(
            SCREENSHOT_ID,
            SCREENSHOT_DESC,
        ),
        spec::<observe::ObserveInput, observe::ObserveOutput>(OBSERVE_ID, OBSERVE_DESC),
        spec::<act::ActInput, act::ActOutput>(ACT_ID, ACT_DESC),
        spec::<frame::ScreencastStartInput, frame::AckOutput>(
            SCREENCAST_START_ID,
            SCREENCAST_START_DESC,
        ),
        spec::<frame::ScreencastStopInput, frame::AckOutput>(
            SCREENCAST_STOP_ID,
            SCREENCAST_STOP_DESC,
        ),
        spec::<frame::FrameInput, frame::FrameOutput>(FRAME_ID, FRAME_DESC),
    ]
}

pub fn register_all(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    register_sessions_start(iii, sessions);
    register_sessions_list(iii, sessions);
    register_sessions_stop(iii, sessions);
    register_displays(iii);
    register_screenshot(iii, sessions);
    register_observe(iii, sessions);
    register_act(iii, sessions);
    register_screencast_start(iii, sessions);
    register_screencast_stop(iii, sessions);
    register_frame(iii, sessions);
}

// ---------------------------------------------------------------------------
// Handler helpers
// ---------------------------------------------------------------------------

async fn get_session(sessions: &Arc<Sessions>, id: &str) -> Result<Arc<Session>, Error> {
    sessions.get(id).await.ok_or_else(|| {
        Error::Handler(format!(
            "unknown session '{id}' (start one with computer::sessions::start)"
        ))
    })
}

fn require_xy(req: &act::ActInput) -> Result<(i64, i64), Error> {
    match (req.x, req.y) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(Error::Handler(format!(
            "action '{}' needs x and y",
            req.action
        ))),
    }
}

fn xy_or_center(req: &act::ActInput, screen: Screen) -> (i64, i64) {
    (
        req.x.unwrap_or(screen.width as i64 / 2),
        req.y.unwrap_or(screen.height as i64 / 2),
    )
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Local displays, enumerated live from the OS. Empty on non-desktop targets.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn host_displays() -> Vec<crate::driver::DisplayInfo> {
    crate::driver::native::list_displays().unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn host_displays() -> Vec<crate::driver::DisplayInfo> {
    Vec::new()
}

fn register_displays(iii: &Arc<IIIClient>) {
    iii.register_function(
        DISPLAYS_ID,
        RegisterFunction::new_async(|_req: displays::DisplaysInput| async move {
            Ok::<_, Error>(displays::DisplaysOutput {
                displays: host_displays(),
            })
        })
        .description(DISPLAYS_DESC),
    );
}

fn register_sessions_start(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        SESSIONS_START_ID,
        RegisterFunction::new_async(move |req: sessions::StartInput| {
            let sessions = sessions.clone();
            async move {
                let session = sessions
                    .start(req.image, req.endpoint, req.os, req.monitor)
                    .await
                    .map_err(Error::Handler)?;
                Ok::<_, Error>(sessions::StartOutput {
                    session_id: session.id.clone(),
                    endpoint: session.endpoint.clone(),
                    os: session.os.clone(),
                    screen: session.screen,
                })
            }
        })
        .description(SESSIONS_START_DESC),
    );
}

fn register_sessions_list(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        SESSIONS_LIST_ID,
        RegisterFunction::new_async(move |_req: sessions::ListInput| {
            let sessions = sessions.clone();
            async move {
                let infos = sessions
                    .list()
                    .await
                    .iter()
                    .map(|s| sessions::SessionInfo {
                        session_id: s.id.clone(),
                        endpoint: s.endpoint.clone(),
                        os: s.os.clone(),
                        screen: s.screen,
                        created_ms: s.created_ms,
                        last_used_ms: s.last_used_ms(),
                        screencast_active: s.screencast_active(),
                    })
                    .collect();
                Ok::<_, Error>(sessions::ListOutput { sessions: infos })
            }
        })
        .description(SESSIONS_LIST_DESC),
    );
}

fn register_sessions_stop(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        SESSIONS_STOP_ID,
        RegisterFunction::new_async(move |req: sessions::StopInput| {
            let sessions = sessions.clone();
            async move {
                let was_running = sessions.stop(&req.session_id, "stopped").await;
                Ok::<_, Error>(sessions::StopOutput {
                    ok: true,
                    was_running,
                })
            }
        })
        .description(SESSIONS_STOP_DESC),
    );
}

fn register_screenshot(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        SCREENSHOT_ID,
        RegisterFunction::new_async(move |req: screenshot::ScreenshotInput| {
            let sessions = sessions.clone();
            async move {
                let session = get_session(&sessions, &req.session_id).await?;
                session.touch();
                let shot = session
                    .driver()
                    .screenshot()
                    .await
                    .map_err(Error::Handler)?;
                let bytes = shot.byte_len();
                Ok::<_, Error>(screenshot::ScreenshotOutput {
                    content: vec![
                        screenshot::ContentBlock::image(shot.mime.clone(), shot.to_base64()),
                        screenshot::ContentBlock::text(format!(
                            "Desktop of session {} ({}x{}, {bytes} bytes)",
                            session.id, session.screen.width, session.screen.height
                        )),
                    ],
                    details: screenshot::ScreenshotDetails {
                        session_id: session.id.clone(),
                        width: session.screen.width,
                        height: session.screen.height,
                        mime: shot.mime,
                    },
                })
            }
        })
        .description(SCREENSHOT_DESC),
    );
}

fn register_observe(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        OBSERVE_ID,
        RegisterFunction::new_async(move |req: observe::ObserveInput| {
            let sessions = sessions.clone();
            async move {
                let session = get_session(&sessions, &req.session_id).await?;
                session.touch();
                let driver = session.driver();
                let shot = driver.screenshot().await.map_err(Error::Handler)?;
                let accessibility = if req.include_a11y.unwrap_or(false) {
                    match driver.accessibility_tree().await {
                        Ok(v) if !v.is_null() => Some(v),
                        Ok(_) => None,
                        Err(e) => {
                            tracing::debug!(session = %session.id, error = %e, "observe: a11y tree unavailable");
                            None
                        }
                    }
                } else {
                    None
                };
                let bytes = shot.byte_len();
                let has_a11y = accessibility.is_some();
                Ok::<_, Error>(observe::ObserveOutput {
                    content: vec![
                        screenshot::ContentBlock::image(shot.mime.clone(), shot.to_base64()),
                        screenshot::ContentBlock::text(format!(
                            "Desktop of session {} ({}x{}, {bytes} bytes){}",
                            session.id,
                            session.screen.width,
                            session.screen.height,
                            if has_a11y {
                                ", accessibility tree attached"
                            } else {
                                ""
                            }
                        )),
                    ],
                    details: observe::ObserveDetails {
                        session_id: session.id.clone(),
                        screen: session.screen,
                        mime: shot.mime,
                    },
                    accessibility,
                })
            }
        })
        .description(OBSERVE_DESC),
    );
}

fn register_act(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        ACT_ID,
        RegisterFunction::new_async(move |req: act::ActInput| {
            let sessions = sessions.clone();
            async move {
                let session = get_session(&sessions, &req.session_id).await?;
                session.touch();
                let driver = session.driver();
                let detail = match req.action.as_str() {
                    "click" | "left_click" => {
                        let (x, y) = require_xy(&req)?;
                        driver.left_click(x, y).await.map_err(Error::Handler)?;
                        format!("left click at ({x}, {y})")
                    }
                    "right_click" => {
                        let (x, y) = require_xy(&req)?;
                        driver.right_click(x, y).await.map_err(Error::Handler)?;
                        format!("right click at ({x}, {y})")
                    }
                    "double_click" => {
                        let (x, y) = require_xy(&req)?;
                        driver.double_click(x, y).await.map_err(Error::Handler)?;
                        format!("double click at ({x}, {y})")
                    }
                    "move" | "move_cursor" => {
                        let (x, y) = require_xy(&req)?;
                        driver.move_cursor(x, y).await.map_err(Error::Handler)?;
                        format!("move to ({x}, {y})")
                    }
                    "scroll" => {
                        let (x, y) = xy_or_center(&req, session.screen);
                        let sx = req.scroll_x.unwrap_or(0);
                        let sy = req.scroll_y.unwrap_or(3);
                        driver.scroll(x, y, sx, sy).await.map_err(Error::Handler)?;
                        format!("scroll ({sx}, {sy}) at ({x}, {y})")
                    }
                    "drag" => {
                        let (x, y) = require_xy(&req)?;
                        let (tx, ty) = match (req.to_x, req.to_y) {
                            (Some(tx), Some(ty)) => (tx, ty),
                            _ => {
                                return Err(Error::Handler(
                                    "action 'drag' needs to_x and to_y".to_string(),
                                ))
                            }
                        };
                        let button = req.button.as_deref().unwrap_or("left");
                        driver
                            .drag((x, y), (tx, ty), button)
                            .await
                            .map_err(Error::Handler)?;
                        format!("drag ({x}, {y}) -> ({tx}, {ty})")
                    }
                    "type" => {
                        let text = req.text.as_deref().ok_or_else(|| {
                            Error::Handler("action 'type' needs text".to_string())
                        })?;
                        driver.type_text(text).await.map_err(Error::Handler)?;
                        format!("type {} chars", text.chars().count())
                    }
                    "press" | "hotkey" | "key" => {
                        let keys =
                            req.keys.as_ref().filter(|k| !k.is_empty()).ok_or_else(|| {
                                Error::Handler(format!(
                                    "action '{}' needs a non-empty keys array",
                                    req.action
                                ))
                            })?;
                        driver.keypress(keys).await.map_err(Error::Handler)?;
                        format!("press {}", keys.join("+"))
                    }
                    other => {
                        return Err(Error::Handler(format!(
                            "unknown action '{other}' (click, right_click, double_click, move, \
                             drag, scroll, type, press, hotkey)"
                        )))
                    }
                };
                Ok::<_, Error>(act::ActOutput { ok: true, detail })
            }
        })
        .description(ACT_DESC),
    );
}

fn register_screencast_start(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        SCREENCAST_START_ID,
        RegisterFunction::new_async(move |req: frame::ScreencastStartInput| {
            let sessions = sessions.clone();
            async move {
                let session = get_session(&sessions, &req.session_id).await?;
                session.start_screencast().await;
                Ok::<_, Error>(frame::AckOutput { ok: true })
            }
        })
        .description(SCREENCAST_START_DESC)
        .metadata(serde_json::json!({ "internal": true })),
    );
}

fn register_screencast_stop(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        SCREENCAST_STOP_ID,
        RegisterFunction::new_async(move |req: frame::ScreencastStopInput| {
            let sessions = sessions.clone();
            async move {
                if let Some(session) = sessions.get(&req.session_id).await {
                    session.stop_screencast().await;
                }
                Ok::<_, Error>(frame::AckOutput { ok: true })
            }
        })
        .description(SCREENCAST_STOP_DESC)
        .metadata(serde_json::json!({ "internal": true })),
    );
}

fn register_frame(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sessions = sessions.clone();
    iii.register_function(
        FRAME_ID,
        RegisterFunction::new_async(move |req: frame::FrameInput| {
            let sessions = sessions.clone();
            async move {
                let session = get_session(&sessions, &req.session_id).await?;
                let active = session.screencast_active();
                let out = match session.latest_frame() {
                    Some(f) => {
                        // Fast no-change poll copies only metadata; the base64
                        // is cloned once, only when a new frame is delivered.
                        let unchanged = req.since_frame == Some(f.frame_seq);
                        frame::FrameOutput {
                            frame: if unchanged {
                                None
                            } else {
                                Some(f.data_b64.clone())
                            },
                            mime: f.mime.clone(),
                            width: f.width,
                            height: f.height,
                            frame_seq: f.frame_seq,
                            timestamp: f.timestamp,
                            active,
                        }
                    }
                    None => frame::FrameOutput {
                        frame: None,
                        mime: "image/png".to_string(),
                        width: session.screen.width,
                        height: session.screen.height,
                        frame_seq: 0,
                        timestamp: 0,
                        active,
                    },
                };
                Ok::<_, Error>(out)
            }
        })
        .description(FRAME_DESC)
        .metadata(serde_json::json!({ "internal": true })),
    );
}
