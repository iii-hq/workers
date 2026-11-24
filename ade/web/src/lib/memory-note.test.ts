import { describe, expect, it } from 'vitest'
import { parseMemoryNote } from './memory-note'

const HEADER =
  'Relevant remembered memories ("the user" is the person you are talking to; recalled 2026-11-23 Monday; each dated when last stated; verify anything surprising):'
const REMINDER = 'Standing instructions — apply every one to this reply:'

describe('parseMemoryNote', () => {
  it('reads a recall: bank, day, each memory and its date', () => {
    const text = `<memory bank="segralim">\n${HEADER}\n- The user prefers tea (2026-11-20)\n- Uses Result<T, E> (v2) daily (2026-11-21)\n</memory>`
    expect(parseMemoryNote(text)).toEqual({
      bank: 'segralim',
      recalled: '2026-11-23 Monday',
      memories: [
        { text: 'The user prefers tea', date: '2026-11-20' },
        { text: 'Uses Result<T, E> (v2) daily', date: '2026-11-21' },
      ],
      reminder: null,
      rulesUpdate: null,
    })
  })

  it('keeps every line after the reminder as the reminder, dashes included', () => {
    const text = `<memory bank="b">\n${HEADER}\n- a fact (2026-11-20)\n${REMINDER}\n- Never use emoji.\n- Reply in Portuguese.\n</memory>`
    const note = parseMemoryNote(text)
    expect(note?.memories).toEqual([{ text: 'a fact', date: '2026-11-20' }])
    expect(note?.reminder).toBe('- Never use emoji.\n- Reply in Portuguese.')
  })

  it('reads a reminder alone, and a pointer reminder on its opening line', () => {
    const alone = parseMemoryNote(
      `<memory bank="b">\n${REMINDER} the rules in the memory section of your system prompt (style, git).\n</memory>`,
    )
    expect(alone).toMatchObject({
      bank: 'b',
      memories: [],
      reminder:
        'the rules in the memory section of your system prompt (style, git).',
    })
  })

  it('reads a rules update as its new section', () => {
    const note = parseMemoryNote(
      '<memory update="rules">\nThe memory rules changed. This block replaces the memory section of the system prompt and any earlier update:\n- Never use emoji.\n</memory>',
    )
    expect(note).toMatchObject({
      bank: null,
      memories: [],
      rulesUpdate: '- Never use emoji.',
    })
  })

  it('reads an older recall with no header or dates', () => {
    expect(
      parseMemoryNote('<memory bank="b">\n- a fact\n</memory>')?.memories,
    ).toEqual([{ text: 'a fact', date: null }])
  })

  it('is null for anything it does not know, so the raw text shows', () => {
    for (const text of [
      'plain text',
      '<memory bank="m">likes tea</memory>',
      '<memory bank="m">\n- a fact\nsomething else\n</memory>',
      '<memory bank="m">\n</memory>',
      '<memory update="other">\n- x\n</memory>',
      '<mentions>\n- x\n</mentions>',
    ]) {
      expect(parseMemoryNote(text), text).toBeNull()
    }
  })
})
