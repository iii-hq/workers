import { describe, expect, it } from 'vitest'
import type { SessionSearchHit } from '@/lib/sessions/api'
import { chatSearchEntries, focusSnippet } from './chat-search'

function hit(partial: Partial<SessionSearchHit>): SessionSearchHit {
  return {
    session_id: 's_1',
    title: 'Setup notes',
    updated_at: 1,
    matched_in: 'content',
    title_ranges: [],
    ...partial,
  }
}

describe('focusSnippet', () => {
  it('leaves a snippet whose match is already near the start', () => {
    expect(focusSnippet('…the setup wizard', [[5, 10]])).toEqual({
      text: '…the setup wizard',
      ranges: [[5, 10]],
    })
  })

  it('cuts the lead to a word boundary and shifts the ranges', () => {
    const text =
      '…a long run of words before anything interesting finally the needle shows'
    const at = text.indexOf('needle')
    const focused = focusSnippet(text, [[at, at + 6]], 10)
    expect(focused.text.startsWith('…')).toBe(true)
    const [from, to] = focused.ranges[0]
    expect(focused.text.slice(from, to)).toBe('needle')
    expect(from).toBeLessThanOrEqual(12)
    // It starts on a whole word of the original.
    const rest = focused.text.slice(1)
    const cut = text.indexOf(rest)
    expect(cut).toBeGreaterThan(0)
    expect(text[cut - 1]).toBe(' ')
  })
})

describe('focusSnippet surrogates', () => {
  it('never opens on half of an astral character', () => {
    const text = `xxx😀${'漢'.repeat(23)}needle`
    const at = text.indexOf('needle')
    const focused = focusSnippet(text, [[at, at + 6]])
    for (let i = 0; i < focused.text.length; i += 1) {
      const unit = focused.text.charCodeAt(i)
      if (unit >= 0xdc00 && unit <= 0xdfff) {
        const before = focused.text.charCodeAt(i - 1)
        expect(before >= 0xd800 && before <= 0xdbff).toBe(true)
      }
    }
    const [from, to] = focused.ranges[0]
    expect(focused.text.slice(from, to)).toBe('needle')
  })
})

describe('chatSearchEntries', () => {
  const chats = new Map([
    ['s_1', { id: 's_1', title: 'Setup notes', updatedAt: 0 }],
    ['s_2', { id: 's_2', title: 'Renamed locally', updatedAt: 0 }],
  ])

  it('keeps the chat row id, the server order and only known chats', () => {
    const opened: string[] = []
    const rows = chatSearchEntries(
      [
        hit({ session_id: 's_2', title: 'Old title', title_ranges: [[0, 3]] }),
        hit({ session_id: 's_gone' }),
        hit({
          session_id: 's_1',
          title_ranges: [[0, 5]],
          snippet: {
            entry_id: 'e1',
            role: 'user',
            text: 'streamline it',
            ranges: [[0, 10]],
          },
        }),
      ],
      chats,
      (id) => opened.push(id),
    )
    expect(rows.map((row) => row.id)).toEqual(['chat:s_2', 'chat:s_1'])
    // Ranges measured on another title do not fit this one.
    expect(rows[0].titleRanges).toBeUndefined()
    expect(rows[1].titleRanges).toEqual([[0, 5]])
    expect(rows[1].snippet).toEqual({
      text: 'streamline it',
      ranges: [[0, 10]],
    })
    rows[1].run()
    expect(opened).toEqual(['s_1'])
  })
})
