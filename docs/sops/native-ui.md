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
| The SDK: components, lists, the `iii` object, hooks, build, test harness, conformance files | `packages/native-ui/README.md` |
| The canonical schema (types, props, events, enums, codes, limits) | `packages/native-ui/catalog.json` (changes go through the ide copy too) |
| The crate: defaults, overrides, what `register` does | `crates/native-ui/README.md`, `crates/native-ui/src/lib.rs` |
| A small complete app | `packages/native-ui/examples/hello` |

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
app; `typecheck` checks both.

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

## 2. Build: limits and smoke run

`pnpm --dir <worker>/native-ui build` writes `dist/native.js` (git-ignored):
one minified IIFE bundle. The build fails above 256 KiB, warns above
128 KiB, and runs the bundle in the test harness first (it must send `ready`
with a `root` page and mount `root` without an error); a bundle that fails
is not written. The app refuses bundles above 1 MiB.

## 3. Keep `native-ui/dist` fresh from `build.rs`

The Rust worker embeds the bundle with `include_str!`, so its `build.rs`
must guarantee `native-ui/dist/native.js` exists and is current, exactly as
it does for `ui/dist` (copy the pattern of `state/build.rs`):

- `rerun-if-changed` for `native-ui/src`, `native-ui/build.mjs`,
  `native-ui/package.json`, `native-ui/tsconfig.json` and the root
  `../pnpm-lock.yaml`; never `native-ui/dist`;
- return early when the bundle is at least as new as every source;
- otherwise `pnpm install` then `pnpm build` inside `native-ui/`, and panic
  if `dist/native.js` is still missing;
- honour `SKIP_UI_BUILD=1` (use the existing `dist/` as-is) and `PNPM=<path>`,
  the same switches as the console UI.

The CI `rust` job pre-builds `web/` and `ui/` projects only; a worker whose
`build.rs` builds `native-ui/` needs pnpm and Node there too (the job sets
them up when a `ui/package.json` exists).

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
  `^[a-z0-9][a-z0-9_-]{0,63}$`, a title of 1–40 characters, a blurb of
  1–120, a kebab-case Lucide icon (outside the app's subset it draws a
  neutral icon), a path under `<ns>/` ending in `.js`, a bundle up to 1 MiB.
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

With the env var set (`1`/`true` = `native-ui/dist`, or a directory), the
worker polls the bundle every second. On a new bundle it swaps the served
bytes and registers the function again with `dev: true` and the new hash;
while one of the app's pages is visible, the iii app asks for the
descriptor every 1.5 s and reloads the app when the hash changes (the
pages' state starts over, the deck stays). The build writes the bundle
atomically and only after its smoke run, so the poller never serves a
broken file. `<NS>` is the namespace upper-cased, `-` as `_`
(`III_IOS_SIMULATOR_NATIVE_UI_WATCH`).

## 6. Tests

- **The app**, with the harness (`@iii-dev/native-ui/testing`): build the
  bundle once (`buildNativeUi` in `beforeAll`), then `createTestHost` with
  `calls` answering the worker functions the app uses, and drive it like a
  person (`press`, `type`, `itemPress`, `emitTrigger`, `tick`, `env`).
  Assert on `tree()`, `requests`, `toasts`, `menus`. The harness fails the
  test on any protocol violation, so a passing test also means the host
  will accept the ops. See `packages/native-ui/test/*.test.ts` and
  `examples/hello`.
- **The worker**, in `src/native_ui.rs`: what only it knows. Constructing
  `native_ui()` runs the builder's validation; the embedded bundle is
  non-empty; `native_ui().metadata()["native_ui"]["bundle"]["hash"]` equals
  `iii_native_ui::sha256_hex(BUNDLE)`. The registration machinery itself is
  tested once, in the crate.
- **On a device**: the iii app lists the worker's app in the environment's
  hub once the worker runs; a bundle with a fatal error shows the app's
  failure screen with Restart and the message in "Copy details".

## Checklist

- `<worker>/native-ui/` builds clean (`pnpm build`: types, size, smoke run),
  `pnpm typecheck` passes for the app and the tests, and it is listed in
  `pnpm-workspace.yaml`.
- The app's tests run in the harness with `calls` for every function it uses.
- `Cargo.toml` links `iii-native-ui` by path; `build.rs` builds
  `native-ui/` and honours `SKIP_UI_BUILD`.
- `src/native_ui.rs` embeds `native-ui/dist/native.js`; boot calls
  `native_ui::register` after the functions; its tests pass.
- With `III_<NS>_NATIVE_UI_WATCH=1`, a rebuild reaches an open page of the
  app within a few seconds.
