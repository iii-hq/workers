//! Native host backend: drive the machine this worker runs on directly, with
//! no external computer-server. Screen capture via `xcap`, mouse/keyboard via
//! `enigo`.
//!
//! Captures are downscaled to `max_dimension` (longest edge) and JPEG-encoded:
//! a full Retina frame is tens of megabytes of PNG, which floods the model's
//! context and the frame stream. The model therefore sees the DOWNSCALED image
//! and its coordinate space; pointer actions scale those coordinates back up to
//! real screen pixels before injecting.
//!
//! Capture and input are synchronous, blocking OS calls, so each runs on a
//! blocking thread. `enigo` state is not `Send`, so an `Enigo` is created, used,
//! and dropped inside each blocking closure. On macOS the worker process needs
//! Screen Recording (capture) and Accessibility (input) permission, or capture
//! is black and input is ignored.

use std::io::Cursor;
use std::sync::OnceLock;

use async_trait::async_trait;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{resize, FilterType};
use image::DynamicImage;
use tokio::task::spawn_blocking;
use xcap::Monitor;

use super::{Backend, Screen, Shot};

pub struct NativeHost {
    max_dimension: u32,
    jpeg_quality: u8,
    /// Real display pixel size, learned on the first capture. Pointer actions
    /// scale downscaled-space coordinates back up against this.
    real: OnceLock<(u32, u32)>,
}

impl NativeHost {
    pub fn new(max_dimension: u32, jpeg_quality: u8) -> Self {
        Self {
            max_dimension: max_dimension.max(320),
            jpeg_quality: jpeg_quality.clamp(1, 100),
            real: OnceLock::new(),
        }
    }

    fn note_real(&self, dims: (u32, u32)) {
        let _ = self.real.set(dims);
    }
}

fn primary_monitor() -> Result<Monitor, String> {
    let monitors = Monitor::all().map_err(|e| format!("enumerate monitors: {e}"))?;
    monitors
        .into_iter()
        .next()
        .ok_or_else(|| "no monitor found".to_string())
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

/// Capture the primary display, downscale to `max_dim`, JPEG-encode. Returns
/// (jpeg bytes, target w, target h, real w, real h).
fn capture(max_dim: u32, quality: u8) -> Result<(Vec<u8>, u32, u32, u32, u32), String> {
    let monitor = primary_monitor()?;
    let img = monitor
        .capture_image()
        .map_err(|e| format!("screen capture failed: {e}"))?;
    let (rw, rh) = (img.width(), img.height());
    let (tw, th) = target_dims(rw, rh, max_dim);
    let rgb = if (tw, th) == (rw, rh) {
        DynamicImage::ImageRgba8(img).to_rgb8()
    } else {
        DynamicImage::ImageRgba8(resize(&img, tw, th, FilterType::Triangle)).to_rgb8()
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut Cursor::new(&mut out), quality)
        .encode_image(&rgb)
        .map_err(|e| format!("jpeg encode failed: {e}"))?;
    Ok((out, tw, th, rw, rh))
}

/// Run a closure with a fresh `Enigo` plus the x/y scale from downscaled space
/// to real pixels (1.0 when the real size is not yet known).
async fn with_enigo<F>(real: Option<(u32, u32)>, max_dim: u32, f: F) -> Result<(), String>
where
    F: FnOnce(&mut Enigo, f64, f64) -> Result<(), String> + Send + 'static,
{
    spawn_blocking(move || {
        let (sx, sy) = match real {
            Some((rw, rh)) => {
                let (tw, th) = target_dims(rw, rh, max_dim);
                (rw as f64 / tw as f64, rh as f64 / th as f64)
            }
            None => (1.0, 1.0),
        };
        let mut enigo =
            Enigo::new(&Settings::default()).map_err(|e| format!("enigo init failed: {e}"))?;
        f(&mut enigo, sx, sy)
    })
    .await
    .map_err(|e| format!("input task failed: {e}"))?
}

fn input_err(e: enigo::InputError) -> String {
    format!("input failed: {e}")
}

fn scaled(x: i64, y: i64, sx: f64, sy: f64) -> (i32, i32) {
    (
        (x as f64 * sx).round() as i32,
        (y as f64 * sy).round() as i32,
    )
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
        let (max_dim, q) = (self.max_dimension, self.jpeg_quality);
        let (_, tw, th, rw, rh) = spawn_blocking(move || capture(max_dim, q))
            .await
            .map_err(|e| format!("capture task failed: {e}"))??;
        self.note_real((rw, rh));
        Ok(Screen {
            width: tw,
            height: th,
        })
    }

    async fn screenshot(&self) -> Result<Shot, String> {
        let (max_dim, q) = (self.max_dimension, self.jpeg_quality);
        let (bytes, _, _, rw, rh) = spawn_blocking(move || capture(max_dim, q))
            .await
            .map_err(|e| format!("capture task failed: {e}"))??;
        if bytes.is_empty() {
            return Err("screenshot: capture produced an empty image".to_string());
        }
        self.note_real((rw, rh));
        Ok(Shot::new(bytes))
    }

    async fn left_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(
            self.real.get().copied(),
            self.max_dimension,
            move |e, sx, sy| {
                let (rx, ry) = scaled(x, y, sx, sy);
                e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)?;
                e.button(Button::Left, Direction::Click).map_err(input_err)
            },
        )
        .await
    }

    async fn right_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(
            self.real.get().copied(),
            self.max_dimension,
            move |e, sx, sy| {
                let (rx, ry) = scaled(x, y, sx, sy);
                e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)?;
                e.button(Button::Right, Direction::Click).map_err(input_err)
            },
        )
        .await
    }

    async fn double_click(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(
            self.real.get().copied(),
            self.max_dimension,
            move |e, sx, sy| {
                let (rx, ry) = scaled(x, y, sx, sy);
                e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)?;
                e.button(Button::Left, Direction::Click)
                    .map_err(input_err)?;
                e.button(Button::Left, Direction::Click).map_err(input_err)
            },
        )
        .await
    }

    async fn move_cursor(&self, x: i64, y: i64) -> Result<(), String> {
        with_enigo(
            self.real.get().copied(),
            self.max_dimension,
            move |e, sx, sy| {
                let (rx, ry) = scaled(x, y, sx, sy);
                e.move_mouse(rx, ry, Coordinate::Abs).map_err(input_err)
            },
        )
        .await
    }

    async fn scroll(&self, x: i64, y: i64, scroll_x: i64, scroll_y: i64) -> Result<(), String> {
        with_enigo(
            self.real.get().copied(),
            self.max_dimension,
            move |e, sx, sy| {
                let (rx, ry) = scaled(x, y, sx, sy);
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
            },
        )
        .await
    }

    async fn drag(&self, from: (i64, i64), to: (i64, i64), _button: &str) -> Result<(), String> {
        with_enigo(
            self.real.get().copied(),
            self.max_dimension,
            move |e, sx, sy| {
                let (fx, fy) = scaled(from.0, from.1, sx, sy);
                let (tx, ty) = scaled(to.0, to.1, sx, sy);
                e.move_mouse(fx, fy, Coordinate::Abs).map_err(input_err)?;
                e.button(Button::Left, Direction::Press)
                    .map_err(input_err)?;
                e.move_mouse(tx, ty, Coordinate::Abs).map_err(input_err)?;
                e.button(Button::Left, Direction::Release)
                    .map_err(input_err)
            },
        )
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        let text = text.to_string();
        with_enigo(None, self.max_dimension, move |e, _, _| {
            e.text(&text).map_err(input_err)
        })
        .await
    }

    async fn keypress(&self, keys: &[String]) -> Result<(), String> {
        let keys: Vec<String> = keys.to_vec();
        with_enigo(None, self.max_dimension, move |e, _, _| {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_dims_caps_longest_edge() {
        assert_eq!(target_dims(3024, 1964, 1280), (1280, 831));
        assert_eq!(target_dims(800, 600, 1280), (800, 600));
        assert_eq!(target_dims(0, 0, 1280), (0, 0));
    }

    #[test]
    fn scaled_maps_downscaled_to_real() {
        // A 1280-wide view of a 2560-wide screen: 2x scale.
        assert_eq!(scaled(640, 400, 2.0, 2.0), (1280, 800));
        assert_eq!(scaled(100, 100, 1.0, 1.0), (100, 100));
    }
}
