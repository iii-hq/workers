/**
 * Build the worker's two console assets:
 *
 *   page.tsx   → dist/page.js    (injected over `console:script`)
 *   styles.css → dist/styles.css (injected over `console:style`)
 *
 * The five shared specifiers stay EXTERNAL — they resolve at runtime
 * through the console's import map (a bundled React copy would surface as
 * a cryptic "Invalid hook call"). Everything else the page needs (zod)
 * gets bundled in. `--watch` pairs with the worker's
 * III_III_DIRECTORY_UI_WATCH poller for the hot-reload dev loop.
 */

import esbuild from 'esbuild'

const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  external: [
    'react',
    'react-dom',
    'react-dom/client',
    'react/jsx-runtime',
    '@iii-dev/console-ui',
  ],
  logLevel: 'info',
}

if (process.argv.includes('--watch')) {
  const ctx = await esbuild.context(options)
  await ctx.watch()
} else {
  await esbuild.build(options)
}
