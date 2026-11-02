/**
 * The kanban page's own Tailwind build.
 *
 * Worker UIs are injected into the console as a separate bundle, so the console's Tailwind never sees their
 * class names. This compiles the utilities the kanban sources use (no preflight: the console already resets),
 * with the default theme inlined — `bg-neutral-900` is a literal colour, no `:root` variables — and every
 * selector prefixed with `[data-iii-ui="kanban"]`, so nothing leaks into the rest of the console. The result
 * lands in `.tailwind/kanban.css`, which `styles.css` imports; it is generated, never edited.
 *
 * The console's tokens are mapped in as colours (`text-ink`, `bg-panel-raised`, `border-edge`…), so Tailwind
 * classes and the hand-written rules share one palette. `dark:` follows the console's `html[data-theme]`.
 */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { compile } from '@tailwindcss/node'
import { Scanner } from '@tailwindcss/oxide'
import postcss from 'postcss'

const ROOT = dirname(fileURLToPath(import.meta.url))
const SCOPE = '[data-iii-ui="kanban"]'
export const OUTPUT = join(ROOT, '.tailwind', 'kanban.css')

const TOKENS = [
  'bg',
  'panel',
  'panel-raised',
  'surface',
  'surface-hover',
  'surface-selected',
  'ink',
  'ink-faint',
  'ink-ghost',
  'ink-disabled',
  'edge',
  'accent',
  'accent-fg',
  'alert',
  'warn',
  'ok',
  'glyph-blue',
  'glyph-purple',
  'glyph-teal',
  'glyph-green',
  'glyph-amber',
  'glyph-rose',
]

const INPUT = `@import "tailwindcss/theme.css" theme(reference inline) prefix(tw);
@import "tailwindcss/utilities.css";
@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));
@theme inline reference {
${TOKENS.map((name) => `  --color-${name}: var(--color-${name});`).join('\n')}
}
@source "./src";
@source "./page.tsx";
`

/** Prefix every rule with the worker scope; keyframe steps are left alone. */
const scopePlugin = {
  postcssPlugin: 'kanban-scope',
  Rule(rule) {
    if (rule.parent?.type === 'atrule' && /keyframes$/i.test(rule.parent.name)) return
    if (rule.selector.startsWith(SCOPE)) return
    rule.selectors = rule.selectors.map((selector) =>
      selector === ':root' || selector === ':host' ? SCOPE : `${SCOPE} ${selector}`,
    )
  },
}

export async function buildTailwind() {
  const compiler = await compile(INPUT, { base: ROOT, onDependency() {} })
  const scanner = new Scanner({ sources: compiler.sources })
  const css = compiler.build(scanner.scan())
  const scoped = postcss([scopePlugin]).process(css, { from: undefined }).css
  mkdirSync(dirname(OUTPUT), { recursive: true })
  let previous = ''
  try {
    previous = readFileSync(OUTPUT, 'utf8')
  } catch {
    previous = ''
  }
  // Rewrite only on change, so a watch build does not loop on its own output.
  if (previous !== scoped) writeFileSync(OUTPUT, scoped)
  return OUTPUT
}

/** esbuild plugin: regenerate the utilities before every build (watch builds included). */
export const tailwindPlugin = {
  name: 'kanban-tailwind',
  setup(build) {
    build.onStart(async () => {
      await buildTailwind()
    })
  },
}
