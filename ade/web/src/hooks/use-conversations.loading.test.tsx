// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ConversationLoadNotice } from '@/components/chat/ConversationLoadNotice'
import { getIiiClient, type IIIConnectionState } from '@/lib/iii-client'
import {
  fetchTranscriptTail,
  getSession,
  listSessions,
} from '@/lib/sessions/api'
import {
  type SessionTranscriptHandlers,
  subscribeSessionTranscript,
} from '@/lib/sessions/events'
import type { MessageAddedEvent, SessionMeta } from '@/lib/sessions/types'
import { type ConversationsApi, useConversations } from './use-conversations'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/sessions/api', async (original) => ({
  ...(await original<typeof import('@/lib/sessions/api')>()),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  fetchTranscriptTail: vi.fn(),
}))
vi.mock('@/lib/sessions/events', () => ({
  subscribeSessionDirectory: () => () => {},
  subscribeSessionTranscript: vi.fn(),
}))

const meta: SessionMeta = {
  session_id: 'saved-chat',
  title: 'Saved chat',
  description: '',
  status: 'done',
  metadata: {},
  created_at: 1,
  updated_at: 2,
  message_count: 1,
}
let api: ConversationsApi
let root: Root
let host: HTMLDivElement
let connectionChanged: (state: IIIConnectionState) => void

function Probe() {
  api = useConversations(undefined, false, true)
  return (
    <ConversationLoadNotice
      loading={api.conversationsLoading}
      error={api.conversationsError || api.conversationLoadErrors['saved-chat']}
      onRetry={api.retryConversations}
    />
  )
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  localStorage.clear()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  vi.mocked(getIiiClient).mockResolvedValue({
    addConnectionStateListener: (listener: typeof connectionChanged) => {
      connectionChanged = listener
      listener('connected')
      return () => {}
    },
  } as never)
  vi.mocked(subscribeSessionTranscript).mockReset()
  vi.mocked(subscribeSessionTranscript).mockReturnValue(() => {})
  vi.mocked(listSessions).mockResolvedValue([meta])
  vi.mocked(getSession).mockResolvedValue(meta)
  vi.mocked(fetchTranscriptTail).mockResolvedValue({
    items: [],
    hasMore: false,
  })
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function mount() {
  await act(async () => root.render(<Probe />))
}

describe('conversation loading feedback', () => {
  it('shows a failed directory read and retries without reloading or removing local drafts', async () => {
    vi.mocked(listSessions).mockRejectedValueOnce(new Error('read failed'))
    await mount()
    const draft = api.conversations.find((item) => item.draft)?.id
    expect(host.textContent).toContain('Unable to load conversations.')
    await act(async () => host.querySelector('button')?.click())
    expect(listSessions).toHaveBeenCalledTimes(2)
    expect(api.conversations.some((item) => item.id === 'saved-chat')).toBe(
      true,
    )
    expect(api.conversations.some((item) => item.id === draft)).toBe(true)
    expect(host.textContent).toBe('')
  })

  it('distinguishes a pending read from a successfully empty directory', async () => {
    let resolve!: (value: SessionMeta[]) => void
    vi.mocked(listSessions).mockReturnValue(
      new Promise((done) => {
        resolve = done
      }),
    )
    await mount()
    expect(host.textContent).toContain('Loading conversations…')
    await act(async () => resolve([]))
    expect(api.conversationsError).toBeNull()
    expect(host.textContent).toBe('')
  })

  it('does not let a stale failure overwrite a successful retry', async () => {
    let reject!: (error: Error) => void
    vi.mocked(listSessions).mockReturnValueOnce(
      new Promise((_, fail) => {
        reject = fail
      }),
    )
    await mount()
    await act(async () => api.retryConversations())
    await act(async () => reject(new Error('old request')))
    expect(api.conversationsError).toBeNull()
    expect(api.conversationsLoading).toBe(false)
  })

  it('reports bootstrap failure and handles rejected background subscriptions', async () => {
    vi.mocked(getIiiClient).mockRejectedValue(new Error('runtime failed'))
    await mount()
    expect(api.connectionState).toBe('failed')
    expect(host.textContent).toContain('Unable to load conversations.')
  })

  it('preserves cached conversations when a refresh fails and recovers on reconnect', async () => {
    await mount()
    vi.mocked(listSessions).mockRejectedValueOnce(new Error('unavailable'))
    await act(async () => api.retryConversations())
    expect(api.conversations.some((item) => item.id === meta.session_id)).toBe(
      true,
    )
    expect(host.textContent).toContain('Unable to load conversations.')
    await act(async () => connectionChanged('reconnecting'))
    await act(async () => connectionChanged('connected'))
    expect(api.conversationsError).toBeNull()
  })

  it('reports transcript failures until messages have actually loaded', async () => {
    await mount()
    vi.mocked(fetchTranscriptTail).mockRejectedValueOnce(
      new Error('read failed'),
    )
    await act(async () => {
      api.watchConversation(meta.session_id)
    })
    expect(host.textContent).toContain('Unable to load messages.')
    await act(async () => api.retryConversations())
    expect(api.conversationLoadErrors[meta.session_id]).toBeUndefined()
    expect(
      api.conversations.find((item) => item.id === meta.session_id)?.hydrated,
    ).toBe(true)
  })
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}

describe.each(['retry', 'disconnect'] as const)(
  'metadata invalidation on %s',
  (transition) => {
    it.each(['metadata', 'missing', 'error'] as const)(
      'ignores stale %s while the next directory read is pending',
      async (result) => {
        vi.mocked(listSessions).mockResolvedValueOnce([])
        const oldRead = deferred<SessionMeta | null>()
        vi.mocked(getSession).mockReturnValueOnce(oldRead.promise)
        await mount()
        await act(async () => {
          api.watchConversation(meta.session_id)
        })
        const directory = deferred<SessionMeta[]>()
        vi.mocked(listSessions).mockReturnValueOnce(directory.promise)
        await act(async () => {
          if (transition === 'retry') api.retryConversations()
          else connectionChanged('reconnecting')
        })
        await act(async () => {
          if (result === 'error') oldRead.reject(new Error('obsolete read'))
          else {
            oldRead.resolve(
              result === 'missing'
                ? null
                : { ...meta, title: 'Obsolete title' },
            )
          }
        })
        expect(api.conversations.some((c) => c.id === meta.session_id)).toBe(
          false,
        )
        expect(api.conversationLoadErrors[meta.session_id]).toBeUndefined()
        expect(api.missingConversationIds.has(meta.session_id)).toBe(false)
        if (transition === 'disconnect') {
          await act(async () => connectionChanged('connected'))
        }
        await act(async () => directory.resolve([meta]))
        expect(
          api.conversations.find((c) => c.id === meta.session_id)?.title,
        ).toBe(meta.title)
      },
    )
  },
)

const liveMessage = (id: string, text: string): MessageAddedEvent => ({
  session_id: meta.session_id,
  entry_id: id,
  parent_id: null,
  timestamp: 10,
  message: {
    role: 'assistant',
    stop_reason: 'end',
    model: 'test',
    provider: 'test',
    content: [{ type: 'text', text }],
    timestamp: 10,
  },
})

describe('transcript subscription recovery', () => {
  it.each(['timer', 'retry', 'reconnect'] as const)(
    'recovers through %s without losing other panels or live updates',
    async (recovery) => {
      vi.useFakeTimers()
      const sibling = { ...meta, session_id: 'sibling-chat' }
      vi.mocked(listSessions).mockResolvedValue([meta, sibling])
      vi.mocked(getSession).mockImplementation(async (id) =>
        id === sibling.session_id ? sibling : meta,
      )
      const handlers = new Map<string, SessionTranscriptHandlers>()
      const off = vi.fn()
      let fail = true
      vi.mocked(subscribeSessionTranscript).mockImplementation(
        (_, id, handler) => {
          if (id === meta.session_id && fail) throw new Error('register failed')
          handlers.set(id, handler)
          return off
        },
      )
      await mount()
      await act(async () => {
        api.watchConversation(meta.session_id)
        api.watchConversation(sibling.session_id)
      })
      expect(
        api.conversations.find((c) => c.id === meta.session_id)?.hydrated,
      ).toBe(true)
      expect(api.conversationLoadErrors[meta.session_id]).toContain(
        'Unable to receive live messages',
      )
      expect(handlers.has(sibling.session_id)).toBe(true)
      fail = false
      const missed = liveMessage('missed-entry', 'Recovered message')
      vi.mocked(fetchTranscriptTail).mockResolvedValue({
        items: [{ entry_id: missed.entry_id, message: missed.message }],
        hasMore: false,
      })
      await act(async () => {
        if (recovery === 'timer') vi.advanceTimersByTime(2_000)
        else if (recovery === 'retry') api.retryConversations()
        else connectionChanged('reconnecting')
      })
      if (recovery === 'reconnect') {
        await act(async () => connectionChanged('connected'))
      }
      expect(api.conversationLoadErrors[meta.session_id]).toBeUndefined()
      expect(
        api.conversations.find((c) => c.id === meta.session_id)?.messages,
      ).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ content: 'Recovered message' }),
        ]),
      )
      expect(off).not.toHaveBeenCalled()
      // Successful handlers survive reconciliation and still deliver.
      await act(async () => {
        handlers
          .get(meta.session_id)
          ?.onMessageAdded(liveMessage('live-entry', 'Live message'))
      })
      expect(
        api.conversations.find((c) => c.id === meta.session_id)?.messages,
      ).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ content: 'Live message' }),
        ]),
      )
      expect(
        vi
          .mocked(subscribeSessionTranscript)
          .mock.calls.filter(([, id]) => id === sibling.session_id),
      ).toHaveLength(1)
    },
  )

  it('cancels retries on unwatch and rejects callbacks from a previous watch lifecycle', async () => {
    vi.useFakeTimers()
    vi.mocked(subscribeSessionTranscript).mockImplementation(() => {
      throw new Error('register failed')
    })
    await mount()
    let release!: () => void
    await act(async () => {
      release = api.watchConversation(meta.session_id)
    })
    await act(async () => release())
    await act(async () => vi.advanceTimersByTime(5_000))
    expect(subscribeSessionTranscript).toHaveBeenCalledOnce()
    expect(api.conversationLoadErrors[meta.session_id]).toBeUndefined()

    const handlers: SessionTranscriptHandlers[] = []
    const off = vi.fn()
    vi.mocked(subscribeSessionTranscript).mockImplementation(
      (_, __, handler) => {
        handlers.push(handler)
        return off
      },
    )
    await act(async () => {
      release = api.watchConversation(meta.session_id)
    })
    await act(async () => {
      release()
      release = api.watchConversation(meta.session_id)
    })
    expect(off).toHaveBeenCalledOnce()
    await act(async () => {
      handlers[0].onMessageAdded(liveMessage('stale-entry', 'Stale callback'))
      handlers[1].onMessageAdded(liveMessage('fresh-entry', 'Current callback'))
    })
    const messages = api.conversations.find(
      (c) => c.id === meta.session_id,
    )?.messages
    expect(messages).not.toEqual(
      expect.arrayContaining([
        expect.objectContaining({ content: 'Stale callback' }),
      ]),
    )
    expect(messages).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ content: 'Current callback' }),
      ]),
    )
  })
})
