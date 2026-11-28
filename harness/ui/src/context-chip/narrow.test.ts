import { describe, expect, it } from 'vitest'

// Typed by hand: the worker UI tsconfig carries no Node types.
const fs: { readFileSync(path: URL, encoding: 'utf8'): string } = await import('node:fs' as string)
const css = fs.readFileSync(new URL('../../styles.css', import.meta.url), 'utf8')

/** The rule bodies inside each `@container (max-width: N)` block, by N. */
function containerRules(): Map<string, string> {
  const blocks = new Map<string, string>()
  const rule = /@container \(max-width: ([\d.]+rem)\) \{([\s\S]*?)\n\}/g
  for (let m = rule.exec(css); m; m = rule.exec(css)) {
    blocks.set(m[1], `${blocks.get(m[1]) ?? ''}${m[2]}`)
  }
  return blocks
}

// QA 2026-11-16: in a ~245px chat pane the chip's "21.4k / 852k" slid over
// the header's back arrow and the chat's title disappeared. The chip answers
// to the pane (ChatView's `@container`): the caret goes under 30rem, the
// counts under 24rem, and the ring always stays.
describe('context chip in a narrow pane', () => {
  it('drops the counts under 24rem, the caret under 30rem, never the ring', () => {
    const blocks = containerRules()
    expect(blocks.get('24rem')).toMatch(/\.harness-ui-chip-counts \{\s*display: none;/)
    expect(blocks.get('30rem')).toMatch(/\.harness-ui-chip-caret \{\s*display: none;/)
    for (const body of blocks.values()) {
      expect(body).not.toMatch(/\.harness-ui-chip-ring \{\s*display: none;/)
    }
  })

  // chat-projects §3.3.3: the "auto 160k" label is for wide surfaces; a
  // narrow pane keeps the ring's tick only.
  it('drops the auto-compact label under 30rem', () => {
    expect(containerRules().get('30rem')).toMatch(/\.harness-ui-chip-auto \{\s*display: none;/)
  })
})
