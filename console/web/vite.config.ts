import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { configDefaults, defineConfig } from 'vitest/config'

// Engine WebSocket URL, same knob as the console binary's `--url`
// (DEFAULT_ENGINE_URL in src/config.rs).
const ENGINE_URL = process.env.III_ENGINE_URL ?? 'ws://127.0.0.1:49134'

export default defineConfig({
  // Relative asset paths so the embedded SPA works behind a reverse
  // proxy that mounts the console at an arbitrary subpath.
  base: './',
  plugins: [react(), tailwindcss()],
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
    },
  },
})
