// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  fetchTranscriptTail,
  getSession,
  listSessions,
  patchSessionMeta,
  setSessionMeta,
} from '@/lib/sessions/api'
import {
  type SessionDirectoryHandlers,
  subscribeSessionDirectory,
} from '@/lib/sessions/events'
import type { SessionMeta } from '@/lib/sessions/types'
import {
  __resetWorktreeClaimsForTests,
  recordConsoleClaim,
} from '@/lib/worktree-claims'
import { releaseWorktree } from '@/lib/worktrees'
import { type ConversationsApi, useConversations } from './use-conversations'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/sessions/api', async (original) => ({
  ...(await original<typeof import('@/lib/sessions/api')>()),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  fetchTranscriptTail: vi.fn(),
  setSessionMeta: vi.fn(),
  patchSessionMeta: vi.fn(),
}))
vi.mock('@/lib/sessions/events', () => ({
  subscribeSessionDirectory: vi.fn(),
  subscribeSessionTranscript: () => () => {},
}))
vi.mock('@/lib/worktrees', async (original) => ({
  ...(await original<typeof import('@/lib/worktrees')>()),
  releaseWorktree: vi.fn(),
}))

const OLD_MODEL = 'anthropic::claude-opus-5-5'
const NEW_MODEL = 'openai-codex::codex/gpt-6-astra'
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
let handlers: SessionDirectoryHandlers

function Probe() {
  api = useConversations(undefined, false, true)
  return null
}

function lastMetadata() {
  return vi.mocked(setSessionMeta).mock.lastCall?.[0].metadata
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
    (_client, callbacks) => {
      handlers = callbacks
      return () => {}
    },
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

describe('session metadata writes', () => {
  // "Continue in Work" on the limit card: the account alone, never the
  // conversation's other fields from a stale draft, never through set-meta.
  it("pins a conversation's Claude Code account with a patch of that key alone", async () => {
    vi.mocked(patchSessionMeta).mockResolvedValue(undefined as never)
    await act(() => api.setClaudeAccount(id, 'default'))
    expect(patchSessionMeta).toHaveBeenCalledExactlyOnceWith({
      session_id: id,
      set: { claude_account: 'default' },
    })
    expect(setSessionMeta).not.toHaveBeenCalled()
    const conv = () => api.conversations.find((c) => c.id === id)
    expect(conv()?.sessionMetadata?.claude_account).toBe('default')
    expect(conv()?.title).toBe('Saved chat')

    await act(() => api.setClaudeAccount(id, null))
    expect(patchSessionMeta).toHaveBeenLastCalledWith({
      session_id: id,
      unset: ['claude_account'],
    })
    expect(conv()?.sessionMetadata).not.toHaveProperty('claude_account')

    // A refused patch puts the local value back and says why.
    vi.mocked(patchSessionMeta).mockRejectedValueOnce(new Error('refused'))
    await act(() =>
      expect(api.setClaudeAccount(id, 'personal')).rejects.toThrow('refused'),
    )
    expect(conv()?.sessionMetadata).not.toHaveProperty('claude_account')
    expect(setSessionMeta).not.toHaveBeenCalled()
  })

  // ModelPicker.selectModel: onChange(next), then onThinkingLevelChange.
  it('sends a model pick and its effort as one write', async () => {
    await act(async () => {
      api.setModel(id, NEW_MODEL)
      api.setThinkingLevel(id, 'low')
    })
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalledTimes(1))
    expect(setSessionMeta).toHaveBeenCalledWith({
      session_id: id,
      metadata: expect.objectContaining({
        model: NEW_MODEL,
        thinking_level: 'low',
      }),
    })
  })

  it('keeps the new model when the pick resets the effort to default', async () => {
    await act(async () => {
      api.setModel(id, NEW_MODEL)
      api.setThinkingLevel(id, 'default')
    })
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalledTimes(1))
    expect(lastMetadata()).toMatchObject({ model: NEW_MODEL })
    expect(lastMetadata()).not.toHaveProperty('thinking_level')
    expect(api.conversations.find((c) => c.id === id)?.model).toBe(NEW_MODEL)
  })

  // The live failure: the echo of a stale write put the old model back, and
  // the next send used it.
  it('keeps the pick after the server echoes the write', async () => {
    vi.mocked(setSessionMeta).mockImplementation(async (input) => {
      handlers.onMetaUpdated?.({
        session_id: input.session_id,
        title: meta.title,
        description: '',
        metadata: input.metadata,
        timestamp: 10,
      })
      return { meta }
    })
    await act(async () => {
      api.setModel(id, NEW_MODEL)
      api.setThinkingLevel(id, 'default')
    })
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalled())
    await act(async () => {})
    // The clock proves the echo landed; the pick alone would pass otherwise.
    expect(api.conversations.find((c) => c.id === id)).toMatchObject({
      model: NEW_MODEL,
      thinkingLevel: 'default',
      serverMetadataUpdatedAt: 10,
    })
  })

  it('keeps a worker control key set in the same tick as a model pick', async () => {
    await act(async () => {
      api.setSessionMetadata(id, { judge_provider: 'semif' })
      api.setModel(id, NEW_MODEL)
    })
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalledTimes(1))
    expect(lastMetadata()).toMatchObject({
      judge_provider: 'semif',
      model: NEW_MODEL,
    })
  })

  it('carries a rename and a model pick of the same tick in one write', async () => {
    await act(async () => {
      api.rename(id, '  Renamed  ')
      api.setModel(id, NEW_MODEL)
    })
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalledTimes(1))
    expect(setSessionMeta).toHaveBeenCalledWith({
      session_id: id,
      title: 'Renamed',
      metadata: expect.objectContaining({
        model: NEW_MODEL,
        title_manual: true,
      }),
    })
  })

  it('builds on earlier edits when called through an older render', async () => {
    const earlier = api
    await act(async () => api.setModel(id, NEW_MODEL))
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalledTimes(1))
    await act(async () => earlier.setMemoryBank(id, 'notes'))
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalledTimes(2))
    expect(lastMetadata()).toMatchObject({
      model: NEW_MODEL,
      memory_bank: 'notes',
    })
  })

  it('keeps a local draft off the wire', async () => {
    const draftId = api.conversations.find((c) => c.draft)?.id ?? ''
    expect(draftId).not.toBe('')
    await act(async () => {
      api.setModel(draftId, NEW_MODEL)
      api.setModel(id, NEW_MODEL)
    })
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalledTimes(1))
    expect(vi.mocked(setSessionMeta).mock.calls[0]?.[0].session_id).toBe(id)
  })
})

describe('the time of a conversation is its session updated_at (D50)', () => {
  const time = () => api.conversations.find((c) => c.id === id)?.updatedAt
  const projectEvent = (projectId: string, timestamp: number) =>
    handlers.onMetaUpdated?.({
      session_id: id,
      title: meta.title,
      description: '',
      metadata: { ...meta.metadata, project_id: projectId },
      timestamp,
    })

  const reads = () =>
    vi.mocked(getSession).mock.calls.filter(([s]) => s === id).length

  it('keeps the age and the place when a move keeps updated_at', async () => {
    expect(time()).toBe(2)
    const before = reads()
    // Move to project, then No project: patch-meta touch:false, the event
    // arrives now, the session's updated_at stays 2.
    await act(async () => projectEvent('prj_acme', 1_795_463_536_081))
    await act(async () => projectEvent('none', 1_795_463_536_090))
    expect(api.conversations.find((c) => c.id === id)?.projectId).toBe('none')
    expect(time()).toBe(2)
    // One read for the burst.
    await vi.waitFor(() => expect(reads()).toBe(before + 1))
    await new Promise((resolve) => setTimeout(resolve, 400))
    expect(reads()).toBe(before + 1)
    expect(time()).toBe(2)
  })

  it('takes a newer updated_at from the read, not from the event', async () => {
    vi.mocked(getSession).mockResolvedValue({ ...meta, updated_at: 40 })
    await act(async () => projectEvent('prj_acme', 99))
    expect(time()).toBe(2)
    await vi.waitFor(() => expect(time()).toBe(40))
  })
})

describe('working directory moved by the harness', () => {
  const workingDir = () =>
    api.conversations.find((c) => c.id === id)?.workingDir
  const moved = (movedFrom: string, timestamp: number) =>
    handlers.onMetaUpdated?.({
      session_id: id,
      title: meta.title,
      description: '',
      metadata: { fs_scope: { root: '/repo.feat', moved_from: movedFrom } },
      timestamp,
    })

  it('follows the move without writing it back and releases the old claim', async () => {
    // The chat shows /repo, the folder the move starts from.
    await act(async () => root.unmount())
    localStorage.clear()
    const rooted = {
      ...meta,
      metadata: { ...meta.metadata, fs_scope: { root: '/repo' } },
    }
    vi.mocked(listSessions).mockResolvedValue([rooted])
    vi.mocked(getSession).mockResolvedValue(rooted)
    root = createRoot(host)
    await act(async () => root.render(<Probe />))
    expect(workingDir()).toBe('/repo')
    __resetWorktreeClaimsForTests()
    recordConsoleClaim(id, { worktreeId: 'wt-1', path: '/repo' })

    // A move from a folder this chat does not show is ignored.
    await act(async () => moved('/other', 10))
    expect(workingDir()).toBe('/repo')
    expect(releaseWorktree).not.toHaveBeenCalled()

    await act(async () => moved('/repo', 11))
    expect(workingDir()).toBe('/repo.feat')
    expect(releaseWorktree).toHaveBeenCalledWith('wt-1', id)
    expect(setSessionMeta).not.toHaveBeenCalled()
  })

  it('keeps the moved folder when a metadata edit lands before render', async () => {
    await act(async () => root.unmount())
    localStorage.clear()
    const rooted = {
      ...meta,
      metadata: { ...meta.metadata, fs_scope: { root: '/repo' } },
    }
    vi.mocked(listSessions).mockResolvedValue([rooted])
    vi.mocked(getSession).mockResolvedValue(rooted)
    root = createRoot(host)
    await act(async () => root.render(<Probe />))

    // A non-click editor (a worker control) writes in the same task as the move.
    await act(async () => {
      moved('/repo', 11)
      api.setSessionMetadata(id, { judge_provider: 'semif' })
    })
    await vi.waitFor(() => expect(setSessionMeta).toHaveBeenCalledTimes(1))
    expect(lastMetadata()).toMatchObject({ fs_scope: { root: '/repo.feat' } })
  })
})
