import path from 'node:path'
import { fileURLToPath } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const root = path.dirname(fileURLToPath(import.meta.url))
const boundaries = path.join(root, 'boundaries.tsx')
export default defineConfig({
  root,
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: [
      { find: '@/lib/conversations-context', replacement: boundaries },
      { find: '@/lib/iii-client', replacement: boundaries },
      { find: '@/lib/sessions/removal-preview', replacement: boundaries },
      {
        find: '@/components/sidebar/ConversationSidebar',
        replacement: boundaries,
      },
      { find: /\.\/ChatView$/, replacement: boundaries },
      { find: '@', replacement: path.resolve(root, '../../src') },
    ],
  },
  server: { host: '127.0.0.1', port: 4179, strictPort: true },
})
