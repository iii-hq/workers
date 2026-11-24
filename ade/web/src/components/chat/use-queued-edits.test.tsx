// @vitest-environment jsdom

import { act, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { UserMessage } from '@/types/chat'
import { useQueuedEdits } from './use-queued-edits'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mounted: Array<() => void> = []
afterEach(() => {
  for (const unmount of mounted.splice(0)) unmount()
})

function renderHook<T>(use: () => T) {
  const container = document.createElement('div')
  const root = createRoot(container)
  let latest: T | undefined
  function Probe() {
    latest = use()
    return null
  }
  act(() => root.render(<Probe />))
  mounted.push(() => act(() => root.unmount()))
  return {
    result: {
      get current() {
        return latest as T
      },
    },
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

const queued: UserMessage = {
  id: 'q1',
  role: 'user',
  content: 'original ask',
  createdAt: 1,
} as UserMessage

function setup(opts: {
  removeQueued: (cid: string, id: string) => Promise<boolean>
  listQueued?: (cid: string) => Promise<{ id: string }[]>
  canSend?: boolean
}) {
  const send = vi.fn()
  const keepAsDraft = vi.fn()
  const warn = vi.fn()
  const hook = renderHook(() => {
    const [drafts, setDrafts] = useState<UserMessage[]>([queued])
    const ref = { current: drafts }
    const edits = useQueuedEdits({
      conversationId: 'c1',
      removeQueued: opts.removeQueued,
      listQueued: opts.listQueued,
      queuedDraftsRef: ref,
      setQueuedDrafts: setDrafts,
      send,
      canSend: () => opts.canSend ?? true,
      keepAsDraft,
      warn,
    })
    return { drafts, edits }
  })
  return { hook, send, keepAsDraft, warn }
}

const flush = () => act(async () => {})

describe('useQueuedEdits', () => {
  it('holds a browsed message until its edit settles, then sends the edit', async () => {
    const { hook, send } = setup({ removeQueued: async () => true })
    act(() => void hook.result.current.edits.reserve('q1'))
    expect(hook.result.current.edits.held.has('q1')).toBe(true)
    await flush()
    act(() =>
      hook.result.current.edits.edit('q1', { text: 'edited', attachments: [] }),
    )
    await flush()
    expect(send).toHaveBeenCalledWith(
      { text: 'edited', attachments: [] },
      'edit',
    )
    expect(hook.result.current.edits.held.size).toBe(0)
    expect(hook.result.current.drafts).toEqual([])
  })

  it('never sends an edit whose original already reached the agent', async () => {
    // Enter lands before the reservation answers; the original drained.
    const reservation = deferred<boolean>()
    const removeQueued = vi
      .fn()
      .mockReturnValueOnce(reservation.promise)
      .mockResolvedValue(false)
    const { hook, send, keepAsDraft } = setup({ removeQueued })
    act(() => void hook.result.current.edits.reserve('q1'))
    act(() =>
      hook.result.current.edits.edit('q1', { text: 'edited', attachments: [] }),
    )
    reservation.resolve(false)
    await flush()
    expect(send).not.toHaveBeenCalled()
    expect(keepAsDraft).toHaveBeenCalledWith(
      'c1',
      { text: 'edited', attachments: [] },
      'edit',
    )
    // Delivered, so its row stays to pop when the transcript row lands.
    expect(hook.result.current.drafts).toEqual([queued])
    expect(hook.result.current.edits.held.size).toBe(0)
  })

  it('sends the edit once a retried removal confirms the original is out', async () => {
    // The first unqueue failed, but the queue still lists the message, so
    // it was not removed and Enter may try again.
    const removeQueued = vi
      .fn()
      .mockRejectedValueOnce(new Error('timeout'))
      .mockResolvedValue(true)
    const { hook, send } = setup({
      removeQueued,
      listQueued: async () => [{ id: 'q1' }],
    })
    act(() => void hook.result.current.edits.reserve('q1'))
    await flush()
    act(() =>
      hook.result.current.edits.edit('q1', { text: 'edited', attachments: [] }),
    )
    await flush()
    expect(send).toHaveBeenCalledTimes(1)
  })

  it('keeps an edit as a draft when the chat cannot send', async () => {
    const { hook, send, keepAsDraft } = setup({
      removeQueued: async () => true,
      canSend: false,
    })
    act(() => void hook.result.current.edits.reserve('q1'))
    await flush()
    act(() =>
      hook.result.current.edits.edit('q1', { text: 'edited', attachments: [] }),
    )
    await flush()
    expect(send).not.toHaveBeenCalled()
    expect(keepAsDraft).toHaveBeenCalledWith(
      'c1',
      { text: 'edited', attachments: [] },
      'edit',
    )
  })

  it('keeps a released original as a draft when the chat cannot send', async () => {
    const { hook, send, keepAsDraft } = setup({
      removeQueued: async () => true,
      canSend: false,
    })
    act(() => void hook.result.current.edits.reserve('q1'))
    await flush()
    act(() => hook.result.current.edits.release('q1'))
    await flush()
    expect(send).not.toHaveBeenCalled()
    expect(keepAsDraft).toHaveBeenCalledWith(
      'c1',
      { text: 'original ask', attachments: [] },
      'original',
    )
  })

  it('browsing back in before the reservation answers keeps the message', async () => {
    // ↑, ↓, ↑ faster than the harness answers: one request, nothing sent,
    // and the message is still the composer's to edit.
    const reservation = deferred<boolean>()
    const removeQueued = vi.fn().mockReturnValue(reservation.promise)
    const { hook, send, warn } = setup({ removeQueued })
    act(() => void hook.result.current.edits.reserve('q1'))
    act(() => hook.result.current.edits.release('q1'))
    act(() => void hook.result.current.edits.reserve('q1'))
    reservation.resolve(true)
    await flush()
    expect(removeQueued).toHaveBeenCalledTimes(1)
    expect(send).not.toHaveBeenCalled()
    expect(warn).not.toHaveBeenCalled()
    expect(hook.result.current.drafts).toEqual([queued])
    expect(hook.result.current.edits.held.has('q1')).toBe(true)
    act(() =>
      hook.result.current.edits.edit('q1', { text: 'edited', attachments: [] }),
    )
    await flush()
    expect(send).toHaveBeenCalledTimes(1)
    expect(send).toHaveBeenCalledWith(
      { text: 'edited', attachments: [] },
      'edit',
    )
    expect(hook.result.current.drafts).toEqual([])
    expect(hook.result.current.edits.held.size).toBe(0)
  })

  it('releases an adopted reservation once, after the last browse away', async () => {
    // ↑, ↓, ↑, ↓ before the answer: the original goes back exactly once.
    const reservation = deferred<boolean>()
    const removeQueued = vi.fn().mockReturnValue(reservation.promise)
    const { hook, send } = setup({ removeQueued })
    act(() => void hook.result.current.edits.reserve('q1'))
    act(() => hook.result.current.edits.release('q1'))
    act(() => void hook.result.current.edits.reserve('q1'))
    act(() => hook.result.current.edits.release('q1'))
    reservation.resolve(true)
    await flush()
    expect(removeQueued).toHaveBeenCalledTimes(1)
    expect(send).toHaveBeenCalledTimes(1)
    expect(send).toHaveBeenCalledWith(
      { text: 'original ask', attachments: [] },
      'original',
    )
    expect(hook.result.current.drafts).toEqual([])
    expect(hook.result.current.edits.held.size).toBe(0)
  })

  it('lets go of a re-browsed message whose reservation answers false', async () => {
    const reservation = deferred<boolean>()
    const removeQueued = vi.fn().mockReturnValue(reservation.promise)
    const { hook, send, warn } = setup({ removeQueued })
    act(() => void hook.result.current.edits.reserve('q1'))
    act(() => hook.result.current.edits.release('q1'))
    let adopted: boolean | undefined
    act(() => {
      void hook.result.current.edits.reserve('q1').then((r) => {
        adopted = r
      })
    })
    reservation.resolve(false)
    await flush()
    expect(adopted).toBe(false)
    expect(removeQueued).toHaveBeenCalledTimes(1)
    expect(warn).toHaveBeenCalledTimes(1)
    expect(send).not.toHaveBeenCalled()
    // Delivered already: its row stays until the transcript row lands.
    expect(hook.result.current.drafts).toEqual([queued])
    expect(hook.result.current.edits.held.size).toBe(0)
  })

  it('moves an abandoned edit into the draft instead of losing it', async () => {
    const { hook, send, keepAsDraft } = setup({
      removeQueued: async () => true,
    })
    act(() => void hook.result.current.edits.reserve('q1'))
    await flush()
    act(() =>
      hook.result.current.edits.abandon('q1', {
        text: 'half typed',
        attachments: [],
      }),
    )
    await flush()
    expect(send).not.toHaveBeenCalled()
    expect(keepAsDraft).toHaveBeenCalledWith(
      'c1',
      { text: 'half typed', attachments: [] },
      'edit',
    )
    expect(hook.result.current.edits.held.size).toBe(0)
  })

  it('leaves an abandoned message alone when it was never taken back', async () => {
    const { hook, keepAsDraft } = setup({ removeQueued: async () => false })
    act(() => void hook.result.current.edits.reserve('q1'))
    await flush()
    expect(hook.result.current.edits.held.size).toBe(0)
    act(() =>
      hook.result.current.edits.abandon('q1', {
        text: 'original ask',
        attachments: [],
      }),
    )
    await flush()
    expect(keepAsDraft).not.toHaveBeenCalled()
  })

  it('keeps a changed edit when leaving races a reservation that fails', async () => {
    // Edited, then switched chats before unqueue answered "already gone":
    // the original still delivers, and the edit lands in the draft.
    const reservation = deferred<boolean>()
    const { hook, send, keepAsDraft } = setup({
      removeQueued: () => reservation.promise,
    })
    act(() => void hook.result.current.edits.reserve('q1'))
    act(() =>
      hook.result.current.edits.abandon('q1', {
        text: 'edited ask',
        attachments: [],
      }),
    )
    reservation.resolve(false)
    await flush()
    expect(send).not.toHaveBeenCalled()
    expect(keepAsDraft).toHaveBeenCalledWith(
      'c1',
      { text: 'edited ask', attachments: [] },
      'edit',
    )
    expect(hook.result.current.drafts).toEqual([queued])
  })

  it('never resends an original whose unqueue answer was lost', async () => {
    // The call rejected and the queue no longer lists the message: it may
    // have been removed, so it is kept as a draft rather than sent again.
    const { hook, send, keepAsDraft, warn } = setup({
      removeQueued: async () => {
        throw new Error('timeout')
      },
      listQueued: async () => [],
    })
    let reserved: boolean | undefined
    act(() => {
      void hook.result.current.edits.reserve('q1').then((r) => {
        reserved = r
      })
    })
    await flush()
    expect(reserved).toBe(true)
    expect(warn).toHaveBeenCalledTimes(1)
    act(() => hook.result.current.edits.release('q1'))
    await flush()
    expect(send).not.toHaveBeenCalled()
    expect(keepAsDraft).toHaveBeenCalledWith(
      'c1',
      { text: 'original ask', attachments: [] },
      'original',
    )
    expect(hook.result.current.drafts).toEqual([])
    expect(hook.result.current.edits.held.size).toBe(0)
  })

  it('lets a failed unqueue the queue still lists be delivered as queued', async () => {
    const { hook, send, keepAsDraft } = setup({
      removeQueued: async () => {
        throw new Error('timeout')
      },
      listQueued: async () => [{ id: 'q1' }],
    })
    let reserved: boolean | undefined
    act(() => {
      void hook.result.current.edits.reserve('q1').then((r) => {
        reserved = r
      })
    })
    await flush()
    expect(reserved).toBe(false)
    expect(hook.result.current.edits.held.size).toBe(0)
    expect(send).not.toHaveBeenCalled()
    expect(keepAsDraft).not.toHaveBeenCalled()
  })

  it('an earlier attempt settling late never clears a newer hold', async () => {
    // Enter on a browsed message, then ↑ back into the same row before the
    // first reservation answers: only the newest browse may let go.
    const first = deferred<boolean>()
    const second = deferred<boolean>()
    const removeQueued = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const { hook } = setup({ removeQueued })
    act(() => void hook.result.current.edits.reserve('q1'))
    act(() =>
      hook.result.current.edits.edit('q1', { text: 'edited', attachments: [] }),
    )
    act(() => void hook.result.current.edits.reserve('q1'))
    first.resolve(true)
    await flush()
    expect(hook.result.current.edits.held.has('q1')).toBe(true)
    second.resolve(false)
    await flush()
    expect(hook.result.current.edits.held.size).toBe(0)
  })

  it('a release that crosses a Stop goes to the draft, not out', async () => {
    // ↑, ↓, then Stop before unqueue answers: no new turn starts.
    const reservation = deferred<boolean>()
    const { hook, send, keepAsDraft } = setup({
      removeQueued: () => reservation.promise,
    })
    act(() => void hook.result.current.edits.reserve('q1'))
    act(() => hook.result.current.edits.release('q1'))
    act(() => hook.result.current.edits.stopped())
    reservation.resolve(true)
    await flush()
    expect(send).not.toHaveBeenCalled()
    expect(keepAsDraft).toHaveBeenCalledWith(
      'c1',
      { text: 'original ask', attachments: [] },
      'original',
    )
    expect(hook.result.current.edits.held.size).toBe(0)
  })
})
