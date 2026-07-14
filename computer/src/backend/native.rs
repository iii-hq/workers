//! Native host backend: drive the machine this worker runs on directly, with
//! no external computer-server. Screen capture via `xcap`, mouse/keyboard via
//! `enigo`. Chosen when a session starts with no endpoint.
//!
//! Capture and input are synchronous, blocking OS calls, so each runs on a
//! blocking thread (`spawn_blocking`). `enigo` platform state is not `Send`, so
//! an `Enigo` is created, used, and dropped inside each blocking closure rather
//! than held on the struct. On macOS the worker process (or the engine that
//! spawns it) needs Screen Recording (capture) and Accessibility (input)
//! permission, or capture returns black and input no-ops.

use std::io::Cursor;

use async_trait::async_trait;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use tokio::task::spawn_blocking;
use xcap::image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use xcap::Monitor;

use super::{Backend, Screen, Shot};

#[derive(Default)]
pub struct NativeHost;

impl NativeHost {
    pub fn new() -> Self {
        Self
    }
}

fn primary_monitor() -> Result<Monitor, String> {
    let monitors = Monitor::all().map_err(|e| format!("enumerate monitors: {e}"))?;
    monitors
        .into_iter()
        .next()
        .ok_or_else(|| "no monitor found".to_string())
}

/// Capture the primary display as PNG bytes plus its pixel dimensions.
fn capture() -> Result<(Vec<u8>, u32, u32), String> {
    let monitor = primary_monitor()?;
    let img = monitor
        .capture_image()
        .map_err(|e| format!("screen capture failed: {e}"))?;
    let (w, h) = (img.width(), img.height());
    let mut out = Vec::new();
    PngEncoder::new(&mut Cursor::new(&mut out))
        .write_image(img.as_raw(), w, h, ExtendedColorType::Rgba8)
        .map_err(|e| format!("png encode failed: {e}"))?;
    Ok((out, w, h))
}

/// Run a closure with a fresh `Enigo` on a blocking thread.
async fn with_enigo<F>(f: F) -> Result<(), String>
where
    F: FnOnce(&mut Enigo) -> Result<(), String> + Send + 'static,
{
    spawn_blocking(move || {
        let mut enigo =
            Enigo::new(&Settings::default()).map_err(|e| format!("enigo init failed: {e}"))?;
        f(&mut enigo)
    })
    .await
    .map_err(|e| format!("input task failed: {e}"))?
}

fn input_err(e: enigo::InputError) -> String {
    format!("input failed: {e}")
}

/// Map a key name (from `press`/`hotkey`) onto an enigo key.
fn key_for(name: &str) -> Key {
    match name.to_ascii_lowercase().as_str() {
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
        other => other.chars().next().map(Key::Unicode).unwrap_or(Key::Space),
    }
}

#[async_trait]
impl Backend for NativeHost {
    async fn screen_size(&self) -> Result<Screen, String> {
        let (_, w, h) = spawn_blocking(capture)
            .await
            .map_err(|e| format!("capture task failed: {e}"))??;
        Ok(Screen {
            width: w,
            height: h,
        })
    }

    async fn screenshot(&self) -> Result<Shot, String> {
        let (bytes, _, _) = spawn_blocking(capture)
            .await
            .map_err(|e| format!("capture task failed: {e}"))??;
        if bytes.is_empty() {
            return Err("screenshot: capture produced an empty image".to_string());
        }
        Ok(Shot::new(bytes))
    }

    async fn left_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(move |e| {
            e.move_mouse(x as i32, y as i32, Coordinate::Abs)
                .map_err(input_err)?;
            e.button(Button::Left, Direction::Click).map_err(input_err)
        })
        .await
    }

    async fn right_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(move |e| {
            e.move_mouse(x as i32, y as i32, Coordinate::Abs)
                .map_err(input_err)?;
            e.button(Button::Right, Direction::Click).map_err(input_err)
        })
        .await
    }

    async fn double_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(move |e| {
            e.move_mouse(x as i32, y as i32, Coordinate::Abs)
                .map_err(input_err)?;
            e.button(Button::Left, Direction::Click)
                .map_err(input_err)?;
            e.button(Button::Left, Direction::Click).map_err(input_err)
        })
        .await
    }

    async fn move_cursor(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(move |e| {
            e.move_mouse(x as i32, y as i32, Coordinate::Abs)
                .map_err(input_err)
        })
        .await
    }

    async fn scroll(&self, x: i64, y: i64, scroll_x: i64, scroll_y: i64) -> Result<(), String> {
        with_enigo(move |e| {
            e.move_mouse(x as i32, y as i32, Coordinate::Abs)
                .map_err(input_err)?;
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

    async fn drag(&self, from: (i64, i64), to: (i64, i64), _button: &str) -> Result<(), String> {
        with_enigo(move |e| {
            e.move_mouse(from.0 as i32, from.1 as i32, Coordinate::Abs)
                .map_err(input_err)?;
            e.button(Button::Left, Direction::Press)
                .map_err(input_err)?;
            e.move_mouse(to.0 as i32, to.1 as i32, Coordinate::Abs)
                .map_err(input_err)?;
            e.button(Button::Left, Direction::Release)
                .map_err(input_err)
        })
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        let text = text.to_string();
        with_enigo(move |e| e.text(&text).map_err(input_err)).await
    }

    async fn keypress(&self, keys: &[String]) -> Result<(), String> {
        let keys: Vec<String> = keys.to_vec();
        with_enigo(move |e| {
            // A single key is a tap; a chord holds all but the last as
            // modifiers, taps the last, then releases the modifiers.
            let Some((last, mods)) = keys.split_last() else {
                return Ok(());
            };
            for m in mods {
                e.key(key_for(m), Direction::Press).map_err(input_err)?;
            }
            let tapped = e.key(key_for(last), Direction::Click).map_err(input_err);
            for m in mods.iter().rev() {
                let _ = e.key(key_for(m), Direction::Release);
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
