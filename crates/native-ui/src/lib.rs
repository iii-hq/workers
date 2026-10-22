//! Worker-side helper for a native app in the iii app (design:
//! `ide/projects/worker-native-ui/design.md` §2; authoring guide:
//! `workers/docs/sops/native-ui.md`).
//!
//! A worker declares a native app by registering **one** internal function,
//! `<ns>::native-ui`, which is both the descriptor (its metadata carries
//! `native_ui`) and the content function (`{path}` → `{content, hash,
//! size}`). The app finds it in `engine::functions::list`, fetches the
//! bundle, checks its sha256 against the descriptor, compiles it and runs it
//! in a QuickJS VM. This crate owns that contract:
//!
//! ```rust,ignore
//! use iii_native_ui::NativeUi;
//!
//! NativeUi::new("browser")
//!     .app("Browser", "globe", "The engine's browser: tabs, a live view of each page, its console and network.")
//!     .bundle(
//!         "browser/native.js",
//!         include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/native-ui/dist/native.js")),
//!     )
//!     .register(&iii);
//! ```
//!
//! The builder panics on what the app would refuse (a bad namespace, title,
//! icon, blurb or path, a bundle above 1 MiB), like `iii-console-ui`: an
//! authoring error should fail the first unit test, not log on a device.
//!
//! In development, `III_<NS>_NATIVE_UI_WATCH` (`1`/`true` = `native-ui/dist`,
//! or a directory) makes the worker poll the built bundle every second; when
//! it changes, the served bytes swap and the function is registered again
//! with `dev: true` and the new hash, which the app's dev poll picks up.
//!
//! A separate crate from `iii-console-ui` on purpose: the descriptor needs
//! sha256, and adding `sha2` there would change the `Cargo.lock` of every
//! worker that links it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

/// The descriptor format this crate writes (`native_ui.v`).
pub const DESCRIPTOR_VERSION: u32 = 1;
/// The error code of a fetch for a path this worker does not serve.
pub const UNKNOWN_ASSET: &str = "UNKNOWN_NATIVE_UI_ASSET";
/// `limits.maxBundleBytes` of the catalogue: the app refuses a larger bundle.
pub const MAX_BUNDLE_BYTES: usize = 1024 * 1024;

const MAX_TITLE_CHARS: usize = 40;
const MAX_BLURB_CHARS: usize = 120;
const MAX_PATH_LEN: usize = 512;
const MAX_ICON_LEN: usize = 64;
const WATCH_POLL: Duration = Duration::from_millis(1000);

/// Input of `<ns>::native-ui`: the app asks for the bundle by its path.
/// Transport fields the engine adds (`_caller_worker_id`) are ignored.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NativeUiInput {
    /// The descriptor's `bundle.path` (e.g. `browser/native.js`).
    pub path: String,
}

/// Output of `<ns>::native-ui`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NativeUiContent {
    /// The bundle's JavaScript source, verbatim.
    pub content: String,
    /// sha256 of the UTF-8 bytes of `content`, lower-case hex.
    pub hash: String,
    /// Size of `content` in UTF-8 bytes.
    pub size: usize,
}

#[derive(Debug, Clone)]
struct AppInfo {
    title: String,
    icon: String,
    blurb: String,
}

/// Builder for a worker's native app. Construct with [`NativeUi::new`], give
/// it [`app`](NativeUi::app) and [`bundle`](NativeUi::bundle), then
/// [`register`](NativeUi::register).
pub struct NativeUi {
    ns: String,
    function_id: String,
    watch_env: String,
    watch_default_dir: PathBuf,
    app: Option<AppInfo>,
    bundle_path: Option<String>,
    source: Option<String>,
    catalog: u32,
}

impl NativeUi {
    /// Starts a builder for namespace `ns` (the prefix of the worker's
    /// functions, `browser` in `browser::navigate`). Defaults: function
    /// `<ns>::native-ui`, watch env `III_<NS>_NATIVE_UI_WATCH`, watch dir
    /// `native-ui/dist` (relative to the worker's cwd), catalog 1.
    ///
    /// Panics unless `ns` matches `^[a-z0-9][a-z0-9_-]{0,63}$`.
    pub fn new(ns: impl Into<String>) -> Self {
        let ns = ns.into();
        validate_ns(&ns);
        let env_stem: String = ns
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        Self {
            function_id: format!("{ns}::native-ui"),
            watch_env: format!("III_{env_stem}_NATIVE_UI_WATCH"),
            watch_default_dir: PathBuf::from("native-ui/dist"),
            ns,
            app: None,
            bundle_path: None,
            source: None,
            catalog: 1,
        }
    }

    /// The app as the hub, the toolbar, the palette and Settings show it:
    /// `title` (1–40 characters), a Lucide `icon` name in kebab-case, and a
    /// `blurb` (1–120 characters).
    pub fn app(
        mut self,
        title: impl Into<String>,
        icon: impl Into<String>,
        blurb: impl Into<String>,
    ) -> Self {
        let (title, icon, blurb) = (title.into(), icon.into(), blurb.into());
        validate_text("title", &title, MAX_TITLE_CHARS);
        validate_text("blurb", &blurb, MAX_BLURB_CHARS);
        validate_icon(&icon);
        self.app = Some(AppInfo { title, icon, blurb });
        self
    }

    /// The bundle (what `buildNativeUi` of `@iii-dev/native-ui/build` wrote,
    /// typically `include_str!`-embedded) and the path the app fetches it by:
    /// `[a-z0-9._/-]`, at most 512 bytes, ending in `.js`, first segment `<ns>`.
    pub fn bundle(mut self, path: impl Into<String>, source: impl Into<String>) -> Self {
        let (path, source) = (path.into(), source.into());
        validate_path(&self.ns, &path);
        validate_source(&source);
        self.bundle_path = Some(path);
        self.source = Some(source);
        self
    }

    /// The lowest catalogue version the bundle needs (default 1); an app with
    /// an older catalogue says "Needs a newer iii app".
    pub fn catalog(mut self, min: u32) -> Self {
        assert!(min >= 1, "native ui catalog version must be at least 1");
        self.catalog = min;
        self
    }

    /// Overrides the hot-reload env var (default `III_<NS>_NATIVE_UI_WATCH`).
    pub fn watch_env(mut self, name: impl Into<String>) -> Self {
        self.watch_env = name.into();
        self
    }

    /// Overrides the directory polled when the env var is `1`/`true`
    /// (default `native-ui/dist`).
    pub fn watch_default_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.watch_default_dir = dir.into();
        self
    }

    /// The function this builder registers (`<ns>::native-ui`).
    pub fn function_id(&self) -> &str {
        &self.function_id
    }

    /// The function's metadata as the app reads it (`internal` and
    /// `native_ui`), for the bundle as given. Panics without `app`/`bundle`.
    pub fn metadata(&self) -> Value {
        let (app, path, source) = self.parts();
        metadata(
            app,
            path,
            &BundleState::of(source.to_string(), false),
            self.catalog,
        )
    }

    fn parts(&self) -> (&AppInfo, &str, &str) {
        let app = self.app.as_ref().unwrap_or_else(|| {
            panic!(
                "NativeUi::new(\"{}\") needs .app(title, icon, blurb)",
                self.ns
            )
        });
        let (Some(path), Some(source)) = (self.bundle_path.as_deref(), self.source.as_deref())
        else {
            panic!("NativeUi::new(\"{}\") needs .bundle(path, source)", self.ns)
        };
        (app, path, source)
    }

    /// Registers `<ns>::native-ui` (internal, with the descriptor in its
    /// metadata) and, when the watch env var is set, starts the dev poller.
    /// Call once, after the worker's own functions. Must run inside a tokio
    /// runtime when the watcher is enabled.
    ///
    /// Panics without [`app`](Self::app) and [`bundle`](Self::bundle).
    pub fn register(self, iii: &Arc<IIIClient>) -> NativeUiHandle {
        let (app, path, source) = self.parts();
        let state = BundleState::of(source.to_string(), false);
        let served = Arc::new(Served {
            path: path.to_string(),
            state: RwLock::new(state.clone()),
        });
        let spec = Arc::new(Spec {
            function_id: self.function_id.clone(),
            ns: self.ns.clone(),
            app: app.clone(),
            path: path.to_string(),
            catalog: self.catalog,
        });
        let function = register_content_function(iii, &spec, &served, &state);
        tracing::info!(
            function = spec.function_id,
            path = spec.path,
            hash = state.hash,
            size = state.size,
            "registered native ui"
        );
        let handle = NativeUiHandle {
            function_id: spec.function_id.clone(),
            served: served.clone(),
            function: Arc::new(Mutex::new(Some(function))),
        };
        if let Some(dir) = watch_target(&self.watch_env, &self.watch_default_dir) {
            spawn_watcher(iii.clone(), spec, served, handle.function.clone(), dir);
        }
        handle
    }
}

/// What [`NativeUi::register`] returns: the registration, kept so the dev
/// watcher can replace it and the worker can drop it.
pub struct NativeUiHandle {
    function_id: String,
    served: Arc<Served>,
    function: Arc<Mutex<Option<FunctionRef>>>,
}

impl NativeUiHandle {
    /// `<ns>::native-ui`.
    pub fn function_id(&self) -> &str {
        &self.function_id
    }

    /// The sha256 of the bundle served now.
    pub async fn hash(&self) -> String {
        self.served.state.read().await.hash.clone()
    }

    /// Unregisters the function (the app drops the app's VM at its next probe).
    pub fn unregister(&self) {
        if let Some(function) = self
            .function
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            function.unregister();
        }
    }
}

struct Spec {
    function_id: String,
    ns: String,
    app: AppInfo,
    path: String,
    catalog: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundleState {
    content: String,
    hash: String,
    size: usize,
    dev: bool,
}

impl BundleState {
    fn of(content: String, dev: bool) -> Self {
        Self {
            hash: sha256_hex(&content),
            size: content.len(),
            content,
            dev,
        }
    }
}

/// The bundle behind the function, swapped in place by the dev watcher.
struct Served {
    path: String,
    state: RwLock<BundleState>,
}

impl Served {
    async fn content_for(&self, path: &str) -> Result<NativeUiContent, Error> {
        if path != self.path {
            return Err(Error::Remote {
                code: UNKNOWN_ASSET.to_string(),
                message: format!(
                    "'{path}' is not this worker's native ui bundle (expected '{}')",
                    self.path
                ),
                stacktrace: None,
            });
        }
        let state = self.state.read().await;
        Ok(NativeUiContent {
            content: state.content.clone(),
            hash: state.hash.clone(),
            size: state.size,
        })
    }
}

/// sha256 of the UTF-8 bytes of `content`, lower-case hex (`bundle.hash`).
pub fn sha256_hex(content: &str) -> String {
    Sha256::digest(content.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn metadata(app: &AppInfo, path: &str, state: &BundleState, catalog: u32) -> Value {
    json!({
        "internal": true,
        "native_ui": {
            "v": DESCRIPTOR_VERSION,
            "app": { "title": app.title, "icon": app.icon, "blurb": app.blurb },
            "bundle": { "path": path, "hash": state.hash, "size": state.size },
            "catalog": catalog,
            "dev": state.dev,
        }
    })
}

fn register_content_function(
    iii: &IIIClient,
    spec: &Spec,
    served: &Arc<Served>,
    state: &BundleState,
) -> FunctionRef {
    let served = served.clone();
    iii.register_function(
        spec.function_id.clone(),
        RegisterFunction::new_async(move |input: NativeUiInput| {
            let served = served.clone();
            async move { served.content_for(&input.path).await }
        })
        .description(format!(
            "Serve the {} worker's native app bundle for the iii app.",
            spec.ns
        ))
        // App plumbing, not an API for agents or other workers.
        .metadata(metadata(&spec.app, &spec.path, state, spec.catalog)),
    )
}

fn validate_ns(ns: &str) {
    let mut chars = ns.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let rest_ok =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    assert!(
        first_ok && rest_ok && ns.len() <= 64,
        "native ui namespace '{ns}' must match ^[a-z0-9][a-z0-9_-]{{0,63}}$"
    );
}

fn validate_text(what: &str, text: &str, max: usize) {
    let n = text.chars().count();
    assert!(
        n >= 1 && n <= max && !text.trim().is_empty(),
        "native ui app {what} must have 1 to {max} characters, got {n}: '{text}'"
    );
}

fn validate_icon(icon: &str) {
    let kebab = !icon.is_empty()
        && icon.len() <= MAX_ICON_LEN
        && icon.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
    assert!(
        kebab,
        "native ui app icon '{icon}' must be a Lucide name in kebab-case (globe, refresh-cw)"
    );
}

fn validate_path(ns: &str, path: &str) {
    assert!(
        !path.is_empty() && path.len() <= MAX_PATH_LEN,
        "native ui bundle path '{path}' must have 1 to {MAX_PATH_LEN} bytes"
    );
    assert!(
        !path.starts_with('/'),
        "native ui bundle path '{path}' must not start with '/'"
    );
    for segment in path.split('/') {
        assert!(
            !segment.is_empty() && segment != "." && segment != "..",
            "native ui bundle path '{path}' has an empty, '.' or '..' segment"
        );
        assert!(
            segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c)),
            "native ui bundle path '{path}' has characters outside [a-z0-9._/-]"
        );
    }
    assert!(
        path.ends_with(".js"),
        "native ui bundle path '{path}' must end with '.js'"
    );
    assert!(
        path.split('/').next() == Some(ns),
        "native ui bundle path '{path}' must start with '{ns}/'"
    );
}

fn validate_source(source: &str) {
    assert!(!source.is_empty(), "native ui bundle is empty");
    assert!(
        source.len() <= MAX_BUNDLE_BYTES,
        "native ui bundle has {} bytes, above the app's {MAX_BUNDLE_BYTES}",
        source.len()
    );
}

fn watch_target(env: &str, default_dir: &Path) -> Option<PathBuf> {
    parse_watch_target(&std::env::var(env).ok()?, default_dir)
}

fn parse_watch_target(raw: &str, default_dir: &Path) -> Option<PathBuf> {
    if raw.is_empty() || raw == "0" || raw.eq_ignore_ascii_case("false") {
        return None;
    }
    if raw == "1" || raw.eq_ignore_ascii_case("true") {
        return Some(default_dir.to_path_buf());
    }
    let path = PathBuf::from(raw);
    // A built .js file means its directory.
    if path.extension().is_some_and(|ext| ext == "js") {
        return Some(path.parent().map(PathBuf::from).unwrap_or_default());
    }
    Some(path)
}

/// Reads the built bundle; the next state when it changed and the app would
/// take it, `None` otherwise (missing, unchanged, empty, too large).
async fn poll_once(file: &Path, current_hash: &str) -> Option<BundleState> {
    let next = tokio::fs::read_to_string(file).await.ok()?;
    if next.is_empty() {
        return None;
    }
    if next.len() > MAX_BUNDLE_BYTES {
        tracing::warn!(
            file = %file.display(),
            size = next.len(),
            "native ui bundle above the app's limit; not served"
        );
        return None;
    }
    let state = BundleState::of(next, true);
    (state.hash != current_hash).then_some(state)
}

/// Dev-loop hot reload: on a new bundle, swap the served bytes, then register
/// the function again with `dev: true` and the new hash. The SDK refuses a
/// second registration of a live id, so the previous one goes first.
fn spawn_watcher(
    iii: Arc<IIIClient>,
    spec: Arc<Spec>,
    served: Arc<Served>,
    function: Arc<Mutex<Option<FunctionRef>>>,
    dir: PathBuf,
) {
    let file_name = spec
        .path
        .rsplit('/')
        .next()
        .unwrap_or("native.js")
        .to_string();
    let file = dir.join(file_name);
    tokio::spawn(async move {
        tracing::info!(file = %file.display(), "native ui watch enabled: hot reload on rebuild");
        loop {
            tokio::time::sleep(WATCH_POLL).await;
            let current = served.state.read().await.hash.clone();
            let Some(next) = poll_once(&file, &current).await else {
                continue;
            };
            *served.state.write().await = next.clone();
            let mut slot = function.lock().unwrap_or_else(|e| e.into_inner());
            let Some(previous) = slot.take() else {
                // The worker unregistered the app; stop watching.
                return;
            };
            previous.unregister();
            *slot = Some(register_content_function(&iii, &spec, &served, &next));
            tracing::info!(
                function = spec.function_id,
                hash = next.hash,
                size = next.size,
                "native ui re-registered (hot reload)"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE: &str = "(()=>{globalThis.x=1})();";

    fn browser() -> NativeUi {
        NativeUi::new("browser")
            .app(
                "Browser",
                "globe",
                "The engine's browser: tabs, a live view of each page, its console and network.",
            )
            .bundle("browser/native.js", BUNDLE)
    }

    #[test]
    fn defaults_derive_from_the_namespace() {
        let ui = NativeUi::new("browser");
        assert_eq!(ui.function_id(), "browser::native-ui");
        assert_eq!(ui.watch_env, "III_BROWSER_NATIVE_UI_WATCH");
        assert_eq!(ui.watch_default_dir, PathBuf::from("native-ui/dist"));
        assert_eq!(ui.catalog, 1);

        let ui = NativeUi::new("ios-simulator");
        assert_eq!(ui.function_id(), "ios-simulator::native-ui");
        assert_eq!(ui.watch_env, "III_IOS_SIMULATOR_NATIVE_UI_WATCH");
    }

    #[test]
    fn overrides_replace_the_defaults() {
        let ui = NativeUi::new("demo")
            .watch_env("DEMO_WATCH")
            .watch_default_dir("web/out")
            .catalog(2);
        assert_eq!(ui.watch_env, "DEMO_WATCH");
        assert_eq!(ui.watch_default_dir, PathBuf::from("web/out"));
        assert_eq!(ui.catalog, 2);
    }

    #[test]
    fn metadata_is_the_descriptor_the_app_reads() {
        let meta = browser().metadata();
        assert_eq!(
            meta,
            json!({
                "internal": true,
                "native_ui": {
                    "v": 1,
                    "app": {
                        "title": "Browser",
                        "icon": "globe",
                        "blurb": "The engine's browser: tabs, a live view of each page, its console and network.",
                    },
                    "bundle": { "path": "browser/native.js", "hash": sha256_hex(BUNDLE), "size": BUNDLE.len() },
                    "catalog": 1,
                    "dev": false,
                }
            })
        );
    }

    #[test]
    fn the_hash_is_sha256sum_of_the_utf8_bytes() {
        // Vectors from `printf '…' | sha256sum`.
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex("iii — ✓ native"),
            "78568b522f7c117c28cea9d8931ac0ae9c379117abea3790bf0d045320ba16b1"
        );
        let state = BundleState::of("iii — ✓ native".to_string(), false);
        assert_eq!(state.size, "iii — ✓ native".len());
        assert_eq!(state.size, 18);
    }

    fn served(content: &str) -> Served {
        Served {
            path: "browser/native.js".to_string(),
            state: RwLock::new(BundleState::of(content.to_string(), false)),
        }
    }

    #[tokio::test]
    async fn serves_the_bundle_with_its_hash_and_size() {
        let out = served(BUNDLE)
            .content_for("browser/native.js")
            .await
            .unwrap();
        assert_eq!(
            out,
            NativeUiContent {
                content: BUNDLE.to_string(),
                hash: sha256_hex(BUNDLE),
                size: BUNDLE.len(),
            }
        );
    }

    #[tokio::test]
    async fn refuses_other_paths_with_the_unknown_asset_code() {
        let err = served(BUNDLE)
            .content_for("browser/other.js")
            .await
            .unwrap_err();
        match err {
            Error::Remote { code, message, .. } => {
                assert_eq!(code, UNKNOWN_ASSET);
                assert!(message.contains("browser/other.js"));
                assert!(message.contains("browser/native.js"));
            }
            other => panic!("expected a remote error, got {other:?}"),
        }
    }

    #[test]
    fn the_input_ignores_engine_metadata() {
        let input: NativeUiInput = serde_json::from_value(json!({
            "path": "browser/native.js",
            "_caller_worker_id": "00000000-0000-4000-8000-000000000002",
        }))
        .unwrap();
        assert_eq!(input.path, "browser/native.js");
    }

    #[tokio::test]
    async fn the_watcher_swaps_only_on_a_new_valid_bundle() {
        let dir = std::env::temp_dir().join(format!("iii-native-ui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("native.js");
        assert_eq!(
            poll_once(&file, "").await,
            None,
            "a missing file is no change"
        );

        std::fs::write(&file, BUNDLE).unwrap();
        assert_eq!(
            poll_once(&file, &sha256_hex(BUNDLE)).await,
            None,
            "same bytes, same hash"
        );

        std::fs::write(&file, "(()=>{})();").unwrap();
        let next = poll_once(&file, &sha256_hex(BUNDLE)).await.unwrap();
        assert_eq!(next.hash, sha256_hex("(()=>{})();"));
        assert!(next.dev);
        let meta = metadata(&browser().app.unwrap(), "browser/native.js", &next, 1);
        assert_eq!(meta["native_ui"]["dev"], json!(true));
        assert_eq!(meta["native_ui"]["bundle"]["hash"], json!(next.hash));

        std::fs::write(&file, "").unwrap();
        assert_eq!(
            poll_once(&file, "x").await,
            None,
            "an empty file is a build in progress"
        );
        std::fs::write(&file, "x".repeat(MAX_BUNDLE_BYTES + 1)).unwrap();
        assert_eq!(poll_once(&file, "x").await, None, "too large for the app");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parse_watch_target_conventions() {
        let dist = Path::new("native-ui/dist");
        assert_eq!(parse_watch_target("", dist), None);
        assert_eq!(parse_watch_target("0", dist), None);
        assert_eq!(parse_watch_target("FALSE", dist), None);
        assert_eq!(parse_watch_target("1", dist), Some(dist.to_path_buf()));
        assert_eq!(parse_watch_target("true", dist), Some(dist.to_path_buf()));
        assert_eq!(parse_watch_target("out", dist), Some(PathBuf::from("out")));
        assert_eq!(
            parse_watch_target("out/native.js", dist),
            Some(PathBuf::from("out"))
        );
    }

    #[test]
    fn accepts_the_limits_exactly() {
        let ns = "a".repeat(64);
        let _ = NativeUi::new(&ns);
        let _ = NativeUi::new("9-lives_ok");
        let title = "t".repeat(40);
        let blurb = "b".repeat(120);
        let _ = NativeUi::new("demo").app(title, "refresh-cw", blurb);
        let path = format!("demo/{}.js", "p".repeat(MAX_PATH_LEN - 8));
        assert_eq!(path.len(), MAX_PATH_LEN);
        let _ = NativeUi::new("demo").bundle(path, "x".repeat(MAX_BUNDLE_BYTES));
    }

    #[test]
    #[should_panic(expected = "must match ^[a-z0-9][a-z0-9_-]{0,63}$")]
    fn uppercase_namespace_panics() {
        let _ = NativeUi::new("Browser");
    }

    #[test]
    #[should_panic(expected = "must match")]
    fn leading_dash_namespace_panics() {
        let _ = NativeUi::new("-browser");
    }

    #[test]
    #[should_panic(expected = "must match")]
    fn long_namespace_panics() {
        let _ = NativeUi::new("a".repeat(65));
    }

    #[test]
    #[should_panic(expected = "title must have 1 to 40 characters")]
    fn long_title_panics() {
        let _ = NativeUi::new("demo").app("t".repeat(41), "globe", "Blurb.");
    }

    #[test]
    #[should_panic(expected = "title must have 1 to 40 characters")]
    fn blank_title_panics() {
        let _ = NativeUi::new("demo").app("  ", "globe", "Blurb.");
    }

    #[test]
    #[should_panic(expected = "blurb must have 1 to 120 characters")]
    fn long_blurb_panics() {
        let _ = NativeUi::new("demo").app("Demo", "globe", "b".repeat(121));
    }

    #[test]
    #[should_panic(expected = "kebab-case")]
    fn camel_case_icon_panics() {
        let _ = NativeUi::new("demo").app("Demo", "refreshCw", "Blurb.");
    }

    #[test]
    #[should_panic(expected = "must start with 'browser/'")]
    fn path_outside_the_namespace_panics() {
        let _ = NativeUi::new("browser").bundle("state/native.js", BUNDLE);
    }

    #[test]
    #[should_panic(expected = "must end with '.js'")]
    fn path_without_js_panics() {
        let _ = NativeUi::new("browser").bundle("browser/native.mjs", BUNDLE);
    }

    #[test]
    #[should_panic(expected = "outside [a-z0-9._/-]")]
    fn uppercase_path_panics() {
        let _ = NativeUi::new("browser").bundle("browser/Native.js", BUNDLE);
    }

    #[test]
    #[should_panic(expected = "'.' or '..' segment")]
    fn dotdot_path_panics() {
        let _ = NativeUi::new("browser").bundle("browser/../native.js", BUNDLE);
    }

    #[test]
    #[should_panic(expected = "must not start with '/'")]
    fn absolute_path_panics() {
        let _ = NativeUi::new("browser").bundle("/browser/native.js", BUNDLE);
    }

    #[test]
    #[should_panic(expected = "must have 1 to 512 bytes")]
    fn long_path_panics() {
        let _ =
            NativeUi::new("demo").bundle(format!("demo/{}.js", "p".repeat(MAX_PATH_LEN)), BUNDLE);
    }

    #[test]
    #[should_panic(expected = "bundle is empty")]
    fn empty_bundle_panics() {
        let _ = NativeUi::new("browser").bundle("browser/native.js", "");
    }

    #[test]
    #[should_panic(expected = "above the app's 1048576")]
    fn large_bundle_panics() {
        let _ =
            NativeUi::new("browser").bundle("browser/native.js", "x".repeat(MAX_BUNDLE_BYTES + 1));
    }

    #[test]
    #[should_panic(expected = "catalog version must be at least 1")]
    fn catalog_zero_panics() {
        let _ = NativeUi::new("demo").catalog(0);
    }

    #[test]
    #[should_panic(expected = "needs .app(title, icon, blurb)")]
    fn metadata_without_app_panics() {
        let _ = NativeUi::new("demo")
            .bundle("demo/native.js", BUNDLE)
            .metadata();
    }

    #[test]
    #[should_panic(expected = "needs .bundle(path, source)")]
    fn metadata_without_bundle_panics() {
        let _ = NativeUi::new("demo")
            .app("Demo", "globe", "Blurb.")
            .metadata();
    }
}
