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
import type { SessionMeta } from '@/lib/sessions/types'
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
  subscribeSessionTranscript: () => () => {},
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
