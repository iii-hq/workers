import { readFileSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

/*
 * The auto-compact harness (chat-projects phase 3): the real dialog, meter
 * and transcript over an in-memory engine. Same shape as the Workers
 * harness: no engine or console proxies, no development socket.
 */
export default defineConfig({
  root: fileURLToPath(new URL('../..', import.meta.url)),
  plugins: [
    react(),
    tailwindcss(),
    {
      name: 'autocompact-harness-no-dev-socket',
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
            `/e2e/autocompact-harness/index.html${requested.search}`,
          )
          response.end()
        })
        server.middlewares.use('/@vite/client', (_request, response) => {
          response.setHeader('Content-Type', 'text/javascript')
          response.end(
            readFileSync(
              new URL('../workers-harness/style-client.js', import.meta.url),
              'utf8',
            ),
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
        find: /^@\/lib\/conversations-context$/,
        replacement: fileURLToPath(
          new URL('./conversations.js', import.meta.url),
        ),
      },
      {
        find: '@',
        replacement: fileURLToPath(new URL('../../src', import.meta.url)),
      },
    ],
  },
  server: {
    host: '127.0.0.1',
    port: 4181,
    strictPort: true,
    hmr: false,
    ws: false,
  },
  build: {
    outDir: fileURLToPath(
      new URL('../../../../target/autocompact-harness-dist', import.meta.url),
    ),
    emptyOutDir: true,
    rollupOptions: {
      input: fileURLToPath(new URL('./index.html', import.meta.url)),
    },
  },
  preview: { host: '127.0.0.1', port: 4181, strictPort: true },
})
