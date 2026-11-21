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
    expect(send).toHaveBeenCalledWith({ text: 'edited', attachments: [] })
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
    const removeQueued = vi
      .fn()
      .mockRejectedValueOnce(new Error('timeout'))
      .mockResolvedValue(true)
    const { hook, send } = setup({ removeQueued })
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
      hook.result.current.edits.abandon('q1', { text: 'x', attachments: [] }),
    )
    await flush()
    expect(keepAsDraft).not.toHaveBeenCalled()
  })
})
