/**
 * buildNativeUi: a worker's native app (TSX) → one IIFE bundle the iii app
 * evaluates in its QuickJS VM (design §10.5).
 *
 *   esbuild (bundle, iife, es2020, platform neutral, browser + production
 *   conditions, minified) with every .tsx/.jsx going through babel
 *   (babel-preset-solid in universal mode against
 *   @iii-dev/native-ui/renderer, then @babel/preset-typescript);
 *   then the size limits of catalog.json (warn above 128 KiB, fail above
 *   256 KiB) and a smoke run in the test harness: the bundle evaluates
 *   without a DOM, sends `ready` with a `root` page within the load budget,
 *   and mounts `root` without an error.
 *
 * The bundle is written only after it passes, atomically (a temp file and a
 * rename), so the worker's III_<NS>_NATIVE_UI_WATCH poller never serves a
 * broken or half-written file. `watch: true` keeps rebuilding.
 */

import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { transformAsync } from '@babel/core'
import tsPreset from '@babel/preset-typescript'
import solidPreset from 'babel-preset-solid'
import esbuild from 'esbuild'
import { createTestHost } from './testing/index.mjs'

const pkgRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const catalog = JSON.parse(readFileSync(join(pkgRoot, 'catalog.json'), 'utf8'))

/** A worker namespace (`browser`), as the app accepts it (design §2.2). */
export const NS_PATTERN = /^[a-z0-9][a-z0-9_-]{0,63}$/

/**
 * The esbuild plugin of the build: JSX through babel-preset-solid
 * (universal) and the SDK's own modules resolved from this package, so a
 * bundle always holds one renderer and one solid-js.
 */
export function nativeUiPlugin() {
  return {
    name: 'iii-native-ui',
    setup(build) {
      build.onResolve({ filter: /^@iii-dev\/native-ui(\/renderer)?$/ }, (args) => ({
        path: join(pkgRoot, 'src', args.path.endsWith('/renderer') ? 'renderer.ts' : 'index.ts'),
      }))
      build.onLoad({ filter: /\.[jt]sx$/ }, async (args) => {
        const source = await readFile(args.path, 'utf8')
        const out = await transformAsync(source, {
          filename: args.path,
          babelrc: false,
          configFile: false,
          sourceMaps: false,
          presets: [
            [solidPreset, { generate: 'universal', moduleName: '@iii-dev/native-ui/renderer' }],
            [tsPreset, { isTSX: true, allExtensions: true }],
          ],
        })
        return { contents: out?.code ?? '', loader: 'js', resolveDir: dirname(args.path) }
      })
    },
  }
}

function esbuildOptions({ entry, ns, minify }) {
  return {
    entryPoints: [entry],
    bundle: true,
    write: false,
    outfile: 'native.js',
    format: 'iife',
    target: 'es2020',
    platform: 'neutral',
    conditions: ['browser', 'production'],
    mainFields: ['module', 'main'],
    minify,
    define: { 'process.env.NODE_ENV': '"production"', __IIIUI_NS__: JSON.stringify(ns) },
    legalComments: 'none',
    logLevel: 'silent',
    plugins: [nativeUiPlugin()],
  }
}

/** Size limits and the smoke run; throws with the reason when the bundle must not ship. */
export async function checkBundle(source, { ns, smoke = true } = {}) {
  const bytes = Buffer.byteLength(source, 'utf8')
  const warnings = []
  if (bytes > catalog.limits.bundleFailBytes) {
    throw new Error(
      `the bundle is ${kib(bytes)}, above the ${kib(catalog.limits.bundleFailBytes)} limit (catalog.json limits.bundleFailBytes)`,
    )
  }
  if (bytes > catalog.limits.bundleWarnBytes) {
    warnings.push(`the bundle is ${kib(bytes)}, above ${kib(catalog.limits.bundleWarnBytes)} (limits.bundleWarnBytes)`)
  }
  let ready = null
  if (smoke) {
    const host = await createTestHost({ bundle: source, ns, strict: true })
    try {
      ready = host.ready
      if (!ready.pages.some((p) => p.name === catalog.protocol.rootPage)) throw new Error('ready has no "root" page')
      await host.mount('root', {})
      const errors = host.errors.filter((e) => e.fatal)
      if (errors.length) throw new Error(`mounting "root" failed: ${errors[0].message}`)
      if (!host.roots.get(1)) throw new Error('mounting "root" rendered nothing')
    } finally {
      host.dispose()
    }
  }
  return { bytes, warnings, ready }
}

const kib = (n) => `${(n / 1024).toFixed(1)} KiB`

async function writeAtomic(file, content) {
  await mkdir(dirname(file), { recursive: true })
  const tmp = `${file}.${process.pid}.tmp`
  await writeFile(tmp, content)
  await rename(tmp, file)
}

/**
 * Builds `entry` into `outfile`. Resolves with
 * `{ outfile, bytes, sha256, warnings, ready }`; in watch mode, with
 * `{ stop() }` after the first build, and rebuilds on every change.
 */
export async function buildNativeUi({ entry, outfile, ns, minify = true, smoke = true, watch = false, log = console }) {
  if (!entry || !outfile) throw new Error('buildNativeUi needs entry and outfile')
  if (typeof ns !== 'string' || !NS_PATTERN.test(ns)) {
    throw new Error(
      `buildNativeUi: ns must match ${NS_PATTERN} (the worker's function prefix), got ${JSON.stringify(ns)}`,
    )
  }
  const target = resolve(outfile)
  const options = esbuildOptions({ entry: resolve(entry), ns, minify })

  const finish = async (result) => {
    if (result.errors.length) throw new Error(result.errors.map((e) => e.text).join('\n'))
    const source = result.outputFiles[0].text
    const { bytes, warnings, ready } = await checkBundle(source, { ns, smoke })
    await writeAtomic(target, source)
    const sha256 = createHash('sha256').update(source, 'utf8').digest('hex')
    for (const w of warnings) log.warn(`native-ui: ${w}`)
    return { outfile: target, bytes, sha256, warnings, ready }
  }

  if (!watch) return finish(await esbuild.build(options))

  const ctx = await esbuild.context({
    ...options,
    plugins: [
      ...options.plugins,
      {
        name: 'iii-native-ui-watch',
        setup(build) {
          build.onEnd(async (result) => {
            try {
              const out = await finish(result)
              log.info(`native-ui: built ${out.outfile} (${kib(out.bytes)}, sha256 ${out.sha256.slice(0, 12)})`)
            } catch (err) {
              log.error(`native-ui: build failed, ${target} unchanged: ${err.message}`)
            }
          })
        },
      },
    ],
  })
  await ctx.watch()
  return { stop: () => ctx.dispose() }
}
