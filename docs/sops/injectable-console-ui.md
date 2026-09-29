# Injectable console UI — the Rust worker side

How a Rust worker in this repo embeds its built console UI assets, registers
them through the `iii-console-ui` crate, keeps them fresh from `build.rs`,
hot-reloads them in development, and tests the embedding. Nothing here is
about what the UI contains.

## What lives elsewhere

Everything an author reads before touching `ui/` is a console skill. This SOP
does not restate any of it.

| Topic | Read |
|---|---|
| How injection works, the `ui/` project layout, `setup(host)` and every slot, the wire contract, the shared build driver and its lint, scoped CSS, hooks/format/icons, debugging, the testing layers, the definition of done | `ade/skills/injectable-ui.md` (`ade/injectable-ui`) |
| Responsive UX, archetypes, configuration and provider forms, the validation matrix | `ade/skills/design-console-ui.md` (`ade/design-console-ui`) |
| Tokens, typography, shared components, every number | `ade/skills/design-system.md` (`ade/design-system`) |
| The console worker: functions, trigger types, `console::ui-manifest` | `ade/skills/SKILL.md` |
| Trigger-activity renderer authoring and its test matrix | `ade/web/docs/custom-trigger-components.md` |
| Per-worker conformance inventory and the deliberate local exceptions | `docs/sops/console-ui-conformance.md` |
| The crate: defaults, overrides, what `register` does | `crates/console-ui/README.md`, `crates/console-ui/src/lib.rs` |
| Node workers | No helper exists; implement the wire contract directly (`ade/injectable-ui` › Registration). `pi`, `vscode`, `onboarding` and `claude-code` are the examples. |

## Reference workers

| Piece | Path |
|---|---|
| Two assets, the template to copy | `state/Cargo.toml`, `state/build.rs`, `state/src/ui.rs`, `state/src/boot.rs` |
| Extra script assets (vendor bundles) | `canvas/src/ui.rs`, `canvas/build.rs` |
| Custom content function id | `code-runner/src/ui.rs` |
| A worker that owns a trigger type | `cron/src/ui.rs` |

## 1. Link the crate

```toml
# <worker>/Cargo.toml
[dependencies]
iii-console-ui = { path = "../crates/console-ui" }
```

Linked by path, never published: it versions with the console worker in this
repo, not with the SDK.

## 2. Keep `ui/dist` fresh from `build.rs`

`src/ui.rs` embeds `ui/dist/page.js` and `ui/dist/styles.css` with
`include_str!`, so the build script must guarantee they exist and are current
before rustc runs. Copy `state/build.rs`. It:

- forwards `TARGET` as `env!("TARGET")` for the manifest;
- declares `rerun-if-changed` for `ui/page.tsx`, `ui/styles.css`, `ui/src`,
  `ui/build.mjs`, `ui/package.json`, `ui/tsconfig.json` and the root
  `../pnpm-lock.yaml` — never `ui/dist`, which would rebuild-loop on its own
  output;
- returns early when every dist asset is at least as new as every source
  (`dist_is_fresh`);
- otherwise runs `pnpm install` then `pnpm build` inside `ui/` and panics if
  an asset is still missing afterwards;
- honours `SKIP_UI_BUILD=1` (use the existing `dist/` as-is, panic if an
  asset is missing) and `PNPM=<path>` when pnpm is not on `PATH`.

Extend the dist asset list when the worker ships more than `page.js` and
`styles.css` (`canvas/build.rs` lists four).

## 3. Embed and register in `src/ui.rs`

```rust
use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "mywork/page.js";
pub const STYLES_PATH: &str = "mywork/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("mywork")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Call once, after the worker's regular functions are registered.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}
```

- `pub mod ui;` in `lib.rs`; `crate::ui::register(&iii)` in the boot path
  right after the functions are registered (`state/src/boot.rs`). The content
  function must exist before the triggers name it. The console may come up
  later: the engine parks the registration until a console owns the type.
- The name passed to `ConsoleUi::new` is the worker's registry name. Every
  asset path starts with it: the first segment becomes the `data-iii-ui`
  style scope and the only attribution, and the builder warns when it does
  not match.
- One more `console:script` (a vendor bundle, a second page) is one more
  `.script(path, include_str!(…))` here, one more dist asset in `build.rs`,
  and one more `entryPoints` entry in `ui/build.mjs`.
- The builder panics on a path the console would reject: empty, over 512
  characters, a leading `/`, an empty, `.` or `..` segment, characters outside
  `[a-z0-9._-]`, an extension that does not match the asset kind, or a
  duplicate. Runtime trigger failures are warn-logged, not fatal.
- Defaults derive from the worker name (`<worker>::ui-content`,
  `III_<WORKER>_UI_WATCH`, `ui/dist`) and each has a builder override
  (`.content_function_id`, `.watch_env`, `.watch_default_dir`). Only
  `code-runner` overrides one today.

## 4. Hot reload in development

With the watch env var set, the crate polls the build output once a second.
`1` or `true` means the default directory; any other value names a directory
(a `.js` file path means its parent). On change it swaps the served bytes,
registers a fresh trigger for the same path, then unregisters the previous
handle, so open tabs never see a zero-trigger window and the SDK holds one
trigger per path across reconnects.

```bash
pnpm --dir mywork/ui watch                    # esbuild --watch → ui/dist
cd mywork && III_MYWORK_UI_WATCH=1 cargo run  # poll ui/dist, re-register on change
```

`workers-dev` does both for a container whose `ui/` declares a `watch`
script: `w` on the container, or `--ui-watch` for every one
(`workers-dev/README.md`). A release binary embeds the bytes, so without the
watcher a UI change needs a `cargo build`, which `build.rs` turns into a
`pnpm build`.

## 5. Tests

The registration machinery is tested once, in the crate: path validation,
dispatch, content types, byte swapping, watch-target parsing. A worker asserts
only what it alone knows, as `state/src/ui.rs` does:

- `ui_builder_accepts_the_assets` — constructing the builder runs the path
  validation;
- `embedded_page_is_nonempty_esm` — the built script contains `export`;
- `embedded_styles_are_scoped` — the built sheet contains
  `[data-iii-ui="mywork"]` or the unquoted `[data-iii-ui=mywork]` esbuild
  prints.

Everything past the embedding — the manifest, the served bytes, the hash
moving on hot reload, rendering — is the ladder in `ade/injectable-ui` ›
Testing.

## Checklist

- `ui/` exists per `ade/injectable-ui` › Project layout and builds clean.
- `Cargo.toml` links the crate by path.
- `build.rs` is copied and its dist asset list matches `ui/dist`.
- `src/ui.rs` embeds every asset, `lib.rs` exports the module, boot calls
  `ui::register` after the functions.
- `cargo test` runs the three UI tests.
- `console::ui-manifest` lists the paths with an empty `warnings` array.
