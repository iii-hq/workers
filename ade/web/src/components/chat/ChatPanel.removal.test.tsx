// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { Conversation } from '@/types/chat'

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
  mocks.target = 'child2'
  mocks.remove.mockResolvedValue(undefined)
  mocks.getRemovalPreview.mockReset().mockResolvedValue({
    id: 'child2',
    title: 'Selected child',
    parentId: 'parent',
    hasChildren: true,
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
        hasRunningWork,
        empty: false,
      })
      await click('Request removal')
      expect(button(label).disabled).toBe(false)
      expect(dialog()?.querySelector('h2')?.textContent).toBe(
        `${label} conversation?`,
      )
      expect(dialog()?.textContent?.includes('subagent')).toBe(hasChildren)
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
    expect(dialog()?.textContent).toContain('its subagent conversations')
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
