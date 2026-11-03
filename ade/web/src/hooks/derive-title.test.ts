import { afterEach, describe, expect, it, vi } from 'vitest'
import { iiiMentionRuntime, setMentionRuntime } from '@/lib/mentions/runtime'
import { primeMentionView } from '@/lib/mentions/views'
import { deriveTitle } from './use-conversations'

vi.mock('@/lib/iii-client', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  getIiiClient: () => new Promise(() => {}),
}))

afterEach(() => {
  setMentionRuntime(iiiMentionRuntime)
})

describe('deriveTitle', () => {
  it('reads a mention as its item when known, else as its provider', () => {
    primeMentionView('kanban', {
      id: '6ac4f6df-ec84-83e9-b480-4b54b9931ce0',
      label: 'Fix login redirect',
      hint: 'KAN-12',
    })
    primeMentionView('session', { id: 's_1', label: 'Hello there' })
    expect(
      deriveTitle(
        'Why is @kanban(id="6ac4f6df-ec84-83e9-b480-4b54b9931ce0") stuck?',
      ),
    ).toBe('Why is @KAN-12 stuck?')
    expect(deriveTitle('summarize @session(id="s_1")')).toBe(
      'summarize @Hello there',
    )
    expect(deriveTitle('open @trace(id="ab12cd34ef56")')).toBe('open @trace')
  })

  it('keeps the plain-text rules and the casing as typed', () => {
    expect(deriveTitle('   ')).toBe('New chat')
    expect(deriveTitle('A'.repeat(70))).toBe(`${'A'.repeat(60)}…`)
  })

  it('reads a file mention as its name, not its path', () => {
    expect(
      deriveTitle('fix #file(/Users/me/repo/src/App.tsx:12-40) please'),
    ).toBe('fix @App.tsx:12-40 please')
    expect(deriveTitle('what is in #file(src/lib/)')).toBe('what is in @lib/')
  })
})
