//! The macOS/Windows half of the native host: capture via `xcap`, keyboard
//! via `enigo`, the pointer via CoreGraphics on macOS and `enigo` on Windows.
//!
//! `enigo` state is not `Send`, so an `Enigo` is created, used and dropped
//! inside each call. On macOS the worker process needs Screen Recording
//! (capture) and Accessibility (input) permission. Input is preflighted with
//! `AXIsProcessTrusted` so a dropped click fails loudly instead of a false
//! success. Screen Recording is NOT preflighted: `CGPreflightScreenCaptureAccess`
//! reports per-process state that a child of a granted terminal does not
//! inherit, so it false-negatives on a capture that actually works; a missing
//! grant instead surfaces as a wallpaper-only image.

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
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

/// Without Accessibility, macOS drops synthetic input while the posting call
/// still succeeds, so the agent would think a click landed when it did not.
/// Fail clearly instead.
fn preflight() -> Result<(), String> {
    if perms::accessibility_granted() {
        return Ok(());
    }
    Err(
        "Accessibility is not granted to this process, so macOS is silently \
         dropping synthetic input. Enable it in System Settings > Privacy & \
         Security > Accessibility for the app running this worker, then retry."
            .to_string(),
    )
}

/// Run `f` with a fresh `Enigo`, after preflighting Accessibility.
fn with_enigo(f: impl FnOnce(&mut Enigo) -> Result<(), String>) -> Result<(), String> {
    preflight()?;
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("enigo init failed: {e}"))?;
    f(&mut enigo)
}

fn input_err(e: enigo::InputError) -> String {
    format!("input failed: {e}")
}

pub fn click(_: &Conn, at: (i32, i32), b: Btn, count: u8) -> Result<(), String> {
    pointer::click(at, b, count)
}

pub fn move_to(_: &Conn, at: (i32, i32)) -> Result<(), String> {
    pointer::move_to(at)
}

pub fn scroll(_: &Conn, at: (i32, i32), sx: i64, sy: i64) -> Result<(), String> {
    pointer::scroll(at, sx, sy)
}

pub fn drag(_: &Conn, from: (i32, i32), to: (i32, i32), b: Btn) -> Result<(), String> {
    pointer::drag(from, to, b)
}

/// macOS pointer input straight through CoreGraphics: every event carries its
/// own position, so nothing reads the pointer back. enigo 0.3 does, through
/// `NSEvent::mouseLocation`, whose screen geometry a process without an AppKit
/// run loop never updates after a display change (clicks then miss by the
/// height difference), right after a synthetic move the window server has not
/// applied yet (clicks land where the pointer was), and it picks moved vs
/// dragged from a button state that lags the press (drags arrive as moves).
#[cfg(target_os = "macos")]
mod pointer {
    use std::ffi::c_void;
    use std::time::Duration;

    use super::{preflight, Btn};

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) struct Point {
        pub(super) x: f64,
        pub(super) y: f64,
    }

    /// One mouse event to post. Down/Up carry their click count.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) enum Ev {
        Move(Point),
        Down(Btn, Point, i64),
        Up(Btn, Point, i64),
        Drag(Btn, Point),
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreateMouseEvent(
            source: *const c_void,
            kind: u32,
            at: Point,
            button: u32,
        ) -> *mut c_void;
        fn CGEventCreateScrollWheelEvent2(
            source: *const c_void,
            units: u32,
            wheel_count: u32,
            wheel1: i32,
            wheel2: i32,
            wheel3: i32,
        ) -> *mut c_void;
        fn CGEventSetLocation(event: *mut c_void, at: Point);
        fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
        fn CGEventPost(tap: u32, event: *mut c_void);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    const HID_TAP: u32 = 0; // kCGHIDEventTap
    const CLICK_STATE: u32 = 1; // kCGMouseEventClickState
    const MOUSE_MOVED: u32 = 5; // kCGEventMouseMoved
    const LINE: u32 = 1; // kCGScrollEventUnitLine

    // ponytail: 10 steps 8 ms apart is enough for AppKit drag tracking (the
    // display arrangement in System Settings); slow it if an app needs more.
    const DRAG_STEPS: i32 = 10;
    const DRAG_STEP_DELAY: Duration = Duration::from_millis(8);

    /// (down, up, dragged, button) CGEventType / CGMouseButton values.
    fn kinds(b: Btn) -> (u32, u32, u32, u32) {
        match b {
            Btn::Left => (1, 2, 6, 0),
            Btn::Right => (3, 4, 7, 1),
            Btn::Middle => (25, 26, 27, 2),
        }
    }

    fn point((x, y): (i32, i32)) -> Point {
        Point {
            x: f64::from(x),
            y: f64::from(y),
        }
    }

    pub(super) fn click_events(at: (i32, i32), b: Btn, count: u8) -> Vec<Ev> {
        let at = point(at);
        let mut evs = vec![Ev::Move(at)];
        for n in 1..=i64::from(count) {
            evs.extend([Ev::Down(b, at, n), Ev::Up(b, at, n)]);
        }
        evs
    }

    pub(super) fn drag_events(from: (i32, i32), to: (i32, i32), b: Btn) -> Vec<Ev> {
        let (f, t) = (point(from), point(to));
        let mut evs = vec![Ev::Move(f), Ev::Down(b, f, 1)];
        for i in 1..=DRAG_STEPS {
            let k = f64::from(i) / f64::from(DRAG_STEPS);
            evs.push(Ev::Drag(
                b,
                Point {
                    x: f.x + (t.x - f.x) * k,
                    y: f.y + (t.y - f.y) * k,
                },
            ));
        }
        evs.push(Ev::Up(b, t, 1));
        evs
    }

    fn post(ev: Ev) -> Result<(), String> {
        let (kind, at, button, count) = match ev {
            Ev::Move(at) => (MOUSE_MOVED, at, 0, None),
            Ev::Down(b, at, n) => (kinds(b).0, at, kinds(b).3, Some(n)),
            Ev::Up(b, at, n) => (kinds(b).1, at, kinds(b).3, Some(n)),
            Ev::Drag(b, at) => (kinds(b).2, at, kinds(b).3, None),
        };
        unsafe {
            let e = CGEventCreateMouseEvent(std::ptr::null(), kind, at, button);
            if e.is_null() {
                return Err("input failed: could not create a mouse event".to_string());
            }
            if let Some(n) = count {
                CGEventSetIntegerValueField(e, CLICK_STATE, n);
            }
            CGEventPost(HID_TAP, e);
            CFRelease(e);
        }
        Ok(())
    }

    fn post_all(evs: &[Ev]) -> Result<(), String> {
        preflight()?;
        let mut held = None;
        for &ev in evs {
            if let Err(e) = post(ev) {
                // Never leave a button held: it turns every later move into
                // a drag.
                if let Some((b, at)) = held {
                    let _ = post(Ev::Up(b, at, 1));
                }
                return Err(e);
            }
            match ev {
                Ev::Down(b, at, _) => held = Some((b, at)),
                Ev::Drag(b, at) => {
                    held = Some((b, at));
                    std::thread::sleep(DRAG_STEP_DELAY);
                }
                Ev::Up(..) => held = None,
                Ev::Move(_) => {}
            }
        }
        Ok(())
    }

    pub(super) fn click(at: (i32, i32), b: Btn, count: u8) -> Result<(), String> {
        post_all(&click_events(at, b, count))
    }

    pub(super) fn move_to(at: (i32, i32)) -> Result<(), String> {
        post_all(&[Ev::Move(point(at))])
    }

    pub(super) fn drag(from: (i32, i32), to: (i32, i32), b: Btn) -> Result<(), String> {
        post_all(&drag_events(from, to, b))
    }

    /// enigo's convention: a positive amount scrolls down / right.
    pub(super) fn scroll(at: (i32, i32), sx: i64, sy: i64) -> Result<(), String> {
        let at = point(at);
        post_all(&[Ev::Move(at)])?;
        for (len, wheels, w1, w2) in [(sx, 2, 0, -sx), (sy, 1, -sy, 0)] {
            if len == 0 {
                continue;
            }
            unsafe {
                let e = CGEventCreateScrollWheelEvent2(
                    std::ptr::null(),
                    LINE,
                    wheels,
                    w1 as i32,
                    w2 as i32,
                    0,
                );
                if e.is_null() {
                    return Err("input failed: could not create a scroll event".to_string());
                }
                CGEventSetLocation(e, at);
                CGEventPost(HID_TAP, e);
                CFRelease(e);
            }
        }
        Ok(())
    }
}

/// Windows pointer input through enigo: its button input carries no
/// position, so it lands where the move just put the pointer.
#[cfg(not(target_os = "macos"))]
mod pointer {
    use enigo::{Axis, Button, Coordinate, Direction, Mouse};

    use super::{input_err, with_enigo, Btn};

    fn button(b: Btn) -> Button {
        match b {
            Btn::Left => Button::Left,
            Btn::Right => Button::Right,
            Btn::Middle => Button::Middle,
        }
    }

    pub(super) fn click((x, y): (i32, i32), b: Btn, count: u8) -> Result<(), String> {
        with_enigo(|e| {
            e.move_mouse(x, y, Coordinate::Abs).map_err(input_err)?;
            for _ in 0..count {
                e.button(button(b), Direction::Click).map_err(input_err)?;
            }
            Ok(())
        })
    }

    pub(super) fn move_to((x, y): (i32, i32)) -> Result<(), String> {
        with_enigo(|e| e.move_mouse(x, y, Coordinate::Abs).map_err(input_err))
    }

    pub(super) fn scroll((x, y): (i32, i32), sx: i64, sy: i64) -> Result<(), String> {
        with_enigo(|e| {
            e.move_mouse(x, y, Coordinate::Abs).map_err(input_err)?;
            if sx != 0 {
                e.scroll(sx as i32, Axis::Horizontal).map_err(input_err)?;
            }
            if sy != 0 {
                e.scroll(sy as i32, Axis::Vertical).map_err(input_err)?;
            }
            Ok(())
        })
    }

    pub(super) fn drag((fx, fy): (i32, i32), (tx, ty): (i32, i32), b: Btn) -> Result<(), String> {
        with_enigo(|e| {
            e.move_mouse(fx, fy, Coordinate::Abs).map_err(input_err)?;
            e.button(button(b), Direction::Press).map_err(input_err)?;
            // Release even when the move fails: a held button turns every
            // later move into a drag.
            let moved = e.move_mouse(tx, ty, Coordinate::Abs).map_err(input_err);
            let released = e.button(button(b), Direction::Release).map_err(input_err);
            moved.and(released)
        })
    }
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

#[cfg(all(test, target_os = "macos"))]
mod pointer_tests {
    use super::pointer::{click_events, drag_events, Ev, Point};
    use super::Btn;

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn a_click_carries_its_target_on_every_event_and_counts_repeats() {
        let p = at(10.0, 20.0);
        assert_eq!(
            click_events((10, 20), Btn::Left, 2),
            vec![
                Ev::Move(p),
                Ev::Down(Btn::Left, p, 1),
                Ev::Up(Btn::Left, p, 1),
                Ev::Down(Btn::Left, p, 2),
                Ev::Up(Btn::Left, p, 2),
            ]
        );
    }

    #[test]
    fn a_drag_is_dragged_all_the_way_between_press_and_release() {
        let evs = drag_events((0, 0), (100, 50), Btn::Right);
        assert_eq!(evs[0], Ev::Move(at(0.0, 0.0)));
        assert_eq!(evs[1], Ev::Down(Btn::Right, at(0.0, 0.0), 1));
        assert_eq!(evs.last(), Some(&Ev::Up(Btn::Right, at(100.0, 50.0), 1)));
        // Between press and release only dragged events, never a plain move,
        // advancing to the target.
        let path: Vec<Point> = evs[2..evs.len() - 1]
            .iter()
            .map(|ev| match ev {
                Ev::Drag(Btn::Right, p) => *p,
                other => panic!("not a drag between press and release: {other:?}"),
            })
            .collect();
        assert!(path.len() > 1, "{path:?}");
        assert!(path.windows(2).all(|w| w[1].x > w[0].x && w[1].y > w[0].y));
        assert_eq!(path.last(), Some(&at(100.0, 50.0)));
    }
}
