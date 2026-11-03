# iii-console-ui

The worker-side half of the injectable console UI, as a crate: one builder
that registers the *content function* (`<worker>::ui-content`), one observer
of the console's `console::ui::asset` function per asset, and the dev-loop
hot-reload watcher — the whole registration discipline from the authoring SOP
(`workers/docs/sops/injectable-console-ui.md`), implemented once.

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

`.script(path, content)` adds a `script` (an ES module every console tab
imports and calls `setup(host)` on), `.style(path, content)` a `style` (a
stylesheet scoped under `[data-iii-ui="<worker>"]`), and
`.module(path, content)` a `module`: a `.js` asset the console serves but
never imports at mount, loaded by a script through `host.importModule(path)`
(the ide's xterm terminal).

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
- observes `console::ui::asset` once per asset, with the content function
  as the target: modules and styles at once, and each script once every
  module observer is active or has failed, so the console commits the
  modules a script imports before the script (whether `register` runs before
  or after the connection is up, and whether the console registers before or
  after the worker; after a reconnect the SDK replays the observers in its
  own order):

  ```rust
  iii.observe(
      Source::function("console::ui::asset")
          .args(json!({ "kind": "script", "path": "state/page.js" })),
      "state::ui-content",
  );
  ```

  The console accepts the observer, then calls the content function through
  it with `{ path }` and serves the result at `/ui/<path>`. Injected UI dies
  and revives with its worker: the engine disposes the observers once the
  worker stays disconnected, and the SDK replays them by id on reconnect.
  An observer stays `pending` while the console is not running. A rejection
  (`UI_ASSET_FETCH_FAILED`, an invalid `kind` or `path`) is warn-logged, not
  fatal;
- when the watch env var is set, polls the build output (1 s) and on change
  swaps the served bytes, observes the same path afresh, *then* disposes the
  previous observer — observe-first avoids a flash without the asset in tabs
  (the console supersedes the old observer with `UI_ASSET_SUPERSEDED`), the
  trailing dispose keeps the SDK's replay map at one observer per path.

The builder panics on paths the console would reject (wrong extension,
uppercase, `..` segments, duplicates) — an authoring error should fail the
first unit test, not warn-log on a running engine.

Reference adopter: `workers/state/src/ui.rs`. The Rust wiring SOP:
`workers/docs/sops/injectable-console-ui.md`. Authoring the UI itself (assets,
builds, styling, slots): `workers/ade/skills/injectable-ui.md`.
