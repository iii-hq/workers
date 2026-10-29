//! `browser::resize` — set the session's live viewport size. The console
//! calls this as its browser pane resizes so the streamed frame fills the
//! pane exactly (no letterboxing) and click coordinates map 1:1; the device
//! toolbar calls it with a preset. Applied as a CDP device-metrics override.
//! `mobile: true` emulates a phone for real: besides the mobile viewport the
//! page gets touch input and an Android Chrome user agent (with client
//! hints) of the running Chromium's major version.

use chromiumoxide::cdp::browser_protocol::emulation::{UserAgentBrandVersion, UserAgentMetadata};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bounds a viewport dimension can take, so a stray pane measurement can't
/// ask Chromium for something absurd.
pub const MIN_DIM: u32 = 200;
pub const MAX_DIM: u32 = 4000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResizeInput {
    pub session_id: String,
    /// Viewport width in CSS pixels (clamped 200..4000).
    pub width: u32,
    /// Viewport height in CSS pixels (clamped 200..4000).
    pub height: u32,
    /// Device pixel ratio. Default 1.
    #[serde(default)]
    pub device_scale_factor: Option<f64>,
    /// Emulate a phone: the mobile viewport (viewport meta, overlay
    /// scrollbars), touch input (`navigator.maxTouchPoints`,
    /// `(pointer: coarse)`, an `act` left click arrives as a tap) and an
    /// Android Chrome user agent with matching client hints. Default false,
    /// which restores the desktop browser. The user agent reaches the page
    /// on its next load: reload once when `mode_changed`. The device toolbar
    /// sets this for phone presets.
    #[serde(default)]
    pub mobile: Option<bool>,
    /// Mark this resize as a pane auto-fit; a fit is refused (current size
    /// returned) while more than one viewer watches the session.
    // Keeps two open panes from fighting over the shared viewport; explicit
    // resizes (device toolbar, agents) always apply.
    #[serde(default)]
    pub fit: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResizeOutput {
    pub ok: bool,
    /// The clamped size actually applied.
    pub width: u32,
    pub height: u32,
    /// The emulation now in force: true for a phone, false for the desktop
    /// browser. A fit that was refused or only grown (several viewers)
    /// keeps the mode the tab had.
    pub mobile: bool,
    /// True when this call switched between phone and desktop; the page
    /// sees the other user agent only after it loads again.
    pub mode_changed: bool,
}

pub fn clamp(value: u32) -> u32 {
    value.clamp(MIN_DIM, MAX_DIM)
}

/// The `Chrome/<version>` a user agent or a `Browser.getVersion` product
/// names (`HeadlessChrome/…` included): `156.0.8078.4`.
pub fn chrome_version(user_agent: &str) -> Option<&str> {
    let start = user_agent.find("Chrome/")? + "Chrome/".len();
    let version = user_agent[start..].split_whitespace().next()?;
    version
        .split('.')
        .next()
        .is_some_and(|major| !major.is_empty() && major.bytes().all(|b| b.is_ascii_digit()))
        .then_some(version)
}

/// The Android Chrome user agent a phone of the running Chromium's major
/// version sends (the reduced form Chrome ships: minor parts zeroed).
pub fn android_user_agent(major: &str) -> String {
    format!(
        "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) \
         Chrome/{major}.0.0.0 Mobile Safari/537.36"
    )
}

/// The client hints (`Sec-CH-UA-*`, `navigator.userAgentData`) matching
/// `android_user_agent`.
pub fn android_user_agent_metadata(full_version: &str) -> UserAgentMetadata {
    let major = full_version.split('.').next().unwrap_or(full_version);
    let brand = |brand: &str, version: &str| UserAgentBrandVersion::new(brand, version);
    UserAgentMetadata {
        brands: Some(vec![brand("Chromium", major), brand("Not.A/Brand", "99")]),
        full_version_list: Some(vec![
            brand("Chromium", full_version),
            brand("Not.A/Brand", "99.0.0.0"),
        ]),
        platform: "Android".to_string(),
        platform_version: "14.0.0".to_string(),
        architecture: String::new(),
        model: "Pixel 8".to_string(),
        mobile: true,
        bitness: None,
        wow64: None,
        form_factors: Some(vec!["Mobile".to_string()]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chrome_version_comes_from_a_user_agent_or_a_product() {
        let desktop = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                       HeadlessChrome/156.0.8078.4 Safari/537.36";
        assert_eq!(chrome_version(desktop), Some("156.0.8078.4"));
        assert_eq!(chrome_version("Chrome/148.0.7778.5"), Some("148.0.7778.5"));
        assert_eq!(chrome_version("Lightpanda/1.0"), None);
        assert_eq!(chrome_version("Chrome/ beta"), None);
    }

    #[test]
    fn the_android_user_agent_and_hints_carry_the_major_version() {
        assert_eq!(
            android_user_agent("156"),
            "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/156.0.0.0 Mobile Safari/537.36"
        );
        let hints = android_user_agent_metadata("156.0.8078.4");
        assert!(hints.mobile);
        assert_eq!(hints.platform, "Android");
        let brands = hints.brands.unwrap();
        assert!(brands
            .iter()
            .any(|b| b.brand == "Chromium" && b.version == "156"));
        assert!(hints
            .full_version_list
            .unwrap()
            .iter()
            .any(|b| b.brand == "Chromium" && b.version == "156.0.8078.4"));
    }
}
