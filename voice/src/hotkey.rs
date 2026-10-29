//! The fn (globe) key as the dictation key, the way Wispr Flow uses it:
//! press and hold to talk, let go to have the text typed where you are.
//!
//! Browsers never see a bare fn: Chromium and WebKit drop the
//! `flagsChanged` event for keyCode 63 before the page gets it. So on macOS
//! this worker listens for it with a listen-only Quartz event tap and
//! publishes `voice::hotkey` (`down`, `up`, or `chord` when another key is
//! pressed while fn is held, as in fn+arrow). No key codes leave the
//! worker: only fn's own state.
//!
//! A listen-only tap needs the Input Monitoring permission, granted to the
//! app that launched the worker (the terminal running iii). Without it the
//! tap cannot be created; [`status`] says so and the console keeps the
//! right Option key as the dictation key.

use std::sync::{Mutex, OnceLock};

use schemars::JsonSchema;
use serde::Serialize;

/// What the console needs to know about the fn key.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct HotkeyStatus {
    /// fn presses are being published as `voice::hotkey`.
    pub available: bool,
    /// Why not, when not: another OS, or the permission is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// One change of the fn key.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    Down,
    Up,
    /// Another key went down while fn was held: a shortcut, not dictation.
    Chord,
}

/// `voice::hotkey` — the fn key changed.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HotkeyEvent {
    pub key: &'static str,
    pub action: HotkeyAction,
    pub timestamp_ms: i64,
}

fn status_cell() -> &'static Mutex<HotkeyStatus> {
    static CELL: OnceLock<Mutex<HotkeyStatus>> = OnceLock::new();
    CELL.get_or_init(|| {
        Mutex::new(HotkeyStatus {
            available: false,
            problem: Some("the fn key listener has not started".into()),
        })
    })
}

pub fn status() -> HotkeyStatus {
    status_cell().lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn set_status(next: HotkeyStatus) {
    *status_cell().lock().unwrap_or_else(|p| p.into_inner()) = next;
}

/// Turns raw key events into fn's down/up/chord, dropping repeats.
#[derive(Debug, Default)]
pub struct FnTracker {
    down: bool,
    chorded: bool,
}

impl FnTracker {
    /// A modifier change; `fn_down` is whether the fn flag is now set.
    pub fn flags_changed(&mut self, key_code: i64, fn_down: bool) -> Option<HotkeyAction> {
        if key_code != FN_KEY_CODE {
            // Another modifier with fn held is a chord too (fn+⌘…).
            return self.other_key();
        }
        match (self.down, fn_down) {
            (false, true) => {
                self.down = true;
                self.chorded = false;
                Some(HotkeyAction::Down)
            }
            (true, false) => {
                self.down = false;
                Some(HotkeyAction::Up)
            }
            _ => None,
        }
    }

    /// Any other key went down.
    pub fn other_key(&mut self) -> Option<HotkeyAction> {
        if self.down && !self.chorded {
            self.chorded = true;
            return Some(HotkeyAction::Chord);
        }
        None
    }
}

/// kVK_Function.
pub const FN_KEY_CODE: i64 = 63;

/// Start listening on a thread of its own; `emit` is called from that
/// thread for every change.
pub fn spawn(emit: impl Fn(HotkeyAction) + Send + 'static) {
    #[cfg(target_os = "macos")]
    mac::spawn(Box::new(emit));
    #[cfg(not(target_os = "macos"))]
    {
        let _ = emit;
        set_status(HotkeyStatus {
            available: false,
            problem: Some("the fn key is only available on macOS".into()),
        });
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease};
    use core_foundation_sys::mach_port::{CFMachPortCreateRunLoopSource, CFMachPortRef};
    use core_foundation_sys::runloop::{
        kCFRunLoopCommonModes, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRun,
    };

    use super::{set_status, FnTracker, HotkeyAction, HotkeyStatus};

    type CGEventRef = *mut c_void;
    type CGEventTapProxy = *mut c_void;
    type CGEventType = u32;
    type CGEventMask = u64;
    type CGEventTapCallBack = extern "C" fn(
        proxy: CGEventTapProxy,
        kind: CGEventType,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef;

    const KCG_HID_EVENT_TAP: u32 = 0;
    const KCG_HEAD_INSERT_EVENT_TAP: u32 = 0;
    const KCG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
    const KCG_EVENT_KEY_DOWN: CGEventType = 10;
    const KCG_EVENT_FLAGS_CHANGED: CGEventType = 12;
    const KCG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFF_FFFE;
    const KCG_EVENT_TAP_DISABLED_BY_USER_INPUT: CGEventType = 0xFFFF_FFFF;
    const KCG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
    const KCG_EVENT_FLAG_MASK_SECONDARY_FN: u64 = 0x0080_0000;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: CGEventMask,
            callback: CGEventTapCallBack,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        fn CGEventGetFlags(event: CGEventRef) -> u64;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    const PERMISSION_PROBLEM: &str = "macOS needs to allow it: turn on Input Monitoring for the app that runs iii (System Settings › Privacy & Security › Input Monitoring), then restart the voice worker";

    struct Listener {
        tracker: FnTracker,
        emit: Box<dyn Fn(HotkeyAction) + Send>,
        tap: CFMachPortRef,
    }

    extern "C" fn callback(
        _proxy: CGEventTapProxy,
        kind: CGEventType,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef {
        // SAFETY: `user_info` is the Listener leaked in `spawn`, used only on
        // this run-loop thread.
        let listener = unsafe { &mut *(user_info as *mut Listener) };
        let change = match kind {
            // macOS turns a slow tap off; turn it back on.
            KCG_EVENT_TAP_DISABLED_BY_TIMEOUT | KCG_EVENT_TAP_DISABLED_BY_USER_INPUT => {
                unsafe { CGEventTapEnable(listener.tap, true) };
                None
            }
            KCG_EVENT_FLAGS_CHANGED => {
                let code = unsafe { CGEventGetIntegerValueField(event, KCG_KEYBOARD_EVENT_KEYCODE) };
                let flags = unsafe { CGEventGetFlags(event) };
                listener
                    .tracker
                    .flags_changed(code, flags & KCG_EVENT_FLAG_MASK_SECONDARY_FN != 0)
            }
            KCG_EVENT_KEY_DOWN => listener.tracker.other_key(),
            _ => None,
        };
        if let Some(action) = change {
            (listener.emit)(action);
        }
        // Listen-only: the event goes on unchanged.
        event
    }

    pub fn spawn(emit: Box<dyn Fn(HotkeyAction) + Send>) {
        let spawned = std::thread::Builder::new()
            .name("voice-fn-key".into())
            .spawn(move || {
                // A tap without Input Monitoring is created but never hears a
                // key; ask for it (macOS shows its prompt once) and say so.
                // SAFETY: argument-less Quartz queries.
                if !unsafe { CGPreflightListenEventAccess() } {
                    unsafe { CGRequestListenEventAccess() };
                    tracing::warn!("fn key listener needs Input Monitoring for the app running iii");
                    set_status(HotkeyStatus {
                        available: false,
                        problem: Some(PERMISSION_PROBLEM.into()),
                    });
                    return;
                }
                let listener = Box::into_raw(Box::new(Listener {
                    tracker: FnTracker::default(),
                    emit,
                    tap: std::ptr::null_mut(),
                }));
                let mask: CGEventMask =
                    (1 << KCG_EVENT_FLAGS_CHANGED) | (1 << KCG_EVENT_KEY_DOWN);
                // SAFETY: plain Quartz calls; the listener outlives the tap
                // (both live until the process exits).
                unsafe {
                    let tap = CGEventTapCreate(
                        KCG_HID_EVENT_TAP,
                        KCG_HEAD_INSERT_EVENT_TAP,
                        KCG_EVENT_TAP_OPTION_LISTEN_ONLY,
                        mask,
                        callback,
                        listener as *mut c_void,
                    );
                    if tap.is_null() {
                        drop(Box::from_raw(listener));
                        tracing::warn!(
                            "fn key listener unavailable: grant Input Monitoring to the app running iii"
                        );
                        set_status(HotkeyStatus {
                            available: false,
                            problem: Some(PERMISSION_PROBLEM.into()),
                        });
                        return;
                    }
                    (*listener).tap = tap;
                    let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
                    CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
                    CFRelease(source as *const c_void);
                    CGEventTapEnable(tap, true);
                    set_status(HotkeyStatus {
                        available: true,
                        problem: None,
                    });
                    tracing::info!("fn key listener started");
                    CFRunLoopRun();
                }
            });
        if let Err(e) = spawned {
            set_status(HotkeyStatus {
                available: false,
                problem: Some(format!("the fn key listener could not start: {e}")),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fn_press_and_release_are_one_down_and_one_up() {
        let mut t = FnTracker::default();
        assert_eq!(t.flags_changed(FN_KEY_CODE, true), Some(HotkeyAction::Down));
        // A repeated flags event with fn still set is not a second press.
        assert_eq!(t.flags_changed(FN_KEY_CODE, true), None);
        assert_eq!(t.flags_changed(FN_KEY_CODE, false), Some(HotkeyAction::Up));
        assert_eq!(t.flags_changed(FN_KEY_CODE, false), None);
    }

    #[test]
    fn another_key_while_fn_is_held_is_one_chord() {
        let mut t = FnTracker::default();
        assert_eq!(t.other_key(), None);
        t.flags_changed(FN_KEY_CODE, true);
        assert_eq!(t.other_key(), Some(HotkeyAction::Chord));
        assert_eq!(t.other_key(), None);
        // Another modifier counts as well, on a fresh press.
        t.flags_changed(FN_KEY_CODE, false);
        t.flags_changed(FN_KEY_CODE, true);
        assert_eq!(t.flags_changed(56, true), Some(HotkeyAction::Chord));
    }
}
