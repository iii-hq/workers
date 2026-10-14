//! The macOS/Windows half of the native host: capture via `xcap`, mouse and
//! keyboard via `enigo`.
//!
//! `enigo` state is not `Send`, so an `Enigo` is created, used and dropped
//! inside each call. On macOS the worker process needs Screen Recording
//! (capture) and Accessibility (input) permission. Input is preflighted with
//! `AXIsProcessTrusted` so a dropped click fails loudly instead of a false
//! success. Screen Recording is NOT preflighted: `CGPreflightScreenCaptureAccess`
//! reports per-process state that a child of a granted terminal does not
//! inherit, so it false-negatives on a capture that actually works; a missing
//! grant instead surfaces as a wallpaper-only image.

use std::time::{Duration, Instant};

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use xcap::Monitor;

use super::{pin_process_dpi, Btn, Grab};
use crate::driver::DisplayInfo;

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

/// List the local displays for `computer::displays`.
pub fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    pin_process_dpi();
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

/// macOS/Windows need no connection; the type keeps the OS-half signatures
/// the same on every OS.
#[derive(Default)]
pub struct Conn;

/// Capture the resolved display at full resolution.
pub fn grab(_: &Conn, pinned_id: Option<u32>, override_idx: Option<u32>) -> Result<Grab, String> {
    let monitor = resolve_monitor(pinned_id, override_idx)?;
    // Pin only an id the OS actually reported: a placeholder would pin the
    // session to a display that does not exist and defeat the fallback.
    let id = monitor.id().ok();
    let img = monitor
        .capture_image()
        .map_err(|e| format!("screen capture failed: {e}"))?;
    // The display's size + global origin as `xcap` reports them: logical
    // points on macOS, physical pixels on Windows (per-monitor-v2). `enigo`
    // absolute coordinates and `Monitor::x/y/width/height` share this space;
    // never multiply by `scale_factor()`. Fall back to the backing pixels at
    // origin (0,0) if the geometry query fails.
    Ok(Grab {
        x: monitor.x().unwrap_or(0),
        y: monitor.y().unwrap_or(0),
        w: monitor.width().unwrap_or(img.width()),
        h: monitor.height().unwrap_or(img.height()),
        id,
        img,
    })
}

/// Run `f` with a fresh `Enigo`, after preflighting Accessibility.
fn with_enigo(f: impl FnOnce(&mut Enigo) -> Result<(), String>) -> Result<(), String> {
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
    f(&mut enigo)
}

fn input_err(e: enigo::InputError) -> String {
    format!("input failed: {e}")
}

/// Move the pointer, and on macOS wait until the OS reports it there. The
/// window server applies a synthetic move asynchronously, while enigo 0.3
/// posts a button at the position it reads back (`NSEvent::mouseLocation`)
/// and a new scroll event carries the current position: either one, sent right
/// after the move, lands where the pointer was. ±1 absorbs the truncation of a
/// fractional pointer position.
fn move_abs(e: &mut Enigo, (x, y): (i32, i32)) -> Result<(), String> {
    e.move_mouse(x, y, Coordinate::Abs).map_err(input_err)?;
    if cfg!(not(target_os = "macos")) {
        return Ok(());
    }
    let start = Instant::now();
    loop {
        let (cx, cy) = e.location().map_err(input_err)?;
        if (cx - x).abs() <= 1 && (cy - y).abs() <= 1 {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(1) {
            return Err(format!(
                "input failed: the pointer did not reach screen point ({x}, {y}) \
                 within 1 s; it is at ({cx}, {cy})"
            ));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn button(b: Btn) -> Button {
    match b {
        Btn::Left => Button::Left,
        Btn::Right => Button::Right,
        Btn::Middle => Button::Middle,
    }
}

pub fn click(_: &Conn, (x, y): (i32, i32), b: Btn, count: u8) -> Result<(), String> {
    with_enigo(|e| {
        move_abs(e, (x, y))?;
        for _ in 0..count {
            e.button(button(b), Direction::Click).map_err(input_err)?;
        }
        Ok(())
    })
}

pub fn move_to(_: &Conn, (x, y): (i32, i32)) -> Result<(), String> {
    with_enigo(|e| move_abs(e, (x, y)))
}

pub fn scroll(_: &Conn, (x, y): (i32, i32), sx: i64, sy: i64) -> Result<(), String> {
    with_enigo(|e| {
        move_abs(e, (x, y))?;
        if sx != 0 {
            e.scroll(sx as i32, Axis::Horizontal).map_err(input_err)?;
        }
        if sy != 0 {
            e.scroll(sy as i32, Axis::Vertical).map_err(input_err)?;
        }
        Ok(())
    })
}

pub fn drag(_: &Conn, (fx, fy): (i32, i32), (tx, ty): (i32, i32), b: Btn) -> Result<(), String> {
    with_enigo(|e| {
        move_abs(e, (fx, fy))?;
        e.button(button(b), Direction::Press).map_err(input_err)?;
        // Release even when the move fails: a held button turns every later
        // move into a drag.
        let moved = move_abs(e, (tx, ty));
        let released = e.button(button(b), Direction::Release).map_err(input_err);
        moved.and(released)
    })
}

pub fn type_text(_: &Conn, text: &str) -> Result<(), String> {
    with_enigo(|e| e.text(text).map_err(input_err))
}

pub fn keys(_: &Conn, names: &[String]) -> Result<(), String> {
    // Resolve every name before pressing anything: a chord that fails
    // halfway would leave modifiers held down on the desktop.
    let keys = names
        .iter()
        .map(|k| key_for(k))
        .collect::<Result<Vec<Key>, String>>()?;
    with_enigo(|e| {
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
}
