import path from 'node:path'
import { fileURLToPath } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const root = path.dirname(fileURLToPath(import.meta.url))

export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: {
    'process.env.NODE_ENV': JSON.stringify('production'),
  },
  build: {
    lib: {
      entry: path.resolve(root, 'src/extension.tsx'),
      formats: ['es'],
      fileName: () => 'extension.js',
    },
    outDir: 'dist',
    emptyOutDir: true,
    cssCodeSplit: false,
    minify: 'oxc',
    rollupOptions: {
      output: {
        assetFileNames: (asset) =>
          asset.names.some((name) => name.endsWith('.css'))
            ? 'extension.css'
            : '[name][extname]',
      },
    },
  },
})
