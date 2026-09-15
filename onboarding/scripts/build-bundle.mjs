#!/usr/bin/env node
/**
 * The publishable shape: one `dist/bundle/index.mjs` with the worker and its
 * dependencies inside it, and the two page assets beside it.
 *
 * The assets stay separate files rather than string constants in the bundle,
 * because `onboarding::ui-content` reads them from disk at call time — the
 * same code path the watcher uses in development.
 */

import { copyFile, mkdir, readFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const bundleDir = join(root, 'dist', 'bundle')

// 1. The injected page, the same build the console loads in development.
await build({
  entryPoints: [join(root, 'ui', 'page.tsx'), join(root, 'ui', 'styles.css')],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: join(root, 'ui', 'dist'),
  external: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', '@iii-dev/console-ui'],
  logLevel: 'info',
})

/**
 * iii-sdk reads its own version through `createRequire(import.meta.url)` on
 * `../package.json`, which no longer resolves once the SDK is inlined. The
 * version is substituted at build time instead. Same fix as `kanban`.
 */
const packageJsonRequirePattern = /createRequire\(\s*import\.meta\.url\s*\)\s*\(\s*["']\.\.\/package\.json["']\s*\)/
const inlinePackageJson = {
  name: 'inline-sdk-package-json',
  setup(builder) {
    builder.onLoad({ filter: /iii-sdk[\\/]dist[\\/]index\.mjs$/ }, async (args) => {
      const [source, pkg] = await Promise.all([
        readFile(args.path, 'utf8'),
        readFile(join(root, 'node_modules', 'iii-sdk', 'package.json'), 'utf8'),
      ])
      const { version } = JSON.parse(pkg)
      if (!packageJsonRequirePattern.test(source)) {
        throw new Error('iii-sdk package.json lookup pattern was not found during bundling')
      }
      return {
        contents: source.replace(new RegExp(packageJsonRequirePattern.source, 'g'), JSON.stringify({ version })),
        loader: 'js',
      }
    })
  },
}

// 2. The worker itself.
await build({
  entryPoints: [join(root, 'src', 'index.mjs')],
  outfile: join(bundleDir, 'index.mjs'),
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  legalComments: 'none',
  external: ['fsevents'],
  banner: {
    js: "import{createRequire as __iiiCR}from'module';const require=__iiiCR(import.meta.url);",
  },
  define: { 'process.env.NODE_ENV': '"production"' },
  plugins: [inlinePackageJson],
  logLevel: 'info',
})

// 3. The assets, beside the bundle, where `ui-content` looks for them first.
await mkdir(bundleDir, { recursive: true })
for (const file of ['page.js', 'styles.css']) {
  await copyFile(join(root, 'ui', 'dist', file), join(bundleDir, file))
}
console.log(`  dist/bundle/ page.js, styles.css`)
