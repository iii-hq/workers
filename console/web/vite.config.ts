import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    proxy: {
      // iii-browser-sdk hits ws(s)://${host}/iii/ws by default; the proxy
      // forwards to the local engine WebSocket on :49134 in dev.
      '/iii/ws': {
        target: 'ws://127.0.0.1:49134',
        ws: true,
        changeOrigin: false,
        rewrite: (path) => path.replace(/^\/iii\/ws/, ''),
      },
    },
  },
})
