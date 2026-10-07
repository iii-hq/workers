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
    ).toBe('why is @kan-12 stuck?')
    expect(deriveTitle('summarize @session(id="s_1")')).toBe(
      'summarize @hello there',
    )
    expect(deriveTitle('open @trace(id="ab12cd34ef56")')).toBe('open @trace')
  })

  it('keeps the plain-text rules', () => {
    expect(deriveTitle('   ')).toBe('new chat')
    expect(deriveTitle('A'.repeat(40))).toBe(`${'a'.repeat(32)}…`)
  })
})
