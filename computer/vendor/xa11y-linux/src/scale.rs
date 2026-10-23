//! Best-effort HiDPI scale detection for Linux, used by the AT-SPI provider
//! and the native X11 window backend.
//!
//! The cross-platform contract (see `xa11y_core`) is that `Element::bounds`
//! are **logical** (device-independent) coordinates.
//!
//! [`coordinate_scale`] is the factor relating AT-SPI bounds to logical
//! coordinates. Non-`1.0` **only on a pure-X11 session** with integer scaling,
//! read from `Xft.dpi` in the X `RESOURCE_MANAGER` (the same signal GTK/Qt
//! use). On X11, AT-SPI `GetExtents(screen)` is in physical pixels, so bounds
//! are divided by this. On Wayland it is `1.0`: AT-SPI coordinates are already
//! logical.
//!
//! # Caveats (documented, not silently guessed)
//!
//! - **Fractional X11 scaling** (e.g. `Xft.dpi` 120/144): GTK's X11 window
//!   scale is integer-only, so a fractional `Xft.dpi` scales fonts, not the
//!   coordinate space. [`coordinate_scale`] returns `1.0` for non-integer DPI.
//!
//! Reporting `1.0` never breaks round-trips (bounds and native coordinates
//! stay in the same space); it only means bounds are physical on those
//! configurations.

use std::sync::OnceLock;

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

/// "100%" DPI baseline: `scale = dpi / 96`.
const BASE_DPI: f64 = 96.0;
/// Largest integer scale we will honor; guards against a garbage reading.
const MAX_SCALE: i64 = 8;
/// How close `dpi / 96` must be to an integer to be treated as integer scaling.
const INTEGER_EPS: f64 = 0.05;

/// The scale relating AT-SPI bounds and input points to logical coordinates.
/// Detected once, then cached. `1.0` on anything but a pure-X11 integer-scaled
/// session — see the module docs.
pub fn coordinate_scale() -> f64 {
    static SCALE: OnceLock<f64> = OnceLock::new();
    *SCALE.get_or_init(detect_x11_scale)
}

/// X11 `Xft.dpi`-derived integer scale. `1.0` under Wayland (coordinates are
/// logical there) or when there is no X display.
fn detect_x11_scale() -> f64 {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return 1.0;
    }
    if std::env::var_os("DISPLAY").is_none() {
        return 1.0;
    }
    match read_xft_dpi() {
        Some(dpi) => scale_from_dpi(dpi),
        None => 1.0,
    }
}

/// Read the `Xft.dpi` resource from the X `RESOURCE_MANAGER` root-window
/// property. Returns `None` on any X failure or if the resource is absent —
/// callers degrade to `1.0`, never an error (a DPI query must not fail a tree
/// read or a capture).
fn read_xft_dpi() -> Option<f64> {
    let (conn, screen_num) = RustConnection::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let reply = conn
        .get_property(
            false,
            root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            // RESOURCE_MANAGER is small (KBs); cap the read generously in
            // 32-bit units as the X protocol expects.
            1 << 18,
        )
        .ok()?
        .reply()
        .ok()?;
    let text = String::from_utf8_lossy(&reply.value);
    parse_xft_dpi(&text)
}

/// Parse `Xft.dpi:<whitespace><number>` out of an xrdb `RESOURCE_MANAGER`
/// dump. Pure — unit-tested without an X server.
fn parse_xft_dpi(resources: &str) -> Option<f64> {
    for line in resources.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Xft.dpi:") {
            return rest.trim().parse::<f64>().ok().filter(|d| *d > 0.0);
        }
    }
    None
}

/// Map an `Xft.dpi` value to a coordinate scale, honoring **integer** scales
/// only. `dpi / 96` is snapped to the nearest integer when within
/// [`INTEGER_EPS`]; otherwise (fractional or out-of-range) returns `1.0`.
/// Pure — unit-tested.
fn scale_from_dpi(dpi: f64) -> f64 {
    if !dpi.is_finite() || dpi <= 0.0 {
        return 1.0;
    }
    let raw = dpi / BASE_DPI;
    let nearest = raw.round();
    if (raw - nearest).abs() <= INTEGER_EPS && nearest >= 1.0 && (nearest as i64) <= MAX_SCALE {
        nearest
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_xft_dpi, scale_from_dpi};

    #[test]
    fn parse_extracts_dpi_line() {
        let xrdb = "*customization:\t-color\nXft.dpi:\t192\nXft.antialias:\t1\n";
        assert_eq!(parse_xft_dpi(xrdb), Some(192.0));
    }

    #[test]
    fn parse_tolerates_spaces_and_position() {
        assert_eq!(parse_xft_dpi("Xft.dpi:   96"), Some(96.0));
        assert_eq!(parse_xft_dpi("a:1\n  Xft.dpi:\t144  \nb:2"), Some(144.0));
    }

    #[test]
    fn parse_missing_or_bad_is_none() {
        assert_eq!(parse_xft_dpi(""), None);
        assert_eq!(parse_xft_dpi("Xft.antialias:\t1"), None);
        assert_eq!(parse_xft_dpi("Xft.dpi:\tnope"), None);
        assert_eq!(parse_xft_dpi("Xft.dpi:\t-5"), None);
    }

    #[test]
    fn integer_scales_are_honored() {
        assert_eq!(scale_from_dpi(96.0), 1.0);
        assert_eq!(scale_from_dpi(192.0), 2.0);
        assert_eq!(scale_from_dpi(288.0), 3.0);
        // GNOME sometimes reports 95.9/96.1-ish; snap within epsilon.
        assert_eq!(scale_from_dpi(191.5), 2.0);
    }

    #[test]
    fn fractional_scales_fall_back_to_one() {
        // 1.25 and 1.5 are font-only on X11; do not treat as coordinate scale.
        assert_eq!(scale_from_dpi(120.0), 1.0);
        assert_eq!(scale_from_dpi(144.0), 1.0);
    }

    #[test]
    fn out_of_range_or_bad_dpi_is_one() {
        assert_eq!(scale_from_dpi(0.0), 1.0);
        assert_eq!(scale_from_dpi(-96.0), 1.0);
        assert_eq!(scale_from_dpi(f64::NAN), 1.0);
        // 9× (864 dpi) exceeds MAX_SCALE.
        assert_eq!(scale_from_dpi(864.0), 1.0);
    }
}
