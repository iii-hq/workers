import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import type { Conversation } from '@/types/chat'
import {
  startDecisionSideChatLifetime,
  withDecisionSideChatLock,
} from './decision-side-chat-lifetime'

const tick = () => new Promise((resolve) => setTimeout(resolve, 0))

function copy(
  id: string,
  key: string,
  kind: 'question' | 'held',
  keep = false,
): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    createdAt: 1,
    updatedAt: 1,
    sessionMetadata: {
      side_chat_of: key.split('/')[0],
      side_chat_question: key,
      side_chat_question_kind: kind,
      ...(keep ? { side_chat_keep: true } : {}),
    },
  }
}

function setup(
  copies: Conversation[],
  asks: unknown[] = [],
  holds: unknown[] = [],
  options: {
    board?: unknown[]
    pages?: Conversation[][]
    fail?: string[]
  } = {},
) {
  const handlers = new Map<string, (payload: unknown) => void>()
  let connection: (state: string) => void = () => {}
  const server = new Map(copies.map((copy) => [copy.id, copy]))
  const call = vi.fn(
    async (functionId: string, payload?: Record<string, unknown>) => {
      if (options.fail?.includes(functionId))
        throw new Error(`${functionId} down`)
      if (functionId === 'session::list') {
        const pages = options.pages ?? [copies]
        const index = payload?.cursor ? Number(payload.cursor) : 0
        return {
          sessions: (pages[index] ?? []).map((row) => ({
            session_id: row.id,
            metadata: row.sessionMetadata,
          })),
          ...(index + 1 < pages.length
            ? { next_cursor: String(index + 1) }
            : {}),
        }
      }
      if (functionId === 'harness::ask::list') return { entries: asks }
      if (functionId === 'approval::list-pending') return { pending: holds }
      if (functionId === 'kanban::question::list')
        return { questions: options.board ?? [] }
      if (functionId === 'session::get') {
        const found = server.get(String(payload?.session_id))
        return found ? { meta: { metadata: found.sessionMetadata } } : null
      }
      throw new Error(`unexpected ${functionId}`)
    },
  )
  const client = {
    browserId: 'test',
    call,
    on: vi.fn((id: string, handler: (payload: unknown) => void) => {
      handlers.set(id, handler)
      return vi.fn()
    }),
    observe: vi.fn(() => vi.fn()),
    addConnectionStateListener: vi.fn((handler: (state: string) => void) => {
      connection = handler
      return vi.fn()
    }),
  } as unknown as IiiClient
  const remove = vi.fn(async (id: string) => {
    server.delete(id)
  })
  const stop = startDecisionSideChatLifetime(client, () => copies, remove)
  return {
    client,
    call,
    remove,
    stop,
    server,
    fire: (id: string, payload: unknown) => handlers.get(id)?.(payload),
    reconnect: () => connection('connected'),
  }
}

describe('decision side chat lifetime', () => {
  it('deletes question and held copies when their sources close', async () => {
    const harness = setup(
      [
        copy('question-copy', 'session/question', 'question'),
        copy('held-copy', 'session/call', 'held'),
      ],
      [{ session_id: 'session', question_id: 'question', state: 'open' }],
      [{ session_id: 'session', function_call_id: 'call' }],
    )
    await tick()
    expect(harness.remove).not.toHaveBeenCalled()

    harness.fire('iii::console::decision_side_chat_ask_settled', {
      session_id: 'session',
      question_id: 'question',
      state: 'answered',
    })
    harness.fire('iii::console::decision_side_chat_hold_resolved', {
      session_id: 'session',
      function_call_id: 'call',
    })
    await tick()
    expect(harness.remove).toHaveBeenCalledWith('question-copy')
    expect(harness.remove).toHaveBeenCalledWith('held-copy')
    harness.stop()
  })

  it('reconciles terminal asks and missing holds after reload or reconnect', async () => {
    const harness = setup(
      [
        copy('answered', 's/q', 'question'),
        copy('open', 's/open', 'question'),
        copy('resolved', 's/fc', 'held'),
      ],
      [
        { session_id: 's', question_id: 'q', state: 'answered' },
        { session_id: 's', question_id: 'open', state: 'expired' },
      ],
      [],
    )
    await tick()
    expect(harness.remove.mock.calls.map(([id]) => id).sort()).toEqual([
      'answered',
      'resolved',
    ])
    harness.reconnect()
    await tick()
    expect(harness.call).toHaveBeenCalledWith('harness::ask::list', {
      limit: 200,
    })
    harness.stop()
  })

  it('preserves kept copies and skips cleanup when a source read fails', async () => {
    const kept = copy('kept', 's/q', 'question', true)
    const unknown = copy('unknown', 's/missing', 'question')
    const harness = setup([kept, unknown], [], [], {
      fail: ['harness::ask::list'],
    })
    await tick()
    expect(harness.remove).not.toHaveBeenCalled()

    harness.fire('iii::console::decision_side_chat_ask_settled', {
      session_id: 's',
      question_id: 'q',
      state: 'answered',
    })
    await tick()
    expect(harness.remove).not.toHaveBeenCalled()
    harness.stop()
  })

  it('discovers paginated copies and uses the terminal board mirror', async () => {
    const listed = copy('listed-on-page-two', 's/q', 'question')
    const harness = setup([], [], [], {
      pages: [[], [listed]],
      board: [
        {
          state: 'answered',
          comment: { question: { session_id: 's', question_id: 'q' } },
        },
      ],
    })
    harness.server.set(listed.id, listed)
    await tick()
    await tick()
    expect(harness.call).toHaveBeenCalledWith('session::list', {
      limit: 200,
      cursor: '1',
    })
    expect(harness.remove).toHaveBeenCalledWith('listed-on-page-two')
    harness.stop()
  })

  it('serializes Keep and cleanup with one Web Lock', async () => {
    const original = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
    let tail = Promise.resolve<unknown>(undefined)
    const request = vi.fn(<T>(_name: string, run: () => Promise<T>) => {
      const next = tail.then(run)
      tail = next.catch(() => undefined)
      return next
    })
    Object.defineProperty(globalThis, 'navigator', {
      configurable: true,
      value: { locks: { request } },
    })
    const order: string[] = []
    let release: () => void = () => {}
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const first = withDecisionSideChatLock('copy', async () => {
      order.push('keep-start')
      await gate
      order.push('keep-end')
    })
    const second = withDecisionSideChatLock('copy', async () => {
      order.push('delete')
    })
    await tick()
    expect(order).toEqual(['keep-start'])
    release()
    await Promise.all([first, second])
    expect(order).toEqual(['keep-start', 'keep-end', 'delete'])
    expect(request.mock.calls.map(([name]) => name)).toEqual([
      'iii-decision-side-chat-lifetime:copy',
      'iii-decision-side-chat-lifetime:copy',
    ])
    if (original) Object.defineProperty(globalThis, 'navigator', original)
    else Reflect.deleteProperty(globalThis, 'navigator')
  })

  it('re-reads metadata so Keep wins a cleanup race', async () => {
    const decision = copy('copy', 's/q', 'question')
    const harness = setup(
      [decision],
      [{ session_id: 's', question_id: 'q', state: 'open' }],
    )
    await tick()
    const serverCopy = harness.server.get('copy')
    expect(serverCopy).toBeDefined()
    if (serverCopy?.sessionMetadata)
      serverCopy.sessionMetadata.side_chat_keep = true
    harness.fire('iii::console::decision_side_chat_ask_settled', {
      session_id: 's',
      question_id: 'q',
      state: 'answered',
    })
    await tick()
    expect(harness.remove).not.toHaveBeenCalled()
    harness.stop()
  })
})
