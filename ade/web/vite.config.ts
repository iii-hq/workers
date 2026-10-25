import { existsSync, readFileSync } from 'node:fs'
import { extname, join, resolve } from 'node:path'
import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import type { Plugin } from 'vite'
import { configDefaults, defineConfig } from 'vitest/config'

// Engine WebSocket URL, same knob as the console binary's `--url`
// (DEFAULT_ENGINE_URL in src/config.rs).
const ENGINE_URL = process.env.III_ENGINE_URL ?? 'ws://127.0.0.1:49134'

/**
 * Dev-only: serve a worker's injected UI from a local build instead of the
 * bundle the running worker embeds, so a UI change under `<worker>/ui` shows
 * in the console without rebuilding and restarting that worker.
 *
 *   III_WORKER_UI_OVERRIDES="harness=../../harness/ui/dist,voice=../../voice/ui/dist"
 *
 * Only `/ui/<scope>/<file>` requests for a listed scope are answered here;
 * everything else still proxies to the console worker, which keeps the
 * `console:script` registration (and the hash in `?v=`) the worker's own.
 */
function workerUiOverrides(): Plugin {
  const spec = process.env.III_WORKER_UI_OVERRIDES ?? ''
  const dirs = new Map<string, string>()
  for (const entry of spec.split(',')) {
    const [scope, dir] = entry.split('=')
    if (scope?.trim() && dir?.trim()) {
      dirs.set(scope.trim(), resolve(process.cwd(), dir.trim()))
    }
  }
  const types: Record<string, string> = {
    '.js': 'text/javascript',
    '.mjs': 'text/javascript',
    '.css': 'text/css',
    '.map': 'application/json',
  }
  return {
    name: 'iii-worker-ui-overrides',
    apply: 'serve',
    configureServer(server) {
      if (dirs.size === 0) return
      for (const [scope, dir] of dirs) {
        server.config.logger.info(`[worker-ui] /ui/${scope}/* ← ${dir}`)
      }
      server.middlewares.use((req, res, next) => {
        const match = /^\/ui\/([^/?]+)\/([^?]+)/.exec(req.url ?? '')
        const dir = match && dirs.get(match[1])
        if (!dir) return next()
        const file = join(dir, match[2])
        if (!file.startsWith(dir) || !existsSync(file)) return next()
        res.setHeader(
          'Content-Type',
          types[extname(file)] ?? 'application/octet-stream',
        )
        res.setHeader('Cache-Control', 'no-store')
        res.end(readFileSync(file))
      })
    },
  }
}

export default defineConfig({
  // Relative asset paths so the embedded SPA works behind a reverse
  // proxy that mounts the console at an arbitrary subpath.
  base: './',
  plugins: [react(), tailwindcss(), workerUiOverrides()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  test: {
    exclude: [...configDefaults.exclude, 'e2e/**'],
  },
  // Dev twin of the console worker's HTTP server (src/server.rs): the same
  // /ws proxy contract, bound on 0.0.0.0 like the binary so the dev server
  // is reachable from containers/LAN. The binary's proxy additionally stamps
  // `metadata.internal = true` on browser `registerfunction` frames, but
  // only as defense against STALE bundles — dev always serves the current
  // bundle, which stamps its own registrations (src/lib/iii-client.ts) —
  // so this proxy stays a dumb pipe.
  server: {
    host: '0.0.0.0',
    allowedHosts: true,
    proxy: {
      // iii-browser-sdk hits ws(s)://${host}/ws by default; the proxy
      // forwards to the engine WebSocket. In prod the `console` worker
      // exposes the same /ws contract.
      '/ws': {
        target: ENGINE_URL,
        ws: true,
        changeOrigin: false,
        rewrite: (path) => path.replace(/^\/ws/, ''),
      },
      // Injected UI assets are registered with (and served by) the console
      // WORKER, not this dev server — proxy the asset routes to it so the
      // injectable-UI dev loop works against `vite dev` too. (`/vendor/*`
      // needs no proxy: the shims live in public/ and Vite serves them.)
      '/ui': {
        target: process.env.III_CONSOLE_URL ?? 'http://127.0.0.1:3113',
        changeOrigin: true,
      },
      '/runtime': {
        target: process.env.III_CONSOLE_URL ?? 'http://127.0.0.1:3113',
        changeOrigin: true,
      },
    },
  },
})
