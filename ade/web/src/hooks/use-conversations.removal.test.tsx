// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  fetchTranscriptTail,
  getSession,
  listSessions,
} from '@/lib/sessions/api'
import {
  deleteSessionTree,
  type SessionTreeDeletionSnapshot,
} from '@/lib/sessions/delete-tree'
import {
  type SessionDirectoryHandlers,
  subscribeSessionDirectory,
  subscribeSessionTranscript,
} from '@/lib/sessions/events'
import type { SessionMeta } from '@/lib/sessions/types'
import { releaseConsoleClaimIfAny } from '@/lib/worktree-claims'
import { type ConversationsApi, useConversations } from './use-conversations'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/sessions/api', async (original) => ({
  ...(await original<typeof import('@/lib/sessions/api')>()),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  fetchTranscriptTail: vi.fn(),
}))
vi.mock('@/lib/sessions/delete-tree', () => ({ deleteSessionTree: vi.fn() }))
vi.mock('@/lib/sessions/events', () => ({
  subscribeSessionDirectory: vi.fn(),
  subscribeSessionTranscript: vi.fn(),
}))
vi.mock('@/lib/worktree-claims', () => ({ releaseConsoleClaimIfAny: vi.fn() }))

const meta = (id: string, parentId?: string): SessionMeta => ({
  session_id: id,
  title: id,
  description: '',
  status: 'done',
  metadata: parentId ? { parent_session_id: parentId } : {},
  created_at: 1,
  updated_at: 2,
  message_count: 1,
})
const rows = [
  meta('parent'),
  meta('child1', 'parent'),
  meta('child2', 'parent'),
  meta('grandchild1', 'child2'),
]
const completed: SessionTreeDeletionSnapshot = {
  operation_id: 'synthetic-delete-op',
  attempt: 1,
  session_id: 'child2',
  status: 'completed',
  deleted_session_ids: ['child2', 'grandchild1', 'unlisted-descendant'],
}
let api: ConversationsApi
let root: Root
let host: HTMLDivElement
let handlers: SessionDirectoryHandlers
const unsubscribeTranscript = vi.fn()

function Probe() {
  api = useConversations(undefined, false, true)
  return null
}

function deferred() {
  let resolve!: (value: SessionTreeDeletionSnapshot) => void
  let reject!: (error: Error) => void
  const promise = new Promise<SessionTreeDeletionSnapshot>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}
const ids = () => api.conversations.map((item) => item.id)

beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
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
  vi.mocked(subscribeSessionTranscript).mockReturnValue(unsubscribeTranscript)
  vi.mocked(listSessions).mockResolvedValue(rows)
  vi.mocked(getSession).mockImplementation(
    async (id) => rows.find((row) => row.session_id === id) ?? null,
  )
  vi.mocked(fetchTranscriptTail).mockResolvedValue({
    items: [],
    hasMore: false,
  })
  vi.mocked(deleteSessionTree).mockResolvedValue(completed)
  await act(async () => root.render(<Probe />))
  await act(async () => {
    api.select('parent')
    api.watchConversation('child2')
  })
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

describe('useConversations.remove', () => {
  it('retains rows, watches and claims while pending, then removes only backend-confirmed ids', async () => {
    const pending = deferred()
    vi.mocked(deleteSessionTree).mockReturnValueOnce(pending.promise)
    let removal!: Promise<void>
    await act(async () => {
      removal = api.remove('child2')
    })
    expect(ids()).toEqual(
      expect.arrayContaining(rows.map((row) => row.session_id)),
    )
    expect(releaseConsoleClaimIfAny).not.toHaveBeenCalled()
    expect(unsubscribeTranscript).not.toHaveBeenCalled()
    expect(deleteSessionTree).toHaveBeenCalledTimes(1)
    expect(deleteSessionTree).toHaveBeenCalledWith('child2', undefined)
    await act(async () => {
      pending.resolve(completed)
      await removal
    })
    expect(ids()).toContain('parent')
    expect(ids()).toContain('child1')
    expect(ids()).not.toContain('child2')
    expect(ids()).not.toContain('grandchild1')
    expect(api.activeId).toBe('parent')
    expect(unsubscribeTranscript).toHaveBeenCalledTimes(1)
    expect(
      vi.mocked(releaseConsoleClaimIfAny).mock.calls.map(([id]) => id),
    ).toEqual(completed.deleted_session_ids)
  })

  it('exposes failures without losing rows, watches or claims and allows retry', async () => {
    vi.mocked(deleteSessionTree).mockRejectedValueOnce(
      new Error('backend unavailable'),
    )
    await act(async () => {
      await expect(api.remove('child2')).rejects.toThrow('backend unavailable')
    })
    expect(ids()).toEqual(
      expect.arrayContaining(rows.map((row) => row.session_id)),
    )
    expect(unsubscribeTranscript).not.toHaveBeenCalled()
    expect(releaseConsoleClaimIfAny).not.toHaveBeenCalled()
    await act(async () => api.remove('child2'))
    expect(deleteSessionTree).toHaveBeenCalledTimes(2)
    expect(ids()).not.toContain('child2')
  })

  it('removes a local draft without any backend deletion command', async () => {
    let draftId!: string
    await act(async () => {
      draftId = api.createNew()
    })
    expect(api.conversations.find((row) => row.id === draftId)?.draft).toBe(
      true,
    )
    await act(async () => api.remove(draftId))
    expect(deleteSessionTree).not.toHaveBeenCalled()
    expect(ids()).not.toContain(draftId)
    expect(ids()).toContain('parent')
  })

  it('keeps authoritative session::deleted reconciliation and still retries an absent target', async () => {
    await act(async () => {
      handlers.onDeleted?.({ session_id: 'child2', timestamp: 3 })
    })
    expect(ids()).not.toContain('child2')
    expect(ids()).toContain('grandchild1')
    expect(releaseConsoleClaimIfAny).not.toHaveBeenCalled()
    await act(async () => api.remove('child2'))
    expect(deleteSessionTree).toHaveBeenCalledWith('child2', undefined)
    expect(ids()).not.toContain('grandchild1')
    expect(ids()).toContain('child1')
  })

  it('passes client unmount cancellation through without optimistic cleanup', async () => {
    const pending = deferred()
    vi.mocked(deleteSessionTree).mockReturnValueOnce(pending.promise)
    const controller = new AbortController()
    let removal!: Promise<void>
    await act(async () => {
      removal = api.remove('child2', { signal: controller.signal })
    })
    expect(deleteSessionTree).toHaveBeenCalledWith('child2', {
      signal: controller.signal,
    })
    await act(async () => {
      controller.abort()
      pending.reject(new Error('Stopped waiting'))
      await expect(removal).rejects.toThrow('Stopped waiting')
    })
    expect(ids()).toContain('child2')
    expect(releaseConsoleClaimIfAny).not.toHaveBeenCalled()
  })
})
