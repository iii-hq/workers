#!/usr/bin/env node
// Dead-class check in both directions for a worker UI:
//   node packages/console-ui/dead-classes.mjs <worker>/ui [prefix]
// Lists (a) prefixed class tokens the TS/TSX uses that no CSS selector
// defines, and (b) prefixed CSS selectors nothing in the code references.
// Heuristic (regex, prefix-based): confirm each hit with grep before deleting.
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

const root = process.argv[2]
if (!root) {
  console.error('usage: dead-classes.mjs <worker>/ui [prefix]')
  process.exit(2)
}
const files = []
const walk = (d) => {
  for (const e of readdirSync(d, { withFileTypes: true })) {
    const p = join(d, e.name)
    if (e.isDirectory()) {
      if (!/^(node_modules|dist|\..*)$/.test(e.name)) walk(p)
    } else if (/\.(css|tsx?|mjs)$/.test(e.name) && !/\.test\./.test(e.name) && !/\.d\.ts$/.test(e.name)) files.push(p)
  }
}
walk(root)
const css = files
  .filter((f) => f.endsWith('.css'))
  .map((f) => readFileSync(f, 'utf8'))
  .join('\n')
  .replace(/\/\*[\s\S]*?\*\//g, '')
const code = files
  .filter((f) => !f.endsWith('.css'))
  .map((f) => readFileSync(f, 'utf8'))
  .join('\n')
const prefix =
  process.argv[3] ??
  (() => {
    const c = new Map()
    for (const m of css.matchAll(/\.([a-z][a-z0-9]*-)[a-z0-9-]*/g)) c.set(m[1], (c.get(m[1]) ?? 0) + 1)
    return [...c].sort((a, b) => b[1] - a[1])[0]?.[0] ?? ''
  })()
const esc = prefix.replace(/[-]/g, '\\-')
const selectors = new Set([...css.matchAll(/\.([A-Za-z_][\w-]*)/g)].map((m) => m[1]).filter((n) => n.startsWith(prefix)))
const used = new Set([...code.matchAll(new RegExp(`\\b(${esc}[\\w-]*)`, 'g'))].map((m) => m[1]))
// `${prefix}foo-${x}`: a template stem covers every selector that starts with it.
const dynamic = [...code.matchAll(new RegExp(`(${esc}[\\w-]*)-\\$\\{`, 'g'))].map((m) => `${m[1]}-`)
const usedNoCss = [...used].filter((u) => !selectors.has(u) && !u.endsWith('-')).sort()
const cssNoUse = [...selectors].filter((s) => !used.has(s) && !dynamic.some((d) => s.startsWith(d))).sort()
console.log(`prefix "${prefix}": ${selectors.size} selectors in CSS, ${used.size} prefixed tokens in code`)
console.log(`\nused in code, no selector (${usedNoCss.length}):\n  ${usedNoCss.join('\n  ')}`)
console.log(`\nselector without use (${cssNoUse.length}):\n  ${cssNoUse.join('\n  ')}`)
