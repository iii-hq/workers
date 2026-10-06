// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  ensureSession,
  fetchTranscriptTail,
  getSession,
  listSessions,
} from '@/lib/sessions/api'
import { subscribeSessionDirectory } from '@/lib/sessions/events'
import type { SessionMeta } from '@/lib/sessions/types'
import { type ConversationsApi, useConversations } from './use-conversations'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/sessions/api', async (original) => ({
  ...(await original<typeof import('@/lib/sessions/api')>()),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  fetchTranscriptTail: vi.fn(),
  ensureSession: vi.fn(),
}))
vi.mock('@/lib/sessions/events', () => ({
  subscribeSessionDirectory: vi.fn(() => () => {}),
  subscribeSessionTranscript: () => () => {},
}))

let api: ConversationsApi
let root: Root | null = null

function Probe() {
  api = useConversations(undefined, false, true)
  return null
}

/** A fresh mount of the store: what a browser restart does to it. */
async function boot() {
  if (root) await act(async () => root?.unmount())
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root?.render(<Probe />))
}

const newChat = () => {
  const draft = api.conversations.find((c) => c.draft)
  if (!draft) throw new Error('no new chat')
  return draft
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  localStorage.clear()
  vi.mocked(getIiiClient).mockResolvedValue({
    addConnectionStateListener: (listener: (state: string) => void) => {
      listener('connected')
      return () => {}
    },
  } as never)
  vi.mocked(subscribeSessionDirectory).mockImplementation(() => () => {})
  vi.mocked(listSessions).mockResolvedValue([])
  vi.mocked(fetchTranscriptTail).mockResolvedValue({
    items: [],
    hasMore: false,
  })
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
})

describe('the new chat across a browser restart', () => {
  it('comes back with what was typed in it', async () => {
    await boot()
    await act(async () => api.setDraftText(newChat().id, 'half a thought'))
    await boot()
    const draft = newChat()
    expect(api.getDraftText(draft.id)).toBe('half a thought')
  })

  it('forgets the text of a new chat that was removed', async () => {
    await boot()
    const { id } = newChat()
    await act(async () => api.setDraftText(id, 'never mind'))
    await act(async () => api.remove(id))
    await boot()
    expect(api.getDraftText(newChat().id)).toBeUndefined()
  })

  it("keeps another new chat's text when one with the same text is removed", async () => {
    await boot()
    const first = newChat().id
    await act(async () => api.setDraftText(first, 'same words'))
    let second = ''
    await act(async () => {
      second = api.createNew({ text: 'other' })
    })
    await act(async () => api.setDraftText(second, 'same words'))
    await act(async () => api.remove(first))
    await boot()
    expect(api.getDraftText(newChat().id)).toBe('same words')
  })

  it('starts empty once what was typed was sent', async () => {
    await boot()
    const { id } = newChat()
    await act(async () => api.setDraftText(id, 'send me'))
    const meta: SessionMeta = {
      session_id: id,
      title: 'send me',
      description: '',
      status: 'idle',
      metadata: {},
      created_at: 1,
      updated_at: 1,
      message_count: 0,
    }
    vi.mocked(ensureSession).mockResolvedValue({ meta } as never)
    vi.mocked(getSession).mockResolvedValue(meta)
    await act(async () => api.ensureSession(id, 'send me'))
    await boot()
    expect(api.getDraftText(newChat().id)).toBeUndefined()
  })
})
