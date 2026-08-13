/**
 * Build the worker's two console assets:
 *
 *   page.tsx   → dist/page.js    (injected over `console:script`)
 *   styles.css → dist/styles.css (injected over `console:style`)
 *
 * The five shared specifiers stay EXTERNAL — they resolve at runtime through
 * the console's import map. A bundled React copy surfaces as a cryptic
 * "Invalid hook call", because the page's hooks would then be talking to a
 * different React instance than the one that mounted them. Everything else
 * the page needs gets bundled in.
 *
 * `--watch` pairs with the worker's III_CODE_RUNNER_UI_WATCH poller for the
 * hot-reload dev loop.
 *
 * Unlike `sandbox-code-runner/ui`, there is no second bundle here: that worker
 * also builds an `iii-sdk` bundle to plant into each microVM's
 * /node_modules, because its guests are real Node processes with no registry
 * access. code-runner's guests are a V8 isolate and a wasm CPython — their
 * `iii` global is a host-implemented op surface and a framed stdio bridge, so
 * there is nothing to plant and no npm graph to inline.
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
