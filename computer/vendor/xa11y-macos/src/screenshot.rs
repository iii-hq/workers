//! macOS screen capture backend via ScreenCaptureKit.
//!
//! Returns physical (device) pixels as RGBA8. Requires the Screen Recording
//! TCC permission — checked at construction time, same as `MacOSProvider`.

use xa11y_core::{CaptureMapping, Error, Point, Rect, Result, Screenshot, ScreenshotProvider};

use crate::ax::MacOSProvider;

extern "C" {
    fn safe_cg_capture_rgba(
        use_rect: i32,
        rect_x: f64,
        rect_y: f64,
        rect_w: f64,
        rect_h: f64,
        out_pixels: *mut *mut u8,
        out_width: *mut u32,
        out_height: *mut u32,
        out_scale: *mut f64,
        out_origin_x: *mut f64,
        out_origin_y: *mut f64,
    ) -> i32;
    fn safe_cg_free_pixels(pixels: *mut u8);
}

pub struct MacOSScreenshot;

impl MacOSScreenshot {
    pub fn new() -> Result<Self> {
        if !MacOSProvider::has_screen_recording_permission() {
            return Err(Error::PermissionDenied {
                instructions: "Enable Screen Recording in System Settings → Privacy & Security → \
                 Screen & System Audio Recording."
                    .to_string(),
            });
        }
        Ok(Self)
    }

    /// Capture, and report the **logical** screen coordinate of the returned
    /// image's pixel `(0, 0)`.
    ///
    /// For a region that is the requested rect's origin. For a full capture it
    /// is the captured display's `frame.origin`, which is `(0, 0)` only when
    /// `SCShareableContent.displays[0]` happens to be the display at the
    /// coordinate-space origin — on a multi-display Mac it often is not, and
    /// assuming `(0, 0)` misplaces everything a caller maps onto these pixels.
    fn capture(&self, rect: Option<Rect>) -> Result<(Screenshot, Point)> {
        let (use_rect, rx, ry, rw, rh) = match rect {
            Some(r) => (
                1_i32,
                f64::from(r.x),
                f64::from(r.y),
                f64::from(r.width),
                f64::from(r.height),
            ),
            None => (0, 0.0, 0.0, 0.0, 0.0),
        };

        let mut pixels: *mut u8 = std::ptr::null_mut();
        let mut width: u32 = 0;
        let mut height: u32 = 0;
        let mut scale: f64 = 1.0;
        let mut origin_x: f64 = 0.0;
        let mut origin_y: f64 = 0.0;

        let rc = unsafe {
            safe_cg_capture_rgba(
                use_rect,
                rx,
                ry,
                rw,
                rh,
                &mut pixels,
                &mut width,
                &mut height,
                &mut scale,
                &mut origin_x,
                &mut origin_y,
            )
        };

        if rc != 0 || pixels.is_null() {
            // -1: SCShareableContent query failed / returned no displays —
            // typically means the Screen Recording TCC grant is missing for
            // this binary. macOS 15+ silently denies without prompting once
            // a "no" decision is cached.
            let err = match rc {
                -1 => Error::PermissionDenied {
                    instructions: "ScreenCaptureKit could not enumerate displays. Enable Screen \
                                   Recording for this binary in System Settings → Privacy & \
                                   Security → Screen & System Audio Recording."
                        .to_string(),
                },
                -2 => Error::Platform {
                    code: -2,
                    message: "SCScreenshotManager returned no image".into(),
                },
                -3 => Error::Platform {
                    code: -3,
                    message: "pixel buffer allocation failed".into(),
                },
                -4 => Error::Platform {
                    code: -4,
                    message: "bitmap context creation failed".into(),
                },
                -5 => Error::Platform {
                    code: -5,
                    message: "requested rect has zero/negative dimensions".into(),
                },
                -9999 => Error::Platform {
                    code: -9999,
                    message: "ObjC exception during capture".into(),
                },
                other => Error::Platform {
                    code: i64::from(other),
                    message: format!("SCK capture failed with code {other}"),
                },
            };
            return Err(err);
        }
        if width == 0 || height == 0 {
            unsafe { safe_cg_free_pixels(pixels) };
            return Err(Error::Platform {
                code: -2,
                message: "captured image has zero dimensions".into(),
            });
        }

        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| Error::Platform {
                code: -1,
                message: "screenshot dimensions overflow".into(),
            })?;
        let pixels_vec = unsafe { std::slice::from_raw_parts(pixels, size) }.to_vec();
        unsafe { safe_cg_free_pixels(pixels) };

        // The shim reports the origin in logical points; `Point` is integral,
        // matching `Rect` (and therefore `Element::bounds`), so round rather
        // than truncate — a display at y = -899.5 must not become -899.
        let origin = Point::new(origin_x.round() as i32, origin_y.round() as i32);

        Ok((
            Screenshot::new(width, height, pixels_vec, scale as f32)
                .with_mapping(CaptureMapping::single(origin, width, height, scale as f32)),
            origin,
        ))
    }
}

impl ScreenshotProvider for MacOSScreenshot {
    fn capture_full(&self) -> Result<(Screenshot, Point)> {
        self.capture(None)
    }

    fn capture_region(&self, rect: Rect) -> Result<Screenshot> {
        // The origin is `rect`'s own by contract, so there is nothing here the
        // caller does not already have.
        self.capture(Some(rect)).map(|(shot, _)| shot)
    }
}
