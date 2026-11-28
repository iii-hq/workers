import { describe, expect, it } from 'vitest'
import type { FunctionCallMessage, Message } from '@/types/chat'
import {
  assistantHeaderIds,
  changedFilesFromCalls,
  formatReplyClock,
  formatTurnDuration,
  harnessTurnOf,
  turnDurationMs,
} from './turn-summary'

function call(
  functionId: string,
  input: unknown,
  id = functionId,
): FunctionCallMessage {
  return { id, role: 'function-call', functionId, input, createdAt: 0 }
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

  // The live acceptance: the turn failed on its account's limit, Continue
  // re-ran it 27 s later, and the reply came in under a second.
  const rerun: Message[] = [
    { id: 'e_idem_m1', role: 'user', content: 'Go', createdAt: 1_000 },
    {
      id: 'e_t_1_error',
      role: 'system',
      kind: 'turn-failure',
      content: 'Claude Code usage limit reached.',
      createdAt: 1_500,
    },
    {
      id: 'e_t_2_0_assistant:0',
      role: 'assistant',
      content: 'Done',
      createdAt: 28_400,
    },
  ]

  it('measures a re-run turn from its own start, never the failed turn’s message', () => {
    const starts = new Map([['t_2', 27_700]])
    expect(
      turnDurationMs(rerun, 'e_t_2_0_assistant:0', (t) => starts.get(t)),
    ).toBe(700)
    // A re-run this tab didn't time (another client, a reload): no duration.
    expect(
      turnDurationMs(rerun, 'e_t_2_0_assistant:0', () => undefined),
    ).toBeNull()
    // An earlier turn's reply in between says the same.
    const afterReply: Message[] = [
      rerun[0],
      {
        id: 'e_t_1_0_assistant:0',
        role: 'assistant',
        content: 'Part',
        createdAt: 1_400,
      },
      rerun[2],
    ]
    expect(
      turnDurationMs(afterReply, 'e_t_2_0_assistant:0', () => undefined),
    ).toBeNull()
  })

  it("keeps measuring a turn's later steps from its message", () => {
    const steps: Message[] = [
      { id: 'e_idem_m1', role: 'user', content: 'Go', createdAt: 1_000 },
      {
        id: 'e_t_1_0_assistant:0',
        role: 'assistant',
        content: 'Looking',
        createdAt: 2_000,
      },
      {
        id: 'e_t_1_fc_1',
        role: 'function-call',
        functionId: 'shell::exec',
        input: {},
        createdAt: 2_100,
      },
      {
        id: 'e_t_1_1_assistant:0',
        role: 'assistant',
        content: 'Done',
        createdAt: 9_000,
      },
    ]
    expect(turnDurationMs(steps, 'e_t_1_1_assistant:0', () => undefined)).toBe(
      8_000,
    )
    expect(harnessTurnOf('e_t_1_1_assistant:0')).toBe('t_1')
    expect(harnessTurnOf('e_t_1_1_assistant')).toBe('t_1')
    expect(harnessTurnOf('local-uid')).toBeNull()
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

describe('assistantHeaderIds', () => {
  const reply = (
    id: string,
    model = 'claude-sonnet-5-5',
    agent?: string,
  ): Message => ({
    id,
    role: 'assistant',
    content: id,
    model,
    agent,
    createdAt: 0,
  })

  it('names the author once per turn, again when it changes', () => {
    const ids = assistantHeaderIds([
      { id: 'u1', role: 'user', content: 'go', createdAt: 0 },
      reply('a1'),
      call('browser::click', {}),
      reply('a2'),
      reply('a3', 'claude-opus-5-5'),
      reply('a4', 'claude-opus-5-5'),
      { id: 'u2', role: 'user', content: 'again', createdAt: 0 },
      reply('a5', 'claude-opus-5-5'),
      reply('a6', 'claude-opus-5-5', 'pi'),
    ])
    expect([...ids]).toEqual(['a1', 'a3', 'a5', 'a6'])
  })
})
