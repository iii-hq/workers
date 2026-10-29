//! Native app of the browser worker for the iii app
//! (ide/projects/worker-native-ui/design.md §8, §9.1; authoring SOP:
//! workers/docs/sops/native-ui.md).
//!
//! Registers `browser::native-ui`: internal, with the `native_ui` descriptor
//! in its metadata (title, icon, blurb, the bundle's path, sha256 and size),
//! and serving `{path: "browser/native.js"}` → `{content, hash, size}`. The
//! iii app finds it on its probe, fetches the bundle, checks the hash and
//! runs it in a QuickJS VM: the tabs, the live page of the open one (a
//! RemoteScreen fed by `browser::frame-event`), its console, network and
//! history, shaped for a phone and a tablet.
//!
//! The bundle is compiled from `native-ui/` by `buildNativeUi` (TSX over the
//! `@iii-dev/native-ui` SDK) and embedded at compile time, like the console
//! UI in `ui.rs`. The registration machinery (descriptor, content handler,
//! `III_BROWSER_NATIVE_UI_WATCH` hot reload) lives in the shared
//! `iii-native-ui` crate (path-linked from `workers/crates/native-ui`).

use std::sync::Arc;

use iii_native_ui::{NativeUi, NativeUiHandle};
use iii_sdk::IIIClient;

pub const BUNDLE_PATH: &str = "browser/native.js";

/// Built by `build.rs` (`buildNativeUi` over `native-ui/`).
const BUNDLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/native-ui/dist/native.js"
));

pub fn native_ui() -> NativeUi {
    NativeUi::new("browser")
        .app(
            "Browser",
            "globe",
            "The engine's browser: tabs, a live view of each page, its console and network.",
        )
        .bundle(BUNDLE_PATH, BUNDLE)
}

/// Register the browser worker's native app. Call once, after the
/// `browser::*` functions are registered; keep the handle for the life of
/// the worker (the dev watcher replaces the registration through it).
pub fn register(iii: &Arc<IIIClient>) -> NativeUiHandle {
    native_ui().register(iii)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_accepts_the_app() {
        // The builder panics on anything the iii app would refuse (title,
        // icon, blurb, path, size).
        assert_eq!(native_ui().function_id(), "browser::native-ui");
    }

    #[test]
    fn embedded_bundle_is_a_native_ui_app() {
        assert!(!BUNDLE.is_empty(), "native-ui/dist/native.js is empty");
        // One IIFE with the SDK runtime: it defines __vm_receive for the host.
        assert!(
            BUNDLE.contains("__vm_receive"),
            "built native.js looks wrong (no __vm_receive)"
        );
        assert!(
            BUNDLE.len() <= iii_native_ui::MAX_BUNDLE_BYTES,
            "native.js is above the app's 1 MiB limit"
        );
    }

    #[test]
    fn descriptor_carries_the_app_and_the_bundle_hash() {
        let meta = native_ui().metadata();
        assert_eq!(meta["internal"], true);
        let d = &meta["native_ui"];
        assert_eq!(d["v"], 1);
        assert_eq!(d["app"]["title"], "Browser");
        assert_eq!(d["app"]["icon"], "globe");
        assert_eq!(d["bundle"]["path"], BUNDLE_PATH);
        assert_eq!(d["bundle"]["hash"], iii_native_ui::sha256_hex(BUNDLE));
        assert_eq!(d["bundle"]["size"], BUNDLE.len());
        assert_eq!(d["catalog"], 1);
        assert_eq!(d["dev"], false);
    }
}
