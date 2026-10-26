/**
 * Long Markdown in pieces (src/blocks.ts, design §6.2), in Node: the cuts,
 * what is never cut, the merge, the fold; and the seeded KAN-41 of the test
 * environment (ide projects/kanban-native-ui/env/seed.json).
 */
import { describe, expect, it } from 'vitest'
import { chunks, FOLD_AT, folded, lines, PIECE, pieces, SPLIT_AT } from '../src/blocks'
import { longMarkdown } from './fake'

/** The seeded KAN-41: headings, a numbered list with a nested one, a table, a fence, a list of notes, a quote. */
const KAN_41 = [
  '## Goal',
  '',
  'Move a card to another column without opening it: a **long press** lifts the card, and dropping it on another column (or on a tab, on a phone) moves it there. The *Move to* menu stays as the accessible path.',
  '',
  '## Acceptance criteria',
  '',
  "1. A long press lifts the card with a haptic tick; released without moving, it opens the card's menu.",
  '2. Holding the card over a tab for 400 ms switches to that lane.',
  '3. Dropping calls `kanban::ticket::move` with the column and the index:',
  '   - the card shows at its new place at once;',
  '   - on a failure it goes back, and a toast says why.',
  '4. Nothing changes for TalkBack: *Move to* lists every column.',
  '',
  '## Timings',
  '',
  '| Step | Phone | Unfolded |',
  '|---|---|---|',
  '| Lift after the press | 400 ms | 400 ms |',
  '| Lane switch over a tab | 400 ms | \u2014 |',
  '| Auto-scroll near an edge | 48 dp | 48 dp |',
  '',
  '## The call',
  '',
  '```json',
  '{ "id": "KAN-41", "status": "in_review", "index": 0, "actor": "frontend-engineer" }',
  '```',
  '',
  '## Notes',
  '',
  "- The same lift as the chat list's reorder; KAN-8 is the rotation crash we must not bring back.",
  '- Design: [catalogue 2](https://github.com/iii-hq/workers/pull/1514) and the [Material gestures guide](https://m3.material.io/foundations/interaction/gestures).',
  '- ~~Swipe to move~~: dropped, it fights the side swipe (see KAN-83).',
  '',
  '> The board is for following and answering on the phone; planning stays on the desktop.',
].join('\n')

const fences = (text: string) => text.split('\n').filter((line) => /^\s*```/.test(line)).length

describe('pieces', () => {
  it('a short text, and one with link definitions, stays whole', () => {
    expect(pieces('# Title\n\nSome text.')).toEqual(['# Title\n\nSome text.'])
    expect(KAN_41.length).toBeLessThan(SPLIT_AT)
    expect(pieces(KAN_41)).toEqual([KAN_41])
    const defs = `${longMarkdown(4000)}\n\nSee [the spec][s].\n\n[s]: https://example.com/spec`
    expect(pieces(defs)).toEqual([defs])
  })

  it('a 20 KB description becomes pieces of about 1.5 KB that join back into the text', () => {
    const text = longMarkdown()
    const parts = pieces(text)
    expect(parts.length).toBeGreaterThan(8)
    expect(parts.join('')).toBe(text)
    for (const part of parts) {
      // Balanced fences: no piece holds half of one.
      expect(fences(part) % 2).toBe(0)
      // A table stays whole: its header and rule travel together.
      if (part.includes('|---|')) expect(part).toMatch(/\| step \| phone \|\n\|---\|---\|/)
      expect(part.length).toBeLessThan(PIECE * 2)
    }
    // A piece never ends on a heading away from what it heads.
    for (const part of parts.slice(0, -1)) expect(part.trimEnd().split('\n').at(-1)).not.toMatch(/^#/)
  })

  it('never cuts a fence, whatever blank lines it holds, nor a loose list', () => {
    const fence = `\`\`\`\n${Array.from({ length: 200 }, (_, i) => `line ${i}\n`).join('\n')}\`\`\``
    const text = `Before.\n\n${fence}\n\nAfter.`
    const parts = pieces(text, 100, 100)
    expect(parts.join('')).toBe(text)
    expect(parts.find((p) => p.includes('line 0'))).toContain('line 199')
    const list = '- a\n\n- b\n\n  more of b\n\n1. c'
    expect(chunks(`Para.\n\n${list}\n\nEnd.`)).toEqual(['Para.\n\n', `${list}\n\n`, 'End.'])
  })

  it('cuts the seeded KAN-41 at its sections when the pieces are small', () => {
    const parts = pieces(KAN_41, 0, 450)
    expect(parts.join('')).toBe(KAN_41)
    expect(parts.map((p) => p.split('\n')[0])).toEqual(['## Goal', '## Acceptance criteria', '## Timings', '## Notes'])
    // The numbered list with its nested items, the table and the fence are each whole in one piece.
    expect(parts[1]).toContain('1. A long press')
    expect(parts[1]).toContain('   - on a failure it goes back')
    expect(parts[1]).toContain('4. Nothing changes for TalkBack')
    expect(parts[2]).toMatch(/\| Step \| Phone \| Unfolded \|\n\|---\|---\|---\|/)
    expect(parts[2]).toContain('| Auto-scroll near an edge')
    expect(fences(parts[2])).toBe(2)
    expect(parts[3]).toContain('> The board is for following')
  })

  it('cuts a block with no blank line at a line: a list before an item, never inside a fence or a table', () => {
    const checklist = Array.from(
      { length: 300 },
      (_, i) => `- [ ] check number ${i} of the release\n  with a second line`,
    ).join('\n')
    const parts = lines(checklist, PIECE)
    expect(parts.length).toBeGreaterThan(5)
    expect(parts.join('')).toBe(checklist)
    for (const part of parts) expect(part).toMatch(/^- \[ \]/)
    const wall = Array.from({ length: 400 }, (_, i) => `Sentence ${i} of a wall of text with no blank line.`).join('\n')
    expect(pieces(wall).length).toBeGreaterThan(5)
    const table = `| a | b |\n|---|---|\n${Array.from({ length: 300 }, (_, i) => `| ${i} | x |`).join('\n')}`
    expect(lines(table, 100)).toEqual([table])
  })
})

describe('folding a comment', () => {
  it('a long comment shows its first blocks, cut at a block, about 1.5 KB', () => {
    expect(folded('short')).toBeNull()
    const text = longMarkdown(6000)
    expect(text.length).toBeGreaterThan(FOLD_AT)
    const start = folded(text) as string
    expect(start.length).toBeLessThan(2000)
    expect(start.length).toBeGreaterThan(500)
    expect(text.startsWith(start)).toBe(true)
    expect(fences(start) % 2).toBe(0)
  })
})
