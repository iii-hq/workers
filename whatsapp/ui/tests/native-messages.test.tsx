// @vitest-environment jsdom
import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageBrowser } from '../src/messages'
import { DraftProvider, DraftScope, useDraftMemory } from '../src/drafts'
import type { ChatTarget, MessageView } from '../src/inbox-model'

vi.mock('@iii-dev/console-ui', () => {
  const Box = ({ children, ...props }: ComponentProps<'div'>) => <div {...props}>{children}</div>
  return {
    PageBody: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    PageMain: Box, List: Box,
    PageSidebar: ({ header, children }: { header: ReactNode; children: ReactNode }) => <aside>{header}{children}</aside>,
    Button: ({ children, variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props} data-variant={variant}>{children}</button>,
    IconButton: ({ label, children, tooltip: _tooltip, ...props }: ComponentProps<'button'> & { label: string; tooltip?: string }) => <button {...props} aria-label={label}>{children}</button>,
    SearchField: () => <input aria-label="Search conversations" />,
    ListItem: ({ label, onClick }: { label: ReactNode; onClick(): void }) => <button onClick={onClick}>{label}</button>,
    Input: () => <input />,
    EmptyState: ({ title }: { title: string }) => <p>{title}</p>,
    Skeleton: () => <p>Loading…</p>,
    StatusPanel: ({ headline, detail, action }: { headline: ReactNode; detail?: ReactNode; action?: ReactNode }) => <div role="alert">{headline}{detail}{action}</div>,
    LiveRegion: () => null,
  }
})

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
const chat: ChatTarget = { chat_id: '15551234567@s.whatsapp.net', title: 'Support contact', is_group: false }
const message: MessageView = { message_id: 'stored', chat_id: chat.chat_id, sender_id: chat.chat_id, from_me: false, timestamp_ms: 1, kind: 'text', text: 'Stored conversation message', source: 'live' }
const coverage = { capture_started_at_ms: 1, message_limit: 10000, retention_truncated: false }
const cleanups: Array<() => void> = []

async function render(connected = false, narrow = true) {
  let archiveError: Error | null = null
  let memory: ReturnType<typeof useDraftMemory> = null
  const MemoryProbe = () => { memory = useDraftMemory(); return null }
  let target = chat, account = 'native:support:1:active', currentConnection = connected
  const trigger = vi.fn(async (id: string, _input?: unknown) => {
    if (id === 'whatsapp::chats::list') return { chats: [], next_cursor: null, coverage }
    if (id === 'whatsapp::messages::list') {
      if (archiveError) throw archiveError
      return { messages: [message], next_cursor: null, coverage }
    }
    if (id === 'whatsapp::messages::send_text') return { message_id: 'sent', chat_id: target.chat_id }
    throw new Error(`Unexpected function: ${id}`)
  })
  const host = { iii: { trigger } } as unknown as Host
  const onDirty = vi.fn(), onGoConnection = vi.fn(), onSelect = vi.fn(async () => true)
  const container = document.createElement('div'); document.body.append(container)
  const root = createRoot(container)
  const update = (next: boolean) => { currentConnection = next; root.render(<DraftProvider><MemoryProbe /><DraftScope id={account}><MessageBrowser key={account} host={host} sessionId="support" connected={next} narrow={narrow} selected={target} onSelect={onSelect} onDirty={onDirty} onGoConnection={onGoConnection} /></DraftScope></DraftProvider>) }
  await act(async () => update(connected))
  cleanups.push(() => { act(() => root.unmount()); container.remove() })
  return {
    container, trigger, onDirty, onGoConnection,
    draftCount() { return memory?.values.size ?? 0 },
    editor() { return container.querySelector<HTMLTextAreaElement>('textarea')! },
    async conversation(next: ChatTarget) { target = next; await act(async () => update(currentConnection)) },
    async incarnation(next: string) { account = next; await act(async () => update(currentConnection)) },
    async connection(next: boolean) { await act(async () => update(next)) },
    async click(label: string) {
      const button = Array.from(container.querySelectorAll('button')).find((item) => item.getAttribute('aria-label') === label || item.textContent === label)
      expect(button, `Missing button: ${label}`).toBeDefined()
      await act(async () => button!.click())
    },
    async draft(value: string) {
      const field = container.querySelector<HTMLTextAreaElement>('textarea')!
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, value)
        field.dispatchEvent(new Event('input', { bubbles: true }))
      })
    },
    async key(options: KeyboardEventInit = {}) {
      const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true, ...options })
      await act(async () => container.querySelector<HTMLTextAreaElement>('textarea')!.dispatchEvent(event))
      return event
    },
    failArchive() { archiveError = new Error('Archive temporarily unavailable') },
    restoreArchive() { archiveError = null },
  }
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
})
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); vi.unstubAllGlobals(); vi.useRealTimers() })

describe('Native offline message reader', () => {
  it('keeps an empty editor mounted behind the draft action and focuses it when opened', async () => {
    const view = await render()
    const editor = view.editor()
    expect(editor.closest('[hidden]')).not.toBeNull()
    expect(view.container.querySelector('.wa-ui-messages')?.textContent).toContain('Stored conversation message')
    expect(view.container.querySelector('.wa-ui-messages .wa-ui-archive-details')).not.toBeNull()
    expect(view.container.querySelectorAll('.wa-ui-offline-band')).toHaveLength(1)
    await view.click('Write a draft')
    expect(view.editor()).toBe(editor)
    expect(editor.closest('[hidden]')).toBeNull()
    expect(document.activeElement).toBe(editor)
    await view.click('Hide draft editor')
    expect(editor.closest('[hidden]')).not.toBeNull()
    await view.click('Connect')
    expect(view.onGoConnection).toHaveBeenCalledOnce()
    expect(view.trigger.mock.calls.every(([id]) => id.endsWith('::list'))).toBe(true)
  })

  it('preserves an accessible unsent draft and editor instance through disconnection and refresh errors', async () => {
    const view = await render(true)
    const editor = view.editor()
    await view.draft('Keep this unsent draft')
    await view.connection(false)
    expect(view.editor()).toBe(editor)
    expect(editor.value).toBe('Keep this unsent draft')
    expect(editor.closest('[hidden]')).toBeNull()
    expect(view.container.textContent).toContain('Offline · draft kept')
    expect(view.container.querySelector<HTMLButtonElement>('[aria-label="Write a draft"]')).toBeNull()
    expect(Array.from(view.container.querySelectorAll('button')).find((item) => item.textContent === 'Send')?.disabled).toBe(true)
    view.failArchive()
    await view.click('Refresh messages')
    expect(view.container.textContent).toContain('Archive temporarily unavailable')
    expect(view.editor()).toBe(editor)
    expect(editor.value).toBe('Keep this unsent draft')
    view.restoreArchive()
    await view.click('Refresh messages')
    await view.connection(true)
    expect(view.editor()).toBe(editor)
    expect(editor.value).toBe('Keep this unsent draft')
    expect(view.onDirty.mock.calls.at(-1)).toEqual([true])
    expect(view.trigger.mock.calls.every(([id]) => id.endsWith('::list'))).toBe(true)
  })

  it('restores drafts per conversation and clears a submitted draft without sending on restoration', async () => {
    const view = await render(true)
    await view.draft('Draft for the first chat')
    const another = { ...chat, chat_id: '15559876543@s.whatsapp.net', title: 'Another contact' }
    await view.conversation(another)
    expect(view.editor().value).toBe('')
    await view.draft('Draft for the second chat')
    await view.conversation(chat)
    expect(view.editor().value).toBe('Draft for the first chat')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(0)
    await view.click('Send')
    await view.conversation(another)
    expect(view.editor().value).toBe('Draft for the second chat')
    await view.conversation(chat)
    expect(view.editor().value).toBe('')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(1)
  })

  it('isolates a removed and recreated native session from its previous incarnation', async () => {
    const view = await render(true)
    await view.draft('Private draft from the old account')
    await view.incarnation('native:support:2:active')
    expect(view.editor().value).toBe('')
    expect(view.trigger.mock.calls.every(([id]) => id.endsWith('::list'))).toBe(true)
  })

  it('keeps archive availability outside the message scroll area in a wide pane', async () => {
    const view = await render(true, false)
    expect(view.container.querySelector('.wa-ui-messages .wa-ui-archive-details')).toBeNull()
    expect(view.container.querySelector('.wa-ui-reader > .wa-ui-archive-details')).not.toBeNull()
  })

  it.each(['ctrlKey', 'metaKey'] as const)('submits once with %s + Enter and exposes its shortcut to the editor', async (modifier) => {
    const view = await render(true)
    await view.draft('Explicit keyboard send')
    const hintId = view.editor().getAttribute('aria-describedby')
    expect(hintId).toBeTruthy()
    expect(document.getElementById(hintId!)?.textContent).toContain('Ctrl/Cmd+Enter to send')
    const event = await view.key({ [modifier]: true })
    expect(event.defaultPrevented).toBe(true)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(1)
    expect(view.editor().value).toBe('')
  })

  it.each([
    ['plain Enter', {}],
    ['a repeated shortcut', { ctrlKey: true, repeat: true }],
    ['IME composition', { ctrlKey: true, isComposing: true }],
    ['the IME key code', { ctrlKey: true, keyCode: 229 }],
  ] satisfies Array<[string, KeyboardEventInit]>)('does not send on %s', async (_label, options) => {
    const view = await render(true)
    await view.draft('Keep this message')
    await view.key(options)
    expect(view.editor().value).toBe('Keep this message')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(0)
  })

  it('does not send a keyboard shortcut while offline or with an empty draft', async () => {
    const view = await render(false)
    await view.click('Write a draft')
    await view.draft('Offline draft')
    await view.key({ ctrlKey: true })
    expect(view.editor().value).toBe('Offline draft')
    await view.connection(true)
    await view.draft('   ')
    await view.key({ metaKey: true })
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(0)
  })

  it('prevents duplicate keyboard sends while the first request is in flight', async () => {
    const view = await render(true)
    await view.draft('One explicit request')
    const original = view.trigger.getMockImplementation()!
    let complete!: (value: { message_id: string; chat_id: string }) => void
    view.trigger.mockImplementation((id) => id.endsWith('send_text') ? new Promise((resolve) => { complete = resolve }) : original(id))
    await act(async () => {
      view.editor().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true, cancelable: true }))
      view.editor().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', metaKey: true, bubbles: true, cancelable: true }))
    })
    expect(view.editor().disabled).toBe(true)
    await view.key({ metaKey: true })
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(1)
    await act(async () => complete({ message_id: 'sent', chat_id: chat.chat_id }))
    expect(view.editor().value).toBe('')
  })

  it('keeps the draft and blocks keyboard sends while requesting older history', async () => {
    const view = await render(true)
    await view.draft('Wait for history')
    const original = view.trigger.getMockImplementation()!
    let complete!: (value: { request_id: string; requested_count: number }) => void
    view.trigger.mockImplementation((id) => id.endsWith('request_history') ? new Promise((resolve) => { complete = resolve }) : original(id))
    await view.click('Request older messages')
    await view.key({ ctrlKey: true })
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(0)
    expect(view.editor().value).toBe('Wait for history')
    await act(async () => complete({ request_id: 'history-request', requested_count: 50 }))
    await view.key({ ctrlKey: true })
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(1)
  })

  it('retains an unconfirmed send through editing and remount, checks without resending, and requires an explicit resend', async () => {
    const view = await render(true)
    const original = view.trigger.getMockImplementation()!
    view.trigger.mockImplementation(async (id) => {
      if (id.endsWith('send_text')) throw new Error('Timed out waiting for a response')
      return original(id)
    })
    await view.draft('Initial attempt')
    await view.click('Send')
    expect(view.container.textContent).toContain('Message not confirmed')
    expect(view.container.textContent).toContain('Sending again may duplicate the previous message')
    expect(Array.from(view.container.querySelectorAll('button')).some((button) => button.textContent === 'Dismiss')).toBe(false)
    await view.draft('Edited explicit retry')
    expect(view.container.textContent).toContain('Timed out waiting for a response')
    const another = { ...chat, chat_id: '15559876543@s.whatsapp.net', title: 'Another contact' }
    await view.conversation(another)
    await view.conversation(chat)
    expect(view.editor().value).toBe('Edited explicit retry')
    expect(view.container.textContent).toContain('Sending again may duplicate the previous message')
    await view.key({ ctrlKey: true })
    await view.key({ metaKey: true })
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(1)
    const listCount = view.trigger.mock.calls.filter(([id]) => id.endsWith('::list')).length
    await view.click('Check this conversation')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('::list')).length).toBeGreaterThan(listCount)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(1)
    expect(view.container.textContent).toContain('Message not confirmed')
    const resend = Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Send again')!
    const check = Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Check this conversation')!
    expect(resend.dataset.variant).toBe('ghost')
    expect(check.dataset.variant).not.toBe('ghost')
    view.trigger.mockImplementation(original)
    await view.click('Send again')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(2)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text')).at(-1)?.[1]).toEqual({ session_id: 'support', chat_id: chat.chat_id, text: 'Edited explicit retry' })
    expect(view.editor().value).toBe('')
    expect(view.container.textContent).not.toContain('Message not confirmed')
    expect(Array.from(view.container.querySelectorAll('button')).some((button) => button.textContent === 'Send')).toBe(true)
    await view.conversation(another)
    await view.conversation(chat)
    expect(view.container.textContent).not.toContain('Message not confirmed')
  })

  it('keeps an unresolved send warning after clearing its text and reopening the conversation', async () => {
    const view = await render(true)
    const original = view.trigger.getMockImplementation()!
    view.trigger.mockImplementation(async (id) => {
      if (id.endsWith('send_text')) throw new Error('Response is unknown')
      return original(id)
    })
    await view.draft('Unknown attempt')
    await view.click('Send')
    await view.draft('')
    await view.conversation({ ...chat, chat_id: '15559876543@s.whatsapp.net', title: 'Another contact' })
    await view.conversation(chat)
    expect(view.editor().value).toBe('')
    expect(view.container.textContent).toContain('Response is unknown')
    expect(view.container.textContent).toContain('Sending again may duplicate the previous message')
    expect(Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Send again')?.disabled).toBe(true)
    expect(view.draftCount()).toBe(0)
    expect(view.onDirty.mock.calls.at(-1)).toEqual([false])
    await view.click('Check this conversation')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(1)
  })

  it('treats local validation as a known non-send with a dismissible error', async () => {
    const view = await render(true)
    await view.draft('x'.repeat(65_537))
    await view.click('Send')
    expect(view.container.textContent).toContain('Message not sent')
    expect(view.container.textContent).toContain('The message exceeds the 65,536-byte limit.')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('send_text'))).toHaveLength(0)
    expect(view.container.textContent).not.toContain('Check this conversation')
    await view.click('Dismiss')
    expect(view.container.textContent).not.toContain('Message not sent')
    expect(view.editor().value).toHaveLength(65_537)
  })
})
