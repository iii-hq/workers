//! macOS input-simulation backend using Quartz `CGEvent` APIs.
//!
//! Posts synthesised pointer and keyboard events to the HID event tap
//! (`kCGHIDEventTap`). Requires the host process to have the **Accessibility**
//! and **Input Monitoring** privacy grants — absent those, the OS silently
//! drops events (there is no API-level error signal). The backend itself only
//! surfaces failures from `CGEvent*` allocation.
//!
//! Drag events: `pointer_drag` is overridden to emit the distinct
//! `kCGEventLeftMouseDragged` / `…RightMouseDragged` / `…OtherMouseDragged`
//! types between the down and up — drag-and-drop source views filter on
//! those and ignore ordinary `kCGEventMouseMoved`.
//!
//! `type_text` uses `CGEventKeyboardSetUnicodeString` so case, IME, and
//! dead-key composition are produced by the OS text path rather than being
//! synthesised against the active keyboard layout.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use core_foundation::base::{CFRelease, CFTypeRef};

use xa11y_core::input::{InputProvider, Key, MouseButton, Point, ScrollDelta};
use xa11y_core::{Error, Result};

// ── Quartz / CoreGraphics FFI ───────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

type CGEventRef = *mut c_void;
type CGEventSourceRef = *mut c_void;

// CGEventTapLocation
const K_CG_HID_EVENT_TAP: u32 = 0;

// CGEventType
const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const K_CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const K_CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
const K_CG_EVENT_MOUSE_MOVED: u32 = 5;
const K_CG_EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
const K_CG_EVENT_RIGHT_MOUSE_DRAGGED: u32 = 7;
const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const K_CG_EVENT_OTHER_MOUSE_UP: u32 = 26;
const K_CG_EVENT_OTHER_MOUSE_DRAGGED: u32 = 27;

// CGMouseButton
const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
const K_CG_MOUSE_BUTTON_RIGHT: u32 = 1;
const K_CG_MOUSE_BUTTON_CENTER: u32 = 2;

// CGScrollEventUnit
const K_CG_SCROLL_EVENT_UNIT_LINE: u32 = 1;

// CGEventField
const K_CG_MOUSE_EVENT_CLICK_STATE: u32 = 1;

// CGEventFlags (modifier flags applied at post time).
const K_CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
const K_CG_EVENT_FLAG_MASK_CONTROL: u64 = 0x0004_0000;
const K_CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 0x0010_0000;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateMouseEvent(
        source: CGEventSourceRef,
        mouse_type: u32,
        mouse_cursor_position: CGPoint,
        mouse_button: u32,
    ) -> CGEventRef;

    fn CGEventCreateScrollWheelEvent(
        source: CGEventSourceRef,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> CGEventRef;

    fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        virtual_key: u16,
        key_down: bool,
    ) -> CGEventRef;

    fn CGEventPost(tap: u32, event: CGEventRef);

    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);

    fn CGEventSetFlags(event: CGEventRef, flags: u64);

    fn CGEventGetFlags(event: CGEventRef) -> u64;

    fn CGEventKeyboardSetUnicodeString(event: CGEventRef, length: usize, chars: *const u16);
}

// ── Provider ────────────────────────────────────────────────────────

/// CGEvent-backed [`InputProvider`].
///
/// All calls serialise through an internal mutex so interleaving threads can't
/// scramble the posted event stream (macOS delivers events globally to the
/// focused app, so ordering is session-wide).
#[derive(Default)]
pub struct MacOSInputProvider {
    lock: Mutex<()>,
    /// CGEventFlags for the modifier keys currently held via `key_down`.
    ///
    /// macOS stamps an event with modifier state **at creation time**, read
    /// from the window server's combined state — and a *synthetic* key-down
    /// does not update that state synchronously. Posting `key_down(Shift)`
    /// and a click back-to-back, as core's `with_keys_held` does for
    /// `Mouse::click_with`, therefore produced a click carrying no modifier
    /// at all: by the time the window server caught up, the key-up had
    /// already been posted.
    ///
    /// Tracking the held flags here and stamping them onto every event we
    /// post makes the modifier deterministic, with no sleep to tune. It also
    /// covers `drag_with(held: …)`, which reaches the same primitives.
    held_flags: AtomicU64,
}

impl MacOSInputProvider {
    pub fn new() -> Result<Self> {
        Ok(Self::default())
    }

    fn post(&self, event: CGEventRef) -> Result<()> {
        if event.is_null() {
            return Err(Error::Platform {
                code: -1,
                message: "CGEventCreate* returned NULL".to_string(),
            });
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        // Stamp any modifiers held via `key_down` onto this event. See
        // `held_flags` for why the window server's own state can't be relied
        // on here. OR rather than assign, so flags the event already carries
        // (caps lock, a physically-held key) survive.
        let held = self.held_flags.load(Ordering::SeqCst);
        if held != 0 {
            // SAFETY: `event` is a valid CGEventRef we just created.
            unsafe { CGEventSetFlags(event, CGEventGetFlags(event) | held) };
        }
        // SAFETY: `event` is a valid CGEventRef we just created; CGEventPost
        // takes an owned +0 borrow and does not consume it.
        unsafe {
            CGEventPost(K_CG_HID_EVENT_TAP, event);
            CFRelease(event as CFTypeRef);
        }
        Ok(())
    }

    fn mouse_event(&self, ty: u32, at: Point, button: u32) -> Result<CGEventRef> {
        let pt = CGPoint {
            x: at.x as f64,
            y: at.y as f64,
        };
        // SAFETY: nil source is valid per the CGEventSource docs ("Use NULL to
        // create a combined state from the pasted-in event source defaults").
        let ev = unsafe { CGEventCreateMouseEvent(std::ptr::null_mut(), ty, pt, button) };
        if ev.is_null() {
            return Err(Error::Platform {
                code: -1,
                message: format!("CGEventCreateMouseEvent failed (type={ty})"),
            });
        }
        Ok(ev)
    }

    fn keyboard_event(&self, vk: u16, down: bool) -> Result<CGEventRef> {
        // SAFETY: nil source is allowed by CGEventSource contract.
        let ev = unsafe { CGEventCreateKeyboardEvent(std::ptr::null_mut(), vk, down) };
        if ev.is_null() {
            return Err(Error::Platform {
                code: -1,
                message: "CGEventCreateKeyboardEvent failed".to_string(),
            });
        }
        Ok(ev)
    }
}

// ── Mapping helpers ─────────────────────────────────────────────────

fn cg_button(b: MouseButton) -> u32 {
    match b {
        MouseButton::Left => K_CG_MOUSE_BUTTON_LEFT,
        MouseButton::Right => K_CG_MOUSE_BUTTON_RIGHT,
        MouseButton::Middle => K_CG_MOUSE_BUTTON_CENTER,
    }
}

fn cg_button_down(b: MouseButton) -> u32 {
    match b {
        MouseButton::Left => K_CG_EVENT_LEFT_MOUSE_DOWN,
        MouseButton::Right => K_CG_EVENT_RIGHT_MOUSE_DOWN,
        MouseButton::Middle => K_CG_EVENT_OTHER_MOUSE_DOWN,
    }
}

fn cg_button_up(b: MouseButton) -> u32 {
    match b {
        MouseButton::Left => K_CG_EVENT_LEFT_MOUSE_UP,
        MouseButton::Right => K_CG_EVENT_RIGHT_MOUSE_UP,
        MouseButton::Middle => K_CG_EVENT_OTHER_MOUSE_UP,
    }
}

fn cg_button_dragged(b: MouseButton) -> u32 {
    match b {
        MouseButton::Left => K_CG_EVENT_LEFT_MOUSE_DRAGGED,
        MouseButton::Right => K_CG_EVENT_RIGHT_MOUSE_DRAGGED,
        MouseButton::Middle => K_CG_EVENT_OTHER_MOUSE_DRAGGED,
    }
}

/// Virtual key code for a [`Key`]. Codes are the standard HIToolbox
/// `Events.h` keycodes (USB HID usage page → physical key on a US layout
/// for letters/digits).
fn vk_for(key: &Key) -> Result<u16> {
    let vk = match key {
        Key::Shift => 0x38,
        Key::Ctrl => 0x3B,
        Key::Alt => 0x3A,
        Key::Meta => 0x37,
        Key::Enter => 0x24,
        Key::Escape => 0x35,
        Key::Backspace => 0x33,
        Key::Tab => 0x30,
        Key::Space => 0x31,
        Key::Delete => 0x75,
        // macOS has no "Insert" key. The "Help" key on older keyboards
        // occupies the same position and is the conventional mapping.
        Key::Insert => 0x72,
        Key::ArrowUp => 0x7E,
        Key::ArrowDown => 0x7D,
        Key::ArrowLeft => 0x7B,
        Key::ArrowRight => 0x7C,
        Key::Home => 0x73,
        Key::End => 0x77,
        Key::PageUp => 0x74,
        Key::PageDown => 0x79,
        Key::F(n) => match n {
            1 => 0x7A,
            2 => 0x78,
            3 => 0x63,
            4 => 0x76,
            5 => 0x60,
            6 => 0x61,
            7 => 0x62,
            8 => 0x64,
            9 => 0x65,
            10 => 0x6D,
            11 => 0x67,
            12 => 0x6F,
            13 => 0x69,
            14 => 0x6B,
            15 => 0x71,
            16 => 0x6A,
            17 => 0x40,
            18 => 0x4F,
            19 => 0x50,
            20 => 0x5A,
            _ => {
                return Err(Error::InvalidActionData {
                    message: format!("F{n} has no HIToolbox keycode on macOS"),
                });
            }
        },
        Key::Char(c) => vk_for_char(*c)?,
    };
    Ok(vk)
}

/// Virtual key code for `Key::Char`. The [`Key`] contract guarantees `c` is
/// not ASCII uppercase. Letters, digits, and common US-layout punctuation
/// are mapped here; anything else returns [`Error::Unsupported`] and the
/// caller is expected to use `Keyboard::type_text` instead.
fn vk_for_char(c: char) -> Result<u16> {
    let vk = match c {
        'a' => 0x00,
        'b' => 0x0B,
        'c' => 0x08,
        'd' => 0x02,
        'e' => 0x0E,
        'f' => 0x03,
        'g' => 0x05,
        'h' => 0x04,
        'i' => 0x22,
        'j' => 0x26,
        'k' => 0x28,
        'l' => 0x25,
        'm' => 0x2E,
        'n' => 0x2D,
        'o' => 0x1F,
        'p' => 0x23,
        'q' => 0x0C,
        'r' => 0x0F,
        's' => 0x01,
        't' => 0x11,
        'u' => 0x20,
        'v' => 0x09,
        'w' => 0x0D,
        'x' => 0x07,
        'y' => 0x10,
        'z' => 0x06,
        '0' => 0x1D,
        '1' => 0x12,
        '2' => 0x13,
        '3' => 0x14,
        '4' => 0x15,
        '5' => 0x17,
        '6' => 0x16,
        '7' => 0x1A,
        '8' => 0x1C,
        '9' => 0x19,
        ';' => 0x29,
        '/' => 0x2C,
        '`' => 0x32,
        '[' => 0x21,
        '\\' => 0x2A,
        ']' => 0x1E,
        '\'' => 0x27,
        ',' => 0x2B,
        '.' => 0x2F,
        '-' => 0x1B,
        '=' => 0x18,
        _ => {
            return Err(Error::Unsupported {
                feature: format!(
                    "Key::Char('{c}') has no HIToolbox virtual-key mapping; \
                     use Keyboard::type_text for arbitrary characters"
                ),
            });
        }
    };
    Ok(vk)
}

// ── InputProvider impl ──────────────────────────────────────────────

impl InputProvider for MacOSInputProvider {
    fn pointer_move(&self, to: Point) -> Result<()> {
        // For mouse-moved the button argument is ignored.
        let ev = self.mouse_event(K_CG_EVENT_MOUSE_MOVED, to, K_CG_MOUSE_BUTTON_LEFT)?;
        self.post(ev)
    }

    fn pointer_down(&self, button: MouseButton) -> Result<()> {
        // CGEvent needs a position; pass (0,0) — the system uses the current
        // cursor location for button-only events anyway, but supplying a point
        // keeps the struct well-formed.
        let ev = self.mouse_event(cg_button_down(button), Point::new(0, 0), cg_button(button))?;
        self.post(ev)
    }

    fn pointer_up(&self, button: MouseButton) -> Result<()> {
        let ev = self.mouse_event(cg_button_up(button), Point::new(0, 0), cg_button(button))?;
        self.post(ev)
    }

    fn pointer_click(&self, at: Point, button: MouseButton, count: u32) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        self.pointer_move(at)?;
        // `kCGMouseEventClickState` tells the OS which click in a sequence this
        // is (1 = single, 2 = double, …). We emit N paired down/up events
        // with increasing click-state so AppKit/Cocoa recognises multi-clicks
        // regardless of the OS double-click timing window.
        for i in 1..=count {
            let down = self.mouse_event(cg_button_down(button), at, cg_button(button))?;
            // SAFETY: down is a valid CGEventRef.
            unsafe {
                CGEventSetIntegerValueField(down, K_CG_MOUSE_EVENT_CLICK_STATE, i as i64);
            }
            self.post(down)?;
            let up = self.mouse_event(cg_button_up(button), at, cg_button(button))?;
            unsafe {
                CGEventSetIntegerValueField(up, K_CG_MOUSE_EVENT_CLICK_STATE, i as i64);
            }
            self.post(up)?;
        }
        Ok(())
    }

    fn pointer_scroll(&self, at: Point, delta: ScrollDelta) -> Result<()> {
        self.pointer_move(at)?;
        // Quartz scroll convention: positive wheel1 = scroll up (content down),
        // which matches the xa11y ScrollDelta doc invariant ("positive dy
        // scrolls content down").
        // SAFETY: nil source is valid.
        let ev = unsafe {
            CGEventCreateScrollWheelEvent(
                std::ptr::null_mut(),
                K_CG_SCROLL_EVENT_UNIT_LINE,
                2,
                delta.dy,
                delta.dx,
                0,
            )
        };
        if ev.is_null() {
            return Err(Error::Platform {
                code: -1,
                message: "CGEventCreateScrollWheelEvent failed".to_string(),
            });
        }
        self.post(ev)
    }

    fn key_down(&self, key: &Key) -> Result<()> {
        let vk = vk_for(key)?;
        let ev = self.keyboard_event(vk, true)?;
        // Modifier keys posted as an ordinary key-down don't set the
        // event-flags bit Cocoa inspects for "is Shift held". Record the flag
        // *before* posting so this event carries it too, matching hardware —
        // a real Shift key-down already reports shift as set. `post` stamps
        // it onto this and every later event until `key_up` clears it.
        let flags = modifier_flag_for(key);
        if flags != 0 {
            self.held_flags.fetch_or(flags, Ordering::SeqCst);
        }
        self.post(ev)
    }

    fn key_up(&self, key: &Key) -> Result<()> {
        let vk = vk_for(key)?;
        let ev = self.keyboard_event(vk, false)?;
        // Clear before posting, again matching hardware: a real Shift key-up
        // reports the modifier as already released.
        let flags = modifier_flag_for(key);
        if flags != 0 {
            self.held_flags.fetch_and(!flags, Ordering::SeqCst);
        }
        self.post(ev)
    }

    fn type_text(&self, text: &str) -> Result<()> {
        // CGEventKeyboardSetUnicodeString replaces the synthetic keystroke
        // the event would normally generate with the supplied UTF-16 string,
        // so we get correct case/IME without synthesising shift chords.
        for chunk in text.chars().collect::<Vec<_>>().chunks(20) {
            let s: String = chunk.iter().collect();
            let utf16: Vec<u16> = s.encode_utf16().collect();
            let ev = self.keyboard_event(0, true)?;
            // SAFETY: `utf16.as_ptr()` is valid for `utf16.len()` u16 reads
            // for the duration of the call.
            unsafe {
                CGEventKeyboardSetUnicodeString(ev, utf16.len(), utf16.as_ptr());
            }
            self.post(ev)?;
            let ev = self.keyboard_event(0, false)?;
            unsafe {
                CGEventKeyboardSetUnicodeString(ev, utf16.len(), utf16.as_ptr());
            }
            self.post(ev)?;
        }
        Ok(())
    }

    fn pointer_drag(
        &self,
        from: Point,
        to: Point,
        button: MouseButton,
        duration: std::time::Duration,
    ) -> Result<()> {
        // Drag-and-drop source views on macOS filter for
        // kCGEventLeftMouseDragged / …RightMouseDragged / …OtherMouseDragged
        // specifically — ordinary kCGEventMouseMoved between the down and up
        // is ignored by DnD. Synthesise the sequence ourselves using the
        // right dragged-event type instead of relying on the core trait's
        // default (which emits mouse-moved).
        const STEP: std::time::Duration = std::time::Duration::from_millis(16);
        self.pointer_move(from)?;
        self.pointer_down(button)?;
        let step_ms = STEP.as_millis().max(1);
        let steps = (duration.as_millis() / step_ms).max(1) as i32;
        let dragged_ty = cg_button_dragged(button);
        let cg_btn = cg_button(button);
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let x = from.x + ((to.x - from.x) as f64 * t).round() as i32;
            let y = from.y + ((to.y - from.y) as f64 * t).round() as i32;
            let ev = self.mouse_event(dragged_ty, Point::new(x, y), cg_btn)?;
            self.post(ev)?;
            if i < steps {
                std::thread::sleep(STEP);
            }
        }
        self.pointer_up(button)
    }
}

fn modifier_flag_for(key: &Key) -> u64 {
    match key {
        Key::Shift => K_CG_EVENT_FLAG_MASK_SHIFT,
        Key::Ctrl => K_CG_EVENT_FLAG_MASK_CONTROL,
        Key::Alt => K_CG_EVENT_FLAG_MASK_ALTERNATE,
        Key::Meta => K_CG_EVENT_FLAG_MASK_COMMAND,
        _ => 0,
    }
}
