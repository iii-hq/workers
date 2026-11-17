import { describe, expect, it } from 'vitest'

// Typed by hand: the worker UI tsconfig carries no Node types.
const fs: { readFileSync(path: URL, encoding: 'utf8'): string } = await import('node:fs' as string)
const css = fs.readFileSync(new URL('../../styles.css', import.meta.url), 'utf8')

/** Every declaration block for exactly `[data-iii-ui="browser"] .br-ui-pip`. */
function pipBlocks(): string[] {
  const blocks: string[] = []
  const rule = /\[data-iii-ui="browser"\] \.br-ui-pip \{([^}]*)\}/g
  for (let m = rule.exec(css); m; m = rule.exec(css)) blocks.push(m[1])
  return blocks
}

// QA 2026-11-16: docked at a fixed 72px from the viewport top, the deck sat
// on the pane header row (the console's chrome ends at 93px on desktop,
// 113px on phones) and covered the chat's split and close controls and the
// context panel's tabs.
describe('live preview anchor', () => {
  it('docks below the console chrome the overlay layer reports', () => {
    const tops = pipBlocks()
      .flatMap((block) => [...block.matchAll(/(?:^|;|\n)\s*top:\s*([^;]+);/g)])
      .map((m) => m[1].trim())
    expect(tops).toHaveLength(1)
    expect(tops[0]).toContain('var(--overlay-inset-top')
  })

  it('never re-anchors to a fixed corner offset on another layout', () => {
    for (const block of pipBlocks()) {
      expect(block).not.toMatch(/(?:^|\n)\s*top:\s*(?:auto|\d)/)
      expect(block).not.toMatch(/(?:^|\n)\s*bottom:/)
    }
  })
})
