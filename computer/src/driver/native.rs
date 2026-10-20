//! Native host driver: drive the machine this worker runs on directly, with
//! no executor process in between. The OS half is a submodule behind one set
//! of free functions — `desktop` (`xcap` capture; the pointer through
//! CoreGraphics on macOS, `enigo` for the keyboard and on Windows) on macOS
//! and Windows, `x11` (x11rb: GetImage, RandR, XTest) on Linux — and this file
//! holds what the three share.
//!
//! Multi-monitor and HiDPI aware. A session captures ONE display, chosen at
//! its first capture: by default the display under the cursor (so it follows
//! where you are working), pinned by id thereafter for coordinate stability.
//! The captured frame is downscaled to `max_dimension` and JPEG-encoded (a full
//! Retina frame is tens of megabytes of PNG, which floods the model context and
//! the live viewport). The model sees the downscaled image and its coordinate
//! space; pointer actions map those coordinates through the display's size and
//! global origin as the OS half reports them — logical points on macOS,
//! physical pixels on Windows (the process is per-monitor-v2 DPI aware, see
//! `pin_process_dpi`) and on Linux (X11 root pixels) — which is the space that
//! OS's input injection uses, so clicks land on the right display at the right
//! point regardless of scale factor. Never multiply by a scale factor.
//!
//! Capture and input are synchronous, blocking OS calls, so each runs on a
//! blocking thread.

use std::sync::{Arc, OnceLock};

use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use image::{DynamicImage, RgbaImage};
use tokio::task::spawn_blocking;

use super::{encode_jpeg, target_dims, Driver, Screen, Shot};

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod desktop;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use desktop as sys;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use desktop::preflight_screen_capture;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
use x11 as sys;
#[cfg(target_os = "linux")]
pub use x11::{detect, Desktop, NO_DISPLAY, WAYLAND};

pub use sys::list_displays;

/// Pin the process to per-monitor-v2 DPI awareness before any capture, input
/// or accessibility call. xa11y switches the whole process to PMv2 on its
/// first use; pinning it up front keeps one coordinate space for the life of
/// the process instead of a mid-session switch. No-op off Windows.
#[cfg(target_os = "windows")]
pub fn pin_process_dpi() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        #[link(name = "user32")]
        extern "system" {
            fn SetProcessDpiAwarenessContext(value: isize) -> i32;
            fn GetThreadDpiAwarenessContext() -> isize;
            fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> i32;
            fn GetAwarenessFromDpiAwarenessContext(value: isize) -> i32;
        }
        const PER_MONITOR_AWARE_V2: isize = -4;
        unsafe {
            let current = GetThreadDpiAwarenessContext();
            if SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2) == 0
                && AreDpiAwarenessContextsEqual(current, PER_MONITOR_AWARE_V2) == 0
            {
                // 0 unaware, 1 system aware, 2 per-monitor aware.
                tracing::warn!(
                    mode = GetAwarenessFromDpiAwarenessContext(current),
                    "could not set per-monitor-v2 DPI awareness; clicks may miss on scaled displays"
                );
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn pin_process_dpi() {}

/// One capture as the OS half hands it over: the display's full-resolution
/// pixels, its id to pin, and its size and global origin in input space.
struct Grab {
    img: RgbaImage,
    /// `None` when the OS did not report an id for the display.
    id: Option<u32>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Left,
    Right,
    Middle,
}

/// Everything needed to map a downscaled-image coordinate to an absolute
/// input coordinate: the captured display's global origin and the scale from
/// downscaled pixels to the display's size, both in the OS half's space.
#[derive(Clone, Copy)]
struct Geom {
    origin_x: i32,
    origin_y: i32,
    scale_x: f64,
    scale_y: f64,
}

pub struct NativeHost {
    /// The OS half's connection (a unit on macOS/Windows, the X connection
    /// on Linux).
    conn: Arc<sys::Conn>,
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
        pin_process_dpi();
        Self {
            conn: Arc::default(),
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

/// One capture: the encoded frame plus everything the session learns from
/// taking it — which display it came from and how to map its pixels back to
/// absolute input coordinates.
struct Capture {
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
    display_id: Option<u32>,
    geom: Geom,
}

/// Capture the resolved display, downscale + JPEG-encode.
fn capture(
    conn: &sys::Conn,
    pinned_id: Option<u32>,
    override_idx: Option<u32>,
    max_dim: u32,
    quality: u8,
) -> Result<Capture, String> {
    let g = sys::grab(conn, pinned_id, override_idx)?;
    let (tw, th) = target_dims(g.img.width(), g.img.height(), max_dim);
    // Downscaled pixels -> display space -> +origin lands the click on this
    // display at any scale factor.
    let geom = Geom {
        origin_x: g.x,
        origin_y: g.y,
        scale_x: g.w as f64 / tw.max(1) as f64,
        scale_y: g.h as f64 / th.max(1) as f64,
    };
    Ok(Capture {
        jpeg: encode_jpeg(DynamicImage::ImageRgba8(g.img), tw, th, quality)?,
        width: tw,
        height: th,
        display_id: g.id,
        geom,
    })
}

/// Map a downscaled-image coordinate to an absolute input coordinate through
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

/// The mouse button a drag holds down.
fn button_for(name: &str) -> Result<Btn, String> {
    match name.to_ascii_lowercase().as_str() {
        "left" => Ok(Btn::Left),
        "right" => Ok(Btn::Right),
        "middle" => Ok(Btn::Middle),
        other => Err(format!("unknown button '{other}' (left, right, middle)")),
    }
}

/// Run one blocking OS call off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    spawn_blocking(f)
        .await
        .map_err(|e| format!("native task failed: {e}"))?
}

impl NativeHost {
    async fn capture_shot(&self) -> Result<(Vec<u8>, u32, u32), String> {
        let (conn, pinned, over, max_dim, q) = (
            self.conn.clone(),
            self.pinned_id(),
            self.monitor,
            self.max_dimension,
            self.jpeg_quality,
        );
        let shot = blocking(move || capture(&conn, pinned, over, max_dim, q)).await?;
        if let Some(id) = shot.display_id {
            let _ = self.pinned.set(id);
        }
        self.geom.store(Some(Arc::new(shot.geom)));
        self.dims.store(Some(Arc::new(Screen {
            width: shot.width,
            height: shot.height,
        })));
        Ok((shot.jpeg, shot.width, shot.height))
    }

    /// The connection and an image coordinate mapped to input space.
    fn at(&self, x: i64, y: i64) -> (Arc<sys::Conn>, (i32, i32)) {
        (self.conn.clone(), to_global(x, y, self.geom()))
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
        let (c, p) = self.at(x, y);
        blocking(move || sys::click(&c, p, Btn::Left, 1)).await
    }

    async fn right_click(&self, x: i64, y: i64) -> Result<(), String> {
        let (c, p) = self.at(x, y);
        blocking(move || sys::click(&c, p, Btn::Right, 1)).await
    }

    async fn double_click(&self, x: i64, y: i64) -> Result<(), String> {
        let (c, p) = self.at(x, y);
        blocking(move || sys::click(&c, p, Btn::Left, 2)).await
    }

    async fn move_cursor(&self, x: i64, y: i64) -> Result<(), String> {
        let (c, p) = self.at(x, y);
        blocking(move || sys::move_to(&c, p)).await
    }

    async fn scroll(&self, x: i64, y: i64, scroll_x: i64, scroll_y: i64) -> Result<(), String> {
        let (c, p) = self.at(x, y);
        blocking(move || sys::scroll(&c, p, scroll_x, scroll_y)).await
    }

    async fn drag(&self, from: (i64, i64), to: (i64, i64), button: &str) -> Result<(), String> {
        let held = button_for(button)?;
        // One geometry snapshot for both ends: a capture between two loads
        // would map them through different displays or scales.
        let g = self.geom();
        let c = self.conn.clone();
        let (f, t) = (to_global(from.0, from.1, g), to_global(to.0, to.1, g));
        blocking(move || sys::drag(&c, f, t, held)).await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        let (c, text) = (self.conn.clone(), text.to_string());
        blocking(move || sys::type_text(&c, &text)).await
    }

    async fn keypress(&self, keys: &[String]) -> Result<(), String> {
        let (c, keys) = (self.conn.clone(), keys.to_vec());
        blocking(move || sys::keys(&c, &keys)).await
    }

    async fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_for_names_the_three_buttons() {
        assert_eq!(button_for("LEFT"), Ok(Btn::Left));
        assert_eq!(button_for("right"), Ok(Btn::Right));
        assert_eq!(button_for("middle"), Ok(Btn::Middle));
        assert!(button_for("thumb").is_err());
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
