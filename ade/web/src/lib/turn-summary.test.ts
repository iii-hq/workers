import { describe, expect, it } from 'vitest'
import type { FunctionTriggerMessage, Message } from '@/types/chat'
import {
  changedFilesFromCalls,
  formatReplyClock,
  formatTurnDuration,
  turnDurationMs,
} from './turn-summary'

function call(
  functionId: string,
  input: unknown,
  id = functionId,
): FunctionTriggerMessage {
  return { id, role: 'function-trigger', functionId, input, createdAt: 0 }
}

describe('changedFilesFromCalls', () => {
  it('counts lines written by the coder and merges by path', () => {
    const files = changedFilesFromCalls([
      call('coder::create-file', {
        files: [{ path: '/repo/src/App.tsx', content: 'a\nb\nc' }],
      }),
      call(
        'coder::update-file',
        {
          files: [
            {
              path: '/repo/src/App.tsx',
              ops: [
                { op: 'insert', at_line: 1, content: 'x\ny' },
                { op: 'remove', from_line: 2, to_line: 4 },
                { op: 'replace', pattern: 'a', replacement: 'b' },
              ],
            },
          ],
        },
        'u1',
      ),
      call('coder::delete-file', { paths: ['/repo/old.txt'] }),
    ])
    expect(files).toEqual([
      {
        path: '/repo/src/App.tsx',
        name: 'App.tsx',
        additions: 6,
        deletions: 4,
        kind: 'write',
      },
      {
        path: '/repo/old.txt',
        name: 'old.txt',
        additions: 0,
        deletions: 0,
        kind: 'delete',
      },
    ])
  })

  it('reads shell writes and peels the iii envelope', () => {
    const files = changedFilesFromCalls([
      call('shell::fs::write', {
        content: [{ type: 'text', text: '' }],
        details: { path: '/w/notes.md', content: 'one\ntwo' },
      }),
      call('shell::fs::sed', { path: '/w/notes.md' }, 's1'),
      call('shell::exec', { command: 'ls' }),
    ])
    expect(files).toEqual([
      {
        path: '/w/notes.md',
        name: 'notes.md',
        additions: 3,
        deletions: 1,
        kind: 'write',
      },
    ])
  })

  it('ignores calls still waiting for approval', () => {
    expect(
      changedFilesFromCalls([
        {
          ...call('coder::create-file', {
            files: [{ path: '/a', content: 'x' }],
          }),
          pendingApproval: true,
        },
      ]),
    ).toEqual([])
  })
})

describe('turnDurationMs', () => {
  const messages: Message[] = [
    { id: 'u1', role: 'user', content: 'hi', createdAt: 1000 },
    { id: 'a1', role: 'assistant', content: 'hello', createdAt: 4200 },
    { id: 'u2', role: 'user', content: 'more', createdAt: 9000 },
    { id: 'a2', role: 'assistant', content: 'sure', createdAt: 8000 },
  ]
  it('measures from the user message that started the turn', () => {
    expect(turnDurationMs(messages, 'a1')).toBe(3200)
  })
  it('is null without a preceding user message or with a backwards clock', () => {
    expect(turnDurationMs(messages, 'a2')).toBeNull()
    expect(turnDurationMs(messages.slice(1), 'a1')).toBeNull()
  })
})

describe('formatting', () => {
  it('formats durations on a human scale', () => {
    expect(formatTurnDuration(850)).toBe('0.9s')
    expect(formatTurnDuration(48_000)).toBe('48s')
    expect(formatTurnDuration(23 * 60_000 + 37_000)).toBe('23m 37s')
    expect(formatTurnDuration(65 * 60_000)).toBe('1h 05m')
  })
  it('shows only the time for today and adds the day otherwise', () => {
    const now = new Date(2026, 9, 28, 18, 45).getTime()
    expect(formatReplyClock(now, now)).toMatch(/6:45/)
    expect(formatReplyClock(now - 2 * 86_400_000, now)).toMatch(/Oct 26/)
  })
})
