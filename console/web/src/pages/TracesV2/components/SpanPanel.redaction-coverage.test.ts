/**
 * Guard-rail against the exact failure this panel already had once:
 * `SpanTagsTab`/`SpanLogsTab`/`SpanErrorsTab` all render span-attribute or
 * span-event data that can carry a redacted worker's runtime_id, and each
 * one had to be found and fixed separately (console-UI review, three
 * rounds). `SpanPanel` computes one `redact` and CAN thread it into any
 * tab, but nothing stops a future tab from being added without it — the
 * type system doesn't know which tabs need it, and "the other tabs already
 * got fixed" reads as proof the panel was audited even when a new sibling
 * slipped in unreviewed.
 *
 * This test reads `SpanPanel.tsx`'s own source (no jsdom, no rendering —
 * console/web's convention is pure-function tests) and enforces a closed
 * world: every `<Span*Tab` it renders must be on the list below, and must
 * be either wired to `redact={redact}` or carry a written reason in ITS
 * OWN file for why a runtime_id cannot reach it. Add a tab without doing
 * one of those two things and this test fails with the tab's name in the
 * message — loud, not silent.
 */

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const DIR = dirname(fileURLToPath(import.meta.url))
const PANEL_SRC = readFileSync(join(DIR, 'SpanPanel.tsx'), 'utf8')

/** Every `Span*Tab` component SpanPanel's JSX actually renders. */
function tabsRenderedByPanel(): string[] {
  const names = new Set<string>()
  for (const m of PANEL_SRC.matchAll(/<(Span\w*Tab)\b/g)) names.add(m[1])
  return [...names].sort()
}

/** The full `<ComponentName ...>` opening-tag text, so a prop check can't
 * accidentally match a DIFFERENT tab's props elsewhere in the file. */
function openingTagOf(componentName: string): string {
  const start = PANEL_SRC.indexOf(`<${componentName}`)
  if (start === -1) return ''
  const end = PANEL_SRC.indexOf('>', start)
  return end === -1 ? '' : PANEL_SRC.slice(start, end + 1)
}

type Disposition =
  /** Must receive `redact={redact}` straight from SpanPanel. */
  | { kind: 'redact-prop' }
  /** Doesn't take `redact` from SpanPanel because it redacts itself — the
   * file named here must say so and must mention redaction. */
  | { kind: 'self-redacted'; file: string; mustMention: string }
  /** A runtime_id cannot reach this tab at all — the file named here must
   * carry the reasoning, not just this test. */
  | { kind: 'exempt'; file: string; mustMention: string }

/**
 * One entry per tab SpanPanel is allowed to render. Keep this in the same
 * order as SpanPanel.tsx's TabsContent list so a diff against that file is
 * easy to eyeball.
 */
const TAB_DISPOSITIONS: Record<string, Disposition> = {
  SpanInfoTab: {
    kind: 'self-redacted',
    // The info tab's FunctionTriggerCard resolves its own redactor via
    // rawRedactor — covered by redact-raw.test.tsx, not this file.
    file: 'SpanInfoTab.tsx',
    mustMention: 'FunctionTriggerCard',
  },
  SpanTagsTab: { kind: 'redact-prop' },
  SpanLogsTab: { kind: 'redact-prop' },
  SpanErrorsTab: { kind: 'redact-prop' },
  SpanOtelLogsTab: {
    kind: 'exempt',
    file: 'SpanOtelLogsTab.tsx',
    mustMention: 'redact',
  },
  SpanBaggageTab: {
    kind: 'exempt',
    file: 'SpanBaggageTab.tsx',
    mustMention: 'redact',
  },
  SpanLinksTab: {
    kind: 'exempt',
    file: 'SpanLinksTab.tsx',
    mustMention: 'redact',
  },
}

describe('every tab SpanPanel renders has a redaction disposition', () => {
  it('has no tab that is neither listed here nor wired to redact', () => {
    const found = tabsRenderedByPanel()
    const known = new Set(Object.keys(TAB_DISPOSITIONS))
    const unlisted = found.filter((name) => !known.has(name))
    expect(
      unlisted,
      `SpanPanel.tsx renders ${unlisted.join(', ')} but SpanPanel.redaction-coverage.test.ts ` +
        `doesn't know it. Either thread redact={redact} into it, or add a written reason ` +
        `in its own file for why a runtime_id can't reach it — then add it to TAB_DISPOSITIONS.`,
    ).toEqual([])
  })

  it('has no stale entry for a tab SpanPanel no longer renders', () => {
    const found = new Set(tabsRenderedByPanel())
    const stale = Object.keys(TAB_DISPOSITIONS).filter(
      (name) => !found.has(name),
    )
    expect(
      stale,
      `TAB_DISPOSITIONS lists ${stale.join(', ')}, which SpanPanel.tsx no longer renders — ` +
        `update this guard so it can't hide a real gap behind a dead entry.`,
    ).toEqual([])
  })

  for (const [name, disposition] of Object.entries(TAB_DISPOSITIONS)) {
    if (disposition.kind === 'redact-prop') {
      it(`${name}: SpanPanel passes redact={redact} to it`, () => {
        const tag = openingTagOf(name)
        expect(tag, `<${name} ...> not found in SpanPanel.tsx`).not.toBe('')
        expect(
          /redact=\{redact\}/.test(tag),
          `<${name} ...> in SpanPanel.tsx does not pass redact={redact}:\n${tag}`,
        ).toBe(true)
      })
    } else {
      it(`${name}: ${disposition.kind === 'exempt' ? 'exemption' : 'self-redaction'} is written in ${disposition.file}, not just this test`, () => {
        const src = readFileSync(join(DIR, disposition.file), 'utf8')
        expect(
          new RegExp(disposition.mustMention, 'i').test(src),
          `${disposition.file} has no visible reasoning about "${disposition.mustMention}" — ` +
            `a disposition asserted only in this test, not in the component's own file, is a ` +
            `silent gap the next reader won't see.`,
        ).toBe(true)
      })
    }
  }
})
