/**
 * The one esbuild driver every worker's injected console UI runs:
 *
 *   page.tsx   → dist/page.js    (injected over `console:script`)
 *   styles.css → dist/styles.css (injected over `console:style`)
 *
 * The six shared specifiers stay EXTERNAL — they resolve at runtime through
 * the console's import map (a bundled React copy surfaces as a cryptic
 * "Invalid hook call"). Everything else, including this package's own
 * `/format` and `/hooks` subpaths, is bundled in. After every build the
 * output is checked: each sheet must be scoped to the worker, no asset may
 * pass the console's 8 MiB cap, and an unknown design token fails the build.
 * A non-watch build then lints the SOURCE against the design rules
 * (lint-worker-ui.mjs): errors fail, warnings print.
 * `--watch` pairs with the worker's III_<NAME>_UI_WATCH poller.
 */

import { readFileSync, statSync } from 'node:fs'
import { resolve } from 'node:path'
import esbuild from 'esbuild'
import { formatLint, lintWorkerUi } from './lint-worker-ui.mjs'
import { tokenNames } from './token-names.mjs'

const ASSET_CAP = 8 * 1024 * 1024
const TOKEN_PREFIX = /^--(color|shadow|radius|motion|font|spacing|ease)-/

/** Served by the console's import map; never bundled into a worker asset. */
export const workerUiExternals = Object.freeze([
  'react',
  'react-dom',
  'react-dom/client',
  'react/jsx-runtime',
  '@iii-dev/console-ui',
  'lucide-react',
])

/**
 * Exact-match externals. esbuild's `external` list also externalizes every
 * subpath of a package, which would leave `@iii-dev/console-ui/format` and
 * `/hooks` as bare specifiers the import map cannot serve.
 */
export function workerUiExternalsPlugin(extra = []) {
  const names = [...workerUiExternals, ...extra].map((n) => n.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&'))
  const filter = new RegExp(`^(${names.join('|')})$`)
  return {
    name: 'worker-ui-externals',
    setup(build) {
      build.onResolve({ filter }, (args) => ({ path: args.path, external: true }))
    },
  }
}

// Split a selector list on top-level commas only (`:is(.a, .b)` stays whole).
function splitSelectors(head) {
  const out = []
  let depth = 0
  let cur = ''
  for (const ch of head) {
    if (ch === '(') depth++
    else if (ch === ')') depth--
    if (ch === ',' && depth === 0) {
      out.push(cur.trim())
      cur = ''
    } else cur += ch
  }
  out.push(cur.trim())
  return out.filter(Boolean)
}

/**
 * Throw unless every rule in `css` could only ever match inside this
 * worker's subtree. The console mounts every worker's UI into one document,
 * so a single unscoped selector restyles the whole app. Top-level selectors
 * must start with `[data-iii-ui="<scope>"]` (nested rules inherit it),
 * `@keyframes` names — global even inside the scope — must carry one of
 * `keyframePrefixes`, and `@font-face` is refused. `allowUnscopedSelectors`
 * lists selector prefixes that are deliberately global (portal roots).
 */
export function assertScoped(css, { scope, keyframePrefixes = [scope, `${scope}-ui`], allowUnscopedSelectors = [], file = 'styles.css' }) {
  // Minification strips the quotes from an attribute selector, so match both.
  const scopeRe = new RegExp(`^\\[data-iii-ui=("${scope}"|'${scope}'|${scope})\\]`)
  // Strip comments, then strings, so a brace or selector inside either can't
  // desynchronise the scan — except the scope string itself, which the
  // selector check needs intact in unminified output.
  const src = css
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, (m) => (m.slice(1, -1) === scope ? m : '""'))
  const problems = []
  const stack = [] // one entry per open block: 'rule' | 'keyframes' | 'at'
  let buf = ''
  for (const ch of src) {
    if (ch === '{') {
      const head = buf.trim()
      buf = ''
      if (head.startsWith('@')) {
        const name = head.split(/[\s(]/)[0]
        if (name === '@keyframes') {
          const kf = head.slice('@keyframes'.length).trim()
          if (!keyframePrefixes.some((p) => kf.startsWith(p))) problems.push(`@keyframes ${kf} is not prefixed ${keyframePrefixes.join(' or ')}`)
        } else if (name === '@font-face') problems.push('@font-face is not allowed in an injected sheet')
        stack.push(name === '@keyframes' ? 'keyframes' : 'at')
        continue
      }
      // Inside @keyframes the "selectors" are 0%/from/to; inside a rule they
      // are nested and already carry the parent's scope.
      if (head && !stack.includes('keyframes') && !stack.includes('rule')) {
        for (const s of splitSelectors(head)) {
          if (!scopeRe.test(s) && !allowUnscopedSelectors.some((p) => s.startsWith(p))) problems.push(`selector not scoped: ${s}`)
        }
      }
      stack.push('rule')
    } else if (ch === '}') {
      stack.pop()
      buf = ''
    } else if (ch === ';') buf = '' // declarations and statement at-rules (@import) never open a block
    else buf += ch
  }
  if (problems.length) {
    const list = [...new Set(problems)].map((p) => `  ${p}`).join('\n')
    throw new Error(`${problems.length} unscoped rule(s) in ${file}:\n${list}\nEvery rule must start with [data-iii-ui="${scope}"].`)
  }
}

/**
 * Report `var(--color-…)`-style references (the seven design-token
 * families) that are neither in the console's public inventory nor declared
 * in the worker's own CSS. `files` is `[[name, text], …]` of the built
 * assets. Returns `[{ name, uses }]`; `strict` throws when any are found.
 */
export function checkTokens(files, { strict = false, known = tokenNames } = {}) {
  const declared = new Set(known)
  const uses = new Map()
  for (const [file, text] of files) {
    if (file.endsWith('.css')) for (const m of text.matchAll(/(--[\w-]+)\s*:/g)) declared.add(m[1])
    for (const m of text.matchAll(/var\(\s*(--[\w-]+)/g)) {
      if (TOKEN_PREFIX.test(m[1])) uses.set(m[1], (uses.get(m[1]) ?? 0) + 1)
    }
  }
  const unknown = [...uses].filter(([name]) => !declared.has(name)).map(([name, n]) => ({ name, uses: n }))
  for (const { name, uses: n } of unknown) console.warn(`[worker-ui] unknown token ${name} (${n} use${n === 1 ? '' : 's'})`)
  if (strict && unknown.length) throw new Error(`[worker-ui] ${unknown.length} unknown design token(s) referenced`)
  return unknown
}

/**
 * Build (or `--watch`) a worker UI. `scope` is the `data-iii-ui` value the
 * console wraps the worker's render in — the first asset path segment,
 * normally the worker name. Paths resolve against `root` (default
 * `process.cwd()`, the `ui/` dir `pnpm build` runs in; pass
 * `import.meta.dirname` when the script is invoked from elsewhere). Returns
 * the esbuild result, or the watch context. A failed check exits 1 (a watch
 * just reports and keeps going).
 */
export async function buildWorkerUi({
  scope,
  entryPoints = ['page.tsx', 'styles.css'],
  outdir = 'dist',
  root = process.cwd(),
  watch = process.argv.includes('--watch'),
  // The bundle is embedded in the worker binary and the injectable-UI
  // protocol rejects an asset over 8 MiB, so ship it minified; a watch keeps
  // readable stack traces.
  minify = !watch,
  keyframePrefixes,
  allowUnscopedSelectors,
  strictTokens = true,
  // `false` skips the design-rule lint; `{ strict, disable, allow }` tunes it.
  lint = {},
  plugins = [],
  extraExternal = [],
  define,
} = {}) {
  if (!scope) throw new Error('[worker-ui] `scope` is required: the data-iii-ui value, normally the worker name')
  const checks = {
    name: 'worker-ui-checks',
    setup(build) {
      build.onEnd((result) => {
        if (result.errors.length) return
        const files = Object.keys(result.metafile.outputs).map((f) => [f, readFileSync(resolve(root, f), 'utf8')])
        try {
          for (const [file, text] of files) {
            if (statSync(resolve(root, file)).size >= ASSET_CAP) throw new Error(`[worker-ui] ${file} exceeds the console's 8 MiB asset cap`)
            if (file.endsWith('.css')) assertScoped(text, { scope, keyframePrefixes, allowUnscopedSelectors, file })
          }
          checkTokens(files, { strict: strictTokens })
          if (!watch && lint !== false) {
            const result = lintWorkerUi({ root, scope, ...lint })
            console.warn(formatLint(result, { strict: lint.strict }))
            if (result.errors.length) throw new Error(`[worker-ui] ${result.errors.length} design-rule error(s), listed above`)
          }
        } catch (err) {
          console.error(`\n${err.message}\n`)
          if (!watch) process.exit(1)
        }
      })
    },
  }
  const options = {
    entryPoints,
    outdir,
    absWorkingDir: root,
    bundle: true,
    format: 'esm',
    jsx: 'automatic',
    logLevel: 'info',
    minify,
    metafile: true,
    define,
    plugins: [workerUiExternalsPlugin(extraExternal), ...plugins, checks],
  }
  if (!watch) return esbuild.build(options)
  const ctx = await esbuild.context(options)
  await ctx.watch()
  return ctx
}
