/**
 * Build the worker's two console assets:
 *
 *   page.tsx   → dist/page.js    (injected over `console:script`)
 *   styles.css → dist/styles.css (injected over `console:style`)
 *
 * The five shared specifiers stay EXTERNAL — they resolve at runtime
 * through the console's import map (a bundled React copy would surface as
 * a cryptic "Invalid hook call"). Everything else the page needs gets
 * bundled in. `--watch` pairs with the worker's III_HARNESS_UI_WATCH
 * poller for the hot-reload dev loop.
 */

import { readFileSync } from 'node:fs'
import esbuild from 'esbuild'

const watch = process.argv.includes('--watch')

const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  // The bundle is `include_str!`'d into the worker binary and the injectable-UI
  // protocol rejects an asset over 8 MiB outright, so ship it minified. Left
  // off under --watch, where readable stack traces matter more than bytes.
  minify: !watch,
  external: [
    'react',
    'react-dom',
    'react-dom/client',
    'react/jsx-runtime',
    '@iii-dev/console-ui',
  ],
  logLevel: 'info',
}

// Minification strips the quotes from an attribute selector, so the scope
// prefix has to be matched in both spellings.
const SCOPE = '[data-iii-ui="harness"]'
const SCOPE_RE = /^\[data-iii-ui=("harness"|'harness'|harness)\]/
const KEYFRAME_PREFIXES = ['harness-ui-']

/**
 * Fail the build if any rule could escape this worker's subtree.
 *
 * The console mounts every worker's UI into one document, so a single
 * unscoped selector restyles the whole app. The Rust side already asserts
 * that the scope string appears *somewhere* in the sheet, which a file
 * containing one stray `body { }` would also pass. This checks every
 * selector, and runs in CI for free because CI runs `pnpm build`.
 */
function assertScoped(css) {
  // Strip comments, then strings, so a brace or selector inside either can't
  // desynchronise the scan.
  const src = css
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, '""')

  const problems = []
  let depth = 0
  let buf = ''
  // Nested at-rules (@media, @container, @supports) open a block whose
  // children are still top-level selectors, so track them rather than
  // skipping their contents.
  const atStack = []

  for (let i = 0; i < src.length; i++) {
    const ch = src[i]
    if (ch === '{') {
      const head = buf.trim()
      buf = ''
      if (head.startsWith('@')) {
        const name = head.split(/[\s(]/)[0]
        atStack.push(name)
        if (name === '@keyframes') {
          const kf = head.slice('@keyframes'.length).trim()
          if (!KEYFRAME_PREFIXES.some((p) => kf.startsWith(p))) {
            problems.push(`@keyframes ${kf} is not prefixed ${KEYFRAME_PREFIXES.join(' or ')}`)
          }
        } else if (name === '@font-face') {
          problems.push('@font-face is not allowed in an injected sheet')
        }
        depth++
        continue
      }
      // Inside @keyframes the "selectors" are 0%/from/to, not real ones.
      if (!atStack.includes('@keyframes') && head) {
        for (const sel of head.split(',')) {
          const s = sel.trim()
          if (s && !SCOPE_RE.test(s)) {
            problems.push(`selector not scoped: ${s}`)
          }
        }
      }
      depth++
    } else if (ch === '}') {
      depth--
      buf = ''
      if (atStack.length && depth < atStack.length) atStack.pop()
    } else {
      buf += ch
    }
  }

  if (problems.length) {
    console.error(`\n${problems.length} unscoped rule(s) in dist/styles.css:\n`)
    for (const p of [...new Set(problems)]) console.error(`  ${p}`)
    console.error(`\nEvery rule must start with ${SCOPE}.\n`)
    process.exit(1)
  }
}

if (watch) {
  const ctx = await esbuild.context(options)
  await ctx.watch()
} else {
  await esbuild.build(options)
  assertScoped(readFileSync('dist/styles.css', 'utf8'))
}
