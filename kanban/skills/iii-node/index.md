---
name: iii-node
description: >-
  Build and maintain portable TypeScript/Node.js iii workers using a
  single-package backend plus injectable Console UI structure, configuration
  integration, coordinated development watchers, and worker-compose wiring.
---

# iii-node

Use this skill to create or restructure a Node.js/TypeScript iii worker, especially when the worker ships an injectable UI into the iii Console. This skill is self-contained and may be copied into a different project; do not assume any example worker, monorepo, sibling package, or repository-specific path exists.

## Required references

Read the relevant references before implementing:

- [`configuration.md`](./configuration.md) — bundled beside this file: schema-validated configuration registration, reads, updates, and reactive triggers.
- The UI half lives in the `ade-worker-design` skill (`directory::skills::get { "id": "kanban/ade-worker-design/<name>" }`) and belongs to the iii ADE Worker Designer profile:
  - `console-injectable-ui` — the complete injectable UI contract, host APIs, asset registration, hot reload, responsiveness, and validation requirements.
  - `console-design` — the Console visual system, component grammar, tokens, typography, spacing, and interaction rules.
  - `patterns` — concrete recipes for record-shaped UIs: boards with lanes and drag-and-drop, a record screen that opens as its own pane, activity timelines with threaded comments, creation modals, chat cards for agent calls, settings forms, and live updates.

This file still owns the worker-side half of an injectable UI — the build script, the asset content function and its triggers, the dev watchers — because those ship inside the worker package. The pages, renderers, forms and styles themselves are the designer's work; when a ticket needs them, hand them to `ade-worker-designer` rather than improvising markup here.

### Precedence for this Node scaffold

This file is the source of truth for the **single-package Node layout, npm dependency, build outputs, worker-side UI delivery, development process, and Compose block**. Use the references above as the source of truth for the **current host API, UI components, accessibility, responsive behavior, configuration semantics, and visual design**.

The injectable UI reference may describe repository-internal `workspace:*` dependencies, local `file:` dependencies, a root workspace file, or a separate `<worker>/ui/package.json`. Those instructions do **not** apply to this portable Node scaffold. Use one package at the worker root and consume the public npm package:

```json
"@iii-dev/console-ui": "0.1.0"
```

Do not use `file:`, `link:`, or `workspace:*` for this dependency.

Any repository path mentioned by a reference—such as `packages/console-ui`, `console/`, `database/`, `state/`, `iii-directory/`, or `app/`—is an optional upstream example, not a required destination-project file. If such a path is absent, do not search for it, recreate its surrounding monorepo, or block implementation on it. Use the self-contained templates in this file, the installed `@iii-dev/console-ui` public types, and the runtime contracts available in the destination project.

## Choose project identifiers once

Before creating files, resolve these placeholders and use them consistently:

| Placeholder | Meaning | Example form |
|---|---|---|
| `<worker-name>` | Stable lowercase worker identity used by iii and asset paths | `issue-board` |
| `<worker-directory>` | Directory containing the worker package, relative to `worker-compose.yaml` | `./workers/issue-board` |
| `<worker-title>` | Human-readable title in natural casing | `Issue board` |
| `<worker-description>` | One-line capability description | `Tracks project issues and activity.` |
| `<configuration-id>` | Stable configuration form family; normally `<worker-name>` | `issue-board` |
| `<page-id>` | Globally distinct Console extension page id | `issue-board-manager` |
| `<env-prefix>` | Upper-snake-case environment prefix derived from the worker name | `ISSUE_BOARD` |

Do not copy an example name into generated code. Replace every angle-bracket placeholder. Keep `<worker-name>` consistent across:

- `registerWorker({ workerName })`;
- function ids such as `<worker-name>::resource::action`;
- UI content function id `<worker-name>::ui-content`;
- injectable asset paths `<worker-name>/page.js` and `<worker-name>/styles.css`;
- CSS scope `[data-iii-ui="<worker-name>"]`;
- configuration id unless the domain requires a separate stable form family;
- `iii.worker.yaml` name;
- the `worker-compose.yaml` container key when practical.

Use lowercase `[a-z0-9._-]` path segments for injectable assets. Never derive identifiers from a display title at runtime.

## Canonical project shape

Keep backend and UI in one Node package:

```text
<worker-directory>/
  package.json
  pnpm-lock.yaml
  tsconfig.json
  iii.worker.yaml
  scripts/
    dev.mjs
  src/
    index.ts
    ...domain modules
  test/
    ...Node tests
  ui/
    build.mjs
    tsconfig.json
    page.tsx
    styles.css
    ...renderers, parsers, hooks, and widgets
  dist/                         # generated; do not hand-edit
    index.js                    # compiled backend
    ...compiled backend modules
    ui/
      page.js                   # injectable ESM bundle
      styles.css                # injectable scoped CSS
```

This is intentionally **not** a second UI package. The root `package.json` owns TypeScript, React types, esbuild, and `@iii-dev/console-ui`. Backend TypeScript compiles to `dist/`; esbuild writes UI assets to `dist/ui/`.

Keep domain logic, validation, persistence, and iii registrations under `src/`. Keep Console-only parsing, rendering, hooks, and scoped styles under `ui/`. The UI invokes backend functions through `host.iii`; it does not import backend implementation modules or access backend files directly.

## Root `package.json`

Use this shape and adapt only versions the destination project deliberately controls:

```json
{
  "name": "<worker-name>",
  "version": "0.1.0",
  "private": true,
  "description": "<worker-description>",
  "type": "module",
  "engines": {
    "node": ">=22"
  },
  "packageManager": "pnpm@10.18.2",
  "scripts": {
    "build": "tsc -p tsconfig.json && pnpm run build:ui",
    "build:ui": "tsc -p ui/tsconfig.json --noEmit && node ui/build.mjs",
    "typecheck": "tsc -p tsconfig.json --noEmit && tsc -p ui/tsconfig.json --noEmit",
    "test": "tsx --test test/*.test.ts",
    "start": "node dist/index.js",
    "dev": "node scripts/dev.mjs"
  },
  "dependencies": {
    "iii-sdk": "<project-approved-version>"
  },
  "devDependencies": {
    "@iii-dev/console-ui": "0.1.0",
    "@types/node": "^22.10.0",
    "@types/react": "^19.2.14",
    "esbuild": "^0.25.0",
    "tsx": "^4.19.0",
    "typescript": "^5.9.2"
  }
}
```

Pin `@iii-dev/console-ui` to exactly `0.1.0` unless the user explicitly requests another published version. Run `pnpm install` after writing or changing the package file. Verify the lockfile resolves it from npm and contains no `file:`, `link:`, or workspace resolution for that package.

pnpm 10+ refuses to run dependency build scripts until they are approved, and pnpm 11 re-checks that before every `pnpm run` — so `pnpm test` and `pnpm build` fail with `ERR_PNPM_IGNORED_BUILDS` even though `pnpm install` “succeeded”. Approve esbuild declaratively instead of running the interactive `pnpm approve-builds`: create `pnpm-workspace.yaml` beside `package.json` with

```yaml
allowBuilds:
  esbuild: true
```

(add whatever else the install reports as ignored, with `false` for packages that do not need scripts) and run `pnpm install` again.

Do not guess the `iii-sdk` version. Preserve the destination project's compatible version when one exists; otherwise select a published version intentionally and validate it against the current SDK reference.

## TypeScript configuration

Create a backend `tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2023",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "types": ["node"]
  },
  "include": ["src/**/*.ts"]
}
```

Create `ui/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "react-jsx",
    "strict": true,
    "skipLibCheck": true,
    "noEmit": true,
    "types": ["react"]
  },
  "include": ["./**/*.ts", "./**/*.tsx"]
}
```

Use explicit `.js` extensions in relative imports written in backend TypeScript when required by `NodeNext`, even though the source file itself ends in `.ts`.

## Node SDK rules

Before writing or changing worker SDK code, read the current Node SDK reference at <https://iii.dev/docs/reference/sdk-node.md>. Do not write SDK calls from memory.

Import the factory and call registration methods on the returned client:

```ts
import { registerWorker } from 'iii-sdk'

const iii = registerWorker(
  process.env.III_ENGINE_URL ?? process.env.III_URL,
  {
    workerName: '<worker-name>',
    workerDescription: '<worker-description>',
    invocationTimeoutMs: 30_000,
  },
)
```

If passing `undefined` as the address is incompatible with the installed SDK types, omit the first argument and let `III_URL` resolve according to the current SDK reference. Do not invent connection setup beyond the documented API.

Import only actual top-level SDK exports such as `registerWorker` and, when needed, `TriggerAction` or documented types. Do not import or destructure `registerFunction`, `registerTrigger`, or other client methods as top-level exports. Call `iii.registerFunction(...)`, `iii.registerTrigger(...)`, and `iii.registerTriggerType(...)`. The `IIIClient` type comes from `iii-sdk`; `TriggerConfig`/`TriggerHandler` come from `iii-sdk/trigger`.

### Namespaces

A Compose project usually runs its workers in a project namespace (`III_NAMESPACE` is set by Compose); the engine-hosted workers (`configuration`, `state`, `engine::*`) live in `default`. Consequences:

- Calls to engine-hosted functions from the worker need `namespace: 'default'` on `iii.trigger`; calls to the worker's own functions and other project workers omit it.
- `iii.registerTrigger` for an engine-provided type (`configuration`) and for another project worker's type (`console:script`) both resolve without a `trigger_namespace`; do not set one.
- A trigger type the worker registers itself lands in the worker's namespace; console tabs and the harness bind to it without extra configuration.
- The harness and the console reach the worker's functions by bare id; never prefix ids with a namespace.

Every public function must provide:

- a stable namespaced id such as `<worker-name>::resource::action`;
- a concise description;
- `request_format` and `response_format` JSON Schemas;
- a handler whose input/output actually matches those schemas.

A small helper can keep registrations consistent:

```ts
function registerFunction<TInput, TOutput>(
  id: string,
  description: string,
  requestFormat: Record<string, unknown>,
  responseFormat: Record<string, unknown>,
  handler: (payload: TInput) => Promise<TOutput>,
) {
  return iii.registerFunction(id, handler, {
    description,
    request_format: requestFormat,
    response_format: responseFormat,
  })
}
```

Handle shutdown cleanly:

```ts
const shutdown = async () => {
  await iii.shutdown()
  process.exit(0)
}

process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)
```

Keep data validation and domain logic in backend modules rather than duplicating it in the injected UI.

## Configuration

Follow [`configuration.md`](./configuration.md). For a configurable worker:

1. Choose a stable `<configuration-id>`, normally `<worker-name>`.
2. Read any existing value before registration when preserving operator data matters.
3. Call `configuration::register` at startup with an id, name, description, JSON Schema, and an `initial_value` only when no value exists.
4. Read and validate the effective value before constructing resources that depend on it.
5. Register an internal reload function with accurate request/response schemas and `metadata: { internal: true }` when appropriate.
6. Bind that function with an SDK Message-path trigger of type `configuration`, filtered to the configuration id and relevant event types.
7. In the UI, register a purpose-built form with `host.configForms.register(...)`; never fall back to a raw JSON textarea.
8. Set `configurationId` on the page registration so the Console exposes the standard settings action.

A worker-to-worker call still routes through iii:

```ts
async function configurationCall<T>(
  functionId: string,
  payload: Record<string, unknown>,
): Promise<T> {
  return iii.trigger({
    function_id: functionId,
    namespace: 'default',
    payload,
    timeoutMs: 10_000,
  }) as Promise<T>
}
```

Use an explicit namespace only when required by the destination deployment (see Namespaces above; `default` is required for `configuration::*` when the project runs under a Compose namespace). Copy the exact trigger config from the live `configuration` trigger contract rather than assuming it. Preserve unknown configuration fields and `${ENV:default}` templates as required by the bundled references.

Contract details the live registry does not advertise:

- `configuration::register` is hidden from `engine::functions::list` and `engine::functions::info` reports it as not available; it still exists (see `engine::workers::info { name: 'configuration' }`). Its payload is `{ id, name, description, schema, initial_value?, metadata: { ui_form: '<configuration-id>' } }`. Re-registering with `initial_value` **replaces** the stored value, so pass it only when the read below returned nothing.
- `configuration::get { id }` throws when the id is not registered yet; treat that error as “no value” during startup, register, then read again (with defaults expanded) to obtain the effective value.
- The persisted file is `<config dir>/<id>.yaml` with `id`, `name`, `description`, `metadata`, and `value`.
- The `configuration` trigger delivers `{ id, event_type, name, description, schema, old_value, new_value, type: 'configuration' }`; filter with `config: { configuration_id, event_types: ['configuration:updated'] }`.
- Relative paths in configuration should resolve from a stable root. Resolve them against the project root (the nearest ancestor of the worker directory that contains `worker-compose.yaml`, overridable by an env var), not the process cwd, and expose the resolved path through a small `<worker-name>::config::info` function so the settings form can show it.

## Live updates (own trigger type)

When other tabs, agents, or workers must react to changes without polling, the worker provides its own trigger type and fans events out to every binding:

```ts
import { TriggerAction, type IIIClient } from 'iii-sdk'
import type { TriggerConfig } from 'iii-sdk/trigger'

type ChangeConfig = { record_id?: string; events?: string[]; metadata?: unknown }
const subscribers = new Map<string, TriggerConfig<ChangeConfig>>()

/** A subscription's metadata: declared in the config, or on the binding itself. */
function subscriptionMetadata(binding: TriggerConfig<ChangeConfig>): unknown {
  return binding.config.metadata ?? binding.metadata
}

iii.registerTriggerType<ChangeConfig>(
  {
    id: '<worker-name>:change',
    description:
      'Fires after every mutation. Config: { record_id?, events?, metadata? } — metadata rides along to the invoked handler.',
  },
  {
    registerTrigger: async (binding) => {
      subscribers.set(binding.id, {
        ...binding,
        config: { ...binding.config, metadata: subscriptionMetadata(binding) },
      })
    },
    unregisterTrigger: async (binding) => { subscribers.delete(binding.id) },
  },
)

function emit(event: Record<string, unknown>) {
  for (const binding of subscribers.values()) {
    if (!matches(binding.config, event)) continue
    const metadata = subscriptionMetadata(binding)
    iii.trigger({
      function_id: binding.function_id,
      namespace: binding.namespace, // pass the binding's namespace through
      payload: event,
      ...(metadata === undefined ? {} : { metadata }), // never drop the subscriber's metadata
      action: TriggerAction.Void(),  // fire-and-forget; a closed tab must not block the loop
    }).catch(() => undefined)
  }
}
```

Emit from the store after each persisted mutation and include the **whole record** in the payload so consumers can upsert without a round trip. The UI side of this contract is in the designer's `console-injectable-ui` (`host.iii` → live data) and `patterns` §8.

**Every trigger type the worker provides must carry trigger metadata.** A registration's `metadata` arrives at the provider as `binding.metadata` (`TriggerConfig.metadata`) and must come back out on every `iii.trigger` as `metadata`; the bound handler receives it as its **second argument**, a channel separate from the payload, and a fan-out that omits it silently drops it. Because the top-level `metadata` slot also carries the harness's own control fields (`{ payload, event_into }`) for call-to-function bindings, also accept a `metadata` field inside the trigger's config, name it in the trigger type's `description`, and forward whichever the subscriber set (config first). Never merge metadata into the payload.

## Injectable UI builder

Create `ui/build.mjs`:

```js
import esbuild from 'esbuild'

const options = {
  entryPoints: ['ui/page.tsx', 'ui/styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist/ui',
  external: [
    'react',
    'react-dom',
    'react-dom/client',
    'react/jsx-runtime',
    '@iii-dev/console-ui',
  ],
  logLevel: 'info',
}

if (process.argv.includes('--watch')) {
  const context = await esbuild.context(options)
  await context.watch()
  console.log('[ui] watching TSX and CSS files')
} else {
  await esbuild.build(options)
}
```

All five externals are required. Bundling React creates a second React instance and causes invalid hook calls. Bundling `@iii-dev/console-ui` bypasses the Console's runtime contract. Do not bundle an editor; use the Console's shared editor components.

## Injectable UI entrypoint

Structure `ui/page.tsx` as ordinary React that default-exports `setup(host)`. Read the package's public types before using components; never guess an export or prop. In this portable layout the types are at `node_modules/@iii-dev/console-ui/index.d.ts` (read it in full after `pnpm install`; `README.md` beside it documents `host.panels.open` and the chat integrations) — the `packages/console-ui/...` path some references mention does not exist here.

The package exports no icon components and injected bundles must not add an icon dependency: author the few 16 px glyphs the page needs as inline SVG components (Lucide path data, `viewBox="0 0 24 24"`, `width`/`height` 16, `stroke="currentColor"`, `strokeWidth` 2, `aria-hidden`), and pass component types (not elements) where a prop such as `EmptyState.icon` asks for one.

```tsx
import {
  PageHeader,
  PageMain,
  PageShell,
  type Host,
  type PageRenderProps,
} from '@iii-dev/console-ui'

function WorkerPage({
  host,
  onRequestClose,
}: PageRenderProps & { host: Host }) {
  return (
    <PageShell className="<worker-name>-ui-shell">
      <PageHeader
        title="<worker-title>"
        description="<worker-description>"
        onClose={onRequestClose}
      />
      <PageMain className="<worker-name>-ui-main">
        {/* Compose the domain-specific page here. */}
      </PageMain>
    </PageShell>
  )
}

export default function setup(host: Host) {
  host.pages.register({
    id: '<page-id>',
    title: '<worker-title>',
    configurationId: '<configuration-id>',
    render: (props) => <WorkerPage host={host} {...props} />,
  })

  // Register only surfaces that are implemented.
  // host.configForms.register('<configuration-id>', WorkerConfigForm)
  // host.functionTriggers.register(createFunctionRenderer())
  // host.triggerRenderers?.register(createTriggerRenderer())
}
```

If the worker has no configuration, omit both `configurationId` and the config form. If it has configuration, implement and register the form; do not advertise settings without a corresponding interface.

The page body is the designer's work: `kanban/ade-worker-design/console-injectable-ui` covers narrow panes, loading/error/empty states, renderer fallthrough, redaction, dirty state, live triggers, accessibility, and real-Console testing; `kanban/ade-worker-design/console-design` covers visual decisions.

Every selector in `ui/styles.css` must be scoped under `[data-iii-ui="<worker-name>"]`:

```css
[data-iii-ui="<worker-name>"] .<worker-name>-ui-main {
  min-width: 0;
  min-height: 0;
  overflow: auto;
}
```

Use shared tokens and components from the bundled references. Do not use Tailwind classes in injected markup, unscoped selectors, hard-coded theme colors, decorative gradients, or a custom control system. Prefix custom keyframe names with the worker name because keyframes are global.

## Worker-side asset delivery

Node workers implement the injectable UI wire contract directly:

1. Build first, so `dist/ui/page.js` and `dist/ui/styles.css` exist.
2. From compiled `dist/index.js`, read them with URLs relative to `import.meta.url`.
3. Register one content function accepting `{ path }` and returning `{ content, content_type }`.
4. Register one SDK Message-path trigger per asset:
   - `console:script` with `config: { path: '<worker-name>/page.js' }`;
   - `console:style` with `config: { path: '<worker-name>/styles.css' }`.
5. Keep the first path segment equal to `<worker-name>`; it defines the CSS scope.
6. Reject unknown asset paths.

```ts
import { readFile } from 'node:fs/promises'

const UI_CONTENT_FUNCTION = '<worker-name>::ui-content'

const uiAssets: Record<string, { content: string; content_type: string }> = {
  '<worker-name>/page.js': {
    content: await readFile(new URL('./ui/page.js', import.meta.url), 'utf8'),
    content_type: 'text/javascript',
  },
  '<worker-name>/styles.css': {
    content: await readFile(new URL('./ui/styles.css', import.meta.url), 'utf8'),
    content_type: 'text/css',
  },
}

iii.registerFunction(
  UI_CONTENT_FUNCTION,
  async ({ path }: { path: string }) => {
    const asset = uiAssets[path]
    if (!asset) throw new Error(`Unknown UI asset: ${path}`)
    return asset
  },
  {
    description: 'Serve this worker’s injectable Console UI assets.',
    request_format: {
      type: 'object',
      properties: { path: { type: 'string' } },
      required: ['path'],
    },
    response_format: {
      type: 'object',
      properties: {
        content: { type: 'string' },
        content_type: { type: 'string' },
      },
      required: ['content', 'content_type'],
    },
  },
)

iii.registerTrigger({
  type: 'console:script',
  function_id: UI_CONTENT_FUNCTION,
  config: { path: '<worker-name>/page.js' },
})

iii.registerTrigger({
  type: 'console:style',
  function_id: UI_CONTENT_FUNCTION,
  config: { path: '<worker-name>/styles.css' },
})
```

Use SDK Message-path trigger registrations here, not the engine's durable trigger-registration function. SDK registrations are replayed after reconnect and removed when the worker disconnects.

The in-memory asset map is intentionally populated at process startup. During development, rebuilding `dist/ui` restarts the worker, which reads the new bytes and re-registers the same paths with new content hashes.

## Development loop required for injectable UI

Create `scripts/dev.mjs` with this coordinated build/watch process:

```js
import { spawn } from 'node:child_process'
import { resolve } from 'node:path'

const node = process.execPath
const tsc = resolve('node_modules/typescript/bin/tsc')
const children = new Set()
let shuttingDown = false

function spawnNode(args) {
  const child = spawn(node, args, {
    env: process.env,
    stdio: 'inherit',
  })
  children.add(child)
  child.once('exit', () => children.delete(child))
  return child
}

async function runOnce(label, args) {
  console.log(`[dev] ${label}`)
  const child = spawnNode(args)
  const result = await new Promise((resolveResult, reject) => {
    child.once('error', reject)
    child.once('exit', (code, signal) => resolveResult({ code, signal }))
  })
  if (result.code !== 0) {
    throw new Error(`${label} failed (${result.signal ?? `exit ${result.code}`})`)
  }
}

function watch(label, args) {
  console.log(`[dev] watching ${label}`)
  const child = spawnNode(args)
  child.once('error', (error) => {
    if (shuttingDown) return
    console.error(`[dev] ${label} failed to start:`, error)
    shutdown(1)
  })
  child.once('exit', (code, signal) => {
    if (shuttingDown) return
    console.error(`[dev] ${label} stopped (${signal ?? `exit ${code}`})`)
    shutdown(code || 1)
  })
}

function shutdown(code = 0) {
  if (shuttingDown) return
  shuttingDown = true
  for (const child of children) child.kill('SIGTERM')
  setTimeout(() => {
    for (const child of children) child.kill('SIGKILL')
    process.exit(code)
  }, 1_500)
}

process.once('SIGINT', () => shutdown(0))
process.once('SIGTERM', () => shutdown(0))

try {
  await runOnce('building backend', [tsc, '-p', 'tsconfig.json'])
  await runOnce('checking UI types', [tsc, '-p', 'ui/tsconfig.json', '--noEmit'])
  await runOnce('building UI', ['ui/build.mjs'])

  watch('backend TypeScript', [
    tsc,
    '-p',
    'tsconfig.json',
    '--watch',
    '--preserveWatchOutput',
  ])
  watch('UI TypeScript', [
    tsc,
    '-p',
    'ui/tsconfig.json',
    '--noEmit',
    '--watch',
    '--preserveWatchOutput',
  ])
  watch('UI bundle', ['ui/build.mjs', '--watch'])
  watch('<worker-name> worker', [
    '--watch',
    '--watch-path=dist',
    '--watch-preserve-output',
    'dist/index.js',
  ])
} catch (error) {
  console.error('[dev]', error instanceof Error ? error.message : error)
  shutdown(1)
}
```

Replace `<worker-name>` in the watch label. Keep the commands and lifecycle generic; do not hardcode a repository name or absolute path.

Do not replace this script with only `tsc --watch`. It must:

1. build the backend once;
2. type-check the UI once;
3. bundle the UI once;
4. watch backend TypeScript into `dist/`;
5. watch UI TypeScript for type errors;
6. watch the UI bundle so TSX/CSS changes rewrite `dist/ui/`;
7. run the worker with Node watching `dist/`;
8. terminate all children if any watcher fails;
9. forward `SIGINT`/`SIGTERM` and escalate only after a short grace period.

This enables injectable UI hot development: an edit changes `dist/ui`, Node restarts the worker, the worker reconnects and re-registers the same asset paths with new content, and the Console hot-swaps the asset. The Console itself is not rebuilt.

## Worker manifest

Create `iii.worker.yaml` at the worker root:

```yaml
iii: v1
name: <worker-name>
language: javascript
deploy: bundle
manifest: package.json
license: Apache-2.0
tags: [<domain-tag>, console-ui]
description: <worker-description>

runtime:
  kind: javascript

scripts:
  start: node ./dist/index.js

dependencies:
  configuration: "0.x"
  console: "0.x"
```

Replace all placeholders. Declare only dependencies the worker actually uses. If there is no configuration integration, remove `configuration`. If there is no injectable UI, remove `console` and the UI-specific structure from the project.

## Add the worker to `worker-compose.yaml`

Manually edit the destination project's `worker-compose.yaml` and add the worker under its top-level `containers:` map (no Compose function writes this block completely):

```yaml
containers:
  # ...existing containers remain unchanged...

  <worker-name>:
    worker: path://<worker-directory>
    start_after:
      - console
    scripts:
      run: pnpm dev
```

Path rules:

- Resolve `<worker-directory>` relative to the directory containing `worker-compose.yaml`.
- If the worker directory is directly beside that file, use `path://./<actual-directory-name>`.
- If it lives elsewhere, use the correct relative path; do not assume `./workers/` or any particular project layout.
- Preserve all existing Compose entries and indentation.
- Use a unique container key.
- Keep `start_after: [console]` semantics for injectable UI so the Console provider exists before asset registration.
- Use `pnpm dev`, not the production `start` script, for this local development block.

If the destination Compose file already has a declaration for the worker, update that declaration instead of adding a duplicate.

### Making the running daemon adopt the block

A running Compose daemon does not re-read `worker-compose.yaml` on edit, and `compose::up { container }` answers `UNKNOWN_CONTAINER` for a container it has not loaded. To start the worker without restarting the whole project:

1. Write the block above.
2. Call `compose::add { workers: ['./<actual-directory-name>'] }`. It recognises the existing declaration, pins nothing for a `path://` worker, and starts it — but it may rewrite the block and drop `start_after`.
3. Diff the file and restore `start_after: [console]` and any other keys it removed.
4. Confirm with `compose::status`, the worker log under the daemon's `logs/<container>.log`, and `engine::workers::info { name: '<worker-name>' }`.

The first `pnpm dev` run restarts the worker once or twice while the watchers write `dist/`; that is expected.

## Implementation order

1. Resolve all project identifiers and paths.
2. Read this file and the three bundled references.
3. Inspect the destination project's existing package manager, SDK version, Compose shape, and coding conventions.
4. Create the single-package folder structure.
5. Write package and TypeScript configuration, then install dependencies.
6. Implement and test backend domain behavior.
7. Register functions with complete contracts and add configuration integration if needed.
8. Build the injectable UI using shared Console components and scoped CSS.
9. Add worker-side asset delivery and Message-path registrations.
10. Add the coordinated `scripts/dev.mjs` loop.
11. Add or update `iii.worker.yaml`.
12. Manually add or update the `worker-compose.yaml` block.
13. Validate static builds, runtime registration, asset delivery, hot reload, and real rendering. Drive the real Console through the `browser` worker (`browser::sessions::start` on the console URL, then `browser::snapshot`, `browser::act`, `browser::evaluate`, `browser::screenshot`) so both light and dark themes, narrow and wide panes, and the chat renderer are checked with screenshots, not assumptions.

## Validation checklist

Before declaring the worker complete:

- No angle-bracket placeholders remain in generated project files.
- No example project name or repository-specific absolute path leaked into identifiers, scripts, or documentation.
- `pnpm install` succeeds and the lockfile resolves `@iii-dev/console-ui` from npm at `0.1.0`, not through `file:`, `link:`, or `workspace:`.
- `pnpm typecheck`, `pnpm test`, and `pnpm build` pass.
- `pnpm dev` builds all outputs before starting watchers and shuts down cleanly.
- The worker appears in the engine with every intended function and trigger type.
- A worker-provided trigger type forwards every subscription's metadata (`binding.metadata`, or the config's `metadata` field) on every `iii.trigger`, and its description names that field.
- Every public function exposes accurate descriptions and request/response schemas.
- Configuration registration, read, update, and reload behavior work without erasing existing or unknown values.
- `dist/ui/page.js` and `dist/ui/styles.css` are non-empty and React remains external.
- The UI content function serves both registered paths and rejects unknown ones.
- Asset paths, CSS scope, worker name, function prefix, and configuration id are internally consistent.
- The Console manifest (`GET http://127.0.0.1:<console port>/ui`, or `console::ui-manifest`) contains both assets, reports no CSS warnings, and changes hashes after a UI edit.
- A real harness call to the worker (e.g. the agent fetching one record) renders through the worker's chat renderer — the result arrives as a `{ content, details }` envelope and must be unwrapped (see the designer's `console-injectable-ui`); confirm a `[data-iii-ui="<worker-name>"]` wrapper exists inside the chat DOM.
- Live updates reach an open page without a reload: mutate through a function from outside the UI and watch the page change.
- A record opens as its own pane in the same workspace tab through `host.panels.open`, and the collection page adapts when the tab splits (narrow mode).
- The real Console renders the page in narrow and wide panes, light and dark themes, with keyboard navigation, visible focus, stable async states, and no browser-console errors.
- Reconnects and repeated UI edits do not accumulate duplicate functions, triggers, pages, renderers, or forms.
- `worker-compose.yaml` contains exactly one local worker block and Compose launches it with `pnpm dev` after `console`.
