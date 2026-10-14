//! The Linux half of the native host: capture, monitors and input on an X11
//! server (Xorg or Xvfb) over pure-Rust x11rb — GetImage, RandR and XTest. No
//! system C library, so every Linux release triple still builds.
//!
//! Coordinates are physical root-window pixels end to end: RandR monitors,
//! GetImage and XTest all speak them, so the shared `Geom` maps image pixels
//! straight onto the root. Nothing here scales by Xft.dpi, and nothing clicks
//! by AT-SPI bounds (GTK4 reports (0,0) for every element).
//!
//! Text goes in as keysyms. A character the keyboard map lacks (`ç` on Xvfb's
//! US map, `€`, CJK) is typed through a spare keycode bound to it for the
//! length of one call, xdotool-style, and the map is restored afterwards.
//! Every press has its release guaranteed, error or not.

use std::sync::Mutex;
use std::time::Duration;

use image::RgbaImage;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, ImageOrder, Window};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::{Btn, Grab};
use crate::driver::sandbox::MAX_WHEEL_NOTCHES;
use crate::driver::DisplayInfo;

pub const WAYLAND: &str = "this Linux desktop runs Wayland; screenshots and coordinate input \
                           need an X11 session (Xorg or Xvfb)";
pub const NO_DISPLAY: &str =
    "no X11 or Wayland display is reachable; start one or pass an `endpoint`";

/// Wait after binding spare keycodes before typing, and again before
/// restoring them: clients re-read the map on MappingNotify, and one that
/// re-reads after the restore drops the character (xdotool's known race).
const REMAP_SETTLE: Duration = Duration::from_millis(150);

// XTest event types (core protocol event codes).
const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;
const BUTTON_PRESS: u8 = 4;
const BUTTON_RELEASE: u8 = 5;
const MOTION: u8 = 6;

/// What a native Linux session can drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desktop {
    /// A real X server (Xorg, Xvfb): capture and input work.
    X11,
    /// A Wayland compositor, reached directly or through XWayland: only the
    /// accessibility layer works.
    Wayland,
}

/// Decide what this process can drive (spec §5.1). WAYLAND_DISPLAY wins over
/// DISPLAY (the latter is XWayland on a Wayland login); an X server that
/// advertises XWAYLAND is a Wayland desktop too.
pub fn detect() -> Result<Desktop, String> {
    let set = |v: &str| std::env::var_os(v).is_some_and(|s| !s.is_empty());
    classify(set("WAYLAND_DISPLAY"), set("DISPLAY"), || {
        let (c, _) = connect()?;
        let ext = c
            .query_extension(b"XWAYLAND")
            .map_err(xerr)?
            .reply()
            .map_err(xerr)?;
        Ok(ext.present)
    })
}

/// [`detect`] without the environment: `xwayland` connects and asks, and runs
/// only when DISPLAY is the deciding endpoint.
fn classify(
    wayland: bool,
    display: bool,
    xwayland: impl FnOnce() -> Result<bool, String>,
) -> Result<Desktop, String> {
    if wayland {
        return Ok(Desktop::Wayland);
    }
    if !display {
        return Err(NO_DISPLAY.to_string());
    }
    Ok(if xwayland()? {
        Desktop::Wayland
    } else {
        Desktop::X11
    })
}

fn connect() -> Result<(RustConnection, usize), String> {
    RustConnection::connect(None).map_err(|e| {
        format!(
            "cannot open the X display {:?} ({e}); check DISPLAY and XAUTHORITY",
            std::env::var("DISPLAY").unwrap_or_default()
        )
    })
}

fn xerr(e: impl std::fmt::Display) -> String {
    format!("X11: {e}")
}

/// One X connection per driver, opened on first use.
#[derive(Default)]
pub struct Conn(Mutex<Option<(RustConnection, Window)>>);

impl Conn {
    fn with<T>(
        &self,
        f: impl FnOnce(&RustConnection, Window) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            let (c, screen) = connect()?;
            let root = c.setup().roots[screen].root;
            *slot = Some((c, root));
        }
        let (c, root) = slot.as_ref().expect("connected above");
        let out = f(c, *root);
        if out.is_err() {
            // ponytail: reconnects after any failed call, not only a dead
            // connection; a reconnect is one handshake.
            *slot = None;
        }
        out
    }
}

/// One RandR monitor (or the whole root), in physical root pixels.
#[derive(Debug, Clone, PartialEq)]
struct Mon {
    /// The RandR name atom, stable while the monitor exists; `None` for the
    /// root fallback.
    id: Option<u32>,
    name: String,
    primary: bool,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

/// RandR ≥ 1.5 monitors, else the root window's current size.
fn monitors(c: &RustConnection, root: Window) -> Result<Vec<Mon>, String> {
    let randr_15 = c
        .randr_query_version(1, 5)
        .ok()
        .and_then(|ck| ck.reply().ok())
        .is_some_and(|v| (v.major_version, v.minor_version) >= (1, 5));
    if randr_15 {
        let reply = c
            .randr_get_monitors(root, true)
            .map_err(xerr)?
            .reply()
            .map_err(xerr)?;
        let mons: Vec<Mon> = reply
            .monitors
            .iter()
            .map(|m| Mon {
                id: Some(m.name),
                name: c
                    .get_atom_name(m.name)
                    .ok()
                    .and_then(|ck| ck.reply().ok())
                    .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                    .unwrap_or_default(),
                primary: m.primary,
                x: m.x.into(),
                y: m.y.into(),
                w: m.width.into(),
                h: m.height.into(),
            })
            .collect();
        if !mons.is_empty() {
            return Ok(mons);
        }
    }
    let g = c.get_geometry(root).map_err(xerr)?.reply().map_err(xerr)?;
    Ok(vec![Mon {
        id: None,
        name: "screen".to_string(),
        primary: true,
        x: 0,
        y: 0,
        w: g.width.into(),
        h: g.height.into(),
    }])
}

/// The monitor to capture, by the macOS/Windows rule: the pinned one while it
/// exists, else the `monitor` index, else the one under the cursor, else the
/// primary, else the first.
fn pick(
    mons: Vec<Mon>,
    pinned: Option<u32>,
    over: Option<u32>,
    cursor: Option<(i32, i32)>,
) -> Result<Mon, String> {
    if mons.is_empty() {
        return Err("no display found".to_string());
    }
    if let Some(m) = pinned.and_then(|id| mons.iter().find(|m| m.id == Some(id))) {
        return Ok(m.clone());
    }
    if let Some(i) = over {
        return mons
            .into_iter()
            .nth(i as usize)
            .ok_or_else(|| format!("display index {i} is out of range"));
    }
    let under = cursor.and_then(|(cx, cy)| {
        mons.iter()
            .position(|m| cx >= m.x && cy >= m.y && cx < m.x + m.w as i32 && cy < m.y + m.h as i32)
    });
    let i = under
        .or_else(|| mons.iter().position(|m| m.primary))
        .unwrap_or(0);
    Ok(mons.into_iter().nth(i).expect("index from this list"))
}

pub fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    let (c, screen) = connect()?;
    let root = c.setup().roots[screen].root;
    Ok(monitors(&c, root)?
        .into_iter()
        .enumerate()
        .map(|(i, m)| DisplayInfo {
            index: i as u32,
            name: m.name,
            primary: m.primary,
            builtin: false,
            width: m.w,
            height: m.h,
        })
        .collect())
}

/// 4-byte little-endian BGRX pixels → RGBA (alpha forced opaque); `None`
/// when the buffer is not exactly 4 bytes per pixel.
fn bgrx_to_rgba(w: u32, h: u32, mut data: Vec<u8>) -> Option<RgbaImage> {
    if data.len() != w as usize * h as usize * 4 {
        return None;
    }
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    RgbaImage::from_raw(w, h, data)
}

pub fn grab(conn: &Conn, pinned: Option<u32>, over: Option<u32>) -> Result<Grab, String> {
    conn.with(|c, root| {
        let cursor = c
            .query_pointer(root)
            .ok()
            .and_then(|ck| ck.reply().ok())
            .map(|p| (i32::from(p.root_x), i32::from(p.root_y)));
        let m = pick(monitors(c, root)?, pinned, over, cursor)?;
        if c.setup().image_byte_order != ImageOrder::LSB_FIRST {
            return Err("X11: big-endian image byte order is not supported".to_string());
        }
        let reply = c
            .get_image(
                ImageFormat::Z_PIXMAP,
                root,
                m.x as i16,
                m.y as i16,
                m.w as u16,
                m.h as u16,
                !0,
            )
            .map_err(xerr)?
            .reply()
            .map_err(xerr)?;
        let depth = reply.depth;
        let img = matches!(depth, 24 | 32)
            .then(|| bgrx_to_rgba(m.w, m.h, reply.data))
            .flatten()
            .ok_or_else(|| {
                format!(
                    "X11: unsupported screen depth {depth} (need 24 or 32 bits in 4 bytes per \
                     pixel; for Xvfb use -screen 0 WxHx24)"
                )
            })?;
        Ok(Grab {
            img,
            id: m.id,
            x: m.x,
            y: m.y,
            w: m.w,
            h: m.h,
        })
    })
}

fn clamp16(v: i32) -> i16 {
    v.clamp(i16::MIN.into(), i16::MAX.into()) as i16
}

/// One XTest event, flushed at once: a guard that releases on drop must not
/// leave its releases sitting in a buffer.
fn fake(
    c: &RustConnection,
    root: Window,
    kind: u8,
    detail: u8,
    (x, y): (i32, i32),
) -> Result<(), String> {
    c.xtest_fake_input(kind, detail, 0, root, clamp16(x), clamp16(y), 0)
        .map_err(xerr)?;
    c.flush().map_err(xerr)
}

/// Wait until the server has processed everything sent so far, so errors
/// surface here and the next call sees the effect.
fn sync(c: &RustConnection) -> Result<(), String> {
    c.get_input_focus().map_err(xerr)?.reply().map_err(xerr)?;
    Ok(())
}

/// Presses not yet released. Dropping it releases them, latest first, so an
/// error between a press and its release never leaves a key or button held
/// on the user's desktop.
struct Held<S: FnMut(u8, u8) -> Result<(), String>> {
    send: S,
    down: Vec<(u8, u8)>,
}

impl<S: FnMut(u8, u8) -> Result<(), String>> Held<S> {
    fn new(send: S) -> Self {
        Self {
            send,
            down: Vec::new(),
        }
    }

    /// Press a key (`KEY_PRESS`) or button (`BUTTON_PRESS`). Recorded before
    /// it is sent: a send that fails may still have reached the server.
    fn press(&mut self, kind: u8, detail: u8) -> Result<(), String> {
        let release = if kind == KEY_PRESS {
            KEY_RELEASE
        } else {
            BUTTON_RELEASE
        };
        self.down.push((release, detail));
        (self.send)(kind, detail)
    }

    /// Release the latest press.
    fn release(&mut self) -> Result<(), String> {
        let (kind, detail) = self.down.pop().expect("a press to release");
        (self.send)(kind, detail)
    }
}

impl<S: FnMut(u8, u8) -> Result<(), String>> Drop for Held<S> {
    fn drop(&mut self) {
        while let Some((kind, detail)) = self.down.pop() {
            let _ = (self.send)(kind, detail);
        }
    }
}

fn presses(
    c: &RustConnection,
    root: Window,
) -> Held<impl FnMut(u8, u8) -> Result<(), String> + '_> {
    Held::new(move |kind, detail| fake(c, root, kind, detail, (0, 0)))
}

fn button(b: Btn) -> u8 {
    match b {
        Btn::Left => 1,
        Btn::Middle => 2,
        Btn::Right => 3,
    }
}

pub fn move_to(conn: &Conn, p: (i32, i32)) -> Result<(), String> {
    conn.with(|c, root| {
        fake(c, root, MOTION, 0, p)?;
        sync(c)
    })
}

pub fn click(conn: &Conn, p: (i32, i32), b: Btn, count: u8) -> Result<(), String> {
    conn.with(|c, root| {
        fake(c, root, MOTION, 0, p)?;
        let mut held = presses(c, root);
        for _ in 0..count {
            held.press(BUTTON_PRESS, button(b))?;
            held.release()?;
        }
        drop(held);
        sync(c)
    })
}

/// X wheel buttons: 4 up, 5 down, 6 left, 7 right; positive scrolls down /
/// right, as in the sandbox driver.
pub fn scroll(conn: &Conn, p: (i32, i32), sx: i64, sy: i64) -> Result<(), String> {
    conn.with(|c, root| {
        fake(c, root, MOTION, 0, p)?;
        let mut held = presses(c, root);
        for (amount, back, fwd) in [(sy, 4, 5), (sx, 6, 7)] {
            let wheel = if amount < 0 { back } else { fwd };
            for _ in 0..amount.unsigned_abs().min(MAX_WHEEL_NOTCHES as u64) {
                held.press(BUTTON_PRESS, wheel)?;
                held.release()?;
            }
        }
        drop(held);
        sync(c)
    })
}

/// Down, a few moves on the way, up: toolkits that need motion between press
/// and release to start a drag get it.
pub fn drag(conn: &Conn, from: (i32, i32), to: (i32, i32), b: Btn) -> Result<(), String> {
    const STEPS: i32 = 10;
    conn.with(|c, root| {
        fake(c, root, MOTION, 0, from)?;
        let mut held = presses(c, root);
        held.press(BUTTON_PRESS, button(b))?;
        for i in 1..=STEPS {
            let at = (
                from.0 + (to.0 - from.0) * i / STEPS,
                from.1 + (to.1 - from.1) * i / STEPS,
            );
            fake(c, root, MOTION, 0, at)?;
        }
        held.release()?;
        drop(held);
        sync(c)
    })
}

/// The server's core keyboard map: `per` keysyms per keycode, from `min`.
struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keymap {
    fn read(c: &RustConnection) -> Result<Self, String> {
        let (min, max) = (c.setup().min_keycode, c.setup().max_keycode);
        let r = c
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(xerr)?
            .reply()
            .map_err(xerr)?;
        Ok(Self {
            min,
            per: r.keysyms_per_keycode.into(),
            syms: r.keysyms,
        })
    }

    /// The keycode typing `ks` in group 1, and whether it needs Shift.
    fn find(&self, ks: u32) -> Option<(u8, bool)> {
        if ks == 0 {
            return None;
        }
        self.syms.chunks(self.per).enumerate().find_map(|(i, col)| {
            let kc = self.min + i as u8;
            if col.first() == Some(&ks) {
                Some((kc, false))
            } else if col.get(1) == Some(&ks) {
                Some((kc, true))
            } else {
                None
            }
        })
    }

    /// Keycodes with no keysym at all: free to bind.
    fn free(&self) -> Vec<u8> {
        self.syms
            .chunks(self.per)
            .enumerate()
            .filter(|(_, col)| col.iter().all(|&s| s == 0))
            .map(|(i, _)| self.min + i as u8)
            .collect()
    }
}

/// The keysym that types `c`: Return for a line break (`from_char` would give
/// Linefeed), Tab for a tab, else xkb's UTF-32 → keysym table.
fn char_keysym(c: char) -> u32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        c => xkeysym::Keysym::from_char(c).raw(),
    }
}

/// Keysyms for `press`/`hotkey` names, with the macOS/Windows driver's names
/// and the sandbox's aliases.
fn keysym_for(name: &str) -> Result<u32, String> {
    let lower = name.to_ascii_lowercase();
    Ok(match lower.as_str() {
        "enter" | "return" => 0xff0d,
        "tab" => 0xff09,
        "escape" | "esc" => 0xff1b,
        "backspace" => 0xff08,
        "delete" | "del" => 0xffff,
        "space" | "spacebar" => 0x20,
        "up" | "arrowup" => 0xff52,
        "down" | "arrowdown" => 0xff54,
        "left" | "arrowleft" => 0xff51,
        "right" | "arrowright" => 0xff53,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" | "pgup" => 0xff55,
        "pagedown" | "pgdn" => 0xff56,
        "cmd" | "command" | "meta" | "super" | "win" => 0xffeb,
        "ctrl" | "control" => 0xffe3,
        "alt" | "option" => 0xffe9,
        "shift" => 0xffe1,
        other => {
            if let Some(n) = other
                .strip_prefix('f')
                .and_then(|n| n.parse::<u32>().ok())
                .filter(|n| (1..=12).contains(n))
            {
                return Ok(0xffbe + n - 1);
            }
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => char_keysym(c),
                _ => {
                    return Err(format!(
                        "unknown key '{other}' (a single character, or one of enter, tab, escape, \
                         backspace, delete, space, up, down, left, right, home, end, pageup, \
                         pagedown, f1-f12, cmd, ctrl, alt, shift)"
                    ))
                }
            }
        }
    })
}

/// A run of text and the spare keycodes it binds (keycode, keysym).
#[derive(Debug, PartialEq)]
struct Segment {
    /// (keycode, needs Shift), in typing order.
    keys: Vec<(u8, bool)>,
    bind: Vec<(u8, u32)>,
}

/// Split `text` into segments whose missing characters fit the free keycodes.
/// `all_spare` (a non-zero XKB group, where group-1 columns would type the
/// wrong layout) sends every character through a spare keycode.
fn plan_text(text: &str, map: &Keymap, all_spare: bool) -> Result<Vec<Segment>, String> {
    let free = map.free();
    let mut segments = Vec::new();
    let mut cur = Segment {
        keys: Vec::new(),
        bind: Vec::new(),
    };
    for ch in text.replace("\r\n", "\n").chars() {
        let ks = char_keysym(ch);
        if ks == 0 {
            return Err(format!("cannot type U+{:04X}", ch as u32));
        }
        if !all_spare {
            if let Some(k) = map.find(ks) {
                cur.keys.push(k);
                continue;
            }
        }
        let kc = match cur.bind.iter().find(|(_, s)| *s == ks) {
            Some(&(kc, _)) => kc,
            None => {
                if free.is_empty() {
                    return Err(format!(
                        "no free keycode on this X keyboard map to type {ch:?}"
                    ));
                }
                if cur.bind.len() == free.len() {
                    segments.push(std::mem::replace(
                        &mut cur,
                        Segment {
                            keys: Vec::new(),
                            bind: Vec::new(),
                        },
                    ));
                }
                let kc = free[cur.bind.len()];
                cur.bind.push((kc, ks));
                kc
            }
        };
        cur.keys.push((kc, false));
    }
    if !cur.keys.is_empty() {
        segments.push(cur);
    }
    Ok(segments)
}

/// Spare keycodes bound for one segment; dropping it unbinds them (back to
/// NoSymbol), so the map is restored on every path.
struct Bound<'a> {
    c: &'a RustConnection,
    per: u8,
    kcs: Vec<u8>,
}

impl<'a> Bound<'a> {
    /// One ChangeKeyboardMapping per keycode (free keycodes need not be
    /// adjacent), each bound to `[ks, ks]`: a single keysym would be
    /// case-expanded by the server, and Shift would change the character.
    fn new(c: &'a RustConnection, per: usize, bind: &[(u8, u32)]) -> Result<Self, String> {
        let mut bound = Self {
            c,
            per: per as u8,
            kcs: Vec::new(),
        };
        for &(kc, ks) in bind {
            bound.kcs.push(kc);
            let mut syms = vec![0u32; per];
            syms[0] = ks;
            if per > 1 {
                syms[1] = ks;
            }
            c.change_keyboard_mapping(1, kc, bound.per, &syms)
                .map_err(xerr)?;
        }
        Ok(bound)
    }
}

impl Drop for Bound<'_> {
    fn drop(&mut self) {
        let none = vec![0u32; self.per.into()];
        for &kc in &self.kcs {
            let _ = self.c.change_keyboard_mapping(1, kc, self.per, &none);
        }
        let _ = self.c.flush();
    }
}

pub fn type_text(conn: &Conn, text: &str) -> Result<(), String> {
    conn.with(|c, root| {
        let map = Keymap::read(c)?;
        // The XKB group lives in bits 13-14 of the core state.
        let pointer = c.query_pointer(root).map_err(xerr)?.reply().map_err(xerr)?;
        let group = (u16::from(pointer.mask) >> 13) & 3;
        let shift = map.find(0xffe1).map(|(kc, _)| kc);
        for seg in plan_text(text, &map, group != 0)? {
            let _bound = Bound::new(c, map.per, &seg.bind)?;
            if !seg.bind.is_empty() {
                sync(c)?;
                std::thread::sleep(REMAP_SETTLE);
            }
            let mut held = presses(c, root);
            for &(kc, needs_shift) in &seg.keys {
                if needs_shift {
                    let shift = shift.ok_or("this X keyboard map has no Shift key")?;
                    held.press(KEY_PRESS, shift)?;
                }
                held.press(KEY_PRESS, kc)?;
                held.release()?;
                if needs_shift {
                    held.release()?;
                }
            }
            drop(held);
            if !seg.bind.is_empty() {
                sync(c)?;
                std::thread::sleep(REMAP_SETTLE);
            }
        }
        sync(c)
    })
}

/// A single key is a tap; a chord holds all but the last, taps the last,
/// then releases the rest. Every name resolves before anything is pressed.
pub fn keys(conn: &Conn, names: &[String]) -> Result<(), String> {
    let syms = names
        .iter()
        .map(|n| keysym_for(n))
        .collect::<Result<Vec<u32>, String>>()?;
    conn.with(|c, root| {
        let map = Keymap::read(c)?;
        let shift = map.find(0xffe1).map(|(kc, _)| kc);
        let mut codes = Vec::new();
        for (ks, name) in syms.iter().zip(names) {
            // ponytail: a key missing from the map is refused, not bound like
            // type_text does; bind it too if a real hotkey needs that.
            let (kc, needs_shift) = map
                .find(*ks)
                .ok_or_else(|| format!("key '{name}' is not on this X keyboard map"))?;
            if needs_shift {
                codes.push(shift.ok_or("this X keyboard map has no Shift key")?);
            }
            codes.push(kc);
        }
        let mut held = presses(c, root);
        for kc in codes {
            held.press(KEY_PRESS, kc)?;
        }
        drop(held);
        sync(c)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn mon(id: u32, x: i32, w: u32, primary: bool) -> Mon {
        Mon {
            id: Some(id),
            name: format!("M{id}"),
            primary,
            x,
            y: 0,
            w,
            h: 800,
        }
    }

    #[test]
    fn classify_never_drives_a_wayland_desktop_through_xwayland() {
        let never = || -> Result<bool, String> { panic!("must not connect") };
        assert_eq!(classify(true, true, never), Ok(Desktop::Wayland));
        assert_eq!(classify(true, false, never), Ok(Desktop::Wayland));
        assert_eq!(classify(false, false, never), Err(NO_DISPLAY.to_string()));
        // DISPLAY alone: a Wayland login that exported only DISPLAY is still
        // Wayland once the server says XWAYLAND.
        assert_eq!(classify(false, true, || Ok(true)), Ok(Desktop::Wayland));
        assert_eq!(classify(false, true, || Ok(false)), Ok(Desktop::X11));
        assert_eq!(
            classify(false, true, || Err("no auth".to_string())),
            Err("no auth".to_string())
        );
    }

    #[test]
    fn pick_follows_pinned_override_cursor_primary_first() {
        let mons = || vec![mon(10, 0, 640, false), mon(11, 640, 640, false)];
        // Xvfb marks none primary: the first.
        assert_eq!(pick(mons(), None, None, None).unwrap().id, Some(10));
        let mut with_primary = mons();
        with_primary[1].primary = true;
        assert_eq!(pick(with_primary, None, None, None).unwrap().id, Some(11));
        assert_eq!(
            pick(mons(), None, None, Some((700, 5))).unwrap().id,
            Some(11)
        );
        assert_eq!(
            pick(mons(), None, Some(1), Some((5, 5))).unwrap().id,
            Some(11)
        );
        assert_eq!(
            pick(mons(), None, Some(2), None).unwrap_err(),
            "display index 2 is out of range"
        );
        assert_eq!(pick(mons(), Some(11), Some(0), None).unwrap().id, Some(11));
        // The pinned monitor was unplugged: a fresh pick.
        assert_eq!(pick(mons(), Some(99), None, None).unwrap().id, Some(10));
        assert_eq!(
            pick(vec![], None, None, None).unwrap_err(),
            "no display found"
        );
    }

    #[test]
    fn bgrx_becomes_opaque_rgba_and_other_depths_are_refused() {
        let img = bgrx_to_rgba(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 0]).unwrap();
        assert_eq!(img.as_raw(), &vec![3, 2, 1, 255, 6, 5, 4, 255]);
        assert!(bgrx_to_rgba(2, 1, vec![0; 4]).is_none(), "16-bit data");
    }

    /// keycode 36 Return; 38 a/A; 50 Shift_L; 200, 201 free.
    fn us_map() -> Keymap {
        let per = 2;
        let mut syms = vec![0u32; (210 - 8) * per];
        let mut set = |kc: usize, a: u32, b: u32| {
            syms[(kc - 8) * per] = a;
            syms[(kc - 8) * per + 1] = b;
        };
        set(36, 0xff0d, 0);
        set(38, 0x61, 0x41);
        set(50, 0xffe1, 0);
        for kc in 8..210 {
            if ![36, 38, 50, 200, 201].contains(&kc) {
                set(kc, 0xfffe, 0); // taken by something we never type
            }
        }
        Keymap { min: 8, per, syms }
    }

    #[test]
    fn keymap_finds_levels_and_free_keycodes() {
        let m = us_map();
        assert_eq!(m.find(0x61), Some((38, false)));
        assert_eq!(m.find(0x41), Some((38, true)));
        assert_eq!(m.find(0xe7), None);
        assert_eq!(m.find(0), None);
        assert_eq!(m.free(), vec![200, 201]);
    }

    #[test]
    fn plan_text_types_mapped_keys_and_binds_the_rest_in_segments() {
        let m = us_map();
        let one = plan_text("aA\r\n", &m, false).unwrap();
        assert_eq!(
            one,
            vec![Segment {
                keys: vec![(38, false), (38, true), (36, false)],
                bind: vec![],
            }]
        );
        // ç twice binds once; € takes the second free keycode.
        let two = plan_text("çç€", &m, false).unwrap();
        assert_eq!(
            two,
            vec![Segment {
                keys: vec![(200, false), (200, false), (201, false)],
                bind: vec![(200, 0xe7), (201, 0x20ac)],
            }]
        );
        // Three distinct missing characters, two free keycodes: two segments.
        let three = plan_text("çé€a", &m, false).unwrap();
        assert_eq!(three.len(), 2);
        assert_eq!(three[0].bind, vec![(200, 0xe7), (201, 0xe9)]);
        assert_eq!(three[1].bind, vec![(200, 0x20ac)]);
        assert_eq!(three[1].keys, vec![(200, false), (38, false)]);
        // A non-zero XKB group: everything through a spare keycode.
        assert_eq!(plan_text("a", &m, true).unwrap()[0].bind, vec![(200, 0x61)]);
        let full = Keymap {
            min: 8,
            per: 1,
            syms: vec![0x61],
        };
        assert!(plan_text("ç", &full, false)
            .unwrap_err()
            .contains("no free keycode"));
    }

    #[test]
    fn key_names_match_the_other_drivers() {
        assert_eq!(keysym_for("enter"), Ok(0xff0d));
        assert_eq!(keysym_for("ArrowDown"), Ok(0xff54));
        assert_eq!(keysym_for("pgdn"), Ok(0xff56));
        assert_eq!(keysym_for("F5"), Ok(0xffc2));
        assert_eq!(keysym_for("super"), Ok(0xffeb));
        assert_eq!(keysym_for("a"), Ok(0x61));
        assert_eq!(keysym_for("f"), Ok(0x66));
        // Silently pressing 'f' would look like the key worked.
        assert!(keysym_for("f13").is_err());
        assert!(keysym_for("").is_err());
        assert_eq!(char_keysym('\n'), 0xff0d);
        assert_eq!(char_keysym('\t'), 0xff09);
        assert_eq!(char_keysym('é'), 0xe9);
        assert_eq!(char_keysym('€'), 0x20ac);
        assert_eq!(char_keysym('😀'), 0x0101_f600);
    }

    #[test]
    fn held_input_is_released_even_when_a_send_fails() {
        let log = RefCell::new(Vec::new());
        let send = |kind: u8, detail: u8| {
            log.borrow_mut().push((kind, detail));
            if log.borrow().len() == 3 {
                Err("connection lost".to_string())
            } else {
                Ok(())
            }
        };
        let r = (|| {
            let mut held = Held::new(send);
            held.press(KEY_PRESS, 50)?;
            held.press(BUTTON_PRESS, 1)?;
            held.press(KEY_PRESS, 38)?; // this send fails
            held.release()
        })();
        assert!(r.is_err());
        assert_eq!(
            log.into_inner(),
            vec![
                (KEY_PRESS, 50),
                (BUTTON_PRESS, 1),
                (KEY_PRESS, 38),
                // Drop released everything, latest first.
                (KEY_RELEASE, 38),
                (BUTTON_RELEASE, 1),
                (KEY_RELEASE, 50),
            ]
        );
    }

    /// Needs an X server: run through tests/run-linux-live.sh.
    #[test]
    #[ignore]
    fn live_x11_capture_pointer_and_remap_restore() {
        assert_eq!(detect().unwrap(), Desktop::X11);
        let displays = list_displays().unwrap();
        assert!(!displays.is_empty());
        let conn = Conn::default();
        let g = grab(&conn, None, Some(0)).unwrap();
        assert_eq!(
            (g.img.width(), g.img.height()),
            (displays[0].width, displays[0].height)
        );
        move_to(&conn, (123, 77)).unwrap();
        let at = conn
            .with(|c, root| c.query_pointer(root).map_err(xerr)?.reply().map_err(xerr))
            .unwrap();
        assert_eq!((at.root_x, at.root_y), (123, 77));
        let before = conn.with(|c, _| Keymap::read(c)).unwrap().syms;
        // 20 distinct characters missing from Xvfb's US map: more than its
        // 15 free keycodes, so two segments.
        type_text(&conn, "çãé€ł日本語中文한국어ñöüßøåæ").unwrap();
        keys(&conn, &["ctrl".to_string(), "a".to_string()]).unwrap();
        let after = conn.with(|c, _| Keymap::read(c)).unwrap().syms;
        assert_eq!(before, after, "keyboard map restored");
    }
}
