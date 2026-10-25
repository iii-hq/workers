import { readdirSync, readFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

/**
 * The console's `accent` token is a FILL (charcoal on cream, soft white on
 * near-black), not shadcn's hover tint. A stock shadcn recipe that paints
 * `bg-accent` under hovered or focused text therefore puts the ink colour on
 * a near-ink background — the white-on-white (dark) / black-on-charcoal
 * (light) hover bug. Interactive hover and focus washes belong on `muted`
 * (or `surface-hover`); `accent` as a background is reserved for live
 * state, gauges and primary actions, where the text is `accent-foreground`
 * / `accent-fg`.
 *
 * This is a ratchet over `src`: no interactive-state utility may paint
 * `bg-accent` (opacity-modified fills like `bg-accent/60` on a bare drag
 * handle carry no text and are allowed).
 */
const srcRoot = fileURLToPath(new URL('../', import.meta.url))

/**
 * A bare drag rail or a demo page may fill with the accent under the
 * pointer: there is no text on it to lose. Everything else is a defect.
 */
const BARE_FILL_ALLOWLIST: Record<string, string> = {
  'components/ui/PageSidebar.tsx':
    'the sidebar resize rail — a 1px line, no text',
  'demo/LandingDemo.tsx': 'the marketing demo, outside the console chrome',
}

const STATE_PREFIXES = [
  'hover:',
  'focus:',
  'focus-visible:',
  'active:',
  'data-highlighted:',
  'data-[highlighted]:',
  'data-[state=open]:',
  'aria-selected:',
  'aria-expanded:',
]

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name)
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name === 'stories') continue
      walk(full, out)
    } else if (
      /\.(tsx|ts|css)$/.test(entry.name) &&
      !/\.test\./.test(entry.name)
    ) {
      out.push(full)
    }
  }
  return out
}

describe('theme hover conformance', () => {
  it('maps shadcn accent-foreground onto the accent fill foreground', () => {
    const css = readFileSync(join(srcRoot, 'index.css'), 'utf8')
    expect(css).toMatch(/--color-accent-foreground:\s*var\(--color-accent-fg\)/)
  })

  it('never paints the accent fill as an interactive hover or focus wash', () => {
    const offenders: string[] = []
    for (const file of walk(srcRoot)) {
      if (BARE_FILL_ALLOWLIST[relative(srcRoot, file)]) continue
      const text = readFileSync(file, 'utf8')
      for (const prefix of STATE_PREFIXES) {
        // `bg-accent` exactly — `bg-accent/60`, `bg-accent-muted` and
        // `bg-accent-fg` are different utilities.
        const re = new RegExp(
          `${prefix.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}bg-accent(?![-/\\w])`,
          'g',
        )
        if (re.test(text))
          offenders.push(`${relative(srcRoot, file)} (${prefix}bg-accent)`)
      }
      if (
        /text-accent-foreground/.test(text) &&
        !/bg-accent(?![-/\w])/.test(text)
      ) {
        offenders.push(
          `${relative(srcRoot, file)} (text-accent-foreground without an accent fill)`,
        )
      }
    }
    expect(offenders).toEqual([])
  })
})
