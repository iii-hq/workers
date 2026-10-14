// @vitest-environment jsdom

import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CloudPanel } from '../src/cloud'
import { DraftProvider } from '../src/drafts'
import type { CloudStatus, CloudTemplate } from '../src/cloud-model'
import type { ChatView, MessagePage, MessageView } from '../src/inbox-model'

// Keep the real Cloud workspace, archive hook, polling and React reconciliation.
// Only the ADE host's visual primitives are replaced in this DOM test.
const configurationBridge = vi.hoisted(() => vi.fn())
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
    Select: ({ options, onChange, value, placeholder, ...props }: { options: Array<{ value: string; label: string; disabled?: boolean }>; onChange(value: string): void; value?: string; placeholder?: string; 'aria-label'?: string; disabled?: boolean }) => <select {...props} value={value ?? ''} onChange={(event) => onChange(event.target.value)}><option value="">{placeholder}</option>{options.map((option) => <option key={option.value} value={option.value} disabled={option.disabled}>{option.label}</option>)}</select>,
    CodeEditor: ({ value, onChange, ...props }: { value: string; onChange(value: string): void; id?: string; 'aria-label'?: string }) => <textarea id={props.id} aria-label={props['aria-label']} value={value} onChange={(event) => onChange(event.target.value)} />,
    SettingsField: ({ field, renderControl }: { field: string; renderControl(props: { id: string }): ReactNode }) => <div>{renderControl({ id: field })}</div>,
    EmptyState: ({ title }: { title: string }) => <p>{title}</p>,
    Skeleton: () => <p>Loading…</p>,
    StatusPanel: ({ headline, detail, action, role = 'alert' }: { headline: string; detail?: ReactNode; action?: ReactNode; role?: string }) => <div role={role}>{headline}{detail}{action}</div>,
    WorkerConfigurationDialog: ({ configurationId }: { configurationId: string | null }) => { configurationBridge(configurationId); return null },
  }
})

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const coverage = { capture_started_at_ms: 1, message_limit: 10000, retention_truncated: false }
const status: CloudStatus = { enabled: true, configured: true, missing_fields: [], api_version: 'v26.0', business_account_id: '123', phone_number_id: '456', access_token_set: true, app_secret_set: true, verify_token_set: true, webhook_ready: true, webhook_path: '/webhooks/whatsapp-cloud/test' }
const approved: CloudTemplate = { id: 'approved', name: 'request_update', language: 'en_US', status: 'APPROVED', category: 'UTILITY', parameter_format: 'NAMED', components: [{ type: 'BODY', text: 'Hello {{customer}}.' }] }
const message = (chat: string, id: string, timestamp: number): MessageView => ({ message_id: id, chat_id: chat, sender_id: chat, from_me: false, timestamp_ms: timestamp, kind: 'text', text: `${chat}: ${id}`, source: 'live' })
const chats: ChatView[] = ['+15551234567', '+15557654321'].map((chat_id, index) => ({ chat_id, title: index ? 'Customer B' : 'Customer A', is_group: false, message_count: 2 }))
const page = (chat: string): MessagePage => ({ messages: [message(chat, 'new', 3000), message(chat, 'middle', 2000)], next_cursor: 'older', coverage })
const cleanups: Array<() => void> = []

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

async function render(currentStatus = status, narrow = false, tunnelEnabled = false, configurationId?: string) {
  vi.useFakeTimers()
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {})
  const pending: Array<Promise<MessagePage>> = []
  let statusError: Error | undefined
  let navigationAllowed = true
  const trigger = vi.fn(async (id: string, input: { chat_id?: string; cursor?: string } = {}) => {
    if (id === 'whatsapp::cloud::status') { if (statusError) throw statusError; return currentStatus }
    if (id === 'whatsapp::cloud::tunnel::stop') tunnelEnabled = false
    if (id === 'whatsapp::cloud::tunnel::status' || id === 'whatsapp::cloud::tunnel::stop') return { enabled: tunnelEnabled, status: tunnelEnabled ? 'ready' : 'stopped', tunnel_id: 'webhooks', public_url: null, webhook_url: null, webhook_path: status.webhook_path, error: null }
    if (id === 'whatsapp::cloud::chats::list') return { chats, next_cursor: null, coverage }
    if (id === 'whatsapp::cloud::templates::list') return { templates: [approved], next_cursor: null }
    if (id === 'whatsapp::cloud::messages::list') return pending.shift() ?? (input.cursor
      ? { messages: [message(input.chat_id!, 'middle', 2000), message(input.chat_id!, 'old', 1000)], next_cursor: null, coverage }
      : page(input.chat_id!))
    throw new Error(`Unexpected function: ${id}`)
  })
  const host = { iii: { trigger, on: () => () => {}, registerTrigger: () => () => {}, browserId: 'test' } } as unknown as Host
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  const navigate = vi.fn(async (action: () => void) => { if (!navigationAllowed) return false; action(); return true })
  let visible = true
  const show = () => root.render(<DraftProvider>{visible && <CloudPanel host={host} narrow={narrow} configurationId={configurationId} onDirty={() => {}} navigate={navigate} />}</DraftProvider>)
  await act(async () => show())
  cleanups.push(() => { act(() => root.unmount()); container.remove() })
  return {
    container, pending, errors, trigger, navigate,
    setStatus(value: CloudStatus) { currentStatus = value },
    failStatus(error: Error) { statusError = error },
    restoreStatus() { statusError = undefined },
    blockNavigation() { navigationAllowed = false },
    allowNavigation() { navigationAllowed = true },
    async visible(next: boolean) { visible = next; await act(async () => show()) },
    async fill(label: string, value: string) {
      const id = Array.from(container.querySelectorAll('label')).find((element) => element.textContent === label)?.htmlFor
      const field = (id ? container.querySelector(`[id="${id}"]`) : container.querySelector(`[aria-label="${label}"]`)) as HTMLInputElement | HTMLSelectElement
      expect(field, `Missing field: ${label}`).toBeTruthy()
      await act(async () => {
        const prototype = field.tagName === 'SELECT' ? HTMLSelectElement.prototype : field.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype
        Object.getOwnPropertyDescriptor(prototype, 'value')!.set!.call(field, value)
        field.dispatchEvent(new Event(field.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }))
      })
    },
    async click(label: string) {
      const button = Array.from(container.querySelectorAll('button')).find((element) => element.textContent === label || element.getAttribute('aria-label') === label)
      expect(button, `Missing button: ${label}`).toBeDefined()
      await act(async () => button!.click())
    },
    texts() { return Array.from(container.querySelectorAll('.wa-ui-message-text'), (element) => element.textContent) },
  }
}

beforeEach(() => {
  configurationBridge.mockClear()
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
})

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('Cloud conversation context', () => {
  it.each([false, true])('shows the recipient once in the header after a new message is accepted (narrow: %s)', async (narrow) => {
    const view = await render(status, narrow)
    const original = view.trigger.getMockImplementation()!
    view.trigger.mockImplementation(async (id, input) => id === 'whatsapp::cloud::messages::send_text'
      ? { chat_id: chats[0].chat_id, message_id: 'wamid.header-test', status: 'accepted' }
      : original(id, input))
    await view.click('Test templates or attachments')
    await view.fill('Recipient', chats[0].chat_id)
    await view.fill('Cloud message text', 'A deliberate test')
    await view.click('Send')
    const header = view.container.querySelector('header')!
    expect(header.querySelector('h3.wa-ui-title')?.textContent).toBe(chats[0].chat_id)
    expect(header.textContent?.split(chats[0].chat_id)).toHaveLength(2)
    expect(header.querySelector('.wa-ui-identity')).toBeNull()
    expect(header.querySelector('[aria-label="Back to Cloud tests"]')).not.toBeNull()
    expect(header.querySelector('[aria-label="Refresh Cloud messages"]')).not.toBeNull()
    expect(view.container.querySelector(`[aria-label="Stored Cloud messages with ${chats[0].chat_id}"]`)).not.toBeNull()
    expect(view.trigger.mock.calls.filter(([id]) => id.includes('::send_'))).toHaveLength(1)
    expect(view.errors).not.toHaveBeenCalled()
  })

  it.each([false, true])('keeps a distinct conversation title and recipient in the header (narrow: %s)', async (narrow) => {
    const view = await render(status, narrow)
    if (narrow) await view.click('Browse stored conversations')
    await view.click('Customer A')
    const header = view.container.querySelector('header')!
    expect(header.querySelector('h3.wa-ui-title')?.textContent).toBe(chats[0].title)
    expect(header.querySelector('.wa-ui-identity')?.textContent).toBe(chats[0].chat_id)
    expect(header.textContent?.split(chats[0].chat_id)).toHaveLength(2)
    expect(view.container.querySelector(`[aria-label="Stored Cloud messages with ${chats[0].title}"]`)).not.toBeNull()
    expect(view.trigger.mock.calls.some(([id]) => id.includes('::send_'))).toBe(false)
  })

  it.each([false, true])('keeps recipient and message type help in the new-message header (narrow: %s)', async (narrow) => {
    const view = await render(status, narrow)
    await view.click('Test templates or attachments')
    const header = view.container.querySelector('header')!
    expect(header.querySelector('h3.wa-ui-title')?.textContent).toBe('New Cloud message')
    expect(header.querySelector('.wa-ui-identity')?.textContent).toBe('Choose a recipient and message type.')
    expect(view.container.querySelector('[aria-label="Send a Cloud message"]')).not.toBeNull()
    expect(view.trigger.mock.calls.some(([id]) => id.includes('::send_'))).toBe(false)
  })

  it.each([undefined, 'configured-whatsapp-worker'])('opens the Cloud configuration shortcut for %s without changing worker state', async (configurationId) => {
    const view = await render({ ...status, enabled: false }, false, false, configurationId)
    expect(configurationBridge.mock.calls.every(([id]) => id === null)).toBe(true)
    await view.click('Configure Cloud API')
    expect(configurationBridge).toHaveBeenLastCalledWith(configurationId ?? 'whatsapp')
    expect(view.trigger.mock.calls.every(([id]) => id === 'whatsapp::cloud::status')).toBe(true)
    expect(view.container.querySelector('[aria-label="Send a Cloud message"]')).toBeNull()
  })

  it.each([
    ['disabled', { ...status, enabled: false }],
    ['incomplete', { ...status, configured: false, app_secret_set: false, webhook_ready: false, missing_fields: ['cloud.app_secret'] }],
  ] as const)('shows setup instead of inactive tools when Cloud API is %s', async (_label, currentStatus) => {
    const view = await render(currentStatus)
    expect(view.container.textContent).toContain('Set up Meta Cloud API')
    expect(view.container.textContent).toContain('Configure Cloud API')
    expect(Array.from(view.container.querySelectorAll('.wa-ui-setup-steps > li strong'), (step) => step.textContent)).toEqual(['Account', 'Webhook', 'Test'])
    expect(view.container.querySelector('aside')).toBeNull()
    expect(view.container.querySelector('[aria-label="Cloud API integration test"]')).toBeNull()
    expect(view.container.querySelector('[aria-label="Send a Cloud message"]')).toBeNull()
    expect(view.container.querySelector('[aria-label="Development webhook"]')).toBeNull()
    await act(async () => { await vi.advanceTimersByTimeAsync(15000) })
    expect(view.trigger.mock.calls.every(([id]) => id === 'whatsapp::cloud::status')).toBe(true)
    expect(view.errors).not.toHaveBeenCalled()
  })

  it('preserves setup guidance and recovery when status refresh fails', async () => {
    const view = await render({ ...status, configured: false, app_secret_set: false, webhook_ready: false, missing_fields: ['cloud.app_secret'] })
    view.failStatus(new Error('Worker unavailable'))
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.container.textContent).toContain('Set up Meta Cloud API')
    expect(view.container.textContent).toContain('Could not refresh Cloud API status')
    expect(view.container.textContent).toContain('Worker unavailable')
    view.restoreStatus()
    await view.click('Try again')
    expect(view.container.textContent).not.toContain('Could not refresh Cloud API status')
    expect(view.trigger.mock.calls.every(([id]) => id === 'whatsapp::cloud::status')).toBe(true)
  })

  it('opens the test workspace after account setup without starting a tunnel or sending', async () => {
    const view = await render({ ...status, configured: false, app_secret_set: false, webhook_ready: false, missing_fields: ['cloud.app_secret'] })
    expect(view.container.textContent).toContain('Still needed: app secret.')
    view.setStatus(status)
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.container.textContent).not.toContain('Set up Meta Cloud API')
    expect(view.container.querySelector('[aria-label="Cloud API integration test"]')).not.toBeNull()
    expect(view.container.querySelector('aside')).not.toBeNull()
    expect(view.trigger.mock.calls.every(([id]) => !id.includes('send_') && !id.endsWith('::start') && !id.endsWith('::stop'))).toBe(true)
  })

  it.each([false, true])('keeps stored conversations accessible during incomplete setup (narrow: %s)', async (narrow) => {
    const view = await render({ ...status, configured: false, verify_token_set: false, webhook_ready: false, missing_fields: ['cloud.verify_token'] }, narrow)
    await view.click('Browse stored conversations')
    expect(view.container.querySelector('aside')).not.toBeNull()
    expect(view.container.querySelector('[aria-label="Cloud API integration test"]')).toBeNull()
    await view.click('Customer A')
    expect(view.texts()).toEqual([`${chats[0].chat_id}: middle`, `${chats[0].chat_id}: new`])
    expect(view.container.querySelector('[aria-label="Send a Cloud message"]')).toBeNull()
    expect(view.trigger.mock.calls.every(([id]) => !id.includes('send_') && !id.endsWith('::start') && !id.endsWith('::stop'))).toBe(true)
    await view.click('Back to setup')
    expect(view.container.textContent).toContain('Set up Meta Cloud API')
  })

  it('lets the user stop an active tunnel from incomplete setup without starting or sending', async () => {
    const view = await render({ ...status, configured: false, app_secret_set: false, webhook_ready: false, missing_fields: ['cloud.app_secret'] }, false, true)
    expect(view.container.querySelector('[aria-label="Development webhook"]')).toBeNull()
    await view.click('Webhook tools')
    const stop = Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Stop tunnel')!
    expect(stop?.disabled).toBe(false)
    await view.click('Stop tunnel')
    expect(view.trigger.mock.calls.filter(([id]) => id === 'whatsapp::cloud::tunnel::stop')).toHaveLength(1)
    expect(view.trigger.mock.calls.every(([id]) => !id.includes('send_') && !id.endsWith('::start'))).toBe(true)
  })

  it('keeps an active tunnel mounted and stoppable during a transient status failure', async () => {
    const view = await render(status, false, true)
    const tunnel = view.container.querySelector('[aria-label="Development webhook"]')
    view.failStatus(new Error('Worker unavailable'))
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.container.querySelector('[aria-label="Development webhook"]')).toBe(tunnel)
    expect(view.container.textContent).toContain('Could not refresh Cloud API status')
    await view.click('Stop tunnel')
    expect(view.trigger.mock.calls.filter(([id]) => id === 'whatsapp::cloud::tunnel::stop')).toHaveLength(1)
    expect(view.trigger.mock.calls.every(([id]) => !id.includes('send_') && !id.endsWith('::start'))).toBe(true)
  })

  it.each(['test', 'conversation'] as const)('preserves the %s draft through a transient status failure', async (form) => {
    const view = await render()
    if (form === 'conversation') await view.click('Customer A')
    const selector = form === 'test' ? '.wa-ui-test-form textarea' : '[aria-label="Cloud message text"]'
    const textarea = view.container.querySelector<HTMLTextAreaElement>(selector)!
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(textarea, 'Keep this unsent draft')
      textarea.dispatchEvent(new Event('input', { bubbles: true }))
    })
    view.failStatus(new Error('Worker unavailable'))
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.container.querySelector(selector)).toBe(textarea)
    expect(textarea.value).toBe('Keep this unsent draft')
    const send = Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === (form === 'test' ? 'Send test message' : 'Send'))!
    expect(send.disabled).toBe(true)
    view.restoreStatus()
    await view.click('Try again')
    expect(view.container.querySelector<HTMLTextAreaElement>(selector)?.value).toBe('Keep this unsent draft')
    expect(view.trigger.mock.calls.every(([id]) => !id.includes('send_') && !id.endsWith('::start'))).toBe(true)
  })

  it.each([false, true])('makes both first-test routes discoverable before the form (narrow: %s)', async (narrow) => {
    const view = await render(status, narrow)
    const heading = Array.from(view.container.querySelectorAll('h2, h3')).find((element) => element.textContent === 'Start a Cloud test')!
    const starter = heading?.closest('section')
    const form = view.container.querySelector('[aria-label="Cloud API integration test"]')!
    expect(starter).not.toBeNull()
    expect(starter?.textContent).toContain('phone you control')
    expect(starter?.textContent).toMatch(/reply/i)
    const template = Array.from(starter!.querySelectorAll('button')).find((button) => button.textContent === 'Use an approved template')!
    expect(template.closest('[hidden]')).toBeNull()
    expect(heading.compareDocumentPosition(form) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    const check = Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Check connection')!
    expect(check.closest('[hidden]')).toBeNull()
    expect(view.trigger.mock.calls.every(([id]) => !id.includes('send_') && !id.endsWith('templates::list'))).toBe(true)
    await view.click('Use an approved template')
    expect(view.container.querySelector('[aria-label="Cloud API integration test"]')).toBeNull()
    expect(view.container.querySelector<HTMLSelectElement>('[aria-label="Cloud message type"]')?.value).toBe('template')
    expect(view.container.querySelector('[aria-label="Cloud message template"]')?.textContent).toContain('request_update')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    expect(view.trigger.mock.calls.every(([id]) => !id.includes('send_') && !id.endsWith('::start') && !id.endsWith('::stop'))).toBe(true)
  })

  it('respects draft navigation before opening the template starter', async () => {
    const view = await render()
    const textarea = view.container.querySelector<HTMLTextAreaElement>('.wa-ui-test-form textarea')!
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(textarea, 'Keep this unsent test')
      textarea.dispatchEvent(new Event('input', { bubbles: true }))
    })
    view.blockNavigation()
    await view.click('Use an approved template')
    expect(view.navigate).toHaveBeenCalledTimes(1)
    expect(view.container.querySelector('.wa-ui-test-form textarea')).toBe(textarea)
    expect(textarea.value).toBe('Keep this unsent test')
    expect(view.trigger.mock.calls.some(([id]) => id.endsWith('templates::list'))).toBe(false)
    view.allowNavigation()
    await view.click('Use an approved template')
    expect(view.container.querySelector<HTMLSelectElement>('[aria-label="Cloud message type"]')?.value).toBe('template')
    expect(view.trigger.mock.calls.some(([id]) => id.includes('send_'))).toBe(false)
  })

  it('keeps the template draft and selected mode during a failed status refresh and recovery', async () => {
    const view = await render()
    await view.click('Use an approved template')
    await view.fill('Recipient', 'US.ENT.Customer123')
    await view.fill('Cloud message template', 'approved')
    await view.fill('Body parameter customer — {{customer}}', 'Ada')
    const field = view.container.querySelector<HTMLInputElement>('[id$="-body:customer"]')!
    const recipient = view.container.querySelector<HTMLInputElement>('[id$="-recipient"]')!
    view.failStatus(new Error('Worker unavailable'))
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.container.querySelector<HTMLSelectElement>('[aria-label="Cloud message type"]')?.value).toBe('template')
    expect(view.container.querySelector('[id$="-body:customer"]')).toBe(field)
    expect(field.value).toBe('Ada')
    expect(recipient.value).toBe('US.ENT.Customer123')
    const send = Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Send template')!
    expect(send.disabled).toBe(true)
    view.restoreStatus()
    await view.click('Try again')
    expect(field.value).toBe('Ada')
    expect(recipient.value).toBe('US.ENT.Customer123')
    expect(send.disabled).toBe(false)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    expect(view.trigger.mock.calls.some(([id]) => id.includes('send_'))).toBe(false)
  })

  it('opens regular conversations and new messages in text mode after using the template starter', async () => {
    const view = await render()
    await view.click('Use an approved template')
    await view.click('Customer A')
    expect(view.container.querySelector<HTMLSelectElement>('[aria-label="Cloud message type"]')?.value).toBe('text')
    await view.click('New Cloud message')
    expect(view.container.querySelector<HTMLSelectElement>('[aria-label="Cloud message type"]')?.value).toBe('text')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    expect(view.trigger.mock.calls.some(([id]) => id.includes('send_'))).toBe(false)
  })

  it.each(['text', 'template'])('keeps a new %s message receipt when it becomes a conversation', async (mode) => {
    const view = await render()
    const original = view.trigger.getMockImplementation()!
    const messageId = `wamid.accepted-${mode}`
    view.trigger.mockImplementation(async (id, input) => id === `whatsapp::cloud::messages::send_${mode}`
      ? { chat_id: chats[0].chat_id, message_id: messageId, status: 'accepted' }
      : original(id, input))
    await view.click(mode === 'template' ? 'Use an approved template' : 'New Cloud message')
    await view.fill('Recipient', chats[0].chat_id)
    if (mode === 'template') {
      await view.fill('Cloud message template', 'approved')
      await view.fill('Body parameter customer — {{customer}}', 'Ada')
    } else await view.fill('Cloud message text', 'A deliberate test')
    await view.click(mode === 'template' ? 'Send template' : 'Send')
    expect(view.container.textContent).toContain('Accepted by Meta')
    expect(view.container.textContent).toContain('Acceptance does not confirm delivery')
    expect(view.container.querySelector<HTMLInputElement>('[id$="-message-id"]')?.value).toBe(messageId)
    expect(view.container.querySelector('[id$="-recipient"]')).toBeNull()
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(view.container.querySelector<HTMLInputElement>('[id$="-message-id"]')?.value).toBe(messageId)
    await view.click('Customer B')
    expect(view.container.textContent).not.toContain('Accepted by Meta')
    await view.click('Customer A')
    expect(view.container.querySelector<HTMLInputElement>('[id$="-message-id"]')?.value).toBe(messageId)
    expect(view.trigger.mock.calls.filter(([id]) => id.includes('::send_'))).toHaveLength(1)
  })

  it('keeps the chosen template mode after a successful send from the general new-message entry', async () => {
    const view = await render()
    const original = view.trigger.getMockImplementation()!
    view.trigger.mockImplementation(async (id, input) => id === 'whatsapp::cloud::messages::send_template'
      ? { chat_id: chats[0].chat_id, message_id: 'wamid.template-general', status: 'accepted' }
      : original(id, input))
    await view.click('Test templates or attachments')
    await view.fill('Cloud message type', 'template')
    await view.fill('Recipient', chats[0].chat_id)
    await view.fill('Cloud message template', 'approved')
    await view.fill('Body parameter customer — {{customer}}', 'Ada')
    await view.click('Send template')
    expect(view.container.querySelector<HTMLSelectElement>('[aria-label="Cloud message type"]')?.value).toBe('template')
    expect(view.container.querySelector<HTMLInputElement>('[id$="-message-id"]')?.value).toBe('wamid.template-general')
    expect(view.container.querySelector<HTMLSelectElement>('[aria-label="Cloud message template"]')?.value).toBe('approved')
    expect(view.container.querySelector<HTMLInputElement>('[id$="-body:customer"]')?.value).toBe('')
    expect(view.container.querySelector<HTMLTextAreaElement>('[aria-label="Cloud template parameters"]')?.value).toBe('')
    expect(Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Send template')?.disabled).toBe(true)
    expect(view.trigger.mock.calls.filter(([id]) => id.includes('::send_'))).toHaveLength(1)
  })

  it('restores composer recipient, selected type and receipt after the Cloud transport remounts, scoped to its account', async () => {
    const view = await render()
    const original = view.trigger.getMockImplementation()!
    view.trigger.mockImplementation(async (id, input) => id === 'whatsapp::cloud::messages::send_template'
      ? { chat_id: chats[0].chat_id, message_id: 'wamid.retained-template', status: 'accepted' }
      : original(id, input))
    await view.click('Use an approved template')
    await view.fill('Recipient', chats[0].chat_id)
    await view.fill('Cloud message template', 'approved')
    await view.fill('Body parameter customer — {{customer}}', 'Ada')
    await view.click('Send template')
    await view.visible(false)
    await view.visible(true)
    expect(view.container.querySelector<HTMLSelectElement>('[aria-label="Cloud message type"]')?.value).toBe('template')
    expect(view.container.querySelector<HTMLInputElement>('[id$="-message-id"]')?.value).toBe('wamid.retained-template')
    expect(view.container.textContent).toContain(chats[0].chat_id)
    view.setStatus({ ...status, business_account_id: 'other-account', phone_number_id: 'other-phone' })
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.container.textContent).not.toContain('Accepted by Meta')
    expect(view.container.querySelector('[id$="-message-id"]')).toBeNull()
    view.setStatus(status)
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.container.querySelector<HTMLInputElement>('[id$="-message-id"]')?.value).toBe('wamid.retained-template')
    expect(view.trigger.mock.calls.filter(([id]) => id.includes('::send_'))).toHaveLength(1)
  })

  it.each(['draft', 'unknown'])('does not resurrect a prior accepted receipt beside a later %s attempt after remount', async (nextAttempt) => {
    const view = await render()
    const original = view.trigger.getMockImplementation()!
    let sendCount = 0
    view.trigger.mockImplementation(async (id, input) => {
      if (id !== 'whatsapp::cloud::messages::send_text') return original(id, input)
      if (++sendCount === 1) return { chat_id: chats[0].chat_id, message_id: 'wamid.previous-accepted', status: 'accepted' }
      throw new Error('Request timed out; outcome unknown')
    })
    await view.click('New Cloud message')
    await view.fill('Recipient', chats[0].chat_id)
    await view.fill('Cloud message text', 'First confirmed attempt')
    await view.click('Send')
    expect(view.container.querySelector<HTMLInputElement>('[id$="-message-id"]')?.value).toBe('wamid.previous-accepted')
    await view.fill('Cloud message text', 'Later attempt')
    if (nextAttempt === 'unknown') await view.click('Send')
    expect(view.container.textContent).not.toContain('Accepted by Meta')
    await view.visible(false)
    await view.visible(true)
    expect(view.container.querySelector<HTMLTextAreaElement>('[aria-label="Cloud message text"]')?.value).toBe('Later attempt')
    expect(view.container.textContent).not.toContain('Accepted by Meta')
    expect(view.container.querySelector('[id$="-message-id"]')).toBeNull()
    if (nextAttempt === 'unknown') {
      expect(view.container.textContent).toContain('Send not confirmed')
      expect(view.container.textContent).toContain('Send again')
    }
    expect(view.trigger.mock.calls.filter(([id]) => id.includes('::send_'))).toHaveLength(nextAttempt === 'unknown' ? 2 : 1)
  })

  it('checks the existing uncertain-send conversation by refreshing its archive without navigating or sending', async () => {
    const view = await render()
    const original = view.trigger.getMockImplementation()!
    view.trigger.mockImplementation(async (id, input) => {
      if (id === 'whatsapp::cloud::messages::send_text') throw new Error('Request timed out')
      return original(id, input)
    })
    await view.click('Customer A')
    await view.fill('Cloud message text', 'Uncertain attempt')
    await view.click('Send')
    const navigationCount = view.navigate.mock.calls.length
    const reads = view.trigger.mock.calls.filter(([id]) => id === 'whatsapp::cloud::messages::list').length
    await view.click('Check this conversation')
    expect(view.navigate).toHaveBeenCalledTimes(navigationCount)
    expect(view.trigger.mock.calls.filter(([id]) => id === 'whatsapp::cloud::messages::list')).toHaveLength(reads + 1)
    expect(view.trigger.mock.calls.filter(([id]) => id.includes('::send_'))).toHaveLength(1)
    expect(view.container.querySelector<HTMLTextAreaElement>('[aria-label="Cloud message text"]')?.value).toBe('Uncertain attempt')
    expect(view.container.textContent).toContain('Send not confirmed')
  })

  it('announces the connection check failure and its recovery result', async () => {
    const view = await render()
    const original = view.trigger.getMockImplementation()!
    let failed = true
    view.trigger.mockImplementation(async (id, input) => {
      if (id !== 'whatsapp::cloud::check_connection') return original(id, input)
      if (failed) throw new Error('Check the account credentials and try again.')
      return { phone: { id: '456', display_phone_number: '+1 555 123 4567' }, phone_in_business_account: true, subscribed_apps: [] }
    })
    await view.click('Check connection')
    expect(view.container.querySelector('[role="alert"]')?.textContent).toContain('Connection check failed')
    failed = false
    await view.click('Check connection')
    expect(view.container.querySelector('[role="status"]')?.textContent).toContain('Business number verified')
    expect(view.container.querySelector('.wa-ui-test-preparation')?.textContent).toContain('+1 555 123 4567')
    expect(view.container.querySelector('.wa-ui-test-preparation')?.textContent).not.toContain('Check connection above')
    expect(view.trigger.mock.calls.some(([id]) => id.includes('::send_'))).toBe(false)
  })

  it('keeps the test visible on narrow panes and provides a route to stored history', async () => {
    const view = await render(status, true)
    expect(view.container.querySelector('[aria-label="Cloud API integration test"]')).not.toBeNull()
    expect(view.container.querySelector('aside')).toBeNull()
    await view.click('Browse stored conversations')
    expect(view.container.querySelector('aside')).not.toBeNull()
    await view.click('Customer A')
    expect(view.container.querySelector('[role="log"]')).not.toBeNull()
    await view.click('Back to Cloud tests')
    expect(view.container.querySelector('[aria-label="Cloud API integration test"]')).not.toBeNull()
  })
  it('collapses completed setup and reveals details without changing the tunnel', async () => {
    const view = await render()
    const toggle = view.container.querySelector('[aria-label="Webhook setup"]')!
    expect(toggle.textContent?.trim()).toBe('Webhook setup')
    expect(toggle.getAttribute('title')).toBe('Webhook setup')
    const panel = view.container.querySelector<HTMLElement>(`[id="${toggle.getAttribute('aria-controls')}"]`)!
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    expect(panel.hidden).toBe(true)
    await view.click('Webhook setup')
    expect(panel.hidden).toBe(false)
    expect(panel.textContent).toContain('Development webhook')
    await view.click('Webhook setup')
    expect(panel.hidden).toBe(true)
    expect(view.trigger.mock.calls.every(([id]) => !id.endsWith('::start') && !id.endsWith('::stop'))).toBe(true)
  })

  it('opens connection details when webhook configuration is incomplete', async () => {
    const view = await render({ ...status, webhook_ready: false })
    expect(view.container.querySelector('[aria-label="Webhook setup"]')?.getAttribute('aria-expanded')).toBe('true')
    expect(view.container.textContent).toContain('Webhook setup needed')
  })

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
