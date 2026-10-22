# iii-native-ui

The worker-side half of a native app in the iii app, as a crate: one
builder that registers `<ns>::native-ui` (internal, with the `native_ui`
descriptor in its metadata), serves the bundle by path with its sha256, and
hot-reloads it in development. The authoring SOP:
`workers/docs/sops/native-ui.md`; the design:
`ide/projects/worker-native-ui/design.md` §2.

**Linked by path, never published**, and separate from `iii-console-ui` on
purpose: the descriptor needs `sha2`, which only workers with a native app
should take into their `Cargo.lock`.

```toml
# <worker>/Cargo.toml
[dependencies]
iii-native-ui = { path = "../crates/native-ui" }
```

## Usage

```rust
use iii_native_ui::NativeUi;

pub const BUNDLE_PATH: &str = "browser/native.js";
const BUNDLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/native-ui/dist/native.js"));

pub fn native_ui() -> NativeUi {
    NativeUi::new("browser")
        .app("Browser", "globe", "The engine's browser: tabs, a live view of each page, its console and network.")
        .bundle(BUNDLE_PATH, BUNDLE)
}

/// After the worker's own functions are registered.
pub fn register(iii: &Arc<IIIClient>) -> iii_native_ui::NativeUiHandle {
    native_ui().register(iii)
}
```

| Default | Value | Override |
|---|---|---|
| Function | `<ns>::native-ui` | (fixed: the app reads the app's identity from it) |
| Hot-reload env var | `III_<NS>_NATIVE_UI_WATCH` | `.watch_env(…)` |
| Watch dir when the env var is `1`/`true` | `native-ui/dist` | `.watch_default_dir(…)` |
| Lowest catalogue version the bundle needs | 1 | `.catalog(n)` |

What `register` does:

- computes the sha256 and size of the bundle and registers the function
  with `metadata = { internal: true, native_ui: { v: 1, app: {title, icon,
  blurb}, bundle: {path, hash, size}, catalog, dev: false } }`;
- the handler answers `{path}` with `{content, hash, size}` (it ignores the
  engine's `_caller_worker_id`) and any other path with the error code
  `UNKNOWN_NATIVE_UI_ASSET`;
- with the watch env var set, polls `<dir>/<file of the bundle path>` every
  second; on a new non-empty bundle under 1 MiB it swaps the served bytes,
  unregisters the function and registers it again with `dev: true` and the
  new hash (the SDK refuses a second registration of a live id, so the old
  one goes first). The app's dev poll (`engine::functions::info` every
  1.5 s) reloads on the new hash.

The builder panics on what the app would refuse: a namespace outside
`^[a-z0-9][a-z0-9_-]{0,63}$`, a title outside 1–40 characters or a blurb
outside 1–120, an icon that is not a kebab-case Lucide name, a bundle path
outside `[a-z0-9._/-]`, over 512 bytes, without `.js`, with `.`/`..`
segments or not under `<ns>/`, an empty bundle or one above 1 MiB, catalog
0. `NativeUi::metadata()` gives the descriptor for a worker's own tests.
