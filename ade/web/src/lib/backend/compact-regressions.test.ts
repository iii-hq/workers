/**
 * `/compact` regressions from the console-session investigation: a parent
 * session coordinating many sub-agent jobs, where compaction failed
 * intermittently and, when it worked, the context meter never moved. The
 * compaction now runs on the harness (harness/src/compaction.rs and
 * harness/tests/compact_command.rs); these pin what the console still owns.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import type { TranscriptItem } from '@/lib/sessions/types'
import { compactionWindow, latestCompactionAnchor } from './compaction-window'
import { realBackend } from './real'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

type Handler = (payload: Record<string, unknown>) => unknown

const trigger = vi.fn()
let handlers: Record<string, Handler> = {}

const user = (id: string, text: string): TranscriptItem =>
  ({
    entry_id: id,
    message: { role: 'user', content: [{ type: 'text', text }], timestamp: 1 },
  }) as TranscriptItem
const assistant = (id: string, text: string): TranscriptItem =>
  ({
    entry_id: id,
    message: {
      role: 'assistant',
      content: [{ type: 'text', text }],
      timestamp: 2,
    },
  }) as TranscriptItem
const spawnCall = (id: string, callId: string): TranscriptItem =>
  ({
    entry_id: id,
    message: {
      role: 'assistant',
      content: [
        {
          type: 'function_call',
          id: callId,
          function_id: 'harness::spawn',
          arguments: {},
        },
      ],
      timestamp: 3,
    },
  }) as TranscriptItem
const spawnResult = (id: string, callId: string): TranscriptItem =>
  ({
    entry_id: id,
    message: {
      role: 'function_result',
      function_call_id: callId,
      function_id: 'harness::spawn',
      content: [{ type: 'text', text: '{"child_session_id":"obs04"}' }],
      timestamp: 4,
    },
  }) as TranscriptItem
const custom = (id: string, customType: string, data: unknown) =>
  ({
    entry_id: id,
    custom: { custom_type: customType, data },
  }) as TranscriptItem

beforeEach(() => {
  handlers = {
    'harness::compact': () => ({
      session_id: 's-1',
      compaction_id: 'cmp_1',
      status: 'ok',
      message: 'compacted 9 tokens',
      tokens_before: 9,
      summary: 'S',
      entry_ids: ['e_cmp_1_started', 'e_cmp_1_outcome'],
    }),
  }
  trigger.mockReset()
  trigger.mockImplementation(async (id: string, payload: never) => {
    const handler = handlers[id]
    if (!handler) throw new Error(`unexpected ${id}`)
    return handler(payload)
  })
  vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
})

const payloadOf = (functionId: string) =>
  trigger.mock.calls.find(([id]) => id === functionId)?.[1] as
    | Record<string, unknown>
    | undefined

describe('/compact budget', () => {
  // The console used to send the catalog window with a placeholder
  // `max_output_tokens: 4096`. context-manager trusts inline limits, so the
  // summariser's fragments grew by the model's real output allowance and a
  // large head was cut into requests the provider rejects. The harness
  // passes no limits; the router resolves them.
  it('sends no limits for the summariser', async () => {
    await realBackend.compactSession?.(
      's-1',
      'anthropic::claude-opus-5-5' as never,
      200_000,
    )
    const sent = payloadOf('harness::compact')
    expect(sent).toEqual({
      session_id: 's-1',
      model: 'claude-opus-5-5',
      provider: 'anthropic',
    })
  })
})

describe('/compact while sub-agents run', () => {
  // A parent that waits on sub-agents (`awaiting_functions`), or that a
  // child's notification just woke (`running`), was refused client-side as
  // "another compaction is in progress". The harness compacts beside a live
  // turn, keeping its unanswered calls in the verbatim tail, so the console
  // no longer gates on the turn at all.
  it('does not refuse on the turn status', async () => {
    handlers['harness::status'] = () => ({
      session_id: 's-1',
      turn_id: 't-1',
      status: 'awaiting_functions',
      pending_function_calls: ['c1'],
      children: [
        { function_call_id: 'c1', session_id: 'obs04', turn_id: 't-child' },
      ],
    })
    const result = await realBackend.compactSession?.(
      's-1',
      'anthropic::claude-opus-5-5' as never,
    )
    expect(result).toMatchObject({ status: 'ok' })
    expect(payloadOf('harness::status')).toBeUndefined()
  })
})

describe('/compact persistence', () => {
  // The browser used to own the round trip: context-manager summarised and
  // the browser persisted the record, so a WebSocket reset mid-summary (the
  // console proxy logged several that night) threw the summary away. The
  // harness persists it now; the browser never writes the record.
  it('a dropped connection is a local error, and the browser writes nothing', async () => {
    handlers['harness::compact'] = () => {
      throw new Error('connection closed before the invocation returned')
    }
    const result = await realBackend.compactSession?.(
      's-1',
      'anthropic::claude-opus-5-5' as never,
    )
    expect(result).toEqual({
      status: 'error',
      message: expect.stringContaining('connection closed'),
    })
    expect(payloadOf('session::append')).toBeUndefined()
  })
})

describe('the model-switch preview window', () => {
  // The investigated session's anchor: the summary's verbatim tail opens
  // mid-turn at an assistant call, `trigger_fired` bookkeeping sits between
  // notifications, and a `model_notice` replays as a user message. The
  // preview's window must match what the harness assembles.
  it('a mid-turn anchor opens the next window at its assistant boundary', () => {
    const items = [
      user('u1', 'Coordinate the observe spec.'),
      spawnCall('a1', 'c1'),
      spawnResult('r1', 'c1'),
      assistant('a2', 'Executor running.'),
      user('n1', '[notification] obs04-kan6-review-a9d3-r2: done'),
      custom('tf1', 'trigger_fired', { key: 'obs04-kan6-review-a9d3' }),
      spawnCall('a3', 'c2'),
      spawnResult('r3', 'c2'),
      assistant('a4', 'Round 3 queued.'),
      custom('cmp', 'compaction', {
        summary: 'S',
        tail_start_entry_id: 'a3',
        tokens_before: 471_486,
        timestamp: 5,
      }),
      user('n2', '[notification] obs04-kan6-exec-e5b1-r3: done'),
      custom('tf2', 'trigger_fired', { key: 'obs04-kan6-exec-e5b1' }),
      custom('mn', 'model_notice', { text: 'runtime context changed' }),
      assistant('a5', 'Reviewing round 3.'),
    ]
    const anchor = latestCompactionAnchor(items)
    expect(anchor).toEqual({
      entryId: 'cmp',
      summary: 'S',
      tailStartEntryId: 'a3',
    })
    const window = compactionWindow(items, anchor)
    expect(window.map((e) => e.entry_id)).toEqual([
      'a3',
      'r3',
      'a4',
      'n2',
      'mn',
      'a5',
    ])
    expect(window[4].message).toMatchObject({ role: 'user' })
  })
})
