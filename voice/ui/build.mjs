/**
 * Build the worker's two console assets:
 *
 *   page.tsx   → dist/page.js    (injected over `console:script`)
 *   styles.css → dist/styles.css (injected over `console:style`)
 *
 * The five shared specifiers stay EXTERNAL — they resolve at runtime
 * through the console's import map (a bundled React copy would surface as
 * a cryptic "Invalid hook call"). Everything else the page needs gets
 * bundled in, kept well under the console's 8 MiB per-asset cap.
 * `--watch` pairs with the worker's III_VOICE_UI_WATCH poller for the
 * hot-reload dev loop.
 */

import esbuild from 'esbuild'
import { statSync } from 'node:fs'

const SIZE_CAP = 8 * 1024 * 1024

const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  external: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', '@iii-dev/console-ui'],
  logLevel: 'info',
}

if (process.argv.includes('--watch')) {
  const ctx = await esbuild.context(options)
  await ctx.watch()
} else {
  await esbuild.build(options)

  let overCap = false
  for (const file of ['dist/page.js', 'dist/styles.css']) {
    const bytes = statSync(file).size
    const mib = (bytes / (1024 * 1024)).toFixed(2)
    const verdict = bytes < SIZE_CAP ? 'ok' : 'OVER THE 8 MiB CONSOLE CAP'
    console.log(`${file}: ${bytes} bytes (${mib} MiB) — ${verdict}`)
    if (bytes >= SIZE_CAP) overCap = true
  }
  if (overCap) {
    console.error('one or more assets exceed the console 8 MiB per-asset cap')
    process.exit(1)
  }
}
