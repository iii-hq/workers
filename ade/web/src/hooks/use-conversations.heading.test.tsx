// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { summarizeChatTitle } from '@/lib/chat-title'
import { getIiiClient } from '@/lib/iii-client'
import {
  fetchTranscriptTail,
  getSession,
  listSessions,
  setSessionMeta,
} from '@/lib/sessions/api'
import {
  type SessionDirectoryHandlers,
  subscribeSessionDirectory,
} from '@/lib/sessions/events'
import type { SessionMeta } from '@/lib/sessions/types'
import { type ConversationsApi, useConversations } from './use-conversations'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/sessions/api', async (original) => ({
  ...(await original<typeof import('@/lib/sessions/api')>()),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  fetchTranscriptTail: vi.fn(),
  setSessionMeta: vi.fn(),
}))
vi.mock('@/lib/chat-title', () => ({ summarizeChatTitle: vi.fn() }))
vi.mock('@/lib/sessions/events', () => ({
  subscribeSessionDirectory: vi.fn(),
  subscribeSessionTranscript: () => () => {},
}))

const OLD_MODEL = 'claude-code::claude-code/claude-opus-5-5'
const ASK =
  'give me an analysis of all the literary works of franz kafka and how each one relates to his life'
const DERIVED = 'give me an analysis of all the literary works of franz kafka…'
const HEADING = "Kafka's literary works and his life"
const meta: SessionMeta = {
  session_id: 'saved-chat',
  title: 'Saved chat',
  description: '',
  status: 'done',
  metadata: { model: OLD_MODEL, thinking_level: 'xhigh' },
  created_at: 1,
  updated_at: 2,
  message_count: 1,
}
const id = meta.session_id
let api: ConversationsApi
let root: Root
let host: HTMLDivElement

function Probe() {
  api = useConversations(undefined, false, true)
  return null
}

beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  localStorage.clear()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  vi.mocked(getIiiClient).mockResolvedValue({
    addConnectionStateListener: (listener: (state: string) => void) => {
      listener('connected')
      return () => {}
    },
  } as never)
  vi.mocked(subscribeSessionDirectory).mockImplementation(
    (_client, _callbacks: SessionDirectoryHandlers) => () => {},
  )
  vi.mocked(listSessions).mockResolvedValue([meta])
  vi.mocked(getSession).mockResolvedValue(meta)
  vi.mocked(fetchTranscriptTail).mockResolvedValue({
    items: [],
    hasMore: false,
  })
  vi.mocked(setSessionMeta).mockResolvedValue({ meta })
  await act(async () => root.render(<Probe />))
  expect(api.conversations.some((c) => c.id === id && !c.draft)).toBe(true)
  expect(setSessionMeta).not.toHaveBeenCalled()
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const ask = (n: number, content = ASK) => ({
  id: `u${n}`,
  role: 'user' as const,
  content,
  createdAt: n,
})

function titleOf() {
  return api.conversations.find((c) => c.id === id)?.title
}

describe('a chat named by its first ask', () => {
  // A session that exists before its first message (a project or Kanban
  // start, a saved draft) never passes through ensureSession's draft path.
  it("renames a server-backed session with the model's heading", async () => {
    vi.mocked(summarizeChatTitle).mockResolvedValue(HEADING)
    await act(async () => api.appendMessage(id, ask(1)))
    expect(summarizeChatTitle).toHaveBeenCalledWith(expect.anything(), {
      sessionId: id,
      prompt: ASK,
      model: OLD_MODEL,
    })
    await vi.waitFor(() =>
      expect(setSessionMeta).toHaveBeenCalledWith(
        expect.objectContaining({ session_id: id, title: HEADING }),
      ),
    )
    expect(titleOf()).toBe(HEADING)
  })

  it('keeps the opening words when no heading comes back', async () => {
    vi.mocked(summarizeChatTitle).mockResolvedValue(null)
    await act(async () => api.appendMessage(id, ask(1)))
    await vi.waitFor(() => expect(summarizeChatTitle).toHaveBeenCalled())
    expect(titleOf()).toBe(DERIVED)
    expect(setSessionMeta).not.toHaveBeenCalledWith(
      expect.objectContaining({ title: expect.anything() }),
    )
  })

  it('lets a rename made before the heading lands win', async () => {
    let resolve: (title: string) => void = () => {}
    vi.mocked(summarizeChatTitle).mockReturnValue(
      new Promise((r) => {
        resolve = r
      }),
    )
    await act(async () => api.appendMessage(id, ask(1)))
    await act(async () => api.rename(id, 'Kafka notes'))
    await act(async () => resolve(HEADING))
    expect(titleOf()).toBe('Kafka notes')
    expect(setSessionMeta).not.toHaveBeenCalledWith(
      expect.objectContaining({ title: HEADING }),
    )
  })

  it('asks once: a later message does not rename the chat', async () => {
    vi.mocked(summarizeChatTitle).mockResolvedValue(HEADING)
    await act(async () => api.appendMessage(id, ask(1)))
    await act(async () => api.appendMessage(id, ask(2, 'and his letters too')))
    expect(summarizeChatTitle).toHaveBeenCalledTimes(1)
  })
})
