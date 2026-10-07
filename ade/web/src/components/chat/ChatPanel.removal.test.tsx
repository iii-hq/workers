// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import {
  SessionTreeDeletionError,
  type SessionTreeDeletionSnapshot,
} from '@/lib/sessions/delete-tree'
import type { Conversation } from '@/types/chat'

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: vi.fn(() => Promise.reject(new Error('isolated test'))),
}))

const mocks = vi.hoisted(() => ({
  useConversationsCtx: vi.fn(),
  remove: vi.fn(),
  getRemovalPreview: vi.fn(),
  watchConversation: vi.fn(() => vi.fn()),
  /** The id the mocked sidebar asks to remove. */
  target: 'child2',
}))
vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtx: mocks.useConversationsCtx,
}))
vi.mock('@/lib/sessions/removal-preview', () => ({
  getRemovalPreview: mocks.getRemovalPreview,
}))
vi.mock('@/hooks/use-container-narrow', () => ({
  useContainerNarrow: () => [() => {}, false],
}))
vi.mock('@/hooks/use-media-query', () => ({ useMediaQuery: () => false }))
vi.mock('@/components/sidebar/ConversationSidebar', () => ({
  ConversationSidebar: ({ onRemove }: { onRemove: (id: string) => void }) => (
    <button type="button" onClick={() => onRemove(mocks.target)}>
      Request removal
    </button>
  ),
}))
vi.mock('./ChatView', () => ({ ChatView: () => <div>Chat</div> }))

import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { ChatPanel } from './ChatPanel'

function conversation(id: string, parentId?: string): Conversation {
  return {
    id,
    title: id === 'child2' ? 'Selected child' : id,
    parentId,
    status: 'done',
    model: null,
    messages: [],
    hydrated: true,
    createdAt: 1,
    updatedAt: 1,
  }
}
const parent = conversation('parent')
const child = conversation('child2', 'parent')
let root: Root
let host: HTMLDivElement

function deferred() {
  let resolve!: () => void
  let reject!: (error: Error) => void
  const promise = new Promise<void>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}

function button(text: string) {
  const match = [...document.querySelectorAll('button')].find(
    (node) => node.textContent === text,
  )
  if (!match) throw new Error(`Missing button: ${text}`)
  return match
}
const dialog = () => document.querySelector('[role="dialog"]')
const click = async (text: string) => act(async () => button(text).click())
function outcomeCounts() {
  return [
    ...(dialog()?.querySelectorAll('[data-deletion-outcome-counts] > div') ??
      []),
  ]
    .map(
      (node) =>
        `${node.querySelector('dt')?.textContent}: ${node.querySelector('dd')?.textContent}.`,
    )
    .join(' ')
}
async function expandOutcomes() {
  await act(async () => {
    const details = dialog()?.querySelector<HTMLDetailsElement>(
      '[data-deletion-outcomes]',
    )
    if (!details) throw new Error('Missing outcome details')
    details.open = true
    details.dispatchEvent(new Event('toggle'))
  })
}
const pressEscape = async () =>
  act(async () => {
    document.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
    )
  })
const clickOutside = async () => {
  // Radix installs the outside-pointer listener on the next task, so the
  // same pointer gesture that opened the dialog cannot also dismiss it.
  await new Promise((resolve) => setTimeout(resolve, 0))
  await act(async () => {
    document.body.dispatchEvent(
      new MouseEvent('pointerdown', { bubbles: true }),
    )
    document.body.dispatchEvent(new MouseEvent('pointerup', { bubbles: true }))
    document.body.click()
  })
}

beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  vi.mocked(getIiiClient)
    .mockReset()
    .mockRejectedValue(new Error('isolated test'))
  mocks.target = 'child2'
  mocks.remove.mockResolvedValue(undefined)
  mocks.getRemovalPreview.mockReset().mockResolvedValue({
    id: 'child2',
    title: 'Selected child',
    parentId: 'parent',
    hasChildren: true,
    descendantCount: 1,
    hasRunningWork: true,
    empty: false,
  })
  mocks.useConversationsCtx.mockReturnValue({
    conversations: [
      parent,
      conversation('child1', 'parent'),
      child,
      conversation('grandchild1', 'child2'),
    ],
    activeId: parent.id,
    active: parent,
    watchConversation: mocks.watchConversation,
    remove: mocks.remove,
    createNew: vi.fn(),
    select: vi.fn(),
    rename: vi.fn(),
    backend: {},
    modelOptions: [],
    catalogLoading: false,
    connectionState: 'connected',
    missingConversationIds: new Set<string>(),
  })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () =>
    root.render(
      <TooltipProvider>
        <ChatPanel />
      </TooltipProvider>,
    ),
  )
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

describe('ChatPanel contextual delete confirmation', () => {
  const blocked = (): SessionTreeDeletionSnapshot => ({
    operation_id: 'op',
    attempt: 1,
    session_id: 'child2',
    status: 'failed',
    mode: 'normal',
    deleted_session_ids: [],
    data_retained: true,
    unconfirmed_session_ids: [],
    remaining_session_ids: ['child2', 'grandchild1'],
    force_eligible: true,
    failure_code: 'blocked',
    blockers: [
      {
        kind: 'unknown_completion',
        session_id: 'grandchild1',
        function_id: 'browser::fetch',
        call_id: 'call-1',
        started_at: 1,
      },
    ],
  })

  it('does not resubscribe and self-refresh when a recovered eligibility snapshot changes', async () => {
    const trigger = vi
      .fn()
      .mockResolvedValueOnce({ ...blocked(), force_eligible: false })
      .mockResolvedValueOnce(blocked())
      .mockRejectedValue(new Error('third redundant read'))
    let refresh!: () => Promise<void>
    vi.mocked(getIiiClient).mockResolvedValue({
      browserId: 'test',
      trigger,
      on: vi.fn((_id, handler) => {
        refresh = handler
        return vi.fn()
      }),
      registerTrigger: vi.fn(() => vi.fn()),
      addConnectionStateListener: vi.fn(() => vi.fn()),
    } as unknown as IiiClient)
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(blocked()))
    await click('Request removal')
    await click('Stop and delete')
    expect(trigger).toHaveBeenCalledTimes(1)
    expect(dialog()?.textContent).not.toContain('Force delete…')
    await act(async () => refresh())
    expect(trigger).toHaveBeenCalledTimes(2)
    expect(dialog()?.textContent).toContain('Force delete…')
    expect(mocks.remove).toHaveBeenCalledTimes(1)
  })

  it('reviews overlap identity explicitly without retrying the parent or escalating the child', async () => {
    mocks.target = 'parent'
    const overlap = {
      ...blocked(),
      session_id: 'parent',
      operation_id: 'parent-op',
      failure_code: 'overlapping_deletion' as const,
      force_eligible: false,
      existing_deletion: { operation_id: 'child-op', session_id: 'child2' },
    }
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(overlap))
    mocks.getRemovalPreview.mockImplementation(async (item: Conversation) => ({
      id: item.id,
      title: item.title,
      hasChildren: true,
      descendantCount: 1,
      hasRunningWork: item.id === 'child2',
      empty: false,
    }))
    await click('Request removal')
    await click('Delete')
    expect(button('Retry delete').disabled).toBe(false)
    expect(dialog()?.textContent).not.toContain('Force delete…')
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(blocked()))
    await click('Review existing deletion')
    expect(mocks.remove).toHaveBeenLastCalledWith('child2', {
      signal: expect.any(AbortSignal),
      reviewOperationId: 'child-op',
    })
    expect(dialog()?.textContent).toContain('Force delete…')
    expect(mocks.remove).toHaveBeenCalledTimes(2)
  })

  it.each(['missing', 'completed'])(
    'allows explicit normal parent Retry after child is %s',
    async (childState) => {
      mocks.target = 'parent'
      const overlap: SessionTreeDeletionSnapshot = {
        ...blocked(),
        session_id: 'parent',
        operation_id: 'parent-op',
        failure_code: 'overlapping_deletion',
        force_eligible: false,
        existing_deletion: { operation_id: 'child-op', session_id: 'child2' },
      }
      const context = mocks.useConversationsCtx()
      mocks.useConversationsCtx.mockReturnValue({
        ...context,
        conversations:
          childState === 'missing'
            ? [parent, conversation('child1', 'parent')]
            : context.conversations,
      })
      await act(async () =>
        root.render(
          <TooltipProvider>
            <ChatPanel />
          </TooltipProvider>,
        ),
      )
      mocks.getRemovalPreview.mockImplementation(
        async (item: Conversation) => ({
          id: item.id,
          title: item.title,
          hasChildren: true,
          descendantCount: 1,
          hasRunningWork: false,
          empty: false,
        }),
      )
      mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(overlap))
      await click('Request removal')
      await click('Delete')
      expect(mocks.remove).toHaveBeenLastCalledWith('parent', {
        signal: expect.any(AbortSignal),
        recover: true,
      })
      expect(button('Retry delete').disabled).toBe(false)
      await click('Review existing deletion')
      if (childState === 'missing') {
        expect(dialog()?.textContent).toContain('unavailable in this workspace')
        expect(mocks.remove).toHaveBeenCalledTimes(1)
        mocks.remove.mockRejectedValueOnce(
          new SessionTreeDeletionError(overlap),
        )
        await click('Refresh status')
        expect(mocks.remove).toHaveBeenLastCalledWith('parent', {
          signal: expect.any(AbortSignal),
          reviewOperationId: 'parent-op',
        })
        // Read-only status still reports stale parent overlap; normal Retry remains.
      } else {
        expect(mocks.remove).toHaveBeenLastCalledWith('child2', {
          signal: expect.any(AbortSignal),
          reviewOperationId: 'child-op',
        })
        expect(dialog()).toBeNull()
        await click('Request removal')
        mocks.remove.mockRejectedValueOnce(
          new SessionTreeDeletionError(overlap),
        )
        await click('Delete')
      }
      expect(button('Retry delete').disabled).toBe(false)
      const before = mocks.remove.mock.calls.length
      await click('Retry delete')
      expect(mocks.remove).toHaveBeenCalledTimes(before + 1)
      expect(mocks.remove).toHaveBeenLastCalledWith('parent', {
        signal: expect.any(AbortSignal),
      })
      expect(mocks.remove.mock.calls.some(([, options]) => options.force)).toBe(
        false,
      )
      expect(dialog()).toBeNull()
    },
  )

  it('refreshes blocked eligibility from native lifecycle events without retrying deletion', async () => {
    const before = blocked()
    let refresh!: () => Promise<void>
    const off = vi.fn()
    const trigger = vi.fn().mockResolvedValue(before)
    vi.mocked(getIiiClient).mockResolvedValueOnce({
      browserId: 'test-browser',
      on: vi.fn((_id, handler) => {
        refresh = handler as () => Promise<void>
        return off
      }),
      registerTrigger: vi.fn(() => off),
      addConnectionStateListener: vi.fn(() => off),
      trigger,
    } as unknown as IiiClient)
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(before))
    await click('Request removal')
    await click('Stop and delete')
    await click('Force delete…')
    trigger.mockResolvedValueOnce({
      ...before,
      blockers: [],
      force_eligible: false,
    })
    await act(async () => refresh())
    expect(dialog()?.textContent).not.toContain('Force delete this chat?')
    expect(dialog()?.textContent).not.toContain('Force delete…')
    expect(mocks.remove).toHaveBeenCalledTimes(1)
    expect(off).not.toHaveBeenCalled()
    await click('Close')
    expect(off).toHaveBeenCalled()
  })

  it('reports failed empty outcomes as unknown, never zero retained scope', async () => {
    mocks.remove.mockRejectedValueOnce(
      new SessionTreeDeletionError({
        ...blocked(),
        failure_code: 'failed',
        force_eligible: false,
        blockers: [],
        remaining_session_ids: [],
        unconfirmed_session_ids: [],
        data_retained: false,
      }),
    )
    await click('Request removal')
    await click('Stop and delete')
    expect(outcomeCounts()).toContain(
      'Not deleted: unknown. Unconfirmed: unknown.',
    )
    expect(outcomeCounts()).not.toContain('Not deleted: 0')
    expect(dialog()?.textContent).not.toContain('data retained')
    expect(dialog()?.textContent).not.toContain('Force delete…')
  })

  it('labels local active processing without claiming a timeout', async () => {
    mocks.remove.mockRejectedValueOnce(
      new SessionTreeDeletionError({
        ...blocked(),
        blockers: [{ kind: 'active_processing', session_id: 'grandchild1' }],
      }),
    )
    await click('Request removal')
    await click('Stop and delete')
    expect(dialog()?.textContent).toContain('Active processing')
    expect(dialog()?.textContent).not.toContain('after timeout')
  })

  it('requires a distinct force confirmation, focuses Back and sends the confirmed identity once', async () => {
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(blocked()))
    await click('Request removal')
    await click('Stop and delete')
    expect(dialog()?.textContent).toContain(
      'No conversations were deleted; data retained.',
    )
    expect(dialog()?.textContent).not.toMatch(/pending/i)
    expect(dialog()?.textContent).toContain('browser::fetch')
    expect(dialog()?.textContent).toContain('Unknown completion after timeout')
    expect(mocks.remove).toHaveBeenCalledTimes(1)
    await click('Force delete…')
    expect(document.activeElement).toBe(button('Back'))
    expect(dialog()?.textContent).toContain('Force delete this chat?')
    expect(dialog()?.textContent).toContain('External operations may continue')
    expect(mocks.remove).toHaveBeenCalledTimes(1)
    await click('Back')
    expect(dialog()?.textContent).not.toContain('Force delete this chat?')
    await click('Force delete…')
    const pending = deferred()
    mocks.remove.mockReturnValueOnce(pending.promise)
    await act(async () => {
      const confirm = button('Force delete')
      confirm.click()
      confirm.click()
    })
    expect(mocks.remove).toHaveBeenCalledTimes(2)
    expect(mocks.remove).toHaveBeenLastCalledWith('child2', {
      signal: expect.any(AbortSignal),
      force: { operation_id: 'op', attempt: 1 },
    })
    expect(button('Force deleting…').disabled).toBe(true)
    expect(dialog()).not.toBeNull()
    await act(async () => pending.resolve())
    expect(dialog()).toBeNull()
  })

  it.each(['network', 'permission', 'failed', 'missing eligibility'])(
    'never offers force for %s failures',
    async (failure) => {
      const snapshot = blocked()
      if (failure === 'failed') snapshot.failure_code = 'failed'
      if (failure === 'missing eligibility') snapshot.force_eligible = false
      mocks.remove.mockRejectedValueOnce(
        ['network', 'permission'].includes(failure)
          ? new Error(failure)
          : new SessionTreeDeletionError(snapshot),
      )
      await click('Request removal')
      await click('Stop and delete')
      expect(dialog()?.textContent).not.toContain('Force delete…')
    },
  )

  it('shows confirmed deleted and unconfirmed partial scope without offering force', async () => {
    const snapshot = blocked()
    snapshot.mode = 'force'
    snapshot.data_retained = false
    snapshot.unconfirmed_session_ids = undefined
    snapshot.force_eligible = false
    snapshot.deleted_session_ids = ['grandchild1']
    snapshot.remaining_session_ids = ['child2']
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(snapshot))
    await click('Request removal')
    await click('Stop and delete')
    await expandOutcomes()
    expect(dialog()?.textContent).toContain('Deleted sessions: grandchild1')
    expect(dialog()?.textContent).toContain(
      'Deletion outcome unconfirmed: child2',
    )
    expect(dialog()?.textContent).not.toMatch(/retained/i)
    expect(dialog()?.textContent).not.toContain('Force delete…')
    await click('Retry force delete')
    expect(mocks.remove).toHaveBeenLastCalledWith('child2', {
      signal: expect.any(AbortSignal),
      force: { operation_id: 'op', attempt: 1 },
    })
  })
  it('distinguishes not deleted from unconfirmed ids in the new snapshot contract', async () => {
    const snapshot = {
      ...blocked(),
      mode: 'force' as const,
      data_retained: false,
      failure_code: 'failed' as const,
      force_eligible: false,
      unconfirmed_session_ids: ['grandchild1'],
    }
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(snapshot))
    await click('Request removal')
    await click('Stop and delete')
    await expandOutcomes()
    expect(dialog()?.textContent).toContain('Not deleted sessions: child2')
    expect(dialog()?.textContent).toContain(
      'Deletion outcome unconfirmed: grandchild1',
    )
    expect(outcomeCounts()).toContain(
      'Confirmed deleted: 0. Not deleted: 1. Unconfirmed: 1.',
    )
    expect(dialog()?.textContent).not.toMatch(/retained/i)
    expect(dialog()?.textContent).not.toContain('Force delete…')
  })

  it('describes a deterministic Force rejection as no change, not unknown completion', async () => {
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(blocked()))
    await click('Request removal')
    await click('Stop and delete')
    await click('Force delete…')
    mocks.remove.mockRejectedValueOnce(
      Object.assign(new Error('stale confirmation'), {
        code: 'invalid_request',
      }),
    )
    await click('Force delete')
    expect(dialog()?.textContent).toContain(
      'Force delete was rejected; nothing changed.',
    )
    expect(dialog()?.textContent).not.toContain(
      'The backend may have continued.',
    )
    expect(dialog()?.textContent).not.toMatch(/retained/i)
  })

  it('keeps network failures unknown and recovers completion only on an explicit check', async () => {
    mocks.remove.mockRejectedValueOnce(new Error('acknowledgement lost'))
    await click('Request removal')
    await click('Stop and delete')
    expect(dialog()?.textContent).toContain(
      'Completion is not confirmed. The backend may have continued.',
    )
    expect(dialog()?.textContent).not.toMatch(/retained/i)
    expect(dialog()).not.toBeNull()
    expect(mocks.remove).toHaveBeenCalledTimes(1)
    await click('Retry stop and delete')
    expect(mocks.remove).toHaveBeenLastCalledWith('child2', {
      signal: expect.any(AbortSignal),
      recover: true,
    })
    expect(dialog()).toBeNull()
  })

  it('never reports retention when refreshing a known blocker fails', async () => {
    let refresh!: () => Promise<void>
    const trigger = vi.fn().mockRejectedValue(new Error('refresh offline'))
    vi.mocked(getIiiClient).mockResolvedValueOnce({
      browserId: 'test-browser',
      on: vi.fn((_id, handler) => {
        refresh = handler as () => Promise<void>
        return vi.fn()
      }),
      registerTrigger: vi.fn(() => vi.fn()),
      addConnectionStateListener: vi.fn(() => vi.fn()),
      trigger,
    } as unknown as IiiClient)
    mocks.remove.mockRejectedValueOnce(new SessionTreeDeletionError(blocked()))
    await click('Request removal')
    await click('Stop and delete')
    await act(async () => refresh())
    expect(dialog()?.textContent).toContain(
      'Unable to refresh deletion status. Completion is not confirmed; the backend may have continued.',
    )
    expect(dialog()?.textContent).not.toMatch(/retained/i)
    expect(dialog()?.textContent).not.toContain('Force delete…')
    expect(mocks.remove).toHaveBeenCalledTimes(1)
  })

  it.each([
    [false, false, 'Delete'],
    [true, false, 'Delete'],
    [false, true, 'Stop and delete'],
    [true, true, 'Stop and delete'],
  ] as const)(
    'children=%s, running=%s labels the action %s',
    async (hasChildren, hasRunningWork, label) => {
      mocks.getRemovalPreview.mockResolvedValueOnce({
        id: 'child2',
        title: 'Selected child',
        hasChildren,
        descendantCount: hasChildren ? 2 : 0,
        hasRunningWork,
        empty: false,
      })
      await click('Request removal')
      expect(button(label).disabled).toBe(false)
      expect(dialog()?.querySelector('h2')?.textContent).toBe(
        `${label} conversation?`,
      )
      expect(dialog()?.textContent?.includes('subagent')).toBe(hasChildren)
      if (hasChildren)
        expect(dialog()?.textContent).toContain('its 2 subagent conversations')
      expect(dialog()?.textContent?.includes('Running work')).toBe(
        hasRunningWork,
      )
      expect(dialog()?.textContent).not.toContain('parent')
      expect(mocks.remove).not.toHaveBeenCalled()
    },
  )

  it('removes a verified empty leaf without opening a confirmation', async () => {
    mocks.getRemovalPreview.mockResolvedValueOnce({
      id: 'child2',
      title: 'Empty chat',
      hasChildren: false,
      hasRunningWork: false,
      empty: true,
    })
    const pending = deferred()
    mocks.remove.mockReturnValueOnce(pending.promise)
    await click('Request removal')
    await click('Request removal')
    expect(dialog()).toBeNull()
    expect(mocks.remove).toHaveBeenCalledTimes(1)
    expect(document.querySelector('[role="status"]')?.textContent).toBe(
      'Deleting…',
    )
    await act(async () => pending.resolve())
    expect(dialog()).toBeNull()
  })

  it('surfaces failure of direct empty deletion with a Delete retry', async () => {
    mocks.getRemovalPreview.mockResolvedValueOnce({
      id: 'child2',
      title: 'Empty chat',
      hasChildren: false,
      hasRunningWork: false,
      empty: true,
    })
    mocks.remove.mockRejectedValueOnce(new Error('disconnected'))
    await click('Request removal')
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      'disconnected',
    )
    expect(dialog()?.textContent).not.toContain('subagent')
    await click('Retry delete')
    expect(mocks.remove).toHaveBeenCalledTimes(2)
    expect(dialog()).toBeNull()
  })

  it('does not assume empty when the preview fails, and retries the read', async () => {
    mocks.getRemovalPreview.mockRejectedValueOnce(new Error('offline'))
    await click('Request removal')
    expect(dialog()).toBeNull()
    expect(mocks.remove).not.toHaveBeenCalled()
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      'offline',
    )
    await click('Try again')
    expect(dialog()).not.toBeNull()
    expect(mocks.getRemovalPreview).toHaveBeenCalledTimes(2)
    expect(mocks.remove).not.toHaveBeenCalled()
  })
  it('clears a failed preview once session::deleted removes that conversation', async () => {
    mocks.getRemovalPreview.mockRejectedValueOnce(new Error('offline'))
    await click('Request removal')
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      'offline',
    )
    mocks.useConversationsCtx.mockReturnValue({
      ...mocks.useConversationsCtx(),
      conversations: [parent],
    })
    await act(async () =>
      root.render(
        <TooltipProvider>
          <ChatPanel />
        </TooltipProvider>,
      ),
    )
    await click('Try again')
    expect(document.querySelector('[role="alert"]')).toBeNull()
    expect(mocks.getRemovalPreview).toHaveBeenCalledTimes(1)
    expect(mocks.remove).not.toHaveBeenCalled()
  })

  it('keeps a failed preview for another conversation when a missing id is requested', async () => {
    mocks.getRemovalPreview.mockRejectedValueOnce(new Error('offline'))
    await click('Request removal')
    mocks.target = 'already-deleted'
    await click('Request removal')
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      'offline',
    )
    await click('Try again')
    expect(mocks.getRemovalPreview).toHaveBeenCalledTimes(2)
    expect(mocks.getRemovalPreview.mock.calls[1][0].id).toBe('child2')
  })

  it('ignores a preview completed after the panel unmounts', async () => {
    let resolve!: (value: unknown) => void
    mocks.getRemovalPreview.mockReturnValueOnce(
      new Promise((done) => {
        resolve = done
      }),
    )
    await click('Request removal')
    await act(async () => root.render(null))
    await act(async () =>
      resolve({
        id: 'child2',
        title: 'Empty chat',
        hasChildren: false,
        hasRunningWork: false,
        empty: true,
      }),
    )
    expect(mocks.remove).not.toHaveBeenCalled()
  })

  it('uses Deleting progress for an inactive conversation', async () => {
    mocks.getRemovalPreview.mockResolvedValueOnce({
      id: 'child2',
      title: 'Selected child',
      hasChildren: false,
      hasRunningWork: false,
      empty: false,
    })
    const pending = deferred()
    mocks.remove.mockReturnValueOnce(pending.promise)
    await click('Request removal')
    await click('Delete')
    expect(button('Deleting…').disabled).toBe(true)
    expect(dialog()?.textContent).not.toContain('Stopping')
    await act(async () => pending.resolve())
  })

  it('only opens confirmation, describes descendants and surviving parent, and focuses Cancel', async () => {
    await click('Request removal')
    expect(dialog()?.textContent).toContain('Selected child')
    expect(dialog()?.textContent).toContain('its 1 subagent conversation')
    expect(dialog()?.textContent).toContain(
      'The parent will be notified, not stopped or deleted.',
    )
    expect(dialog()?.textContent).toContain('This cannot be undone.')
    expect(dialog()?.hasAttribute('data-chat-delete-dialog')).toBe(true)
    expect(dialog()?.querySelector('[data-chat-delete-actions]')).not.toBeNull()
    expect(document.activeElement).toBe(button('Cancel'))
    expect(mocks.remove).not.toHaveBeenCalled()
  })

  it.each(['cancel', 'escape', 'outside'] as const)(
    '%s before acceptance makes no call',
    async (action) => {
      await click('Request removal')
      if (action === 'cancel') await click('Cancel')
      else if (action === 'escape') await pressEscape()
      else await clickOutside()
      expect(dialog()).toBeNull()
      expect(mocks.remove).not.toHaveBeenCalled()
    },
  )

  it('sends the selected id once, prevents double confirmation and pending dismiss, and waits for completion', async () => {
    const pending = deferred()
    mocks.remove.mockReturnValueOnce(pending.promise)
    await click('Request removal')
    await act(async () => {
      const confirm = button('Stop and delete')
      confirm.click()
      confirm.click()
    })
    expect(mocks.remove).toHaveBeenCalledTimes(1)
    expect(mocks.remove).toHaveBeenCalledWith('child2', {
      signal: expect.any(AbortSignal),
      recover: true,
    })
    expect(button('Stopping and deleting…').disabled).toBe(true)
    expect(button('Cancel').disabled).toBe(true)
    await pressEscape()
    await clickOutside()
    await act(async () => {
      dialog()
        ?.querySelector<HTMLButtonElement>('[aria-label="Close"]')
        ?.click()
    })
    expect(dialog()).not.toBeNull()
    expect(dialog()?.getAttribute('aria-busy')).toBe('true')
    expect(document.querySelector('[role="status"]')?.textContent).toContain(
      "Closing the panel won't cancel deletion",
    )
    await act(async () => pending.resolve())
    expect(dialog()).toBeNull()
  })

  it('keeps the title if session::deleted removes the sidebar row during the wait', async () => {
    const pending = deferred()
    mocks.remove.mockReturnValueOnce(pending.promise)
    await click('Request removal')
    await click('Stop and delete')
    mocks.useConversationsCtx.mockReturnValue({
      ...mocks.useConversationsCtx(),
      conversations: [parent],
    })
    await act(async () =>
      root.render(
        <TooltipProvider>
          <ChatPanel />
        </TooltipProvider>,
      ),
    )
    expect(dialog()?.textContent).toContain('Selected child')
    await act(async () => pending.reject(new Error('finalization failed')))
    expect(dialog()?.textContent).toContain('Selected child')
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      'finalization failed',
    )
    await click('Retry stop and delete')
    expect(mocks.remove).toHaveBeenCalledTimes(2)
    expect(mocks.remove.mock.calls[1][0]).toBe('child2')
    expect(dialog()).toBeNull()
  })

  it('keeps long titles and full errors accessible in the compact dialog', async () => {
    const title = 'Very long conversation '.repeat(30)
    const error = 'Remote invocation failed: '.repeat(100)
    mocks.getRemovalPreview.mockResolvedValueOnce({
      id: 'child2',
      title,
      hasChildren: false,
      hasRunningWork: false,
      empty: false,
    })
    mocks.remove.mockRejectedValueOnce(new Error(error))
    await click('Request removal')
    const descriptionId = dialog()?.getAttribute('aria-describedby')
    const description = document.getElementById(descriptionId ?? '')
    expect(description?.textContent).toContain(title)
    expect(description?.textContent).not.toContain('subagent')
    await click('Delete')
    const alert = dialog()?.querySelector('[role="alert"]')
    expect(alert?.hasAttribute('data-chat-delete-error')).toBe(true)
    expect(alert?.textContent).toContain(error)
    expect(button('Close').disabled).toBe(false)
    expect(button('Retry delete').disabled).toBe(false)
  })

  it('preserves the conversation and error after failure, with safe retry', async () => {
    mocks.remove.mockRejectedValueOnce(new Error('function_not_found'))
    await click('Request removal')
    await click('Stop and delete')
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      'function_not_found',
    )
    expect(mocks.useConversationsCtx().conversations).toContain(child)
    expect(dialog()?.textContent).toContain('Selected child')
    expect(button('Retry stop and delete').disabled).toBe(false)
    await click('Retry stop and delete')
    expect(mocks.remove).toHaveBeenCalledTimes(2)
    expect(dialog()).toBeNull()
  })

  it('unmount stops the client wait, not the backend operation', async () => {
    const pending = deferred()
    mocks.remove.mockReturnValueOnce(pending.promise)
    await click('Request removal')
    await click('Stop and delete')
    const signal = mocks.remove.mock.calls[0][1].signal as AbortSignal
    await act(async () => root.render(null))
    expect(signal.aborted).toBe(true)
    await act(async () => pending.resolve())
    expect(mocks.remove).toHaveBeenCalledTimes(1)
  })
})
