# iii-console-ui

The worker-side half of the injectable console UI, as a crate: one builder
that registers the *content function* (`<worker>::ui-content`), one
`console:script` / `console:style` trigger per asset, and the dev-loop
hot-reload watcher — the whole registration discipline from the authoring
SOP (`workers/docs/sops/injectable-console-ui.md`), implemented once.

**Linked by path, never published.** The crate versions with the console
worker in this repo; a worker adopts it with a direct link:

```toml
# <worker>/Cargo.toml
[dependencies]
iii-console-ui = { path = "../crates/console-ui" }
```

## Usage

```rust
use iii_console_ui::ConsoleUi;

pub fn register(iii: &Arc<IIIClient>) {
    ConsoleUi::new("state")
        .script("state/page.js",
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js")))
        .style("state/styles.css",
               include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css")))
        .register(iii);
}
```

That's the entire worker side. Defaults derived from the worker name (each
has a builder override):

| Default | Value | Override |
|---|---|---|
| Content function id | `<worker>::ui-content` | `.content_function_id(…)` |
| Hot-reload env var | `III_<WORKER>_UI_WATCH` | `.watch_env(…)` |
| Watch dir when the env var is `1`/`true` | `ui/dist` | `.watch_default_dir(…)` |

What `register` does:

- registers the content function (`{path}` → `{content, content_type}`,
  MIME derived from the asset kind), flagged `internal: true` so it stays
  console plumbing rather than discoverable API;
- registers one trigger per asset over the SDK **Message path** — injected
  UI dies and revives with its worker (disconnect GC + reconnect replay),
  which is the design; trigger failures are warn-logged, not fatal;
- when the watch env var is set, polls the build output (1 s) and on change
  swaps the served bytes, registers a fresh trigger for the same path,
  *then* unregisters the previous handle — register-first avoids a
  zero-trigger flash in tabs, the trailing unregister keeps the SDK's
  replay map at one entry per path.

The builder panics on paths the console would reject (wrong extension,
uppercase, `..` segments, duplicates) — an authoring error should fail the
first unit test, not warn-log on a running engine.

Reference adopter: `workers/state/src/ui.rs`. Full authoring guide (assets,
builds, styling, slots): `workers/docs/sops/injectable-console-ui.md`.
