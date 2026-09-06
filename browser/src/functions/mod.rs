//! Function registration: the `browser::*` wire surface. One module per
//! function (or small family) holds the typed request/response structs;
//! registration lives here so `register_all` reads as the product surface.

pub mod act;
pub mod attach;
pub mod clear_data;
pub mod console;
pub mod cookies;
pub mod doctor;
pub mod dom;
pub mod downloads;
pub mod evaluate;
pub mod execute;
pub mod find_in_page;
pub mod frame;
pub mod handoff;
pub mod hint;
pub mod history;
pub mod history_list;
pub mod navigate;
pub mod network;
pub mod overlays;
pub mod pdf;
pub mod pick;
pub mod recording;
pub mod resize;
pub mod screenshot;
pub mod sessions;
pub mod snapshot;
pub mod styles;
pub mod upload;
pub mod zoom;

use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chromiumoxide::cdp::browser_protocol::accessibility as cdp_ax;
use chromiumoxide::cdp::browser_protocol::css as cdp_css;
use chromiumoxide::cdp::browser_protocol::dom as cdp_dom;
use chromiumoxide::cdp::browser_protocol::emulation as cdp_emulation;
use chromiumoxide::cdp::browser_protocol::input;
use chromiumoxide::cdp::browser_protocol::network as cdp_network;
use chromiumoxide::cdp::browser_protocol::overlay;
use chromiumoxide::cdp::browser_protocol::page as cdp_page;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::cdp::browser_protocol::storage as cdp_storage;
use chromiumoxide::cdp::js_protocol::runtime as cdp_rt;
use chromiumoxide::page::ScreenshotParams;
use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::json;
use tokio::time::timeout;

use crate::config::{origin_label, origin_policy_config_key_for, origin_policy_for, WorkerConfig};
use crate::events::{EventKind, HandoffRequestedEvent, HandoffResolvedEvent, SessionStartedEvent};
use crate::session::{now_ms, OpenRequest, Session, Sessions, Tab};

pub const SESSIONS_START_ID: &str = "browser::sessions::start";
pub const SESSIONS_START_DESC: &str =
    "Open a browser tab (a session) and return its session_id. Tabs share one browser \
     profile (cookies, logins) and stay open until stopped or until an optional ttl_ms; an \
     unused tab sleeps and wakes on the next call. incognito=true opens a PRIVATE tab: nothing \
     it does is saved, and inactivity closes it for good.";
pub const SESSIONS_LIST_ID: &str = "browser::sessions::list";
pub const SESSIONS_LIST_DESC: &str =
    "List every browser tab, live or asleep, with its current URL, title, and activity.";
pub const SESSIONS_STOP_ID: &str = "browser::sessions::stop";
pub const SESSIONS_STOP_DESC: &str =
    "Close a browser tab for good. Idempotent: closing an unknown or already-closed tab \
     succeeds with was_running=false.";
pub const SESSIONS_ATTACH_ID: &str = "browser::sessions::attach";
pub const SESSIONS_ATTACH_DESC: &str =
    "Attach a session to an already-running browser over CDP (Chrome started with \
     --remote-debugging-port): opens a fresh tab the session owns, or adopts a user tab by URL \
     substring and releases it untouched on stop. Reaches the real profile and its logins; \
     requires allow_attach in config.";
pub const TABS_LIST_ID: &str = "browser::tabs::list";
pub const TABS_LIST_DESC: &str =
    "List the open tabs of a running browser reachable at a CDP endpoint (url, title, and \
     whether a session already adopted each). Read-only; adopt one with \
     browser::sessions::attach.";
pub const NAVIGATE_ID: &str = "browser::navigate";
pub const NAVIGATE_DESC: &str =
    "Navigate a tab to a URL and wait for the page to load. Like a browser, a network failure \
     or an empty HTTP error response leaves Chromium's error page in the tab and is reported \
     in `error` rather than failing the call. Element refs from earlier snapshots are \
     invalidated by navigation.";
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
     scroll, or drag (press at the start point, glide to x2/y2, release). Address elements with \
     a [ref=eN] handle from browser::snapshot (or a pick), or raw viewport coordinates.";
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
    "Go back, go forward, or reload the tab's page. History survives the tab sleeping and the \
     worker restarting. Back/forward at the history edge is a no-op with moved=false.";
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
    "Internal: start the live viewport feed — frames arrive on the browser::frame-event \
     trigger and browser::frame reads the newest. Console-UI plumbing; agents use \
     browser::screenshot. Not an agent function.";
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
pub const FIND_IN_PAGE_ID: &str = "browser::find-in-page";
pub const FIND_IN_PAGE_DESC: &str =
    "Find text in the page like the browser's find bar: highlights every match in the live \
     document, scrolls the current one into view, and steps with next / previous. close \
     clears the highlights. Returns count and the 1-based index.";
pub const ZOOM_ID: &str = "browser::zoom";
pub const ZOOM_DESC: &str =
    "Zoom the page in, out, to a level (50-200 %) or back to 100 %, the way the browser's \
     zoom menu does. The viewport keeps its size; the page scales inside it. The level \
     belongs to the loaded document and resets on navigation.";
pub const PDF_ID: &str = "browser::pdf";
pub const PDF_DESC: &str =
    "Print the page to a PDF (the browser's Print -> Save as PDF) and return it base64 with \
     a file name from the title.";
pub const HISTORY_LIST_ID: &str = "browser::history::list";
pub const HISTORY_LIST_DESC: &str =
    "The session's visited pages, newest first, for a history panel or address-bar \
     suggestions. Filter with query. Distinct from browser::history, which moves back / \
     forward / reloads.";
pub const CLEAR_DATA_ID: &str = "browser::clear-data";
pub const CLEAR_DATA_DESC: &str =
    "Clear the browsing data of the site the tab is on: its cookies, its storage, and the \
     shared cache — like a browser's per-site 'Clear cookies and site data'. Other sites keep \
     their logins; browser::clear-browser-data wipes everything.";
pub const CLEAR_BROWSER_DATA_ID: &str = "browser::clear-browser-data";
pub const CLEAR_BROWSER_DATA_DESC: &str =
    "Clear ALL browser data: closes every tab's page, quits Chromium, and deletes the profile \
     (every site's cookies, logins, storage, cache) and the downloads on disk. Tabs stay and \
     reopen signed out. Incognito tabs are closed.";
pub const DOWNLOADS_LIST_ID: &str = "browser::downloads::list";
pub const DOWNLOADS_LIST_DESC: &str =
    "The files this session downloaded (name, url, size, state), newest first. Downloads are \
     allowed and named per session; read one with browser::download.";
pub const DOWNLOAD_ID: &str = "browser::download";
pub const DOWNLOAD_DESC: &str =
    "Read one downloaded file's bytes by guid (from browser::downloads::list), base64, for \
     saving or attaching to the chat.";
pub const DOWNLOAD_REMOVE_ID: &str = "browser::download::remove";
pub const DOWNLOAD_REMOVE_DESC: &str =
    "Forget a download and delete its file from the session's download dir.";
pub const UPLOAD_ID: &str = "browser::upload";
pub const UPLOAD_DESC: &str =
    "Attach up to eight base64 files to exactly one input[type=file] selected by CSS. Files are \
     staged privately for the session and removed when it stops.";
pub const RESIZE_ID: &str = "browser::resize";
pub const RESIZE_DESC: &str =
    "Set the session's live viewport size (CSS pixels). The console calls this as its browser \
     pane resizes so the streamed frame fills the pane with no letterboxing and clicks map \
     1:1; the device toolbar calls it with a preset. Clamped 200..4000.";
pub const COOKIES_LIST_ID: &str = "browser::cookies::list";
pub const COOKIES_LIST_DESC: &str =
    "The cookies visible to the session's current page (name, value, domain, path, flags).";
pub const COOKIES_SET_ID: &str = "browser::cookies::set";
pub const COOKIES_SET_DESC: &str =
    "Set cookies on the session, like importing a cookie file. A cookie without a domain is \
     scoped to the current page's URL. same_site is Strict, Lax, or None.";
pub const COOKIES_CLEAR_ID: &str = "browser::cookies::clear";
pub const COOKIES_CLEAR_DESC: &str =
    "Delete the cookies the tab's current page can see (its site's cookies). Other sites keep \
     theirs; browser::clear-browser-data removes everything.";

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
        spec::<find_in_page::FindInput, find_in_page::FindOutput>(
            FIND_IN_PAGE_ID,
            FIND_IN_PAGE_DESC,
        ),
        spec::<zoom::ZoomInput, zoom::ZoomOutput>(ZOOM_ID, ZOOM_DESC),
        spec::<pdf::PdfInput, pdf::PdfOutput>(PDF_ID, PDF_DESC),
        spec::<history_list::HistoryListInput, history_list::HistoryListOutput>(
            HISTORY_LIST_ID,
            HISTORY_LIST_DESC,
        ),
        spec::<clear_data::ClearDataInput, clear_data::ClearDataOutput>(
            CLEAR_DATA_ID,
            CLEAR_DATA_DESC,
        ),
        spec::<clear_data::ClearBrowserDataInput, clear_data::ClearBrowserDataOutput>(
            CLEAR_BROWSER_DATA_ID,
            CLEAR_BROWSER_DATA_DESC,
        ),
        spec::<downloads::DownloadsListInput, downloads::DownloadsListOutput>(
            DOWNLOADS_LIST_ID,
            DOWNLOADS_LIST_DESC,
        ),
        spec::<downloads::DownloadInput, downloads::DownloadOutput>(DOWNLOAD_ID, DOWNLOAD_DESC),
        spec::<downloads::DownloadRemoveInput, downloads::DownloadRemoveOutput>(
            DOWNLOAD_REMOVE_ID,
            DOWNLOAD_REMOVE_DESC,
        ),
        spec::<upload::UploadInput, upload::UploadOutput>(UPLOAD_ID, UPLOAD_DESC),
        spec::<resize::ResizeInput, resize::ResizeOutput>(RESIZE_ID, RESIZE_DESC),
        spec::<cookies::CookiesListInput, cookies::CookiesListOutput>(
            COOKIES_LIST_ID,
            COOKIES_LIST_DESC,
        ),
        spec::<cookies::CookiesSetInput, cookies::CookiesSetOutput>(
            COOKIES_SET_ID,
            COOKIES_SET_DESC,
        ),
        spec::<cookies::CookiesClearInput, cookies::CookiesClearOutput>(
            COOKIES_CLEAR_ID,
            COOKIES_CLEAR_DESC,
        ),
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
    register_find_in_page(iii, sessions);
    register_zoom(iii, sessions);
    register_pdf(iii, sessions);
    register_history_list(iii, sessions);
    register_clear_data(iii, sessions);
    register_clear_browser_data(iii, sessions);
    register_downloads_list(iii, sessions);
    register_download(iii, sessions);
    register_download_remove(iii, sessions);
    register_upload(iii, sessions);
    register_resize(iii, sessions);
    register_cookies_list(iii, sessions);
    register_cookies_set(iii, sessions);
    register_cookies_clear(iii, sessions);
    tracing::info!("all functions registered");
}

fn handler_err(msg: impl Into<String>) -> Error {
    Error::Handler(msg.into())
}

/// Download guids are used as file names under the session's download dir;
/// reject anything that could escape it before it reaches the filesystem.
fn ensure_safe_guid(guid: &str) -> Result<(), Error> {
    if guid.contains(['/', '\\']) || guid.contains("..") {
        return Err(handler_err("download guids never contain path parts"));
    }
    Ok(())
}

/// `scheme://host[:port]` of a url, for the storage origin. None for
/// about:blank and other origin-less pages.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(format!("{scheme}://{host}"))
    }
}

/// The tab's live session, waking it if it sleeps: any call is a selection.
async fn get_session(sessions: &Arc<Sessions>, id: &str) -> Result<Arc<Session>, Error> {
    sessions.activate(id).await.map_err(handler_err)
}

/// The tab record, live or asleep.
fn get_tab(sessions: &Sessions, id: &str) -> Result<Arc<Tab>, Error> {
    sessions.tab(id).ok_or_else(|| {
        handler_err(format!(
            "unknown session '{id}'; list tabs with browser::sessions::list"
        ))
    })
}

/// The live session for a read that must not wake a sleeping tab: `None`
/// while the tab sleeps, an error for an unknown id.
fn live_session(sessions: &Sessions, id: &str) -> Result<Option<Arc<Session>>, Error> {
    get_tab(sessions, id)?;
    Ok(sessions.get(id))
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

#[derive(Clone, Copy)]
enum OriginPermission {
    Access,
    Downloads,
    Uploads,
    Scripting,
}

impl OriginPermission {
    fn key(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Downloads => "downloads",
            Self::Uploads => "uploads",
            Self::Scripting => "scripting",
        }
    }

    fn allowed(self, config: &WorkerConfig, url: &str) -> bool {
        let policy = origin_policy_for(config, url);
        match self {
            Self::Access => policy.access,
            Self::Downloads => policy.downloads,
            Self::Uploads => policy.uploads,
            Self::Scripting => policy.scripting,
        }
    }
}

fn check_origin_permission(
    config: &WorkerConfig,
    url: &str,
    permission: OriginPermission,
) -> Result<(), String> {
    if permission.allowed(config, url) {
        return Ok(());
    }
    Err(format!(
        "origin '{}' is denied by {} ({})",
        origin_label(url),
        origin_policy_config_key_for(config, url),
        permission.key()
    ))
}

fn ensure_origin_permission(
    config: &WorkerConfig,
    url: &str,
    permission: OriginPermission,
) -> Result<(), Error> {
    check_origin_permission(config, url, permission).map_err(handler_err)
}

fn register_sessions_start(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        SESSIONS_START_ID,
        RegisterFunction::new_async(move |req: sessions::StartInput| {
            let sx = sx.clone();
            async move {
                if let Some(url) = &req.url {
                    let config = sx.config.load();
                    sessions::check_scheme(&config, url).map_err(handler_err)?;
                    ensure_origin_permission(&config, url, OriginPermission::Access)?;
                }
                let (session, error) = sx
                    .open(OpenRequest {
                        url: req.url,
                        headful: req.headful,
                        read_only: req.read_only.unwrap_or(false),
                        incognito: req.incognito.unwrap_or(false),
                        ttl_ms: req.ttl_ms,
                    })
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
                    incognito: session.incognito,
                    error,
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
                // A live tab's url + title are two independent CDP
                // round-trips, and tabs are independent of each other: join
                // both axes so the whole list costs one round-trip time
                // instead of 2N. A sleeping tab answers from its record. The
                // fresh title is written back so the record (and the tab
                // strip after the tab sleeps) keeps what the page last said.
                let headless_default = sx.config.load().headless;
                let out = futures::future::join_all(sx.list_tabs().into_iter().map(|tab| {
                    let live = sx.get(&tab.id);
                    async move {
                        let (headless, console_entries) = match &live {
                            Some(session) => {
                                let (url, title) =
                                    futures::join!(session.page.url(), session.page.get_title());
                                let url = url.ok().flatten().unwrap_or_default();
                                let title = title.ok().flatten();
                                if !tab.attached || !url.is_empty() {
                                    tab.set_location(&url, title.as_deref());
                                }
                                let entries = session
                                    .console
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner())
                                    .len() as u64;
                                (session.headless, entries)
                            }
                            None => (headless_default, 0),
                        };
                        let title = tab.title();
                        sessions::SessionInfo {
                            session_id: tab.id.clone(),
                            url: tab.url(),
                            title: (!title.is_empty()).then_some(title),
                            headless,
                            read_only: tab.read_only,
                            incognito: tab.incognito,
                            active: live.is_some(),
                            ttl_ms: tab.ttl_ms,
                            created_ms: tab.created_ms,
                            last_used_ms: tab.last_used_ms(),
                            console_entries,
                        }
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
                let tabs = sx.remote_tabs(&req.cdp_url).await.map_err(handler_err)?;
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
                ensure_origin_permission(&cfg, &req.url, OriginPermission::Access)?;
                let session = get_session(&sx, &req.session_id).await?;
                session.touch();
                let wait = Duration::from_millis(cfg.clamp_timeout(req.timeout_ms));
                let _navigation = session.navigation_lock.lock().await;
                session.clear_navigation_error();

                let allow_http = cfg.allowed_schemes.iter().any(|s| s == "http");
                let navigation = session
                    .navigate_like_a_browser(&req.url, allow_http)
                    .await;
                if let Some(policy_error) = session.take_navigation_error() {
                    return Err(handler_err(policy_error));
                }
                let (_, error) = navigation.map_err(handler_err)?;
                // An error page is committed already; only a real load waits.
                let timed_out = error.is_none()
                    && timeout(wait, session.page.wait_for_navigation())
                        .await
                        .is_err();
                if let Some(policy_error) = session.take_navigation_error() {
                    return Err(handler_err(policy_error));
                }

                let url = session.page.url().await.ok().flatten().unwrap_or(req.url);
                let title = session.page.get_title().await.ok().flatten();
                session.tab.commit_location(&url, title.as_deref());
                if session.tab.persists() {
                    sx.persist();
                }
                session.touch();
                Ok::<_, Error>(navigate::NavigateOutput {
                    ok: error.is_none(),
                    url,
                    title,
                    timed_out,
                    error,
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
                let session = get_session(&sx, &req.session_id).await?;
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
                let session = get_session(&sx, &req.session_id).await?;
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
                        width: session.viewport().0,
                        height: session.viewport().1,
                    },
                })
            }
        })
        .description(SCREENSHOT_DESC)
        .metadata(json!({ "display": true })),
    );
}

/// Best-effort: draw the ghost cursor at the acted point so a human watching
/// the streamed viewport sees where the agent acted. Only injected while
/// screencast is active (someone is watching); a failed injection is ignored.
async fn move_ghost_cursor(session: &Session, x: f64, y: f64, click: bool) {
    if session.screencast_on() {
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

/// Press at (x1, y1), glide to (x2, y2) over a few steps, release. The
/// interpolated moves make the drag read as a real gesture so listeners that
/// track pointer movement (drawing surfaces, sliders) follow it.
async fn dispatch_drag(session: &Session, x1: f64, y1: f64, x2: f64, y2: f64) -> Result<(), Error> {
    use input::{DispatchMouseEventParams, DispatchMouseEventType, MouseButton};
    let send = |params| async {
        session
            .page
            .execute(params)
            .await
            .map_err(|e| handler_err(format!("mouse event failed: {e}")))?;
        Ok::<(), Error>(())
    };
    let moved = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MouseMoved)
        .x(x1)
        .y(y1)
        .build()
        .map_err(handler_err)?;
    send(moved).await?;
    let pressed = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MousePressed)
        .x(x1)
        .y(y1)
        .button(MouseButton::Left)
        .click_count(1)
        .build()
        .map_err(handler_err)?;
    send(pressed).await?;
    move_ghost_cursor(session, x1, y1, true).await;
    const STEPS: i64 = 8;
    for step in 1..=STEPS {
        let t = step as f64 / STEPS as f64;
        let (xi, yi) = (x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
        let step_move = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseMoved)
            .x(xi)
            .y(yi)
            .button(MouseButton::Left)
            .build()
            .map_err(handler_err)?;
        send(step_move).await?;
        // Glide the ghost cursor with the drag so a viewer watching the
        // stream sees the gesture, not a jump to the end.
        move_ghost_cursor(session, xi, yi, false).await;
    }
    let released = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MouseReleased)
        .x(x2)
        .y(y2)
        .button(MouseButton::Left)
        .click_count(1)
        .build()
        .map_err(handler_err)?;
    send(released).await
}

fn register_act(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        ACT_ID,
        RegisterFunction::new_async(move |req: act::ActInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
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
                                session.viewport().0 as f64 / 2.0,
                                session.viewport().1 as f64 / 2.0,
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
                    "drag" => {
                        let (x1, y1) = action_point(&session, &req).await?;
                        let (x2, y2) = match (req.x2, req.y2) {
                            (Some(x), Some(y)) => (x, y),
                            _ => return Err(handler_err("drag needs x2 and y2")),
                        };
                        dispatch_drag(&session, x1, y1, x2, y2).await?;
                        format!("dragged ({x1:.0}, {y1:.0}) to ({x2:.0}, {y2:.0})")
                    }
                    other => {
                        return Err(handler_err(format!(
                            "unknown action '{other}' (click, hover, type, press, scroll, drag)"
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
                let session = get_session(&sx, &req.session_id).await?;
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
                let session = get_session(&sx, &req.session_id).await?;
                ensure_writable(&session, "browser::execute")?;
                session.touch();
                let cfg = sx.config.load_full();
                let wait = Duration::from_millis(cfg.clamp_timeout(req.timeout_ms));
                let _navigation = session.navigation_lock.lock().await;
                let page_url = session.page.url().await.ok().flatten().unwrap_or_default();
                ensure_origin_permission(&cfg, &page_url, OriginPermission::Scripting)?;
                session.clear_navigation_error();

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
                if let Some(error) = session.take_navigation_error() {
                    return Err(handler_err(error));
                }
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
                let session = get_session(&sx, &req.session_id).await?;
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
                sx.emitter
                    .emit(
                        EventKind::HandoffResolved,
                        &req.session_id,
                        &HandoffResolvedEvent {
                            session_id: req.session_id.clone(),
                            handoff_id: handoff_id.clone(),
                            via: via.to_string(),
                            timestamp: now_ms(),
                        },
                    )
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

                let active_sessions = sx.live_count() as u64;
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
                    open_tabs: sx.tab_count() as u64,
                    browser_running: sx.browser_running().await,
                    data_dir: sx.data_dir.display().to_string(),
                    allowed_schemes: cfg.allowed_schemes.clone(),
                    configured_origin_policies: cfg
                        .origin_policies
                        .as_ref()
                        .map(|policies| policies.len() as u64)
                        .unwrap_or(0),
                    default_origin_policy_set: cfg.default_origin_policy.is_some(),
                    allow_history_access: cfg.allow_history_access,
                    allow_cookie_import: cfg.allow_cookie_import,
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
                // Reading logs never wakes a sleeping tab: its buffers went
                // with the page, so the answer is simply empty.
                let Some(session) = live_session(&sx, &req.session_id)? else {
                    return Ok::<_, Error>(console::ConsoleReadOutput {
                        entries: Vec::new(),
                        last_seq: req.since_seq.unwrap_or(0),
                        dropped: 0,
                    });
                };
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
                let Some(session) = live_session(&sx, &req.session_id)? else {
                    return Ok::<_, Error>(network::NetworkReadOutput {
                        entries: Vec::new(),
                        last_seq: req.since_seq.unwrap_or(0),
                        dropped: 0,
                    });
                };
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
                let session = get_session(&sx, &req.session_id).await?;
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
                let session = get_session(&sx, &req.session_id).await?;
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

/// Runs an injected script and reads one JSON field from its completion
/// value, for the find / zoom helpers that keep their state in the page.
async fn evaluate_json(
    session: &Session,
    script: String,
    what: &str,
) -> Result<serde_json::Value, Error> {
    let evaluated = session
        .page
        .evaluate(script)
        .await
        .map_err(|e| handler_err(format!("{what} failed: {e}")))?;
    evaluated
        .value()
        .cloned()
        .ok_or_else(|| handler_err(format!("{what} returned nothing")))
}

fn register_find_in_page(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        FIND_IN_PAGE_ID,
        RegisterFunction::new_async(move |req: find_in_page::FindInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                ensure_writable(&session, "browser::find-in-page")?;
                session.touch();
                let action = req.action.as_deref().unwrap_or("search");
                if !matches!(action, "search" | "next" | "previous" | "close") {
                    return Err(handler_err(format!(
                        "unknown action '{action}' (search, next, previous, close)"
                    )));
                }
                let script = find_in_page::find_script(
                    &req.query,
                    action,
                    req.case_sensitive.unwrap_or(false),
                );
                let value = evaluate_json(&session, script, "find").await?;
                let count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                let index = value.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let query = value
                    .get("query")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or(req.query);
                Ok::<_, Error>(find_in_page::FindOutput {
                    ok: true,
                    count,
                    index,
                    query: if action == "close" {
                        String::new()
                    } else {
                        query
                    },
                })
            }
        })
        .description(FIND_IN_PAGE_DESC),
    );
}

fn register_zoom(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        ZOOM_ID,
        RegisterFunction::new_async(move |req: zoom::ZoomInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                let action = req.action.as_deref().unwrap_or(if req.level.is_some() {
                    "set"
                } else {
                    "reset"
                });
                if action != "read" {
                    ensure_writable(&session, "browser::zoom")?;
                }
                session.touch();
                let current = evaluate_json(&session, zoom::read_script().to_string(), "zoom")
                    .await?
                    .as_u64()
                    .map(|n| n as u32)
                    .unwrap_or(100);
                if action == "read" {
                    return Ok::<_, Error>(zoom::ZoomOutput {
                        ok: true,
                        level: zoom::snap(current),
                    });
                }
                let level = match action {
                    "in" => zoom::step(current, true),
                    "out" => zoom::step(current, false),
                    "reset" => 100,
                    "set" => match req.level {
                        Some(level) => zoom::snap(level.clamp(50, 200)),
                        None => return Err(handler_err("set needs a level (50-200)")),
                    },
                    other => {
                        return Err(handler_err(format!(
                            "unknown action '{other}' (in, out, reset, set, read)"
                        )))
                    }
                };
                evaluate_json(&session, zoom::apply_script(level), "zoom").await?;
                Ok::<_, Error>(zoom::ZoomOutput { ok: true, level })
            }
        })
        .description(ZOOM_DESC),
    );
}

fn register_pdf(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        PDF_ID,
        RegisterFunction::new_async(move |req: pdf::PdfInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                session.touch();
                let mut params = cdp_page::PrintToPdfParams::builder()
                    .landscape(req.landscape.unwrap_or(false))
                    .print_background(req.print_background.unwrap_or(true));
                if let Some(scale) = req.scale {
                    params = params.scale(scale.clamp(0.1, 2.0));
                }
                let bytes = session
                    .page
                    .pdf(params.build())
                    .await
                    .map_err(|e| handler_err(format!("print to pdf failed: {e}")))?;
                if bytes.len() > pdf::MAX_PDF_BYTES {
                    return Err(handler_err(format!(
                        "pdf is {} bytes, over the {} byte cap; print a narrower page",
                        bytes.len(),
                        pdf::MAX_PDF_BYTES
                    )));
                }
                let url = session.page.url().await.ok().flatten().unwrap_or_default();
                let title = session
                    .page
                    .get_title()
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                Ok::<_, Error>(pdf::PdfOutput {
                    ok: true,
                    size_bytes: bytes.len() as u64,
                    data: STANDARD.encode(&bytes),
                    file_name: pdf::file_name(&title, &url),
                    url,
                })
            }
        })
        .description(PDF_DESC),
    );
}

fn same_site_from(value: &str) -> Option<cdp_network::CookieSameSite> {
    match value.to_ascii_lowercase().as_str() {
        "strict" => Some(cdp_network::CookieSameSite::Strict),
        "lax" => Some(cdp_network::CookieSameSite::Lax),
        "none" => Some(cdp_network::CookieSameSite::None),
        _ => None,
    }
}

fn same_site_str(value: &cdp_network::CookieSameSite) -> String {
    match value {
        cdp_network::CookieSameSite::Strict => "Strict",
        cdp_network::CookieSameSite::Lax => "Lax",
        cdp_network::CookieSameSite::None => "None",
    }
    .to_string()
}

fn register_cookies_list(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        COOKIES_LIST_ID,
        RegisterFunction::new_async(move |req: cookies::CookiesListInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                let result = session
                    .page
                    .execute(cdp_network::GetCookiesParams::default())
                    .await
                    .map_err(|e| handler_err(format!("get cookies failed: {e}")))?;
                let cookies = result
                    .result
                    .cookies
                    .iter()
                    .map(|c| cookies::CookieSpec {
                        name: c.name.clone(),
                        value: c.value.clone(),
                        domain: Some(c.domain.clone()),
                        path: Some(c.path.clone()),
                        expires: if c.expires < 0.0 {
                            None
                        } else {
                            Some(c.expires)
                        },
                        secure: Some(c.secure),
                        http_only: Some(c.http_only),
                        same_site: c.same_site.as_ref().map(same_site_str),
                    })
                    .collect();
                Ok::<_, Error>(cookies::CookiesListOutput { cookies })
            }
        })
        .description(COOKIES_LIST_DESC),
    );
}

fn register_cookies_set(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        COOKIES_SET_ID,
        RegisterFunction::new_async(move |req: cookies::CookiesSetInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                ensure_writable(&session, "browser::cookies::set")?;
                if !sx.config.load().allow_cookie_import {
                    return Err(handler_err(
                        "browser::cookies::set is denied by allow_cookie_import=false",
                    ));
                }
                session.touch();
                let page_url = session.page.url().await.ok().flatten().unwrap_or_default();
                let count = req.cookies.len();
                let params: Vec<cdp_network::CookieParam> = req
                    .cookies
                    .into_iter()
                    .map(|c| {
                        let mut param = cdp_network::CookieParam::new(c.name, c.value);
                        if c.domain.is_some() {
                            param.domain = c.domain;
                        } else if !page_url.is_empty() {
                            param.url = Some(page_url.clone());
                        }
                        param.path = c.path;
                        param.secure = c.secure;
                        param.http_only = c.http_only;
                        param.expires = c.expires.map(cdp_network::TimeSinceEpoch::new);
                        param.same_site = c.same_site.as_deref().and_then(same_site_from);
                        param
                    })
                    .collect();
                session
                    .page
                    .execute(cdp_network::SetCookiesParams::new(params))
                    .await
                    .map_err(|e| handler_err(format!("set cookies failed: {e}")))?;
                Ok::<_, Error>(cookies::CookiesSetOutput { ok: true, count })
            }
        })
        .description(COOKIES_SET_DESC),
    );
}

/// Delete the cookies the tab's current page can see, one by one: the
/// profile is shared by every regular tab, so the browser-wide
/// `Network.clearBrowserCookies` would sign every other site out too.
async fn delete_site_cookies(session: &Session) -> Result<usize, Error> {
    let result = session
        .page
        .execute(cdp_network::GetCookiesParams::default())
        .await
        .map_err(|e| handler_err(format!("get cookies failed: {e}")))?;
    let cookies = &result.result.cookies;
    for cookie in cookies {
        let mut params = cdp_network::DeleteCookiesParams::new(cookie.name.clone());
        params.domain = Some(cookie.domain.clone());
        params.path = Some(cookie.path.clone());
        session
            .page
            .execute(params)
            .await
            .map_err(|e| handler_err(format!("delete cookie failed: {e}")))?;
    }
    Ok(cookies.len())
}

fn register_cookies_clear(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        COOKIES_CLEAR_ID,
        RegisterFunction::new_async(move |req: cookies::CookiesClearInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                ensure_writable(&session, "browser::cookies::clear")?;
                session.touch();
                delete_site_cookies(&session).await?;
                Ok::<_, Error>(cookies::CookiesClearOutput { ok: true })
            }
        })
        .description(COOKIES_CLEAR_DESC),
    );
}

fn register_resize(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        RESIZE_ID,
        RegisterFunction::new_async(move |req: resize::ResizeInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                ensure_writable(&session, "browser::resize")?;
                session.touch();
                let mut width = resize::clamp(req.width);
                let mut height = resize::clamp(req.height);
                // A pane auto-fit with several viewers grows the shared
                // viewport but never shrinks it: the largest pane wins and the
                // smaller ones letterbox-scale, so one small viewer can't
                // shrink a session another viewer needs bigger. A lone viewer
                // (or an explicit device resize) sizes it freely.
                if req.fit.unwrap_or(false)
                    && session
                        .screencast_consumers
                        .load(std::sync::atomic::Ordering::Relaxed)
                        > 1
                {
                    let (cur_w, cur_h) = session.viewport();
                    width = width.max(cur_w);
                    height = height.max(cur_h);
                    if width == cur_w && height == cur_h {
                        return Ok::<_, Error>(resize::ResizeOutput {
                            ok: true,
                            width,
                            height,
                        });
                    }
                }
                let dpr = req.device_scale_factor.unwrap_or(1.0).clamp(0.5, 3.0);
                let params = cdp_emulation::SetDeviceMetricsOverrideParams::builder()
                    .width(i64::from(width))
                    .height(i64::from(height))
                    .device_scale_factor(dpr)
                    .mobile(req.mobile.unwrap_or(false))
                    .build()
                    .map_err(handler_err)?;
                session
                    .page
                    .execute(params)
                    .await
                    .map_err(|e| handler_err(format!("resize failed: {e}")))?;
                session.set_viewport(width, height);
                // The page content did not change, so the compositor may not
                // push a screencast frame at the new size on its own; nudge
                // one out through the counted screencast paths.
                sx.nudge_screencast(&session).await;
                Ok::<_, Error>(resize::ResizeOutput {
                    ok: true,
                    width,
                    height,
                })
            }
        })
        .description(RESIZE_DESC),
    );
}

fn register_history_list(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        HISTORY_LIST_ID,
        RegisterFunction::new_async(move |req: history_list::HistoryListInput| {
            let sx = sx.clone();
            async move {
                if !sx.config.load().allow_history_access {
                    return Err(handler_err(
                        "browser::history::list is denied by allow_history_access=false",
                    ));
                }
                let tab = get_tab(&sx, &req.session_id)?;
                let visits = tab.history.lock().unwrap_or_else(|p| p.into_inner());
                let out =
                    history_list::select(&visits, req.query.as_deref(), req.limit.unwrap_or(100));
                Ok::<_, Error>(history_list::HistoryListOutput { visits: out })
            }
        })
        .description(HISTORY_LIST_DESC),
    );
}

fn register_clear_data(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        CLEAR_DATA_ID,
        RegisterFunction::new_async(move |req: clear_data::ClearDataInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                ensure_writable(&session, "browser::clear-data")?;
                session.touch();
                let mut cleared = Vec::new();
                if req.cookies.unwrap_or(true) {
                    delete_site_cookies(&session).await?;
                    cleared.push("cookies".to_string());
                }
                if req.cache.unwrap_or(true) {
                    session
                        .page
                        .execute(cdp_network::ClearBrowserCacheParams::default())
                        .await
                        .map_err(|e| handler_err(format!("clear cache failed: {e}")))?;
                    cleared.push("cache".to_string());
                }
                if req.storage.unwrap_or(true) {
                    let url = session.page.url().await.ok().flatten().unwrap_or_default();
                    if let Some(origin) = origin_of(&url) {
                        if let Ok(params) = cdp_storage::ClearDataForOriginParams::builder()
                            .origin(origin)
                            .storage_types("all".to_string())
                            .build()
                        {
                            if session.page.execute(params).await.is_ok() {
                                cleared.push("storage".to_string());
                            }
                        }
                    }
                }
                Ok::<_, Error>(clear_data::ClearDataOutput { ok: true, cleared })
            }
        })
        .description(CLEAR_DATA_DESC),
    );
}

fn register_clear_browser_data(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        CLEAR_BROWSER_DATA_ID,
        RegisterFunction::new_async(move |_req: clear_data::ClearBrowserDataInput| {
            let sx = sx.clone();
            async move {
                let closed_pages = sx.clear_browser_data().await.map_err(handler_err)?;
                Ok::<_, Error>(clear_data::ClearBrowserDataOutput {
                    ok: true,
                    closed_pages: closed_pages as u64,
                    profile_dir: sx.profile_dir().display().to_string(),
                })
            }
        })
        .description(CLEAR_BROWSER_DATA_DESC),
    );
}

fn register_downloads_list(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        DOWNLOADS_LIST_ID,
        RegisterFunction::new_async(move |req: downloads::DownloadsListInput| {
            let sx = sx.clone();
            async move {
                let tab = get_tab(&sx, &req.session_id)?;
                let mut list = tab
                    .downloads
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                list.reverse();
                Ok::<_, Error>(downloads::DownloadsListOutput { downloads: list })
            }
        })
        .description(DOWNLOADS_LIST_DESC),
    );
}

fn register_download(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        DOWNLOAD_ID,
        RegisterFunction::new_async(move |req: downloads::DownloadInput| {
            let sx = sx.clone();
            async move {
                let tab = get_tab(&sx, &req.session_id)?;
                let (file_name, url, dir) = {
                    let downloads = tab.downloads.lock().unwrap_or_else(|p| p.into_inner());
                    let record = downloads
                        .iter()
                        .find(|d| d.guid == req.guid)
                        .ok_or_else(|| handler_err(format!("no download '{}'", req.guid)))?;
                    (
                        record.file_name.clone(),
                        record.url.clone(),
                        tab.downloads_dir.clone(),
                    )
                };
                ensure_origin_permission(&sx.config.load(), &url, OriginPermission::Downloads)?;
                let dir = dir.ok_or_else(|| {
                    handler_err("this session does not own downloads (attached session)")
                })?;
                ensure_safe_guid(&req.guid)?;
                let path = dir.join(&req.guid);
                let size = std::fs::metadata(&path)
                    .map_err(|e| handler_err(format!("read download failed: {e}")))?
                    .len();
                if size > downloads::MAX_DOWNLOAD_BYTES {
                    return Err(handler_err(format!(
                        "download is {size} bytes, over the {} byte cap; it stays on disk at {}",
                        downloads::MAX_DOWNLOAD_BYTES,
                        path.display()
                    )));
                }
                let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
                    .await
                    .map_err(|e| handler_err(format!("read download failed: {e}")))?
                    .map_err(|e| handler_err(format!("read download failed: {e}")))?;
                Ok::<_, Error>(downloads::DownloadOutput {
                    ok: true,
                    size_bytes: bytes.len() as u64,
                    data: STANDARD.encode(&bytes),
                    file_name,
                })
            }
        })
        .description(DOWNLOAD_DESC),
    );
}

fn register_download_remove(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        DOWNLOAD_REMOVE_ID,
        RegisterFunction::new_async(move |req: downloads::DownloadRemoveInput| {
            let sx = sx.clone();
            async move {
                let tab = get_tab(&sx, &req.session_id)?;
                // Only a guid the tab actually recorded names a file the
                // worker wrote; anything else must not reach the filesystem.
                let known = {
                    let downloads = tab.downloads.lock().unwrap_or_else(|p| p.into_inner());
                    downloads.iter().any(|d| d.guid == req.guid)
                };
                if !known {
                    return Err(handler_err(format!("no download '{}'", req.guid)));
                }
                ensure_safe_guid(&req.guid)?;
                if let Some(dir) = &tab.downloads_dir {
                    let _ = std::fs::remove_file(dir.join(&req.guid));
                }
                tab.downloads
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .retain(|d| d.guid != req.guid);
                Ok::<_, Error>(downloads::DownloadRemoveOutput { ok: true })
            }
        })
        .description(DOWNLOAD_REMOVE_DESC),
    );
}

fn register_upload(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        UPLOAD_ID,
        RegisterFunction::new_async(move |req: upload::UploadInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
                ensure_writable(&session, "browser::upload")?;
                upload::validate_files(&req.files).map_err(handler_err)?;
                session.touch();
                let _navigation = session.navigation_lock.lock().await;
                let config = sx.config.load_full();
                let url = session.page.url().await.ok().flatten().unwrap_or_default();
                ensure_origin_permission(&config, &url, OriginPermission::Uploads)?;

                session
                    .page
                    .execute(cdp_dom::EnableParams::default())
                    .await
                    .map_err(|error| handler_err(format!("enable DOM failed: {error}")))?;
                let document = session
                    .page
                    .execute(cdp_dom::GetDocumentParams::default())
                    .await
                    .map_err(|error| handler_err(format!("document read failed: {error}")))?;
                let selected = session
                    .page
                    .execute(cdp_dom::QuerySelectorParams::new(
                        document.root.node_id,
                        req.selector.clone(),
                    ))
                    .await
                    .map_err(|error| handler_err(format!("selector failed: {error}")))?;
                let matches = session
                    .page
                    .execute(cdp_dom::QuerySelectorAllParams::new(
                        document.root.node_id,
                        req.selector.clone(),
                    ))
                    .await
                    .map_err(|error| handler_err(format!("selector failed: {error}")))?;
                if matches.node_ids.len() != 1 {
                    return Err(handler_err(format!(
                        "selector '{}' matched {} elements; expected exactly one input[type=file]",
                        req.selector,
                        matches.node_ids.len()
                    )));
                }

                let described = session
                    .page
                    .execute(
                        cdp_dom::DescribeNodeParams::builder()
                            .node_id(selected.node_id)
                            .build(),
                    )
                    .await
                    .map_err(|error| handler_err(format!("describe file input failed: {error}")))?;
                let is_file_input = described.node.node_name.eq_ignore_ascii_case("input")
                    && described
                        .node
                        .attributes
                        .as_deref()
                        .unwrap_or_default()
                        .chunks(2)
                        .any(|pair| {
                            pair.len() == 2
                                && pair[0].eq_ignore_ascii_case("type")
                                && pair[1].eq_ignore_ascii_case("file")
                        });
                if !is_file_input {
                    return Err(handler_err(format!(
                        "selector '{}' must match an input[type=file]",
                        req.selector
                    )));
                }

                let file_names: Vec<String> =
                    req.files.iter().map(|file| file.name.clone()).collect();
                let staging_dir = session.create_upload_dir().map_err(handler_err)?;
                let paths = tokio::task::spawn_blocking(move || {
                    upload::stage_files(&staging_dir, req.files)
                })
                .await
                .map_err(|error| handler_err(format!("stage upload files failed: {error}")))?
                .map_err(handler_err)?;
                let path_strings: Vec<String> = paths
                    .iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect();
                session
                    .page
                    .execute(
                        cdp_dom::SetFileInputFilesParams::builder()
                            .files(path_strings)
                            .node_id(selected.node_id)
                            .build()
                            .map_err(handler_err)?,
                    )
                    .await
                    .map_err(|error| handler_err(format!("attach upload files failed: {error}")))?;
                session.touch();
                Ok::<_, Error>(upload::UploadOutput {
                    ok: true,
                    attached: file_names.len(),
                    file_names,
                })
            }
        })
        .description(UPLOAD_DESC),
    );
}

fn register_history(iii: &Arc<IIIClient>, sessions: &Arc<Sessions>) {
    let sx = sessions.clone();
    iii.register_function(
        HISTORY_ID,
        RegisterFunction::new_async(move |req: history::HistoryInput| {
            let sx = sx.clone();
            async move {
                let session = get_session(&sx, &req.session_id).await?;
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
                        let back = dir == "back";
                        let history = session
                            .page
                            .execute(cdp_page::GetNavigationHistoryParams::default())
                            .await
                            .map_err(|e| handler_err(format!("history read failed: {e}")))?;
                        let target = if back {
                            history.current_index - 1
                        } else {
                            history.current_index + 1
                        };
                        // The tab's own stack moves with Chromium's so the
                        // two stay aligned; it is also what remains after the
                        // page slept or the worker restarted.
                        let remembered = session
                            .tab
                            .nav
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .neighbour(back);
                        match usize::try_from(target)
                            .ok()
                            .and_then(|i| history.entries.get(i))
                        {
                            Some(entry) => {
                                if let Some((index, _)) = remembered {
                                    session
                                        .tab
                                        .nav
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .set_pending(index);
                                }
                                session
                                    .page
                                    .execute(cdp_page::NavigateToHistoryEntryParams::new(entry.id))
                                    .await
                                    .map_err(|e| {
                                        handler_err(format!("history navigation failed: {e}"))
                                    })?;
                                true
                            }
                            None => match remembered {
                                Some((index, url)) => {
                                    session
                                        .tab
                                        .nav
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .set_pending(index);
                                    session.navigate(&url).await.map_err(handler_err)?;
                                    true
                                }
                                None => false,
                            },
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
                if moved {
                    let title = session.page.get_title().await.ok().flatten();
                    session.tab.commit_location(&url, title.as_deref());
                    if session.tab.persists() {
                        sx.persist();
                    }
                }
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
                let session = get_session(&sx, &req.session_id).await?;
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
                let session = get_session(&sx, &req.session_id).await?;
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
                let session = get_session(&sx, &req.session_id).await?;
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
                let miss = hint::PickHintOutput {
                    hit: false,
                    tag: None,
                    id: None,
                    classes: None,
                    bounds: None,
                };
                // Hover sampling runs at cursor cadence; it never wakes a tab.
                let Some(session) = live_session(&sx, &req.session_id)? else {
                    return Ok::<_, Error>(miss);
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
                let session = get_session(&sx, &req.session_id).await?;
                session.touch();
                // Register a console viewer as a screencast consumer; the CDP
                // screencast starts on the first consumer (see
                // Sessions::acquire_screencast, which uses every_nth_frame(1)
                // and caps the push rate by time in the pump).
                sx.acquire_screencast(&session).await.map_err(handler_err)?;
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
                    // Release this console viewer; the CDP screencast stops
                    // only if no other consumer (e.g. a recording) remains.
                    sx.release_screencast(&session).await;
                    // The viewer's overlays go regardless; they belong to the
                    // watching human, not to a recording consumer.
                    let _ = session.page.evaluate(overlays::remove_badge_script()).await;
                    let _ = session
                        .page
                        .evaluate(overlays::remove_ghost_cursor_script())
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
                let session = get_session(&sx, &req.session_id).await?;
                let (format, codec) =
                    recording::resolve_format(req.format.as_deref()).map_err(handler_err)?;
                sx.start_recording(&session, &req.path, codec)
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
                let Some(session) = live_session(&sx, &req.session_id)? else {
                    return Ok::<_, Error>(frame::FrameOutput {
                        frame: None,
                        width: 0,
                        height: 0,
                        frame_seq: 0,
                        timestamp: 0,
                        active: false,
                    });
                };
                let active = session.screencast_on();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{OriginPolicy, OriginPolicyDecision};

    #[test]
    fn origin_denial_names_the_origin_source_and_permission() {
        let config = WorkerConfig {
            origin_policies: Some(std::collections::BTreeMap::from([(
                "https://x.y".to_string(),
                OriginPolicy {
                    access: Some(OriginPolicyDecision::Deny),
                    ..OriginPolicy::default()
                },
            )])),
            ..WorkerConfig::default()
        };

        assert_eq!(
            check_origin_permission(&config, "https://x.y/path", OriginPermission::Access)
                .unwrap_err(),
            "origin 'https://x.y' is denied by origin_policies (access)"
        );
    }
}
