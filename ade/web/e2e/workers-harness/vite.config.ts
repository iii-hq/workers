import { readFileSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig({
  root: fileURLToPath(new URL('../..', import.meta.url)),
  plugins: [
    react(),
    tailwindcss(),
    {
      name: 'workers-harness-no-dev-socket',
      configureServer(server) {
        server.middlewares.use((request, response, next) => {
          const requested = new URL(request.url ?? '/', 'http://localhost')
          if (requested.pathname !== '/') {
            next()
            return
          }
          response.statusCode = 302
          response.setHeader(
            'Location',
            `/e2e/workers-harness/index.html${requested.search}`,
          )
          response.end()
        })
        // Vite 8.1.5 still injects its connecting client when hmr/ws are false.
        // Replace only development transport, retaining transformed source CSS.
        server.middlewares.use('/@vite/client', (_request, response) => {
          response.setHeader('Content-Type', 'text/javascript')
          response.end(
            readFileSync(new URL('./style-client.js', import.meta.url), 'utf8'),
          )
        })
      },
    },
  ],
  resolve: {
    alias: [
      {
        find: /^@\/lib\/iii-client$/,
        replacement: fileURLToPath(new URL('./mock-iii.js', import.meta.url)),
      },
      {
        find: /^@\/lib\/backend$/,
        replacement: fileURLToPath(new URL('./backend.js', import.meta.url)),
      },
      {
        find: '@',
        replacement: fileURLToPath(new URL('../../src', import.meta.url)),
      },
    ],
  },
  // There are deliberately no live engine or console proxies in this harness.
  server: {
    host: '0.0.0.0',
    port: 4180,
    strictPort: true,
    hmr: false,
    ws: false,
  },
  // Playwright serves a production build: an unbundled dev page costs about
  // four seconds of CPU per load, a built one about one.
  build: {
    outDir: fileURLToPath(
      new URL('../../../../target/workers-harness-dist', import.meta.url),
    ),
    emptyOutDir: true,
    rollupOptions: {
      input: fileURLToPath(new URL('./index.html', import.meta.url)),
    },
  },
  preview: { port: 4180 },
})
