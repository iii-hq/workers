//! Native app of the kanban worker for the iii app
//! (ide/projects/kanban-native-ui/design.md; authoring SOP:
//! workers/docs/sops/native-ui.md).
//!
//! Registers `kanban::native-ui`: internal, with the `native_ui` descriptor in
//! its metadata (title, icon, blurb, the bundle's path, sha256 and size, and
//! catalogue 2), and serving `{path: "kanban/native.js"}` → `{content, hash,
//! size}`. The iii app finds it on its probe, fetches the bundle, checks the
//! hash and runs it in a QuickJS VM: the board as lanes (one per page on a
//! phone, side by side on a wide screen), a ticket with its conversation,
//! new tickets, filters, and the releases and projects to read.
//!
//! The bundle is compiled from `native-ui/` by `buildNativeUi` (TSX over the
//! `@iii-dev/native-ui` SDK) and embedded at compile time, like the console
//! UI in `ui.rs`. The registration machinery (descriptor, content handler,
//! `III_KANBAN_NATIVE_UI_WATCH` hot reload) lives in the shared
//! `iii-native-ui` crate (path-linked from `workers/crates/native-ui`).

use std::sync::Arc;

use iii_native_ui::{NativeUi, NativeUiHandle};
use iii_sdk::IIIClient;

pub const BUNDLE_PATH: &str = "kanban/native.js";

/// The catalogue the bundle needs: Markdown, Lanes, named hues, List drag and
/// drop and `ListDetail.listFill` are catalogue 2. An older iii app shows
/// "Needs a newer iii app" instead of running it.
pub const CATALOG: u32 = 2;

/// Built by `build.rs` (`buildNativeUi` over `native-ui/`).
const BUNDLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/native-ui/dist/native.js"
));

pub fn native_ui() -> NativeUi {
    NativeUi::new("kanban")
        .app(
            "Kanban",
            "columns-2",
            "The board on your phone: follow the agents' tickets, answer them, move and assign.",
        )
        .bundle(BUNDLE_PATH, BUNDLE)
        .catalog(CATALOG)
}

/// Register the kanban worker's native app. Call once, after the `kanban::*`
/// functions are registered; keep the handle for the life of the worker (the
/// dev watcher replaces the registration through it).
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
        assert_eq!(native_ui().function_id(), "kanban::native-ui");
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
        // The app's own calls and its trigger are compiled in.
        for needle in ["kanban:change", "board::get", "ticket::move"] {
            assert!(BUNDLE.contains(needle), "native.js must mention {needle}");
        }
    }

    #[test]
    fn descriptor_carries_the_app_the_bundle_hash_and_catalogue_2() {
        let meta = native_ui().metadata();
        assert_eq!(meta["internal"], true);
        let d = &meta["native_ui"];
        assert_eq!(d["v"], 1);
        assert_eq!(d["app"]["title"], "Kanban");
        assert_eq!(d["app"]["icon"], "columns-2");
        assert!(d["app"]["blurb"].as_str().is_some_and(|b| b.len() <= 120));
        assert_eq!(d["bundle"]["path"], BUNDLE_PATH);
        assert_eq!(d["bundle"]["hash"], iii_native_ui::sha256_hex(BUNDLE));
        assert_eq!(d["bundle"]["size"], BUNDLE.len());
        assert_eq!(d["catalog"], 2);
        assert_eq!(d["dev"], false);
    }
}
