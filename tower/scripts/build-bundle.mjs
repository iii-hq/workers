#!/usr/bin/env node

import { copyFile, mkdir, readFile, rm } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const bundleDir = join(root, 'dist', 'bundle')
const packageJsonRequirePattern =
  /createRequire\(\s*import\.meta\.url\s*\)\s*\(\s*["']\.\.\/package\.json["']\s*\)/

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
        contents: source.replace(
          new RegExp(packageJsonRequirePattern.source, 'g'),
          JSON.stringify({ version }),
        ),
        loader: 'js',
      }
    })
  },
}

await mkdir(bundleDir, { recursive: true })
await build({
  entryPoints: [join(root, 'src', 'index.ts')],
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

await Promise.all([
  copyFile(join(root, 'src', 'result.schema.json'), join(bundleDir, 'result.schema.json')),
  copyFile(join(root, 'skills', 'SKILL.md'), join(root, 'dist', 'SKILL.md')),
])

for (const file of ['page.js', 'styles.css']) {
  const target = join(bundleDir, file)
  await rm(target, { force: true })
  try {
    await copyFile(join(root, 'ui', 'dist', file), target)
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error
  }
}
