/**
 * Build the worker's two console assets:
 *
 *   page.tsx   → dist/page.js    (injected over `console:script`)
 *   styles.css → dist/styles.css (injected over `console:style`)
 *
 * The five shared specifiers stay EXTERNAL — they resolve at runtime
 * through the console's import map (a bundled React copy would surface as
 * a cryptic "Invalid hook call"). Everything else the page needs gets
 * bundled in. `--watch` pairs with the worker's
 * III_SANDBOX_CODE_RUNNER_UI_WATCH poller for the hot-reload dev loop.
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

/**
 * The guest SDK bundle: the published `iii-sdk` npm package, with its whole
 * dependency graph (ws, @opentelemetry/api, @iii-dev/helpers) inlined into
 * ONE node ESM file. sandbox-code-runner embeds it and plants it into every
 * Node runtime at /node_modules/iii-sdk — the guest gets the real SDK with no
 * registry access and no npm install at boot. `bufferutil`/`utf-8-validate`
 * are ws's OPTIONAL native accelerators, referenced in a try/catch it handles
 * being absent — external here so esbuild doesn't fail resolving binaries
 * that were never going to be bundled.
 */
const guestSdkOptions = {
  stdin: {
    contents: "export * from 'iii-sdk'",
    resolveDir: import.meta.dirname,
    sourcefile: 'iii-sdk-guest-entry.mjs',
  },
  bundle: true,
  format: 'esm',
  platform: 'node',
  outfile: 'dist/iii-sdk-guest.mjs',
  external: ['bufferutil', 'utf-8-validate'],
  // CJS dependencies inside the graph (ws and friends) call require() on
  // node builtins at runtime; esbuild's ESM output replaces require with a
  // throwing stub ("Dynamic require of 'events' is not supported") unless
  // a real one is in scope. Standard fix: provide one.
  banner: {
    js: "import { createRequire as __cr } from 'node:module'; const require = __cr(import.meta.url);",
  },
  // DELIBERATELY NOT MINIFIED. An error thrown inside the SDK is part of
  // the guest's error UX: node prints the failing source line before the
  // message, a minified bundle's "line" is hundreds of KB, and the
  // daemon caps exec output — so with minification one wrong
  // registerFunction() call produced a wall of mangled code that CUT OFF
  // the actual TypeError (seen live in console-a2795be8). Unminified,
  // frames carry real identifiers and one-sane-line code frames; the
  // ~1.5MB plant is a one-time cost per runtime creation.
  minify: false,
  logLevel: 'info',
}

if (process.argv.includes('--watch')) {
  const ctx = await esbuild.context(options)
  await ctx.watch()
} else {
  await esbuild.build(options)
  await esbuild.build(guestSdkOptions)

  // The planted package's manifest. The SDK reads `../package.json` at
  // runtime (its own version string), so sandbox-code-runner plants the
  // bundle at /node_modules/iii-sdk/dist/index.mjs with this file beside it —
  // the real package's layout. Generated from the resolved dependency so the
  // version can never drift from what was actually bundled.
  // Read through the node_modules symlink — the package's `exports` map
  // does not expose ./package.json to require().
  const { readFileSync, writeFileSync } = await import('node:fs')
  const real = JSON.parse(
    readFileSync(new URL('./node_modules/iii-sdk/package.json', import.meta.url), 'utf8'),
  )
  writeFileSync(
    'dist/iii-sdk-guest-package.json',
    JSON.stringify(
      {
        name: 'iii-sdk',
        version: real.version,
        type: 'module',
        main: 'dist/index.mjs',
        exports: { '.': './dist/index.mjs' },
      },
      null,
      2,
    ) + '\n',
  )
}
