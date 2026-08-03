//! Native host driver: drive the machine this worker runs on directly, with
//! no executor process in between. Screen capture via `xcap`, mouse/keyboard
//! via `enigo`.
//!
//! Multi-monitor and Retina aware. A session captures ONE display, chosen at
//! its first capture: by default the display under the cursor (so it follows
//! where you are working), pinned by id thereafter for coordinate stability.
//! The captured frame is downscaled to `max_dimension` and JPEG-encoded (a full
//! Retina frame is tens of megabytes of PNG, which floods the model context and
//! the frame stream). The model sees the downscaled image and its coordinate
//! space; pointer actions map those coordinates through the display's LOGICAL
//! point size and its global origin, which is exactly what `enigo`'s absolute
//! coordinates use, so clicks land on the right display at the right point
//! regardless of scale factor.
//!
//! Capture and input are synchronous, blocking OS calls, so each runs on a
//! blocking thread. `enigo` state is not `Send`, so an `Enigo` is created, used,
//! and dropped inside each blocking closure. On macOS the worker process needs
//! Screen Recording (capture) and Accessibility (input) permission. Input is
//! preflighted with `AXIsProcessTrusted` so a dropped click fails loudly instead
//! of a false success. Screen Recording is NOT preflighted:
//! `CGPreflightScreenCaptureAccess` reports per-process state that a child of a
//! granted terminal does not inherit, so it false-negatives on a capture that
//! actually works; a missing grant instead surfaces as a wallpaper-only image.

use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use arc_swap::ArcSwapOption;

use async_trait::async_trait;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{resize, FilterType};
use image::DynamicImage;
use tokio::task::spawn_blocking;
use xcap::Monitor;

use super::{DisplayInfo, Driver, Screen, Shot};

/// macOS permission gates. Two things macOS silently degrades instead of
/// erroring, so the worker must check them or it lies to the model:
///
/// - Accessibility: without it, macOS drops synthetic input while `enigo` still
///   reports success, so `act` would claim a click landed when it did not.
///   Checked with `AXIsProcessTrusted`.
/// - Screen Recording: without it, capture returns the desktop wallpaper and
///   menu bar with every window stripped, so a screenshot looks fine but shows
///   nothing. `request_screen_capture` calls the *request* API
///   (`CGRequestScreenCaptureAccess`), which surfaces the macOS prompt rather
///   than the *preflight* API, which shows no dialog and under-reports for a
///   process launched as a child of a granted terminal.
///
/// Windows has no equivalent gates, so both checks are always true there.
#[cfg(target_os = "macos")]
mod perms {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGRequestScreenCaptureAccess() -> u8;
    }
    pub fn accessibility_granted() -> bool {
        unsafe { AXIsProcessTrusted() != 0 }
    }
    pub fn request_screen_capture() -> bool {
        unsafe { CGRequestScreenCaptureAccess() != 0 }
    }
}

#[cfg(not(target_os = "macos"))]
mod perms {
    pub fn accessibility_granted() -> bool {
        true
    }
    pub fn request_screen_capture() -> bool {
        true
    }
}

/// Ask macOS for Screen Recording (surfacing the prompt) and fail loudly if the
/// worker is not authorized, so a native session never hands back a
/// wallpaper-only screenshot with no explanation. Gated by config
/// (`screen_capture_preflight`), because the TCC APIs under-report for a process
/// launched as a child of an already-granted app, and a user whose capture
/// works must be able to skip the check.
pub fn preflight_screen_capture() -> Result<(), String> {
    if perms::request_screen_capture() {
        Ok(())
    } else {
        Err(
            "Screen Recording is not granted to this worker, so a screenshot \
             would capture only the desktop wallpaper (every window stripped). \
             macOS has been asked for it: open System Settings > Privacy & \
             Security > Screen Recording, enable this worker (or the app that \
             launched it), then start the session again. If capture already \
             works in your setup, set screen_capture_preflight: false in the \
             computer config to skip this check."
                .to_string(),
        )
    }
}

/// Current cursor position in the global point space (origin = main display
/// top-left) — the same space `xcap::Monitor::from_point` and `enigo` absolute
/// coordinates use. Returns `None` if unavailable; callers fall back to the
/// primary display.
#[cfg(target_os = "macos")]
fn cursor_point() -> Option<(i32, i32)> {
    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const core::ffi::c_void) -> *mut core::ffi::c_void;
        fn CGEventGetLocation(event: *mut core::ffi::c_void) -> CGPoint;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const core::ffi::c_void);
    }
    unsafe {
        let ev = CGEventCreate(core::ptr::null());
        if ev.is_null() {
            return None;
        }
        let p = CGEventGetLocation(ev);
        CFRelease(ev as *const _);
        Some((p.x.round() as i32, p.y.round() as i32))
    }
}

#[cfg(not(target_os = "macos"))]
fn cursor_point() -> Option<(i32, i32)> {
    None
}

/// Everything needed to map a downscaled-image coordinate to an absolute
/// `enigo` coordinate: the captured display's global origin (points) and the
/// scale from downscaled pixels to the display's logical points.
#[derive(Clone, Copy)]
struct Geom {
    origin_x: i32,
    origin_y: i32,
    scale_x: f64,
    scale_y: f64,
}

pub struct NativeHost {
    max_dimension: u32,
    jpeg_quality: u8,
    /// Optional display index override (from `sessions::start`); `None` picks
    /// the display under the cursor at first capture.
    monitor: Option<u32>,
    /// The display this session captures, learned on the first capture and
    /// fixed thereafter so coordinates keep meaning the same thing.
    pinned: OnceLock<u32>,
    /// The size of the last capture, so `screen_size` can answer from what the
    /// session already knows instead of capturing and encoding a whole frame.
    dims: ArcSwapOption<Screen>,
    /// That display's coordinate geometry, refreshed on every capture: the
    /// display stays the same, but its resolution or scale factor can change
    /// under us (mode switch, scaling change), and a stale mapping would put
    /// clicks somewhere the model never looked.
    geom: ArcSwapOption<Geom>,
}

impl NativeHost {
    pub fn new(max_dimension: u32, jpeg_quality: u8, monitor: Option<u32>) -> Self {
        Self {
            max_dimension: max_dimension.max(320),
            jpeg_quality: jpeg_quality.clamp(1, 100),
            monitor,
            pinned: OnceLock::new(),
            dims: ArcSwapOption::empty(),
            geom: ArcSwapOption::empty(),
        }
    }

    fn pinned_id(&self) -> Option<u32> {
        self.pinned.get().copied()
    }

    fn geom(&self) -> Option<Geom> {
        self.geom.load().as_deref().copied()
    }
}

/// List the local displays for `computer::displays`.
pub fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    let monitors = Monitor::all().map_err(|e| format!("enumerate monitors: {e}"))?;
    Ok(monitors
        .iter()
        .enumerate()
        .map(|(i, m)| DisplayInfo {
            index: i as u32,
            name: m.name().unwrap_or_default(),
            primary: m.is_primary().unwrap_or(false),
            builtin: m.is_builtin().unwrap_or(false),
            width: m.width().unwrap_or(0),
            height: m.height().unwrap_or(0),
        })
        .collect())
}

/// Resolve which display to capture: a pinned id if the session already chose
/// one, else the index override, else the display under the cursor, else the
/// primary, else the first.
fn resolve_monitor(pinned_id: Option<u32>, override_idx: Option<u32>) -> Result<Monitor, String> {
    let monitors = Monitor::all().map_err(|e| format!("enumerate monitors: {e}"))?;
    if monitors.is_empty() {
        return Err("no display found".to_string());
    }
    if let Some(id) = pinned_id {
        if let Some(m) = monitors
            .into_iter()
            .find(|m| m.id().map(|mid| mid == id).unwrap_or(false))
        {
            return Ok(m);
        }
        // Pinned display went away (unplugged); fall through to a fresh pick.
        return resolve_monitor(None, override_idx);
    }
    if let Some(i) = override_idx {
        return monitors
            .into_iter()
            .nth(i as usize)
            .ok_or_else(|| format!("display index {i} is out of range"));
    }
    if let Some((cx, cy)) = cursor_point() {
        if let Ok(m) = Monitor::from_point(cx, cy) {
            return Ok(m);
        }
    }
    let mut first: Option<Monitor> = None;
    for m in monitors {
        if m.is_primary().unwrap_or(false) {
            return Ok(m);
        }
        if first.is_none() {
            first = Some(m);
        }
    }
    first.ok_or_else(|| "no display found".to_string())
}

/// Downscale target: longest edge capped at `max_dim`, aspect preserved.
fn target_dims(w: u32, h: u32, max_dim: u32) -> (u32, u32) {
    let longest = w.max(h);
    if longest <= max_dim || longest == 0 {
        return (w, h);
    }
    let s = max_dim as f64 / longest as f64;
    (
        ((w as f64 * s).round() as u32).max(1),
        ((h as f64 * s).round() as u32).max(1),
    )
}

/// Capture the resolved display, downscale + JPEG-encode. Returns
/// (jpeg bytes, target w, target h, display id, geometry).
fn capture(
    pinned_id: Option<u32>,
    override_idx: Option<u32>,
    max_dim: u32,
    quality: u8,
) -> Result<(Vec<u8>, u32, u32, u32, Geom), String> {
    let monitor = resolve_monitor(pinned_id, override_idx)?;
    let id = monitor.id().unwrap_or(0);
    let img = monitor
        .capture_image()
        .map_err(|e| format!("screen capture failed: {e}"))?;
    let (rw, rh) = (img.width(), img.height());
    let (tw, th) = target_dims(rw, rh, max_dim);

    // The display's LOGICAL size + global origin (points). `enigo` absolute
    // coordinates and `Monitor::x/y/width/height` share this space, so mapping
    // downscaled pixels -> logical points -> +origin lands the click correctly
    // on this display at any scale factor. Fall back to the backing pixels at
    // origin (0,0) if the geometry query fails.
    let lx = monitor.x().unwrap_or(0);
    let ly = monitor.y().unwrap_or(0);
    let lw = monitor.width().unwrap_or(rw);
    let lh = monitor.height().unwrap_or(rh);
    let geom = Geom {
        origin_x: lx,
        origin_y: ly,
        scale_x: lw as f64 / tw.max(1) as f64,
        scale_y: lh as f64 / th.max(1) as f64,
    };

    let rgb = if (tw, th) == (rw, rh) {
        DynamicImage::ImageRgba8(img).to_rgb8()
    } else {
        DynamicImage::ImageRgba8(resize(&img, tw, th, FilterType::Triangle)).to_rgb8()
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut Cursor::new(&mut out), quality)
        .encode_image(&rgb)
        .map_err(|e| format!("jpeg encode failed: {e}"))?;
    Ok((out, tw, th, id, geom))
}

/// Map a downscaled-image coordinate to an absolute `enigo` coordinate through
/// the pinned display geometry (identity if not yet known).
fn to_global(x: i64, y: i64, geom: Option<Geom>) -> (i32, i32) {
    match geom {
        Some(g) => (
            g.origin_x + (x as f64 * g.scale_x).round() as i32,
            g.origin_y + (y as f64 * g.scale_y).round() as i32,
        ),
        None => (x as i32, y as i32),
    }
}

/// Run a closure with a fresh `Enigo` and the pinned geometry, after preflighting
/// Accessibility.
async fn with_enigo<F>(geom: Option<Geom>, f: F) -> Result<(), String>
where
    F: FnOnce(&mut Enigo, Option<Geom>) -> Result<(), String> + Send + 'static,
{
    spawn_blocking(move || {
        // Without Accessibility, macOS drops the event and enigo reports ok, so
        // the agent thinks a click landed when it did not. Fail clearly instead.
        if !perms::accessibility_granted() {
            return Err(
                "Accessibility is not granted to this process, so macOS is silently \
                 dropping synthetic input. Enable it in System Settings > Privacy & \
                 Security > Accessibility for the app running this worker, then retry."
                    .to_string(),
            );
        }
        let mut enigo =
            Enigo::new(&Settings::default()).map_err(|e| format!("enigo init failed: {e}"))?;
        f(&mut enigo, geom)
    })
    .await
    .map_err(|e| format!("input task failed: {e}"))?
}

fn input_err(e: enigo::InputError) -> String {
    format!("input failed: {e}")
}

/// The mouse button a drag holds down.
fn button_for(name: &str) -> Result<Button, String> {
    match name.to_ascii_lowercase().as_str() {
        "left" => Ok(Button::Left),
        "right" => Ok(Button::Right),
        "middle" => Ok(Button::Middle),
        other => Err(format!("unknown button '{other}' (left, right, middle)")),
    }
}

/// Map a key name (from `press`/`hotkey`) onto an enigo key. An unknown
/// multi-character name is an error rather than its first character: silently
/// pressing `f` for `f5` looks like the key worked.
fn key_for(name: &str) -> Result<Key, String> {
    let key = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "space" => Key::Space,
        "up" | "arrowup" => Key::UpArrow,
        "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow,
        "right" | "arrowright" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "cmd" | "command" | "meta" | "super" | "win" => Key::Meta,
        "ctrl" | "control" => Key::Control,
        "alt" | "option" => Key::Alt,
        "shift" => Key::Shift,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Unicode(c),
                _ => {
                    return Err(format!(
                        "unknown key '{other}' (a single character, or one of enter, tab, escape, \
                         backspace, delete, space, up, down, left, right, home, end, pageup, \
                         pagedown, f1-f12, cmd, ctrl, alt, shift)"
                    ))
                }
            }
        }
    };
    Ok(key)
}

impl NativeHost {
    async fn capture_shot(&self) -> Result<(Vec<u8>, u32, u32), String> {
        let (pinned, over, max_dim, q) = (
            self.pinned_id(),
            self.monitor,
            self.max_dimension,
            self.jpeg_quality,
        );
        let (bytes, tw, th, id, geom) = spawn_blocking(move || capture(pinned, over, max_dim, q))
            .await
            .map_err(|e| format!("capture task failed: {e}"))??;
        let bytes_dims = (tw, th);
        let _ = self.pinned.set(id);
        self.geom.store(Some(Arc::new(geom)));
        self.dims.store(Some(Arc::new(Screen {
            width: bytes_dims.0,
            height: bytes_dims.1,
        })));
        Ok((bytes, tw, th))
    }
}

#[async_trait]
impl Driver for NativeHost {
    async fn screen_size(&self) -> Result<Screen, String> {
        // Answer from the last capture when there is one: a full grab plus JPEG
        // encode is a lot of work to learn a number the session already has.
        if let Some(dims) = self.dims.load().as_deref().copied() {
            return Ok(dims);
        }
        let (_, tw, th) = self.capture_shot().await?;
        Ok(Screen {
            width: tw,
            height: th,
        })
    }

    async fn screenshot(&self) -> Result<Shot, String> {
        let (bytes, _, _) = self.capture_shot().await?;
        if bytes.is_empty() {
            return Err("screenshot: capture produced an empty image".to_string());
        }
        Ok(Shot::new(bytes))
    }

    async fn left_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(self.geom(), move |e, geom| {
            let (rx, ry) = to_global(x, y, geom);
            e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)?;
            e.button(Button::Left, Direction::Click).map_err(input_err)
        })
        .await
    }

    async fn right_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(self.geom(), move |e, geom| {
            let (rx, ry) = to_global(x, y, geom);
            e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)?;
            e.button(Button::Right, Direction::Click).map_err(input_err)
        })
        .await
    }

    async fn double_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(self.geom(), move |e, geom| {
            let (rx, ry) = to_global(x, y, geom);
            e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)?;
            e.button(Button::Left, Direction::Click)
                .map_err(input_err)?;
            e.button(Button::Left, Direction::Click).map_err(input_err)
        })
        .await
    }

    async fn move_cursor(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(self.geom(), move |e, geom| {
            let (rx, ry) = to_global(x, y, geom);
            e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)
        })
        .await
    }

    async fn scroll(&self, x: i64, y: i64, scroll_x: i64, scroll_y: i64) -> Result<(), String> {
        with_enigo(self.geom(), move |e, geom| {
            let (rx, ry) = to_global(x, y, geom);
            e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)?;
            if scroll_x != 0 {
                e.scroll(scroll_x as i32, Axis::Horizontal)
                    .map_err(input_err)?;
            }
            if scroll_y != 0 {
                e.scroll(scroll_y as i32, Axis::Vertical)
                    .map_err(input_err)?;
            }
            Ok(())
        })
        .await
    }

    async fn drag(&self, from: (i64, i64), to: (i64, i64), button: &str) -> Result<(), String> {
        let held = button_for(button)?;
        with_enigo(self.geom(), move |e, geom| {
            let (fx, fy) = to_global(from.0, from.1, geom);
            let (tx, ty) = to_global(to.0, to.1, geom);
            e.move_mouse(fx, fy, Coordinate::Abs).map_err(input_err)?;
            e.button(held, Direction::Press).map_err(input_err)?;
            e.move_mouse(tx, ty, Coordinate::Abs).map_err(input_err)?;
            e.button(held, Direction::Release).map_err(input_err)
        })
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        let text = text.to_string();
        with_enigo(None, move |e, _| e.text(&text).map_err(input_err)).await
    }

    async fn keypress(&self, keys: &[String]) -> Result<(), String> {
        // Resolve every name before pressing anything: a chord that fails
        // halfway would leave modifiers held down on the desktop.
        let keys = keys
            .iter()
            .map(|k| key_for(k))
            .collect::<Result<Vec<Key>, String>>()?;
        with_enigo(None, move |e, _| {
            // A single key is a tap; a chord holds all but the last as
            // modifiers, taps the last, then releases the modifiers.
            let Some((last, mods)) = keys.split_last() else {
                return Ok(());
            };
            for m in mods {
                e.key(*m, Direction::Press).map_err(input_err)?;
            }
            let tapped = e.key(*last, Direction::Click).map_err(input_err);
            for m in mods.iter().rev() {
                let _ = e.key(*m, Direction::Release);
            }
            tapped
        })
        .await
    }

    async fn accessibility_tree(&self) -> Result<serde_json::Value, String> {
        // No native a11y tree yet; observe falls back to the screenshot.
        Ok(serde_json::Value::Null)
    }

    async fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_for_rejects_unknown_multi_char_names() {
        assert!(matches!(key_for("enter"), Ok(Key::Return)));
        assert!(matches!(key_for("a"), Ok(Key::Unicode('a'))));
        assert!(matches!(key_for("F5"), Ok(Key::F5)));
        // Silently pressing 'f' would look like the key worked.
        assert!(key_for("f13").is_err());
        assert!(key_for("").is_err());
    }

    #[test]
    fn button_for_names_the_three_buttons() {
        assert!(matches!(button_for("LEFT"), Ok(Button::Left)));
        assert!(matches!(button_for("right"), Ok(Button::Right)));
        assert!(matches!(button_for("middle"), Ok(Button::Middle)));
        assert!(button_for("thumb").is_err());
    }

    #[test]
    fn target_dims_caps_longest_edge() {
        assert_eq!(target_dims(3024, 1964, 1280), (1280, 831));
        assert_eq!(target_dims(800, 600, 1280), (800, 600));
        assert_eq!(target_dims(0, 0, 1280), (0, 0));
    }

    #[test]
    fn to_global_maps_downscaled_to_display_points() {
        // Built-in Retina: 1280-wide downscale of a 1512-point display -> ~1.18x,
        // origin (0,0).
        let g = Geom {
            origin_x: 0,
            origin_y: 0,
            scale_x: 1512.0 / 1280.0,
            scale_y: 982.0 / 831.0,
        };
        assert_eq!(to_global(640, 415, Some(g)), (756, 490));
        // Secondary display to the right of a 1512-pt main, 1:1 scale.
        let g2 = Geom {
            origin_x: 1512,
            origin_y: 0,
            scale_x: 1.0,
            scale_y: 1.0,
        };
        assert_eq!(to_global(100, 200, Some(g2)), (1612, 200));
        // Identity when the display is not yet known.
        assert_eq!(to_global(50, 60, None), (50, 60));
    }
}
