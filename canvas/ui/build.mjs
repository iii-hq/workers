/**
 * Build the worker's four console assets:
 *
 *   page.tsx          → dist/page.js     (injected over `console:script`)
 *   styles.css        → dist/styles.css  (injected over `console:style`)
 *   mermaid-entry.ts  → dist/mermaid.js  (`console:script`, vendor bundle)
 *   freeform-entry.ts → dist/freeform.js (`console:script`, vendor bundle)
 *
 * page.js keeps the five shared specifiers EXTERNAL — they resolve at runtime
 * through the console's import map. A bundled React copy surfaces as a cryptic
 * "Invalid hook call" with nothing pointing at the cause.
 *
 * The vendor bundles are self-contained ES modules the page lazily imports
 * through src/lib/loaders.ts:
 *   - mermaid.js bundles ALL of mermaid (no externals — mermaid never touches
 *     React) and re-exports its API.
 *   - freeform.js bundles @excalidraw/excalidraw + the mermaid→excalidraw
 *     converter with ONLY the react family external (excalidraw must render in
 *     the console's React tree), plus the excalidraw stylesheet as a text
 *     export the runtime injects once.
 * Both default-export a no-op setup(): the console eagerly import()s every
 * console:script asset and calls its default export.
 *
 * Every dist file must stay under the console's 8 MiB per-asset cap — the
 * build prints sizes and fails past the cap.
 *
 * `--watch` (page assets only) pairs with the worker's III_CANVAS_UI_WATCH
 * poller for the hot-reload dev loop.
 */

import esbuild from 'esbuild'
import { statSync } from 'node:fs'

const SIZE_CAP = 8 * 1024 * 1024

const pageOptions = {
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

const mermaidOptions = {
  entryPoints: ['mermaid-entry.ts'],
  outfile: 'dist/mermaid.js',
  bundle: true,
  format: 'esm',
  minify: true,
  define: { 'process.env.NODE_ENV': '"production"' },
  logLevel: 'info',
}

/**
 * Excalidraw dynamically imports one translation file per language; bundling
 * inlines ALL of them (~1.6 MiB) and pushed freeform.js to 99.9% of the 8 MiB
 * cap. The console renders excalidraw with its default English strings, so
 * every locale file becomes an empty module (excalidraw treats missing keys
 * as en fallbacks).
 */
const stubExcalidrawLocales = {
  name: 'stub-excalidraw-locales',
  setup(build) {
    build.onLoad(
      { filter: /@excalidraw\/excalidraw\/dist\/prod\/locales\/.*\.js$/ },
      () => ({ contents: 'export default {}', loader: 'js' }),
    )
  },
}

const freeformOptions = {
  entryPoints: ['freeform-entry.ts'],
  outfile: 'dist/freeform.js',
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  minify: true,
  external: ['react', 'react-dom', 'react/jsx-runtime', 'react-dom/client'],
  // Excalidraw's CJS interior reaches the react externals through require()
  // calls that survive into the ESM output as esbuild's __require — which
  // throws "Dynamic require of \"react\" is not supported" at runtime. A
  // module-scoped `require` resolving the externals from the console's
  // import map satisfies esbuild's typeof-require fallback.
  banner: {
    js: [
      'import * as __shim_react from "react";',
      'import * as __shim_react_dom from "react-dom";',
      'import * as __shim_react_dom_client from "react-dom/client";',
      'import * as __shim_jsx from "react/jsx-runtime";',
      'var require = (m) => {',
      '  const shims = { react: __shim_react, "react-dom": __shim_react_dom,',
      '    "react-dom/client": __shim_react_dom_client, "react/jsx-runtime": __shim_jsx };',
      '  if (m in shims) return shims[m];',
      '  throw new Error("freeform bundle: unshimmed require of " + m);',
      '};',
    ].join('\n'),
  },
  // The excalidraw package gates its "." and "./index.css" exports behind
  // development/production conditions (index.css has no default), so the
  // condition must be explicit.
  conditions: ['production'],
  loader: {
    '.css': 'text',
    '.woff2': 'dataurl',
    '.woff': 'dataurl',
    '.ttf': 'dataurl',
  },
  define: {
    'process.env.NODE_ENV': '"production"',
    'process.env.IS_PREACT': '"false"',
  },
  plugins: [stubExcalidrawLocales],
  logLevel: 'info',
}

if (process.argv.includes('--watch')) {
  const ctx = await esbuild.context(pageOptions)
  await ctx.watch()
} else {
  await esbuild.build(pageOptions)
  await esbuild.build(mermaidOptions)
  await esbuild.build(freeformOptions)

  let overCap = false
  for (const file of [
    'dist/page.js',
    'dist/styles.css',
    'dist/mermaid.js',
    'dist/freeform.js',
  ]) {
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
