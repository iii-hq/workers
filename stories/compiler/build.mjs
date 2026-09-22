#!/usr/bin/env node
// The stories compiler. Two commands, both print one JSON document on stdout:
//
//   discover --workspace <dir> [--stories <glob>]* [--ignore <glob>]*
//     → { projects: [{ name, path, files: [rel-to-project] }] }
//
//   build --project <dir> --out <dir> [--files <rel>]* [--preview <path>]
//         [--config <path>] [--stories <glob>]* [--ignore <glob>]*
//     → { ok, files: [{ file, html, entry, modules: [{ path, hop }], manifest, error }], warnings }
//
// The build runs the PROJECT's own Vite when it has one (so its plugins,
// aliases and CSS pipeline apply) and falls back to the compiler's Vite. One
// generated HTML entry per story file lands in <project>/node_modules/.stories
// so `react` resolves from the project, never from this package.

// Vite resolves `process.env.NODE_ENV` from the running process when set; a
// shell exporting `development` would bundle React's development build.
process.env.NODE_ENV = 'production'

import { createRequire } from 'node:module'
import { promises as fs, existsSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const DEFAULT_STORIES = ['**/*.stories.@(tsx|ts|jsx|js|mjs)']
const DEFAULT_IGNORE = ['node_modules', 'dist', 'build', 'target', 'data', 'coverage', 'out', '.next', '.git', 'storybook-static', '.stories', 'worktrees']
const PREVIEW_CANDIDATES = ['.storybook/preview.tsx', '.storybook/preview.ts', '.storybook/preview.jsx', '.storybook/preview.js', '.storybook/preview.mjs']
const CONFIG_CANDIDATES = ['vite.config.ts', 'vite.config.mts', 'vite.config.js', 'vite.config.mjs', 'vite.config.cts', 'vite.config.cjs']

function parseArgs(argv) {
  const out = { _: [] }
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    if (!arg.startsWith('--')) {
      out._.push(arg)
      continue
    }
    const key = arg.slice(2)
    const value = argv[i + 1] !== undefined && !argv[i + 1].startsWith('--') ? argv[++i] : 'true'
    if (out[key] === undefined) out[key] = value
    else out[key] = [].concat(out[key], value)
  }
  return out
}

const list = (value, fallback) => (value === undefined ? fallback : [].concat(value))
const emit = (value) => process.stdout.write(`${JSON.stringify(value)}\n`)
const log = (message) => process.stderr.write(`[stories] ${message}\n`)

function ignored(rel, ignore) {
  const segments = rel.split(/[\\/]/)
  for (const pattern of ignore) {
    if (!pattern.includes('*') && !pattern.includes('/')) {
      if (segments.includes(pattern)) return true
    } else if (path.matchesGlob(rel, pattern)) return true
  }
  return false
}

async function globStories(root, patterns, ignore) {
  const files = []
  for await (const entry of fs.glob(patterns, { cwd: root, exclude: (name) => ignored(name, ignore) })) {
    const rel = entry.split(path.sep).join('/')
    if (!ignored(rel, ignore)) files.push(rel)
  }
  return files.sort()
}

function nearestPackage(root, file) {
  let dir = path.dirname(path.join(root, file))
  while (dir.startsWith(root)) {
    if (existsSync(path.join(dir, 'package.json'))) return dir
    if (dir === root) break
    dir = path.dirname(dir)
  }
  return root
}

function packageName(dir) {
  try {
    const name = JSON.parse(readFileSync(path.join(dir, 'package.json'), 'utf8')).name
    if (typeof name === 'string' && name.trim()) return name.trim()
  } catch {}
  return path.basename(dir)
}

async function discover(args) {
  const root = path.resolve(args.workspace ?? '.')
  const patterns = list(args.stories, DEFAULT_STORIES)
  const ignore = [...DEFAULT_IGNORE, ...list(args.ignore, [])]
  const files = await globStories(root, patterns, ignore)
  const projects = new Map()
  for (const file of files) {
    const dir = nearestPackage(root, file)
    const rel = path.relative(root, dir).split(path.sep).join('/') || '.'
    if (!projects.has(rel)) projects.set(rel, { name: packageName(dir), path: rel, files: [] })
    projects.get(rel).files.push(path.relative(dir, path.join(root, file)).split(path.sep).join('/'))
  }
  emit({ projects: [...projects.values()].sort((a, b) => a.path.localeCompare(b.path)) })
}

/* ── build ────────────────────────────────────────────────────────────── */

function slugOf(file) {
  return file
    .replace(/\.[jt]sx?$/, '')
    .replace(/\.stories$/, '')
    .replace(/[^a-zA-Z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .toLowerCase()
}

function relImport(fromDir, target) {
  let rel = path.relative(fromDir, target).split(path.sep).join('/')
  if (!rel.startsWith('.')) rel = `./${rel}`
  return rel
}

async function loadVite(project) {
  const require = createRequire(path.join(project, 'package.json'))
  try {
    const entry = require.resolve('vite')
    const mod = await import(pathToFileURL(entry).href)
    return { vite: mod, source: 'project', version: mod.version }
  } catch {
    const mod = await import('vite')
    return { vite: mod, source: 'compiler', version: mod.version }
  }
}

function findFirst(project, candidates, explicit) {
  if (explicit) return path.resolve(project, explicit)
  for (const candidate of candidates) {
    const abs = path.join(project, candidate)
    if (existsSync(abs)) return abs
  }
  return null
}

function stripBuildOwnership(config) {
  const next = { ...config }
  delete next.base
  delete next.publicDir
  delete next.server
  delete next.preview
  delete next.appType
  if (next.build) {
    const build = { ...next.build }
    delete build.outDir
    delete build.emptyOutDir
    delete build.lib
    delete build.manifest
    delete build.ssr
    delete build.watch
    if (build.rollupOptions) {
      const rollup = { ...build.rollupOptions }
      delete rollup.input
      delete rollup.preserveEntrySignatures
      build.rollupOptions = rollup
    }
    next.build = build
  }
  return next
}

const isProjectModule = (id, project) =>
  !id.startsWith('\0') &&
  !id.includes('/node_modules/') &&
  !id.includes('\\node_modules\\') &&
  path.isAbsolute(id.split('?')[0]) &&
  !id.split('?')[0].startsWith(path.join(project, 'node_modules', '.stories'))

function reachable(graph, start, project) {
  const hops = new Map([[start, 0]])
  const queue = [start]
  while (queue.length) {
    const id = queue.shift()
    const hop = hops.get(id)
    for (const next of graph.get(id) ?? []) {
      if (hops.has(next)) continue
      if (!isProjectModule(next, project)) continue
      hops.set(next, hop + 1)
      queue.push(next)
    }
  }
  hops.delete(start)
  return hops
}

async function build(args) {
  const project = path.resolve(args.project ?? '.')
  const outDir = path.resolve(args.out ?? path.join(project, 'node_modules', '.stories', 'dist'))
  const patterns = list(args.stories, DEFAULT_STORIES)
  const ignore = [...DEFAULT_IGNORE, ...list(args.ignore, [])]
  const files = args.files ? list(args.files, []) : await globStories(project, patterns, ignore)
  const warnings = []
  if (files.length === 0) {
    emit({ ok: true, project, vite: null, files: [], warnings: ['no story files'] })
    return
  }

  const cacheDir = path.join(project, 'node_modules', '.stories')
  await fs.mkdir(path.join(cacheDir, 'shims'), { recursive: true })
  await fs.copyFile(path.join(here, 'runtime.js'), path.join(cacheDir, 'runtime.js'))
  for (const shim of ['storybook-test.js', 'empty-preview.js']) {
    await fs.copyFile(path.join(here, 'shims', shim), path.join(cacheDir, 'shims', shim))
  }
  const previewFile = findFirst(project, PREVIEW_CANDIDATES, args.preview) ?? path.join(cacheDir, 'shims', 'empty-preview.js')
  const configFile = findFirst(project, CONFIG_CANDIDATES, args.config)

  const entries = []
  for (const file of files) {
    const abs = path.join(project, file)
    if (!existsSync(abs)) {
      warnings.push(`missing story file ${file}`)
      continue
    }
    const slug = slugOf(file)
    const tsx = path.join(cacheDir, `${slug}.tsx`)
    const html = path.join(cacheDir, `${slug}.html`)
    await fs.writeFile(
      tsx,
      [
        `import * as React from 'react'`,
        `import * as ReactDOMClient from 'react-dom/client'`,
        `import * as stories from ${JSON.stringify(relImport(cacheDir, abs))}`,
        `import preview from ${JSON.stringify(relImport(cacheDir, previewFile))}`,
        `import { boot } from './runtime.js'`,
        `boot({ React, ReactDOMClient, stories, preview: preview ?? {}, file: ${JSON.stringify(file)} })`,
        '',
      ].join('\n'),
    )
    await fs.writeFile(
      html,
      `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${slug}</title></head><body><div id="story-root"></div><script type="module" src="./${slug}.tsx"></script></body></html>\n`,
    )
    entries.push({ file, abs, slug, html })
  }

  const { vite, source, version } = await loadVite(project)
  log(`vite ${version} (${source}) building ${entries.length} story file(s) in ${project}`)

  let projectConfig = {}
  if (configFile) {
    const loaded = await vite.loadConfigFromFile({ command: 'build', mode: 'production' }, configFile, project)
    projectConfig = stripBuildOwnership(loaded?.config ?? {})
  }

  const graph = new Map()
  const graphPlugin = {
    name: 'iii-stories:graph',
    buildEnd() {
      for (const id of this.getModuleIds()) {
        const info = this.getModuleInfo(id)
        if (info) graph.set(id, [...info.importedIds, ...info.dynamicallyImportedIds])
      }
    },
  }
  const shim = path.join(cacheDir, 'shims', 'storybook-test.js')
  const ours = {
    configFile: false,
    root: project,
    mode: 'production',
    logLevel: 'warn',
    clearScreen: false,
    base: './',
    publicDir: false,
    resolve: {
      alias: [
        { find: /^(storybook\/test|@storybook\/test|storybook\/actions|@storybook\/addon-actions|@storybook\/jest|@storybook\/testing-library)$/, replacement: shim },
      ],
    },
    ...(configFile ? {} : { esbuild: { jsx: 'automatic' } }),
    build: {
      outDir,
      emptyOutDir: true,
      minify: false,
      sourcemap: false,
      manifest: true,
      cssCodeSplit: true,
      target: 'esnext',
      reportCompressedSize: false,
      chunkSizeWarningLimit: 1_000_000,
      rollupOptions: {
        input: Object.fromEntries(entries.map((entry) => [entry.slug, entry.html])),
        onwarn(warning) {
          if (warning.code === 'EMPTY_BUNDLE' || warning.code === 'MODULE_LEVEL_DIRECTIVE') return
          warnings.push(String(warning.message ?? warning))
        },
      },
    },
    plugins: [graphPlugin],
  }
  const config = vite.mergeConfig(projectConfig, ours)
  config.configFile = false
  await vite.build(config)

  const manifestPath = path.join(outDir, '.vite', 'manifest.json')
  const viteManifest = existsSync(manifestPath) ? JSON.parse(await fs.readFile(manifestPath, 'utf8')) : {}
  const previewModules = isProjectModule(previewFile, project) ? reachable(graph, previewFile, project) : new Map()
  const results = []
  for (const entry of entries) {
    const key = path.relative(project, entry.html).split(path.sep).join('/')
    const record = viteManifest[key]
    const hops = reachable(graph, entry.abs, project)
    if (isProjectModule(previewFile, project)) {
      hops.set(previewFile, Math.max(hops.get(previewFile) ?? 2, 2))
      for (const [id, hop] of previewModules) if (!hops.has(id)) hops.set(id, hop + 2)
    }
    const modules = [{ path: entry.abs, hop: 0 }, ...[...hops].map(([id, hop]) => ({ path: id.split('?')[0], hop }))]
    results.push({
      file: entry.file,
      html: record ? key : null,
      entry: record?.file ?? null,
      modules: dedupe(modules),
      manifest: null,
      error: record ? null : 'vite produced no output for this story file',
    })
  }

  await extractManifests(outDir, results)
  emit({ ok: true, project, vite: { version, source }, preview: isProjectModule(previewFile, project) ? previewFile : null, files: results, warnings })
}

function dedupe(modules) {
  const seen = new Map()
  for (const module of modules) {
    const prev = seen.get(module.path)
    if (!prev || prev.hop > module.hop) seen.set(module.path, module)
  }
  return [...seen.values()].sort((a, b) => a.hop - b.hop || a.path.localeCompare(b.path))
}

async function extractManifests(outDir, results) {
  let registrator = null
  try {
    ;({ GlobalRegistrator: registrator } = await import('@happy-dom/global-registrator'))
    registrator.register({ url: 'http://stories.local/', width: 1024, height: 768 })
  } catch (error) {
    for (const result of results) result.error ??= `manifest extraction unavailable: ${error?.message ?? error}`
    return
  }
  try {
    for (const result of results) {
      if (!result.entry) continue
      try {
        await import(pathToFileURL(path.join(outDir, result.entry)).href)
        const api = globalThis.__storiesRegistry?.[result.file]
        if (!api) throw new Error('entry did not register its story file')
        result.manifest = api.manifest()
      } catch (error) {
        result.error = `manifest: ${error?.stack ?? error}`.split('\n').slice(0, 4).join('\n')
      }
    }
  } finally {
    await registrator.unregister().catch(() => {})
  }
}

const args = parseArgs(process.argv.slice(2))
const command = args._[0]
try {
  if (command === 'discover') await discover(args)
  else if (command === 'build') await build(args)
  else {
    process.stderr.write('usage: build.mjs <discover|build> [options]\n')
    process.exit(2)
  }
} catch (error) {
  emit({ ok: false, error: String(error?.stack ?? error) })
  process.exit(1)
}
