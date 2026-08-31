#!/usr/bin/env node

import { readFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

await build({
  entryPoints: [join(root, 'ui/page.tsx'), join(root, 'ui/styles.css')],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: join(root, 'ui/dist'),
  external: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', '@iii-dev/console-ui'],
  logLevel: 'info',
})

const packageJsonRequirePattern = /createRequire\(\s*import\.meta\.url\s*\)\s*\(\s*["']\.\.\/package\.json["']\s*\)/
const uiNamespace = 'kanban-ui-assets'

const inlineAssets = {
  name: 'inline-kanban-ui-assets',
  setup(builder) {
    builder.onResolve({ filter: /^virtual:kanban-ui-assets$/ }, () => ({ path: 'assets', namespace: uiNamespace }))
    builder.onLoad({ filter: /.*/, namespace: uiNamespace }, async () => {
      const page = await readFile(join(root, 'ui/dist/page.js'), 'utf8')
      const styles = await readFile(join(root, 'ui/dist/styles.css'), 'utf8')
      return {
        contents: `export const uiPage = ${JSON.stringify(page)};\nexport const uiStyles = ${JSON.stringify(styles)};`,
        loader: 'js',
      }
    })
  },
}

const inlinePackageJson = {
  name: 'inline-sdk-package-json',
  setup(builder) {
    builder.onLoad({ filter: /iii-sdk[\\/]dist[\\/]index\.mjs$/ }, async (args) => {
      const [source, pkg] = await Promise.all([
        readFile(args.path, 'utf8'),
        readFile(join(root, 'node_modules/iii-sdk/package.json'), 'utf8'),
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

await build({
  entryPoints: [join(root, 'src/index.ts')],
  outfile: join(root, 'dist/bundle/index.mjs'),
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
  plugins: [inlineAssets, inlinePackageJson],
  logLevel: 'info',
})
