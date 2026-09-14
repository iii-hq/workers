---
title: vite
description: Configure, run, and build Vite apps — config surface, modes and env vars, dev-server proxy, dependency pre-bundling, code splitting, and production build hygiene.
type: how-to
---

# Vite

Two different machines wear one config: **dev** is native ESM served straight to the browser
with esbuild doing per-file transforms (no bundling), and **build** is a Rollup pass producing
hashed assets. Almost every confusing Vite bug is one of those two behaving differently from
the other.

Authoritative docs: <https://vite.dev/config/> (config reference) and
<https://vite.dev/guide/>. Read the version's own docs before inventing a workaround — the
config surface changed meaningfully across v4/v5/v6/v7.

## First move: find the shape before you write config

Read `package.json`, `vite.config.*`, `tsconfig*.json`, `.env*`, and the framework plugin in
play. Match the project's existing choices — a project that already uses `defineConfig` with a
react plugin does not want a new Rollup setup bolted on.

```ts
// vite.config.ts
import { fileURLToPath } from 'node:url'
import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '')
  return {
    base: env.PUBLIC_BASE ?? '/',
    plugins: [react()],
    resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
    server: { port: 5173, strictPort: true, host: true },
    build: { sourcemap: true, target: 'es2022' },
  }
})
```

Config changes restart the dev server — editing `vite.config.ts` while a `--watch` build runs
is enough to lose a minute if you do not expect it.

## Env vars and modes

- `.env` → loaded always; `.env.local` → loaded always, gitignored; `.env.[mode]` and
  `.env.[mode].local` → only for that mode. `development` is the default for `vite`,
  `production` for `vite build`.
- **Only `VITE_`-prefixed vars reach client code** (override with `envPrefix`). Everything
  else is build-time only. This is a security boundary, not a style rule: anything exposed
  ships in the bundle and is readable by every visitor.
- Read them as `import.meta.env.VITE_X`. Built-ins: `MODE`, `DEV`, `PROD`, `BASE_URL`, `SSR`.
  Values are **strings** — `import.meta.env.VITE_FLAG === 'true'`, not truthiness.
- Type them so a typo is a compile error:

```ts
// src/vite-env.d.ts
/// <reference types="vite/client" />
interface ImportMetaEnv {
  readonly VITE_III_WS_URL: string
  readonly VITE_API_URL?: string
}
interface ImportMeta { readonly env: ImportMetaEnv }
```

- Custom mode for staging: `vite build --mode staging` reads `.env.staging`.

## The config keys that actually get used

| Key | Use |
| --- | --- |
| `server.host` / `port` / `strictPort` | `host: true` exposes on the LAN — needed when another process (an engine, a phone, a tunnel) must reach you |
| `server.proxy` | Front a backend on the same origin; avoids CORS in dev |
| `resolve.alias` | `@` → `src`. Mirror it in `tsconfig` `paths`, or the editor and the bundler disagree |
| `envPrefix` | Expose a non-`VITE_` prefix. Only when the project already committed to one |
| `optimizeDeps` | Pre-bundling control — see below |
| `build.target` | Output syntax floor. `es2022` is safe for evergreen browsers |
| `build.sourcemap` | `true` for debug builds, or `'hidden'` to ship maps without a comment |
| `build.rollupOptions` | Manual chunks, extra inputs, externalized deps |
| `build.assetsInlineLimit` | Raise/lower the data-URL threshold for small assets |
| `publicDir` | Files copied verbatim; reference them by absolute path, they are never hashed |
| `css.modules` / `css.preprocessorOptions` | CSS Modules naming, Sass/Less options |
| `define` | Compile-time constant replacement. Value must be JSON-stringified |

Dev proxy:

```ts
server: {
  proxy: {
    '/api': { target: 'http://127.0.0.1:3111', changeOrigin: true, rewrite: (p) => p.replace(/^\/api/, '') },
    '/ws': { target: 'ws://127.0.0.1:3111', ws: true },
  },
}
```

`changeOrigin: true` fixes Host-header rejection; `ws: true` is required for WebSocket
upgrades. Proxying only exists in dev — production needs a real reverse proxy with the same
routes, so keep the paths identical and say so in the README.

## Dependency pre-bundling (`optimizeDeps`)

On first dev run Vite scans imports and pre-bundles each dependency with esbuild into
`node_modules/.vite`. Why it exists: CommonJS → ESM conversion, and collapsing a package's many
internal modules into one request.

- Symptom of a stale or wrong pre-bundle: **two copies of one module**, duplicated singletons
  (two Reacts, two clients, two sockets), or `Cannot read properties of undefined` deep inside
  a dependency. Fix: delete `node_modules/.vite` and restart. `vite --force` does the same.
- `optimizeDeps.include` when a dep is only reached through a dynamic import or a non-literal
  path, so the crawler cannot see it.
- `optimizeDeps.exclude` when a dep must stay unbundled (a local linked package with its own
  HMR, or a package with a native/browser-only entry).
- Linked workspace packages usually need `resolve.preserveSymlinks: false` plus `server.watch`
  on the linked source, or edits there do not appear.

## Static assets and import suffixes

```ts
import url from './logo.svg?url'          // emitted asset URL
import raw from './schema.gql?raw'        // file contents as a string
import Worker from './worker?worker'      // a Web Worker constructor
const mods = import.meta.glob('./pages/*.tsx', { eager: true }) // build-time map
```

`import.meta.glob` is resolved at build time and is the supported way to do "all files matching
X". A runtime-generated glob will not work — write the list down.

## Build and code splitting

- **Route-level splitting is the default lever.** `React.lazy(() => import('./routes/x'))`
  plus the router's own lazy-route mechanism keeps the entry chunk to shell + routing.
- **Watch for waterfalls.** Two `await import()` calls in sequence cost two round-trips; issue
  independent dynamic imports in parallel or lift them into the route loader.
- **Vendor chunking.** Group stable, large deps so their hash survives app changes:

```ts
build: {
  rollupOptions: {
    output: {
      manualChunks: {
        react: ['react', 'react-dom'],
        router: ['@tanstack/react-router'],
      },
    },
  },
}
```

  Over-chunking is a real cost (more requests, worse caching graphs). Chunk by *change
  frequency*, not by aesthetic.
- Verify the result rather than trusting it: `npx vite-bundle-visualizer` (or
  `rollup-plugin-visualizer`) to see what actually landed, and compare chunk sizes against a
  budget before and after. A dependency you assumed was tree-shaken frequently is not.

## Commands

```bash
vite                    # dev server
vite --force            # dev server, re-run pre-bundling
vite build              # production build into dist/
vite build --mode staging
vite preview            # serve the built output — the only honest check of the build
vite optimize           # pre-bundle only
```

Always run `vite build && vite preview` before claiming a change works: dev passes for code
that fails on a real production build (case-sensitive paths, `process.env` leaks, CJS interop,
`import.meta` assumptions, missing dynamic-import chunks).

## Trap list

- Importing a Node built-in (`fs`, `path`) into client code — vite cannot polyfill it and the
  build fails or silently degrades. Server-only code belongs behind a proxy or a worker.
- `process.env.X` in client code. It is `undefined` at runtime; use `import.meta.env.X`.
- Assuming dev and build resolve identically: build is stricter about case, extensions, and
  bare specifiers.
- Changing `vite.config.ts` and expecting HMR to pick it up — it does not.
- Committing `.env.local`, or committing a `.env` that points at localhost.
- `base` left at `/` for an app served from a sub-path; every asset URL then 404s.
- Adding a plugin for something the config already does (aliases, defines, minification).
