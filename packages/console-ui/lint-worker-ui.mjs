#!/usr/bin/env node
/**
 * Design-rule lint for a worker's injected console UI SOURCE — `styles.css`,
 * `page.tsx` and everything under `src/` (never `dist/`, `node_modules/` or
 * tests). The rules are the ones `ade/skills/design-system.md` states and the
 * `*-conformance` tests in ade/web already enforce for the Console itself.
 * Regex-based on purpose: no CSS/TS parser, no dependency — a rule is a
 * pattern and a hint, and a false positive is one `allow` entry away.
 * `buildWorkerUi` runs it after every non-watch build.
 *
 *   node lint-worker-ui.mjs <worker>/ui [--strict] [--json]
 *   node lint-worker-ui.mjs --all            (from the repo root)
 */

import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { basename, dirname, join, relative, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

/** rule id → [default level, hint]. `strict` promotes every warning. */
export const rules = Object.freeze({
  'no-window-dialogs': ['error', 'use useConfirm()/ConfirmDialog from @iii-dev/console-ui'],
  'icon-size': ['error', 'application icons are 16px'],
  'accent-selection': ['error', 'selection is neutral: --color-surface-selected + --color-ink (+ --color-edge)'],
  'no-inline-svg': ['warning', 'import from lucide-react'],
  radius: ['warning', 'one 6px radius'],
  'font-family': ['warning', 'use --font-sans/--font-mono/--font-code'],
  'font-size': ['warning', 'UI text floor is 11px'],
  'case-transform': ['warning', 'use the Eyebrow/.iii-ui-eyebrow recipe; no case transforms on interface copy'],
  'focus-stroke': ['warning', 'use --color-rule-focus'],
  shadow: ['warning', 'elevation is --shadow-raised/floating/lift/keycap'],
  'hex-color': ['warning', 'use tokens'],
  'motion-literal': ['warning', 'use --motion-duration-*'],
  'keyframes-shared': ['warning', 'use the shared iii-ui-spin / iii-ui-pulse recipes'],
  'viewport-media': ['warning', 'use @container'],
  'tailwind-in-worker': ['warning', 'worker markup has no Tailwind; use shared components/uiClasses/scoped CSS'],
})

const SELECTION_RE = /selected|\.active\b|current|is-on|is-active|aria-selected|aria-current|data-selected|data-state=/
const RADIUS_OK = /^(?:0|0px|6px|9999px|50%|inherit|initial|unset|var\(--radius-[\w-]+\))$/
const ICON_SIZE_RES = [
  // Mirrors ade/web/src/lib/icon-size-conformance.test.ts.
  /<[A-Z][^>]*\bsize=(?:\{(?:[0-9]|1[0-5])\}|["'](?:[0-9]|1[0-5])["'])/g,
  /\bsize\s*=\s*(?:[0-9]|1[0-5])\b/g,
  /<svg\b[^>]*\b(?:width|height)=(?:\{(?:[0-9]|1[0-5])\}|["'](?:[0-9]|1[0-5])["'])/g,
  /\b(?:size-(?:2\.5|3|3\.5)|w-(?:2\.5|3|3\.5)\s+h-(?:2\.5|3|3\.5)|h-(?:2\.5|3|3\.5)\s+w-(?:2\.5|3|3\.5))\b/g,
]
const TAILWIND_RE = /\b(?:flex|grid|items-center)\b|\b(?:px|py|gap)-\d|\btext-\[|\b(?:bg|rounded)-[a-z]/g

const blankOut = (m) => m.replace(/[^\n]/g, ' ')
/** Comments become same-length whitespace so every index still maps to its line. */
const stripCss = (text) => text.replace(/\/\*[\s\S]*?\*\//g, blankOut)
const stripTs = (text) => stripCss(text).replace(/(^|\s)\/\/.*$/gm, (m, lead) => lead + blankOut(m.slice(lead.length)))

/**
 * `[{ head, body, at }]` for every block, innermost first, from one linear
 * scan. `at` is the body's offset in the original text; a nested rule keeps
 * its own head (`&.selected`) and is blanked out of its parent's body, so a
 * parent sees only its own declarations.
 */
function cssBlocks(text) {
  const src = stripCss(text)
  const blocks = []
  const open = []
  let start = 0 // where the current head (or declaration) began
  for (let i = 0; i < src.length; i++) {
    const ch = src[i]
    if (ch === '{') {
      // A nested rule inherits its parent's selector (`.row.selected & .label`).
      const parent = open.at(-1)
      const own = src.slice(start, i).trim()
      open.push({ head: parent && !parent.head.startsWith('@') ? `${parent.head} ${own}` : own, headAt: start, at: i + 1, children: [] })
      start = i + 1
    } else if (ch === '}' || ch === ';') {
      if (ch === '}' && open.length) {
        const b = open.pop()
        let body = src.slice(b.at, i)
        for (const [s, e] of b.children) body = body.slice(0, s - b.at) + ' '.repeat(e - s) + body.slice(e - b.at)
        blocks.push({ head: b.head, body, at: b.at })
        open.at(-1)?.children.push([b.headAt, i + 1])
      }
      start = i + 1
    }
  }
  return blocks
}

/** The scope element itself (`[data-iii-ui="x"]`, plus theme variants), never a descendant. */
const isScopeRoot = (head) => head.split(',').every((s) => /^(?:\S+\s+)?\[data-iii-ui=[^\]]+\]\S*$/.test(s.trim()))
const topLevelParts = (value) => value.split(/,(?![^(]*\))/).map((s) => s.trim())

function sourceFiles(root) {
  const out = []
  const walk = (dir) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name)
      if (e.isDirectory()) {
        if (!/^(?:dist|node_modules|tests?|__tests__|\..*)$/.test(e.name)) walk(p)
      } else if (/\.(?:css|tsx?)$/.test(e.name) && !/\.test\.|\.d\.ts$/.test(e.name)) out.push(p)
    }
  }
  walk(root)
  return out.sort()
}

/** First `[data-iii-ui="x"]` in styles.css, else the worker dir name. */
export function inferScope(root) {
  const sheet = join(root, 'styles.css')
  const m = existsSync(sheet) && readFileSync(sheet, 'utf8').match(/\[data-iii-ui=["']?([\w-]+)/)
  return m ? m[1] : basename(root) === 'ui' ? basename(dirname(root)) : basename(root)
}

/**
 * Lint one worker UI dir. Returns `{ errors, warnings }` of
 * `{ rule, file, line, excerpt, hint }`. `disable` drops rules; `allow[rule]`
 * lists substrings/regexes of excerpts to ignore; `strict` makes everything
 * an error.
 */
export function lintWorkerUi({ root = process.cwd(), scope = inferScope(root), strict = false, disable = [], allow = {} } = {}) {
  root = resolve(root)
  const errors = []
  const warnings = []
  for (const path of sourceFiles(root)) {
    const text = readFileSync(path, 'utf8')
    const file = relative(root, path)
    const css = path.endsWith('.css')
    const tsx = path.endsWith('.tsx')
    const lines = text.split('\n')
    const report = (rule, index, excerpt) => {
      if (disable.includes(rule)) return
      const line = text.slice(0, index).split('\n').length
      // `lint-allow <rule>` in a comment on the finding's line or the one above.
      if (lines.slice(Math.max(0, line - 2), line).some((l) => l.includes(`lint-allow ${rule}`))) return
      excerpt = (excerpt ?? lines[line - 1]).trim().replace(/\s+/g, ' ').slice(0, 120)
      if ((allow[rule] ?? []).some((a) => (typeof a === 'string' ? excerpt.includes(a) : a.test(excerpt)))) return
      const level = strict ? 'error' : rules[rule][0]
      ;(level === 'error' ? errors : warnings).push({ rule, file, line, excerpt, hint: rules[rule][1] })
    }
    const each = (re, src, fn) => {
      for (const m of src.matchAll(re)) fn(m)
    }

    if (css) {
      const src = stripCss(text)
      each(/@keyframes\s+([\w-]*(?:spin|pulse|shimmer|fade))\b/gi, src, (m) => report('keyframes-shared', m.index, m[0]))
      for (const { head, body, at } of cssBlocks(text)) {
        const focus = /:focus/.test(head)
        if (SELECTION_RE.test(head) && !focus && /var\(--color-accent/.test(body)) report('accent-selection', at, head)
        const root = isScopeRoot(head)
        // Anchored to a declaration start: a free `[\w-]+` would backtrack
        // quadratically through a long data: URI.
        each(/(?:^|;)\s*([\w-]+)\s*:\s*([^;]+)/g, body, (d) => {
          const [, prop, raw] = d
          const value = raw.trim()
          const decl = `${prop}: ${value}`
          const index = at + d.index + d[0].indexOf(prop)
          if (/^border(?:-[a-z]+)*-radius$/.test(prop) && !value.split(/[\s/]+/).every((v) => RADIUS_OK.test(v))) report('radius', index, decl)
          if (prop === 'font-family' && !/^(?:var\(--font-|inherit)/.test(value)) report('font-family', index, decl)
          if (prop === 'font-size') {
            const px = value.match(/^(\d*\.?\d+)px$/)
            const rem = value.match(/^(\d*\.?\d+)rem$/)
            if ((px && +px[1] < 11) || (rem && +rem[1] < 0.6875)) report('font-size', index, decl)
          }
          if (prop === 'text-transform' && /uppercase|lowercase|capitalize/.test(value)) report('case-transform', index, decl)
          if (focus && /^(?:outline|box-shadow|border)/.test(prop) && value.includes('var(--color-accent')) report('focus-stroke', index, decl)
          if (prop === 'box-shadow') {
            const ok = (p) =>
              /^(?:var\(--shadow-[\w-]+\)|none|0|inherit|initial|unset)$/.test(p) ||
              (p.includes('var(--color-') && (/\binset\b/.test(p) || /^0 0 0 [12]px /.test(p)))
            if (!topLevelParts(value).every(ok)) report('shadow', index, decl)
          }
          if (!(root && prop.startsWith('--')) && !value.includes('url(') && /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?)\(/i.test(value)) report('hex-color', index, decl)
          if (/^(?:transition|animation)(?:-duration|-delay)?$/.test(prop) && /\b\d*\.?\d+m?s\b/.test(value.replace(/var\([^)]*\)/g, ''))) report('motion-literal', index, decl)
        })
      }
    } else {
      const src = stripTs(text)
      each(/\bwindow\.(?:confirm|alert|prompt)\s*\(/g, src, (m) => report('no-window-dialogs', m.index))
      if (tsx) {
        for (const re of ICON_SIZE_RES) each(re, src, (m) => report('icon-size', m.index))
        if (!/(?:^|\/)icons?\.tsx$|\/icons\//.test(file)) each(/<svg\b/g, src, (m) => report('no-inline-svg', m.index))
        each(/textTransform:\s*['"](?:uppercase|lowercase|capitalize)/g, src, (m) => report('case-transform', m.index))
        each(/className=(?:"[^"\n]*"|'[^'\n]*'|\{[^}\n]*\})/g, src, (m) => {
          if ((m[0].match(TAILWIND_RE) ?? []).length >= 3) report('tailwind-in-worker', m.index)
        })
      }
    }
    each(/@media[^{;\n]*\((?:max|min)-width/g, css ? stripCss(text) : stripTs(text), (m) => report('viewport-media', m.index))
  }
  return { errors, warnings }
}

/** Findings grouped by rule (up to `max` examples each) plus a summary line, `[worker-ui]`-prefixed. */
export function formatLint({ errors, warnings }, { strict = false, max = 5 } = {}) {
  const lines = []
  const byRule = new Map()
  for (const f of [...errors, ...warnings]) byRule.set(f.rule, [...(byRule.get(f.rule) ?? []), f])
  for (const [rule, list] of [...byRule].sort((a, b) => b[1].length - a[1].length)) {
    lines.push(`[worker-ui] ${rule} (${list.length}): ${rules[rule][1]}`)
    for (const f of list.slice(0, max)) lines.push(`  ${f.file}:${f.line}  ${f.excerpt}`)
    if (list.length > max) lines.push(`  +${list.length - max} more`)
  }
  lines.push(`[worker-ui] lint: ${errors.length} error${errors.length === 1 ? '' : 's'}, ${warnings.length} warning${warnings.length === 1 ? '' : 's'} (strict: ${strict ? 'on' : 'off'})`)
  return lines.join('\n')
}

const topRules = (result, n = 3) => {
  const counts = new Map()
  for (const f of [...result.errors, ...result.warnings]) counts.set(f.rule, (counts.get(f.rule) ?? 0) + 1)
  return [...counts]
    .sort((a, b) => b[1] - a[1])
    .slice(0, n)
    .map(([r, c]) => `${r} ${c}`)
    .join(', ')
}

function main(argv) {
  const strict = argv.includes('--strict')
  const flags = argv.filter((a) => a.startsWith('--'))
  const args = argv.filter((a) => !a.startsWith('--'))
  if (flags.includes('--all')) {
    const rows = readdirSync('.', { withFileTypes: true })
      .filter((e) => e.isDirectory() && existsSync(join(e.name, 'ui/package.json')))
      .map((e) => [e.name, lintWorkerUi({ root: join(e.name, 'ui'), strict })])
      .sort((a, b) => b[1].warnings.length - a[1].warnings.length || b[1].errors.length - a[1].errors.length)
    const total = { errors: rows.flatMap((r) => r[1].errors), warnings: rows.flatMap((r) => r[1].warnings) }
    rows.push([`total (${rows.length})`, total])
    const w = Math.max(...rows.map((r) => r[0].length))
    console.log(`${'worker'.padEnd(w)} | errors | warnings | top rules`)
    console.log(`${'-'.repeat(w)}-|--------|----------|----------`)
    for (const [name, r] of rows) console.log(`${name.padEnd(w)} | ${String(r.errors.length).padStart(6)} | ${String(r.warnings.length).padStart(8)} | ${topRules(r)}`)
    process.exitCode = total.errors.length ? 1 : 0
    return
  }
  const root = resolve(args[0] ?? '.')
  const result = lintWorkerUi({ root, strict })
  console.log(flags.includes('--json') ? JSON.stringify(result, null, 2) : formatLint(result, { strict }))
  process.exitCode = result.errors.length ? 1 : 0
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main(process.argv.slice(2))
