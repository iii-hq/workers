// @vitest-environment jsdom

import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { CloudPanel } from '../src/cloud'
import type { CloudStatus } from '../src/cloud-model'
import type { ChatView, MessagePage, MessageView } from '../src/inbox-model'

// Keep the real Cloud workspace, archive hook, polling and React reconciliation.
// Only the ADE host's visual primitives are replaced in this DOM test.
vi.mock('@iii-dev/console-ui', () => {
  const Box = ({ children }: { children?: ReactNode }) => <div>{children}</div>
  return {
    Badge: Box, List: Box, PageBody: Box, PageMain: Box, SettingsList: Box,
    PageSidebar: ({ children, header }: { children?: ReactNode; header?: ReactNode }) => <aside>{header}{children}</aside>,
    Button: ({ children, variant: _variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props}>{children}</button>,
    IconButton: ({ children, label, ...props }: ComponentProps<'button'> & { label: string }) => <button {...props} aria-label={label}>{children}</button>,
    ListItem: ({ label, onClick }: { label: ReactNode; onClick(): void }) => <button onClick={onClick}>{label}</button>,
    Input: ({ onChange, ...props }: Omit<ComponentProps<'input'>, 'onChange'> & { onChange?(value: string): void }) => <input {...props} onChange={(event) => onChange?.(event.target.value)} />,
    SearchField: () => <input aria-label="Search conversations" />,
    Select: () => <select aria-label="Message type" />,
    CodeEditor: ({ value, onChange, ...props }: { value: string; onChange(value: string): void; id?: string; 'aria-label'?: string }) => <textarea id={props.id} aria-label={props['aria-label']} value={value} onChange={(event) => onChange(event.target.value)} />,
    SettingsField: ({ field, renderControl }: { field: string; renderControl(props: { id: string }): ReactNode }) => <div>{renderControl({ id: field })}</div>,
    EmptyState: ({ title }: { title: string }) => <p>{title}</p>,
    Skeleton: () => <p>Loading…</p>,
    StatusPanel: ({ headline, detail, action }: { headline: string; detail?: string; action?: ReactNode }) => <div role="alert">{headline}{detail}{action}</div>,
  }
})

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const coverage = { capture_started_at_ms: 1, message_limit: 10000, retention_truncated: false }
const status: CloudStatus = { enabled: true, configured: true, missing_fields: [], api_version: 'v26.0', business_account_id: '123', phone_number_id: '456', access_token_set: true, app_secret_set: true, verify_token_set: true, webhook_ready: true, webhook_path: '/webhooks/whatsapp-cloud/test' }
const message = (chat: string, id: string, timestamp: number): MessageView => ({ message_id: id, chat_id: chat, sender_id: chat, from_me: false, timestamp_ms: timestamp, kind: 'text', text: `${chat}: ${id}`, source: 'live' })
const chats: ChatView[] = ['+15551234567', '+15557654321'].map((chat_id, index) => ({ chat_id, title: index ? 'Customer B' : 'Customer A', is_group: false, message_count: 2 }))
const page = (chat: string): MessagePage => ({ messages: [message(chat, 'new', 3000), message(chat, 'middle', 2000)], next_cursor: 'older', coverage })
const cleanups: Array<() => void> = []

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

async function render() {
  vi.useFakeTimers()
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {})
  const pending: Array<Promise<MessagePage>> = []
  const trigger = vi.fn(async (id: string, input: { chat_id?: string; cursor?: string } = {}) => {
    if (id === 'whatsapp::cloud::status') return status
    if (id === 'whatsapp::cloud::tunnel::status') return { enabled: false, status: 'stopped', tunnel_id: 'webhooks', public_url: null, webhook_url: null, webhook_path: status.webhook_path, error: null }
    if (id === 'whatsapp::cloud::chats::list') return { chats, next_cursor: null, coverage }
    if (id === 'whatsapp::cloud::messages::list') return pending.shift() ?? (input.cursor
      ? { messages: [message(input.chat_id!, 'middle', 2000), message(input.chat_id!, 'old', 1000)], next_cursor: null, coverage }
      : page(input.chat_id!))
    throw new Error(`Unexpected function: ${id}`)
  })
  const host = { iii: { trigger, on: () => () => {}, registerTrigger: () => () => {}, browserId: 'test' } } as unknown as Host
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  const navigate = async (action: () => void) => { action(); return true }
  await act(async () => root.render(<CloudPanel host={host} narrow={false} onDirty={() => {}} navigate={navigate} />))
  cleanups.push(() => { act(() => root.unmount()); container.remove() })
  return {
    container, pending, errors, trigger,
    async click(label: string) {
      const button = Array.from(container.querySelectorAll('button')).find((element) => element.textContent === label || element.getAttribute('aria-label') === label)
      expect(button, `Missing button: ${label}`).toBeDefined()
      await act(async () => button!.click())
    },
    texts() { return Array.from(container.querySelectorAll('.wa-ui-message-text'), (element) => element.textContent) },
  }
}

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
  vi.restoreAllMocks()
  vi.useRealTimers()
})

describe('Cloud conversation context', () => {
  it('keeps exactly one history and composer when switching chats repeatedly', async () => {
    const view = await render()
    for (const index of [0, 1, 0, 1, 0]) {
      await view.click(chats[index].title)
      expect(view.container.querySelectorAll('[role="log"]')).toHaveLength(1)
      expect(view.container.querySelectorAll('[aria-label="Send a Cloud message"]')).toHaveLength(1)
      expect(view.texts()).toEqual([`${chats[index].chat_id}: middle`, `${chats[index].chat_id}: new`])
    }
    expect(view.errors).not.toHaveBeenCalled()
  })

  it('clears the old conversation while loading another and ignores its delayed poll', async () => {
    const view = await render()
    await view.click('Customer A')
    const oldPoll = deferred<MessagePage>()
    view.pending.push(oldPoll.promise)
    await view.click('Refresh Cloud messages')
    const nextChat = deferred<MessagePage>()
    view.pending.push(nextChat.promise)
    await view.click('Customer B')
    expect(view.texts()).toEqual([])
    await act(async () => nextChat.resolve(page(chats[1].chat_id)))
    await act(async () => oldPoll.resolve(page(chats[0].chat_id)))
    expect(view.texts()).toEqual([`${chats[1].chat_id}: middle`, `${chats[1].chat_id}: new`])
    expect(view.container.querySelectorAll('[role="log"]')).toHaveLength(1)
  })

  it('shows overlapping history pages once in chronological order through polling', async () => {
    const view = await render()
    await view.click('Customer A')
    await view.click('Load older stored messages')
    const expected = ['old', 'middle', 'new'].map((id) => `${chats[0].chat_id}: ${id}`)
    expect(view.texts()).toEqual(expected)
    await act(async () => { await vi.advanceTimersByTimeAsync(5000) })
    expect(view.texts()).toEqual(expected)
    expect(view.trigger.mock.calls.some(([id]) => id.includes('send_'))).toBe(false)
  })

  it('removes the history when starting a new conversation', async () => {
    const view = await render()
    await view.click('Customer A')
    await view.click('New Cloud message')
    expect(view.container.querySelectorAll('[role="log"]')).toHaveLength(0)
    expect(view.texts()).toEqual([])
    expect(view.container.querySelectorAll('[aria-label="Send a Cloud message"]')).toHaveLength(1)
  })
})
