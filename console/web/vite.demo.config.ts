/**
 * Build config for the landing-page demo (`npm run build:demo`).
 *
 * Produces a standalone page in `dist-demo/` that the marketing site vendors
 * and embeds in an iframe. Separate from the console build on purpose: no
 * engine client, no injectable-UI import map, and two heavy specifiers the
 * demo can never reach are aliased to stubs so their chunks stay out of the
 * committed artifact.
 */

import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const src = fileURLToPath(new URL('./src', import.meta.url))
const pierreStub = fileURLToPath(
  new URL('./src/demo/stubs/pierre-diffs.tsx', import.meta.url),
)

export default defineConfig({
  // Absolute asset paths at the site's mount point. Relative ('./') paths
  // break behind Vercel's cleanUrls, which 308s /console-demo/index.html to
  // /console-demo (no trailing slash), making ./assets/* resolve to the
  // domain root. The marketing site vendors this build at /console-demo/.
  base: '/console-demo/',
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: [
      // `@pierre/diffs` drags in every shiki grammar (~13MB of chunks) for
      // coder diff views the demo never renders. Order matters: the
      // `/react` entry must match before the bare specifier.
      { find: '@pierre/diffs/react', replacement: pierreStub },
      { find: '@pierre/diffs', replacement: pierreStub },
      { find: '@', replacement: src },
    ],
  },
  build: {
    outDir: 'dist-demo',
    emptyOutDir: true,
    rollupOptions: {
      input: fileURLToPath(new URL('./demo.html', import.meta.url)),
    },
  },
})
