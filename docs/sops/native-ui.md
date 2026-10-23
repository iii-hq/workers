# Native UI — shipping a worker's app to the iii app

How a worker in this repo ships a native app to the iii mobile and desktop
app: the TSX project, its build, the Rust side that registers and serves the
bundle, hot reload in development, and the tests. A native app is not the
injected console UI ([`injectable-console-ui.md`](injectable-console-ui.md));
a worker may have both.

The worker publishes a JavaScript bundle; the iii app fetches it, checks its
sha256, compiles it and runs it in a QuickJS VM on the device, and draws
what the script describes with its own native components. The script may
only call the worker's own functions and bind its own trigger types.

## What lives elsewhere

| Topic | Read |
|---|---|
| The design: delivery, the VM, the capability object, the protocol, the catalogue, the SDK | `ide/projects/worker-native-ui/design.md` (§2, §4, §5, §6, §10) |
| The platform as the iii app builds it: discovery, the VM and its limits, what the host decides (layout, sheets, menus, the Mac), `RemoteScreen`, trust | `ide/docs/worker-native-ui.md` |
| How a worker app shows up in the iii app (hub, toolbar, install) | `ide/docs/apps.md` (§2.1; the browser's app in §5.8) |
| The SDK: components, lists, the `iii` object, hooks, build, test harness, conformance files | `packages/native-ui/README.md` |
| The canonical schema (types, props, events, enums, codes, limits) | `packages/native-ui/catalog.json` (changes go through the ide copy too) |
| The crate: defaults, overrides, what `register` does | `crates/native-ui/README.md`, `crates/native-ui/src/lib.rs` |
| A small complete app | `packages/native-ui/examples/hello` |

## Reference workers

| Piece | Path |
|---|---|
| The app project (four pages, a `List` with templates, a `ListDetail`, a `RemoteScreen`) | `browser/native-ui/` |
| `build.rs` with both frontends (`ui/` and `native-ui/`) | `browser/build.rs` (the `FRONTENDS` table) |
| Registering with the crate, and its three tests | `browser/src/native_ui.rs` |
| The boot line | `browser/src/main.rs` (`browser::native_ui::register(&iii)`, after the console UI) |
| App tests: one build for the suite, a fake worker | `browser/native-ui/test/global-setup.ts`, `browser/native-ui/test/fake.ts` |

## 1. The app project

```
<worker>/native-ui/
  package.json
  tsconfig.json          the app (VM code: no DOM, no Node)
  tsconfig.test.json     the tests (Node)
  vitest.config.mjs
  build.mjs
  src/app.tsx
  test/*.test.ts
```

```json
{
  "name": "@iii-workers/<worker>-native-ui",
  "private": true,
  "type": "module",
  "scripts": {
    "build": "tsc --noEmit && node build.mjs",
    "watch": "node build.mjs --watch",
    "test": "vitest run",
    "typecheck": "tsc --noEmit -p tsconfig.json && tsc --noEmit -p tsconfig.test.json"
  },
  "dependencies": { "@iii-dev/native-ui": "workspace:*" },
  "devDependencies": {
    "@types/node": "^25.8.0",
    "solid-js": "catalog:",
    "typescript": "catalog:",
    "vitest": "catalog:"
  }
}
```

`solid-js` is a dev dependency for the types only: TypeScript resolves the
JSX types (`solid-js/jsx-runtime`) from the app's own directory, and under
pnpm's strict layout an app that does not list it fails with TS2875. The
bundle still holds the SDK's one copy (the same catalog version).

```jsonc
// tsconfig.json: the app, with exactly the VM's globals
{ "extends": "@iii-dev/native-ui/tsconfig.app.json", "include": ["src"] }
```

```jsonc
// tsconfig.test.json: the tests run in Node (ES2023, Node types)
{ "extends": "@iii-dev/native-ui/tsconfig.test.json", "include": ["test", "vitest.config.mjs"] }
```

Keep the two apart: the app's config has no Node types (`process`, `fs`
must not compile in a bundle), and the tests need them. `build` checks the
app; `typecheck` checks both. (The browser's `tsconfig.test.json` extends
`tsconfig.app.json` and repeats the test options inline; extending the SDK's
`tsconfig.test.json`, as above, is the shorter form.)

```js
// build.mjs
import { buildNativeUi } from '@iii-dev/native-ui/build'

const result = await buildNativeUi({
  entry: 'src/app.tsx',
  outfile: 'dist/native.js',
  ns: '<ns>',
  watch: process.argv.includes('--watch'),
})
if (result.sha256) console.log(`dist/native.js: ${result.bytes} bytes, sha256 ${result.sha256}`)
```

Add `- <worker>/native-ui` to `pnpm-workspace.yaml` and run `pnpm install`
at the repo root (the lockfile is the root one).

`<ns>` is the prefix of the worker's functions (`browser` in
`browser::navigate`); it is the app's identity in the iii app
(`worker:<ns>`) and the only namespace the script may call. The test
harness and the hosts refuse anything else.

Rules the SDK and the app enforce, so they are worth knowing before the
first line:

- `src/app.tsx` calls `defineApp({ pages: { root, … } })` once, at the top
  level. `root` is the page the app opens; the others are pushed with
  `iii.nav.push`.
- The script decides **what** appears; the host decides **how**: no layout
  per device. The only adaptive input is `iii.env.windowClass`
  (`compact`/`wide`), for choices like where a toolbar sits.
- The VM has no DOM, network, `URL`, `TextEncoder`, `Intl` or `eval`.
  Everything outside the tree goes through `iii` (`call`, `on`, `nav`, `ui`,
  `storage`, `clipboard`, `openUrl`, `share`, `saveFile`).
- Long lists are `List` with `createList` (items on the host, templates
  bound to item fields), never a `For` of a thousand rows.
- Compute the strings a row shows into the item: the host formats nothing.
- JSX is typed for the VM: a component returns `JSX.Element`, a plain
  object is not one, and `<Show when={tab()}>{(t) => …}</Show>` types `t`.
  `Accessor`, `Setter`, `Component`, `ParentProps` and the other Solid types,
  and `createStore` from `solid-js/store`, come from `@iii-dev/native-ui`.
- Stop what costs only while visible when `iii.env.visible` turns false
  (`useTrigger` takes `enabled`).
- Frames, video and anything streaming never go through the script: a
  high-level component (`RemoteScreen`) takes them natively.

### Pitfalls

Each of these has broken an app in review; write the test for it with the
harness:

- **Never run a regex built from user text inside the VM.** Catastrophic
  backtracking passes the 250 ms step deadline and the app stops for good
  (every page shows "stopped responding"). Filter with a substring in the
  script; send a regex only to the worker.
- **Check worker data entry by entry.** Skip a bad entry, don't fail the
  list: one session with a numeric title must not blank the whole tab list.
- **Calls fail and hang.** Always pass `timeoutMs` or rely on the 30 s
  default (the host and the harness both enforce it), and make the UI state
  recover when the call rejects. Test it: a `calls` handler that never
  settles, then `host.tick(30_000)`.
- **Gate what reaches navigation-like calls.** One function decides what
  may be opened, and every path to the worker goes through it: the browser's
  `webUrl` (`browser/native-ui/src/address.ts`) admits only well-formed
  http(s) URLs, from the address bar, Retry and history alike.
- **Unique List keys.** Pass worker data that may repeat a key through
  `lastPerKey(items, key)` before `createList().set`.

## 2. Build: limits and smoke run

`pnpm --dir <worker>/native-ui build` writes `dist/native.js` (git-ignored):
one minified IIFE bundle. The build fails above 256 KiB, warns above
128 KiB, and runs the bundle in the test harness first (it must send `ready`
with a `root` page and mount `root` without an error); a bundle that fails
is not written. The app refuses bundles above 1 MiB.

## 3. Keep `native-ui/dist` fresh from `build.rs`

The Rust worker embeds the bundle with `include_str!`, so its `build.rs`
must guarantee `native-ui/dist/native.js` exists and is current, exactly as
it does for `ui/dist`. Copy `browser/build.rs`: one `Frontend` entry per
embedded project (its directory, its outputs, the files and directories
that make them stale), the same loop for both.

- `rerun-if-changed` for `native-ui/src`, `native-ui/build.mjs`,
  `native-ui/package.json`, `native-ui/tsconfig.json`, the root lockfile
  (`native-ui/../../pnpm-lock.yaml`) **and the SDK the bundle includes**
  (`native-ui/../../packages/native-ui/src`,
  `native-ui/../../packages/native-ui/catalog.json`: unlike the console
  UI's import map, the SDK is bundled in); plus anything else the app
  imports from outside (`browser` imports `ui/src/lib/address.ts`); never
  `native-ui/dist`;
- return early when the bundle is at least as new as every source;
- otherwise `pnpm install` then `pnpm build` inside `native-ui/`, and panic
  if `dist/native.js` is still missing;
- honour `SKIP_UI_BUILD` (any value: use the existing `dist/` as-is, and
  panic if it is missing) and `PNPM=<path>`, the same switches as the
  console UI.

The CI finds `native-ui/` like `web/` and `ui/`: the `rust` job and the
binary build set up pnpm and Node and pre-build it, also for the projects of
a worker's path dependencies (`.github/workflows/ci.yml`). It does not run
the SDK's own tests (below).

## 4. Register with the crate

```toml
# <worker>/Cargo.toml
[dependencies]
iii-native-ui = { path = "../crates/native-ui" }
```

```rust
// <worker>/src/native_ui.rs
use std::sync::Arc;

use iii_native_ui::{NativeUi, NativeUiHandle};
use iii_sdk::IIIClient;

pub const BUNDLE_PATH: &str = "<ns>/native.js";

/// Built by `build.rs` (`buildNativeUi` over `native-ui/`).
const BUNDLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/native-ui/dist/native.js"));

pub fn native_ui() -> NativeUi {
    NativeUi::new("<ns>")
        .app("<Title>", "<lucide-icon>", "<One line for the hub, up to 120 characters.>")
        .bundle(BUNDLE_PATH, BUNDLE)
}

/// Call once, after the worker's own functions are registered.
pub fn register(iii: &Arc<IIIClient>) -> NativeUiHandle {
    native_ui().register(iii)
}
```

- `register` registers `<ns>::native-ui`, internal, with the descriptor in
  its metadata (`native_ui: {v, app, bundle: {path, hash, size}, catalog,
  dev}`); its handler serves `{path}` → `{content, hash, size}` and answers
  any other path with `UNKNOWN_NATIVE_UI_ASSET`. Keep the returned handle
  (dropping it does not unregister; `handle.unregister()` does).
- The builder panics on what the app would refuse: the namespace pattern
  `^[a-z0-9][a-z0-9_-]{0,63}$`; a title of 1–40 characters and a blurb of
  1–120, not only whitespace; a kebab-case Lucide icon of at most 64 bytes
  (outside the app's subset it draws a neutral icon); a path of 1–512 bytes
  in `[a-z0-9._/-]`, under `<ns>/`, ending in `.js`, without a leading `/`
  or an empty, `.` or `..` segment; a bundle that is not empty and at most
  1 MiB; `catalog(0)`; `metadata()` or `register` without `.app` and
  `.bundle`.
- `register` needs a tokio runtime when the watcher is on (it spawns the
  poller). The handle also gives `function_id()` and `hash()` (the served
  bundle's, which the watcher changes).
- Bump `.catalog(n)` only when the bundle uses a catalogue version the
  oldest app you support lacks; that app then shows "Needs a newer iii app".

### Workers in TypeScript or Python

No helper: it is one function (design §2.6). Hash the UTF-8 bytes of the
bundle with sha256 (lower-case hex), register `<ns>::native-ui` with
`metadata: { internal: true, native_ui: { v: 1, app: { title, icon, blurb },
bundle: { path, hash, size }, catalog: 1, dev: false } }`, answer
`{ path }` with `{ content, hash, size }` and throw
`UNKNOWN_NATIVE_UI_ASSET: <path>` for any other path. Build the bundle with
the same `@iii-dev/native-ui/build`.

## 5. Hot reload in development

```bash
pnpm --dir <worker>/native-ui watch                     # rebuild dist/native.js on every change
cd <worker> && III_<NS>_NATIVE_UI_WATCH=1 cargo run     # serve each new bundle
```

With the env var set, the worker checks the bundle every 250 ms (a `stat`;
it reads and hashes the file only when its size or mtime changed, so an
idle loop costs nothing): `1` or
`true` watch `native-ui/dist`; a path ending in `.js` watches its directory;
any other value is a directory, relative to the worker's working directory;
unset, empty, `0` or `false` turn it off. The first registration is always
`dev: false`; it skips a missing, empty, unchanged or over-1-MiB file. On a new bundle it swaps the served
bytes and registers the function again with `dev: true` and the new hash;
while one of the app's pages is visible, the iii app asks for the
descriptor every 1.5 s and reloads the app when the hash changes (the
pages' state starts over, the deck stays). The build writes the bundle
atomically and only after its smoke run, so the poller never serves a
broken file (`pnpm watch` skips the `tsc` of `pnpm build`: run the
typecheck before you commit). `<NS>` is the namespace upper-cased, every
character outside `[A-Z0-9]` as `_` (`III_IOS_SIMULATOR_NATIVE_UI_WATCH`).
`workers-dev` has no switch for it yet (unlike the console UI's
`--ui-watch`): set the variable yourself.

## 6. Tests

- **The app**, with the harness (`@iii-dev/native-ui/testing`): build the
  bundle once for the whole suite (the browser does it in a vitest
  `globalSetup`, `test/global-setup.ts`, and hands it to the tests with
  `provide`/`inject`; a `beforeAll` per file works for a small app), then
  `createTestHost` with `calls` answering the worker functions the app uses
  (the browser keeps them in a fake worker, `test/fake.ts`), and drive it
  like a person (`press`, `type`, `itemPress`, `emitTrigger`, `tick`,
  `env`). Assert on `tree()`, `requests`, `toasts`, `menus`. The harness
  fails the test on any protocol violation, so a passing test also means
  the host will accept the ops. See `packages/native-ui/test/*.test.ts`,
  `examples/hello` and `browser/native-ui/test/`.
- **The worker**, in `src/native_ui.rs`: what only it knows, as the
  browser's three tests do. `builder_accepts_the_app`: constructing
  `native_ui()` runs the builder's validation.
  `embedded_bundle_is_a_native_ui_app`: the embedded bundle is not empty,
  under 1 MiB, and defines `__vm_receive`.
  `descriptor_carries_the_app_and_the_bundle_hash`:
  `native_ui().metadata()["native_ui"]["bundle"]["hash"]` equals
  `iii_native_ui::sha256_hex(BUNDLE)`. The registration machinery itself is
  tested once, in the crate.
- **The SDK** has no CI job yet: after a change in `packages/native-ui`, run
  its tests and the tests of the apps that bundle it by hand
  (`packages/native-ui/README.md` › Working on the package).
- **On a device**: the iii app lists the worker's app in the environment's
  hub once the worker runs; a bundle with a fatal error shows the app's
  failure screen with Restart and the message in "Copy details".

## Checklist

- `<worker>/native-ui/` builds clean (`pnpm build`: types, size, smoke run),
  `pnpm typecheck` passes for the app and the tests, and it is listed in
  `pnpm-workspace.yaml`.
- The app's tests run in the harness with `calls` for every function it uses.
- `Cargo.toml` links `iii-native-ui` by path; `build.rs` builds
  `native-ui/`, reruns on the SDK's sources too, and honours
  `SKIP_UI_BUILD`.
- `src/native_ui.rs` embeds `native-ui/dist/native.js`; boot calls
  `native_ui::register` after the functions; its tests pass.
- With `III_<NS>_NATIVE_UI_WATCH=1`, a rebuild reaches an open page of the
  app within a few seconds.
