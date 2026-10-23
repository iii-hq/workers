// @vitest-environment jsdom
import type { Host } from '@iii-dev/console-ui'
import { act, forwardRef, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TestMessage, testEvidence } from '../src/test-message'
import { DraftProvider, DraftScope, useDraftMemory } from '../src/drafts'
import type { MessagePage, MessageView } from '../src/inbox-model'
import type { SentMessage } from '../src/model'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children, variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props} data-variant={variant ?? 'default'}>{children}</button>,
  Input: forwardRef<HTMLInputElement, Omit<ComponentProps<'input'>, 'onChange'> & { onChange(value: string): void }>(({ onChange, ...props }, ref) => <input ref={ref} {...props} onChange={(event) => onChange(event.target.value)} />),
  StatusPanel: ({ headline, detail, action, className }: { headline: ReactNode; detail: ReactNode; action?: ReactNode; className?: string }) => <div role="alert" className={className}>{headline}{detail}{action}</div>,
}))
;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
const cleanups: Array<() => void> = []
const coverage = { capture_started_at_ms: 1, message_limit: 10000, retention_truncated: false }
const message = (id: string, overrides: Partial<MessageView> = {}): MessageView => ({ message_id: id, chat_id: '+15551234567', sender_id: '15551234567', timestamp_ms: Date.now(), from_me: false, kind: 'text', text: 'Reply from the test phone', source: 'live', ...overrides })

async function render(transport: 'cloud' | 'native' = 'cloud', ready = true) {
  let page: MessagePage = { messages: [], next_cursor: null, coverage }
  let listFailure: Error | undefined, sendFailure: Error | undefined
  let baseline: Promise<MessagePage> | undefined
  let sendResponse: Promise<SentMessage> | undefined
  const trigger = vi.fn(async (id: string, input: { chat_id?: string }) => {
    if (id.endsWith('::list')) {
      if (listFailure) throw listFailure
      return baseline ?? page
    }
    if (sendFailure) throw sendFailure
    return sendResponse ?? { message_id: 'test-send-id', chat_id: input.chat_id }
  })
  const host = { iii: { trigger } } as unknown as Host
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  const onOpen = vi.fn()
  const onDirty = vi.fn()
  let visible = true
  let currentTransport = transport
  let scope = `${transport}:test-session:1`
  let memory: ReturnType<typeof useDraftMemory> = null
  const MemoryProbe = () => { memory = useDraftMemory(); return null }
  const renderTest = () => root.render(<DraftProvider><MemoryProbe /><DraftScope id={scope}>{visible && <TestMessage key={scope} host={host} transport={currentTransport} sessionId="test-session" ready={ready} onDirty={onDirty} onOpenConversation={onOpen} />}</DraftScope></DraftProvider>)
  await act(async () => renderTest())
  let mounted = true
  const unmount = () => { if (mounted) { act(() => root.unmount()); mounted = false }; container.remove() }
  cleanups.push(unmount)
  return {
    container, trigger, onOpen, onDirty, unmount,
    draftCount() { return memory?.values.size ?? 0 },
    async scope(next: string) { scope = next; await act(async () => renderTest()) },
    async visible(next: boolean) { visible = next; await act(async () => renderTest()) },
    async transport(next: 'cloud' | 'native') { currentTransport = next; scope = `${next}:test-session:1`; await act(async () => renderTest()) },
    setPage(messages: MessageView[]) { page = { messages, next_cursor: null, coverage } },
    failList(error: Error) { listFailure = error },
    failSend(error: Error) { sendFailure = error },
    restoreSend() { sendFailure = undefined },
    restoreList() { listFailure = undefined },
    deferBaseline(promise: Promise<MessagePage>) { baseline = promise },
    deferSend(promise: Promise<SentMessage>) { sendResponse = promise },
    async recipient(value = '+15551234567') {
      const input = container.querySelector<HTMLInputElement>('input')!
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value)
        input.dispatchEvent(new Event('input', { bubbles: true }))
      })
    },
    async text(value: string) {
      const input = container.querySelector<HTMLTextAreaElement>('textarea')!
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, value)
        input.dispatchEvent(new Event('input', { bubbles: true }))
      })
    },
    async send() { await act(async () => container.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))) },
    async key(options: KeyboardEventInit) { await act(async () => container.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true, ...options }))) },
    async refresh() { await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Refresh test results"]')!.click()) },
    sendCalls() { return trigger.mock.calls.filter(([id]) => id.endsWith('::send_text')) },
  }
}
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); vi.restoreAllMocks(); vi.useRealTimers() })

describe('Explicit integration tests', () => {
  it.each(['cloud', 'native'] as const)('uses only the %s transport and never sends on mount', async (transport) => {
    const view = await render(transport)
    expect(view.trigger).not.toHaveBeenCalled()
    await view.recipient()
    await view.send()
    expect(view.sendCalls()).toEqual([[transport === 'cloud' ? 'whatsapp::cloud::messages::send_text' : 'whatsapp::messages::send_text', {
      ...(transport === 'native' ? { session_id: 'test-session' } : {}), chat_id: transport === 'cloud' ? '+15551234567' : '15551234567@s.whatsapp.net', text: 'Hello from iii. This is an integration test.',
    }, { timeoutMs: transport === 'cloud' ? 120000 : 610000 }]])
    expect(view.container.textContent).toContain('Accepted')
    expect(view.container.textContent).toContain('Awaiting receipt')
    expect(view.container.textContent).not.toContain('Captured')
    expect(view.container.querySelector<HTMLInputElement>('[readonly]')?.value).toBe('test-send-id')
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
  })

  it.each([
    ['cloud', 'US.ENT.Customer123'],
    ['native', '123456789@g.us'],
  ] as const)('accepts a %s recipient ID without restricting it to telephone input', async (transport, recipient) => {
    const view = await render(transport)
    expect(view.container.querySelector<HTMLInputElement>('input')?.type).toBe('text')
    await view.recipient(recipient)
    await view.send()
    expect(view.sendCalls()[0]?.[1]).toMatchObject({ chat_id: recipient })
    expect(view.container.textContent).toContain('Accepted')
  })

  it.each(['cloud', 'native'] as const)('restores the %s test recipient only within its account and keeps a completed default form clean', async (transport) => {
    const view = await render(transport)
    await view.recipient('+15551234567')
    await view.scope(`${transport}:another-account:2`)
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('')
    await view.scope(`${transport}:test-session:1`)
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('+15551234567')
    expect(view.sendCalls()).toHaveLength(0)
    await view.send()
    await view.scope(`${transport}:another-account:2`)
    await view.scope(`${transport}:test-session:1`)
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('+15551234567')
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestAccepted')
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    expect(view.sendCalls()).toHaveLength(1)
    const freshPage = await render(transport)
    expect(freshPage.container.querySelector<HTMLInputElement>('input')?.value).toBe('')
  })

  it.each(['cloud', 'native'] as const)('retains completed %s test evidence and its baseline across tool switches without resending', async (transport) => {
    vi.useFakeTimers()
    const view = await render(transport)
    const chatId = transport === 'cloud' ? '+15551234567' : '15551234567@s.whatsapp.net'
    const earlier = message('earlier', { chat_id: chatId })
    view.setPage([earlier])
    await view.recipient(); await view.send()
    await view.visible(false)
    await act(async () => { await vi.advanceTimersByTimeAsync(7500) })
    await view.visible(true)
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestAccepted')
    expect(view.container.querySelector('dl')?.textContent).not.toContain('Captured')
    view.setPage([earlier, message('test-send-id', { chat_id: chatId, from_me: true, status: 'delivered' }), message('fresh-reply', { chat_id: chatId })])
    await view.refresh()
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryDelivered')
    expect(view.container.querySelector('dl')?.textContent).toContain('Incoming messageCaptured')
    await view.visible(false)
    view.failList(new Error('Archive unavailable'))
    await view.visible(true)
    expect(view.container.textContent).toContain('Last known test results')
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryDelivered')
    expect(view.container.querySelector('dl')?.textContent).toContain('Incoming messageCaptured')
    expect(view.container.querySelector<HTMLInputElement>('[readonly]')?.value).toBe('test-send-id')
    view.restoreList()
    await view.refresh()
    expect(view.container.textContent).not.toContain('Could not verify results')
    expect(view.sendCalls()).toHaveLength(1)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('::list')).every(([id]) => id === (transport === 'cloud' ? 'whatsapp::cloud::messages::list' : 'whatsapp::messages::list'))).toBe(true)
  })

  it('keeps native and Cloud test results separate and resets them for a new session incarnation', async () => {
    const view = await render('native')
    await view.recipient(); await view.send()
    await view.transport('cloud')
    expect(view.container.querySelector('dl')?.textContent).toContain('Not tested')
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('')
    await view.recipient('US.ENT.Customer123'); await view.send()
    await view.transport('native')
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestAccepted')
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('+15551234567')
    expect(view.container.textContent).toContain('Recipient: +15551234567')
    expect(view.container.querySelector<HTMLInputElement>('[aria-label="Raw recipient ID"]')?.value).toBe('15551234567@s.whatsapp.net')
    await view.scope('native:test-session:2')
    expect(view.container.querySelector('dl')?.textContent).toContain('Not tested')
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('')
    await view.transport('cloud')
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('US.ENT.Customer123')
    expect(view.sendCalls()).toHaveLength(2)
  })

  it('allows clearing a remembered recipient without restoring it on the next tool visit', async () => {
    const view = await render()
    await view.recipient(); await view.send()
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('+15551234567')
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    await view.recipient('')
    await view.visible(false); await view.visible(true)
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('')
    expect(view.container.querySelector<HTMLButtonElement>('[type="submit"]')?.disabled).toBe(true)
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    expect(view.container.querySelector('dl')?.textContent).toContain('Accepted')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it.each(['cloud', 'native'] as const)('does not retain phantom %s drafts after clearing or restoring the remembered recipient', async (transport) => {
    const view = await render(transport)
    await view.recipient()
    expect(view.draftCount()).toBe(1)
    await view.recipient('  ')
    expect(view.draftCount()).toBe(0)
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    await view.visible(false); await view.visible(true)
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('')
    expect(view.draftCount()).toBe(0)
    await view.recipient(); await view.send()
    await view.recipient('+15557654321')
    expect(view.draftCount()).toBe(1)
    await view.recipient('+15551234567')
    expect(view.draftCount()).toBe(0)
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    await view.visible(false); await view.visible(true)
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('+15551234567')
    await view.text('Keep this draft')
    await view.recipient('+15557654321')
    await view.recipient('+15551234567')
    expect(view.draftCount()).toBe(1)
    expect(view.onDirty).toHaveBeenLastCalledWith(true)
    await view.visible(false); await view.visible(true)
    expect(view.container.querySelector('textarea')?.value).toBe('Keep this draft')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it.each(['cloud', 'native'] as const)('keeps the normalized %s unknown attempt target while editing and offers a read-only conversation check', async (transport) => {
    const view = await render(transport)
    view.failSend(new Error('Request timed out; outcome unknown'))
    await view.recipient(transport === 'cloud' ? '+1 (555) 123-4567' : '+15551234567'); await view.send()
    await view.recipient('+15557654321')
    expect(view.container.textContent).toContain('Sending again may duplicate the previous message')
    expect(view.container.querySelector<HTMLButtonElement>('[type="submit"]')?.textContent).toContain('Send again')
    const check = Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Check this conversation')!
    expect(check).toBeDefined()
    await act(async () => check.click())
    expect(view.onOpen).toHaveBeenCalledWith({ chat_id: transport === 'cloud' ? '+15551234567' : '15551234567@s.whatsapp.net', title: '+15551234567', is_group: false })
    expect(view.container.querySelector<HTMLInputElement>('input')?.value).toBe('+15557654321')
    await view.visible(false); await view.visible(true)
    expect(view.container.textContent).toContain('Sending again may duplicate the previous message')
    expect(view.container.querySelector<HTMLButtonElement>('[type="submit"]')?.textContent).toContain('Send again')
    expect(view.sendCalls()).toHaveLength(1)
    view.restoreSend()
    await view.send()
    expect(view.sendCalls()).toHaveLength(2)
    expect(view.sendCalls()[1]?.[1]).toMatchObject({ chat_id: transport === 'cloud' ? '+15557654321' : '15557654321@s.whatsapp.net' })
    expect(view.container.textContent).not.toContain('Sending again may duplicate the previous message')
  })

  it.each(['accepted', 'unknown', 'not_sent'] as const)('reveals an offscreen result summary only after an explicit %s test finishes', async (outcome) => {
    const view = await render()
    const result = view.container.querySelector<HTMLElement>('[role="region"][aria-label="Test results"]')!
    expect(result).not.toBeNull()
    view.container.style.overflowY = 'auto'
    vi.spyOn(view.container, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 390, 600))
    vi.spyOn(result, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 850, 390, 220))
    const scroll = vi.fn()
    Object.defineProperty(result, 'scrollIntoView', { configurable: true, value: scroll })
    const focus = vi.spyOn(result, 'focus')
    if (outcome === 'unknown') view.failSend(new Error('Request timed out; outcome unknown'))
    if (outcome === 'not_sent') view.failList(new Error('Archive unavailable'))
    await view.recipient('+15551234567')
    expect(scroll).not.toHaveBeenCalled()
    await view.send()
    expect(focus).toHaveBeenCalledWith({ preventScroll: true })
    expect(scroll).toHaveBeenCalledWith({ block: 'nearest', inline: 'nearest' })
    expect(document.activeElement).toBe(result)
    if (outcome === 'accepted') { view.setPage([message('test-send-id', { from_me: true, status: 'read' })]); await view.refresh() }
    await view.recipient('+15557654321')
    expect(scroll).toHaveBeenCalledTimes(1)
    expect(focus).toHaveBeenCalledTimes(1)
  })

  it.each(['cloud', 'native'] as const)('keeps the uncertain %s outcome, warning and read-only recovery together in the revealed results', async (transport) => {
    const view = await render(transport)
    const result = view.container.querySelector<HTMLElement>('[role="region"][aria-label="Test results"]')!
    view.container.style.overflowY = 'auto'
    vi.spyOn(view.container, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 390, 600))
    vi.spyOn(result, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 850, 390, 460))
    Object.defineProperty(result, 'scrollIntoView', { configurable: true, value: vi.fn() })
    view.failSend(new Error('Request timed out; outcome unknown'))
    await view.recipient(); await view.send()
    expect(document.activeElement).toBe(result)
    expect(result.textContent).toContain('Send requestOutcome unknown')
    const warning = result.querySelector('[role="alert"]')!
    expect(warning).not.toBeNull()
    expect(warning.textContent).toContain('Sending again may duplicate the previous message')
    expect(warning.textContent).toContain('Attempted recipient: +15551234567')
    expect(view.container.querySelector('form [role="alert"]')).toBeNull()
    const check = warning.querySelector<HTMLButtonElement>('button')!
    expect(check.textContent).toBe('Check this conversation')
    expect(check.type).toBe('button')
    await act(async () => check.click())
    expect(view.onOpen).toHaveBeenCalledWith({ chat_id: transport === 'cloud' ? '+15551234567' : '15551234567@s.whatsapp.net', title: '+15551234567', is_group: false })
    expect(view.sendCalls()).toHaveLength(1)
  })

  it.each([
    ['cloud', 'Request timed out; outcome unknown', 'Outcome unknown'],
    ['native', 'Request timed out; outcome unknown', 'Outcome unknown'],
    ['cloud', 'The send request failed before acceptance was confirmed', 'Not confirmed'],
    ['native', 'The send request failed before acceptance was confirmed', 'Not confirmed'],
  ] as const)('prioritizes a read-only check for an uncertain %s send (%s) and requires an explicit repeat action', async (transport, failure, outcome) => {
    const view = await render(transport)
    await view.recipient('+15551234567')
    expect(view.container.querySelector<HTMLButtonElement>('[type="submit"]')?.dataset.variant).toBe('default')
    view.failSend(new Error(failure))
    await view.send()
    expect(view.container.querySelector('dl')?.textContent).toContain(`Send request${outcome}`)
    const submit = view.container.querySelector<HTMLButtonElement>('[type="submit"]')!
    expect(submit.textContent).toContain('Send again')
    expect(submit.dataset.variant).toBe('ghost')
    const check = view.container.querySelector<HTMLButtonElement>('[role="alert"] button')!
    expect(check.textContent).toBe('Check this conversation')
    expect(check.type).toBe('button')
    expect(check.dataset.variant).toBe('default')
    expect(view.container.textContent).not.toContain('Ctrl/Cmd+Enter to send')
    await view.key({ ctrlKey: true })
    await view.key({ metaKey: true })
    await view.recipient('+15557654321')
    await view.text('A new message after the uncertain attempt')
    await view.key({ ctrlKey: true })
    await view.visible(false); await view.visible(true)
    await view.key({ metaKey: true })
    expect(view.sendCalls()).toHaveLength(1)
    const callsBeforeCheck = view.trigger.mock.calls.length
    const restoredCheck = view.container.querySelector<HTMLButtonElement>('[role="alert"] button')!
    await act(async () => restoredCheck.click())
    expect(view.onOpen).toHaveBeenCalledWith({ chat_id: transport === 'cloud' ? '+15551234567' : '15551234567@s.whatsapp.net', title: '+15551234567', is_group: false })
    expect(view.trigger.mock.calls).toHaveLength(callsBeforeCheck)
    view.restoreSend()
    await act(async () => view.container.querySelector<HTMLButtonElement>('[type="submit"]')!.click())
    expect(view.sendCalls()).toHaveLength(2)
    expect(view.sendCalls()[1]?.[1]).toMatchObject({ chat_id: transport === 'cloud' ? '+15557654321' : '15557654321@s.whatsapp.net', text: 'A new message after the uncertain attempt' })
    expect(view.container.querySelector<HTMLButtonElement>('[type="submit"]')?.dataset.variant).toBe('default')
    expect(view.container.textContent).toContain('Ctrl/Cmd+Enter to send')
  })

  it('keeps focus and scroll stable when completed results already fit the scrollport', async () => {
    const view = await render()
    const result = view.container.querySelector<HTMLElement>('[role="region"][aria-label="Test results"]')!
    expect(result).not.toBeNull()
    view.container.style.overflowY = 'auto'
    vi.spyOn(view.container, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 1000, 600))
    vi.spyOn(result, 'getBoundingClientRect').mockReturnValue(new DOMRect(650, 200, 300, 220))
    const scroll = vi.fn()
    Object.defineProperty(result, 'scrollIntoView', { configurable: true, value: scroll })
    const focus = vi.spyOn(result, 'focus')
    await view.recipient(); await view.send()
    await view.refresh()
    expect(scroll).not.toHaveBeenCalled()
    expect(focus).not.toHaveBeenCalled()
  })

  it('retests the remembered phone only on an explicit action and replaces prior evidence', async () => {
    const view = await render()
    await view.recipient(); await view.send()
    view.setPage([message('test-send-id', { from_me: true, status: 'read' }), message('fresh-reply')])
    await view.refresh()
    expect(view.container.querySelector('dl')?.textContent).toContain('Captured')
    view.failSend(new Error('Request timed out; outcome unknown'))
    await view.send()
    expect(view.container.querySelector('dl')?.textContent).toContain('Outcome unknown')
    expect(view.container.querySelector('dl')?.textContent).not.toContain('Accepted')
    expect(view.container.querySelector('dl')?.textContent).not.toContain('Captured')
    await view.visible(false); await view.visible(true)
    expect(view.container.querySelector('dl')?.textContent).toContain('Outcome unknown')
    expect(view.container.textContent).toContain('never retried automatically')
    expect(view.sendCalls()).toHaveLength(2)
  })

  it('keeps a pending send disabled and reports acceptance, delivery and a reply only after their evidence arrives', async () => {
    vi.useFakeTimers()
    const view = await render()
    let resolve!: (sent: SentMessage) => void
    view.deferSend(new Promise((done) => { resolve = done }))
    await view.recipient()
    await view.send()
    expect(view.container.querySelector<HTMLButtonElement>('[type="submit"]')?.disabled).toBe(true)
    expect(view.container.querySelector<HTMLInputElement>('input')?.disabled).toBe(true)
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestSending…')
    expect(view.container.textContent).not.toContain('Accepted')
    await view.send()
    await act(async () => { await vi.advanceTimersByTimeAsync(7500) })
    expect(view.sendCalls()).toHaveLength(1)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('::list'))).toHaveLength(1)
    await act(async () => resolve({ message_id: 'test-send-id', chat_id: '+15551234567' }))
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestAccepted')
    expect(view.container.textContent).toContain('Awaiting receipt')
    expect(view.container.textContent).not.toContain('Captured')
    view.setPage([message('test-send-id', { from_me: true, status: 'read' }), message('new-reply')])
    await view.refresh()
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryRead')
    expect(view.container.querySelector('dl')?.textContent).toContain('Incoming messageCaptured')
    expect(view.container.querySelectorAll('[data-confirmed="true"]')).toHaveLength(3)
    expect(view.sendCalls()).toHaveLength(1)
  })

  it('prevents sends while disconnected, even if the form is submitted', async () => {
    const view = await render('native', false)
    await view.recipient()
    expect(view.container.querySelector<HTMLButtonElement>('[type="submit"]')!.disabled).toBe(true)
    await view.send()
    expect(view.trigger).not.toHaveBeenCalled()
    expect(view.container.textContent).toContain('Start and connect the session')
  })

  it('requires a valid recipient before querying or sending', async () => {
    const view = await render()
    await view.recipient('not-a-phone')
    await view.send()
    expect(view.trigger).not.toHaveBeenCalled()
    expect(view.container.querySelector('[role="alert"]')).not.toBeNull()
  })

  it.each(['cloud', 'native'] as const)('focuses and describes invalid %s recipient input without revealing completion results or making an RPC', async (transport) => {
    const view = await render(transport)
    const input = view.container.querySelector<HTMLInputElement>('input')!
    const result = view.container.querySelector<HTMLElement>('[role="region"][aria-label="Test results"]')!
    vi.spyOn(result, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 900, 390, 220))
    const resultFocus = vi.spyOn(result, 'focus')
    Object.defineProperty(result, 'scrollIntoView', { configurable: true, value: vi.fn() })
    await view.recipient('not-a-phone')
    await view.send()
    expect(view.trigger).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(input)
    expect(resultFocus).not.toHaveBeenCalled()
    expect(input.getAttribute('aria-invalid')).toBe('true')
    const errorId = input.getAttribute('aria-describedby')!.split(' ').find((id) => id.endsWith('-recipient-error'))!
    expect(document.getElementById(errorId)?.textContent).toMatch(/phone number|digits/i)
    await view.recipient('+15551234567')
    expect(input.hasAttribute('aria-invalid')).toBe(false)
    expect(document.getElementById(errorId)).toBeNull()
    expect(view.trigger).not.toHaveBeenCalled()
  })

  it.each(['cloud', 'native'] as const)('focuses the invalid %s message, associates its error and clears it on correction', async (transport) => {
    const view = await render(transport)
    await view.recipient()
    await view.text(transport === 'cloud' ? 'x'.repeat(4097) : 'x'.repeat(65537))
    const input = view.container.querySelector<HTMLTextAreaElement>('textarea')!
    await view.send()
    expect(view.trigger).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(input)
    expect(input.getAttribute('aria-invalid')).toBe('true')
    const errorId = input.getAttribute('aria-describedby')!.split(' ').find((id) => id.endsWith('-message-error'))!
    expect(document.getElementById(errorId)?.textContent).toMatch(/4,096|65,536/)
    await view.text('Corrected test message')
    expect(input.hasAttribute('aria-invalid')).toBe(false)
    expect(document.getElementById(errorId)).toBeNull()
    expect(view.trigger).not.toHaveBeenCalled()
  })

  it('validates touched fields without adding empty validation-only drafts or announcing untouched initial fields', async () => {
    const view = await render()
    const input = view.container.querySelector<HTMLInputElement>('input')!
    expect(view.container.querySelector('[aria-invalid="true"]')).toBeNull()
    expect(view.container.querySelector('[role="alert"]')).toBeNull()
    await act(async () => { input.focus(); input.blur() })
    expect(input.getAttribute('aria-invalid')).toBe('true')
    expect(view.container.querySelector('textarea')?.hasAttribute('aria-invalid')).toBe(false)
    expect(view.draftCount()).toBe(0)
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    await view.visible(false); await view.visible(true)
    expect(view.container.querySelector<HTMLInputElement>('input')?.getAttribute('aria-invalid')).toBe('true')
    expect(view.draftCount()).toBe(0)
    expect(view.trigger).not.toHaveBeenCalled()
    await view.recipient('+15551234567')
    expect(view.container.querySelector('[aria-invalid="true"]')).toBeNull()
  })

  it('preserves accepted proof while drafting the next message but clears that proof on an explicitly invalid submission', async () => {
    const view = await render()
    await view.recipient(); await view.send()
    await view.recipient('not-a-phone')
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestAccepted')
    expect(view.container.querySelector<HTMLInputElement>('[readonly]')?.value).toBe('test-send-id')
    await view.send()
    expect(view.sendCalls()).toHaveLength(1)
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestNot sent')
    expect(view.container.querySelector('dl')?.textContent).not.toContain('Accepted')
    expect(view.container.querySelector<HTMLInputElement>('[readonly]')).toBeNull()
    expect(document.activeElement).toBe(view.container.querySelector('input'))
  })

  it.each(['cloud', 'native'] as const)('retains the exact %s sent text and dispatch timestamp beside proof while the next draft changes, remounts and polling fails', async (transport) => {
    vi.useFakeTimers()
    const firstTimestamp = new Date('2026-10-08T01:05:06.000Z')
    vi.setSystemTime(firstTimestamp)
    const view = await render(transport)
    const sentText = '  Exact test payload\nsecond line 🙂  '
    await view.recipient(); await view.text(sentText); await view.send()
    const proof = () => view.container.querySelector<HTMLTextAreaElement>('textarea[readonly]')!
    const timestamp = () => view.container.querySelector<HTMLTimeElement>('time')!
    expect(proof().value).toBe(sentText)
    expect(proof().labels?.[0]?.textContent).toBe('Sent message')
    expect(timestamp().dateTime).toBe(firstTimestamp.toISOString())
    const expectedTime = new Intl.DateTimeFormat('en-US', { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', timeZoneName: 'short' }).format(firstTimestamp)
    expect(timestamp().textContent).toBe(expectedTime)
    expect(view.container.textContent).toContain('Sent at')
    vi.setSystemTime(new Date('2026-10-08T03:10:00.000Z'))
    await view.text('Next unsent draft')
    await view.recipient('+15557654321')
    expect(proof().value).toBe(sentText)
    expect(timestamp().dateTime).toBe(firstTimestamp.toISOString())
    view.failList(new Error('Archive unavailable'))
    await view.refresh()
    await view.visible(false); await view.visible(true)
    expect(view.container.querySelector('textarea:not([readonly])')?.value).toBe('Next unsent draft')
    expect(proof().value).toBe(sentText)
    expect(timestamp().dateTime).toBe(firstTimestamp.toISOString())
    expect(view.container.textContent).toContain('Recipient: +15551234567')
    expect(view.container.textContent).toContain('Last known test results')
    expect(view.sendCalls()).toHaveLength(1)
    expect(view.sendCalls()[0]?.[1]).toMatchObject({ text: sentText })
  })

  it('identifies the actual dispatch time rather than the earlier form submission or later acceptance time', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-10-08T01:00:00.000Z'))
    const view = await render()
    let baselineReady!: (page: MessagePage) => void
    let accepted!: (result: SentMessage) => void
    view.deferBaseline(new Promise((done) => { baselineReady = done }))
    view.deferSend(new Promise((done) => { accepted = done }))
    await view.recipient(); await view.text('Exact dispatched message'); await view.send()
    expect(view.sendCalls()).toHaveLength(0)
    const dispatchedAt = new Date('2026-10-08T01:00:10.000Z')
    vi.setSystemTime(dispatchedAt)
    await act(async () => baselineReady({ messages: [], next_cursor: null, coverage }))
    expect(view.sendCalls()).toHaveLength(1)
    vi.setSystemTime(new Date('2026-10-08T01:00:15.000Z'))
    await act(async () => accepted({ message_id: 'test-send-id', chat_id: '+15551234567' }))
    expect(view.container.querySelector<HTMLTimeElement>('time')?.dateTime).toBe(dispatchedAt.toISOString())
    expect(view.container.querySelector<HTMLTextAreaElement>('textarea[readonly]')?.value).toBe('Exact dispatched message')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it('does not count stored history or earlier incoming messages as receiving', async () => {
    const view = await render()
    const earlier = message('earlier')
    view.setPage([earlier])
    await view.recipient(); await view.send()
    view.setPage([earlier, message('history', { source: 'history' }), message('other-chat', { chat_id: 'different' }), message('old', { timestamp_ms: 1 })])
    await view.refresh()
    expect(view.container.textContent).not.toContain('Captured')
    view.setPage([message('new-reply')])
    await view.refresh()
    expect(view.container.textContent).toContain('Captured')
    expect(view.container.textContent).toContain('Reply from the test phone')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it('matches delivery receipts to the exact test message and recipient', async () => {
    const view = await render()
    await view.recipient(); await view.send()
    view.setPage([message('different-id', { from_me: true, status: 'read' }), message('test-send-id', { chat_id: 'different', from_me: true, status: 'read' })])
    await view.refresh()
    expect(view.container.textContent).toContain('Awaiting receipt')
    view.setPage([message('test-send-id', { from_me: true, status: 'read' })])
    await view.refresh()
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryRead')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it('preserves accepted results through a polling error and recovers without resending', async () => {
    const view = await render()
    await view.recipient(); await view.send()
    view.failList(new Error('Archive unavailable'))
    await view.refresh()
    expect(view.container.textContent).toContain('Could not verify results')
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestAccepted')
    expect(view.container.textContent).not.toContain('Captured')
    view.restoreList()
    view.setPage([message('test-send-id', { from_me: true, status: 'failed' })])
    await view.refresh()
    expect(view.container.textContent).not.toContain('Could not verify results')
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestAccepted')
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryFailed')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it('keeps verified receipts and replies when they leave the most recent archive page', async () => {
    const view = await render()
    await view.recipient(); await view.send()
    view.setPage([message('test-send-id', { from_me: true, status: 'read' }), message('fresh-reply')])
    await view.refresh()
    view.setPage([message('later-history', { source: 'history' })])
    await view.refresh()
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryRead')
    expect(view.container.querySelector('dl')?.textContent).toContain('Incoming messageCaptured')
    await view.visible(false); await view.visible(true)
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryRead')
    expect(view.container.querySelector('dl')?.textContent).toContain('Incoming messageCaptured')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it('stops without sending if the baseline cannot be read', async () => {
    const view = await render()
    view.failList(new Error('Archive unavailable'))
    await view.recipient(); await view.send()
    expect(view.sendCalls()).toHaveLength(0)
    expect(view.container.textContent).toContain('No message was sent.')
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestNot sent')
  })

  it('reports an unknown timeout without retrying through result polling', async () => {
    vi.useFakeTimers()
    const view = await render()
    view.failSend(new Error('Request timed out; outcome unknown'))
    await view.recipient(); await view.send()
    await act(async () => { await vi.advanceTimersByTimeAsync(15000) })
    expect(view.sendCalls()).toHaveLength(1)
    expect(view.container.textContent).toContain('Check the conversation before sending again')
    expect(view.container.textContent).toContain('never retried automatically')
    expect(view.container.textContent).not.toContain('Accepted')
    expect(view.container.querySelector('dl')?.textContent).toContain('Send requestOutcome unknown')
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryUnverified')
    expect(view.container.querySelector('dl')?.textContent).not.toContain('Not tested')
    await view.scope('cloud:another-account')
    await view.scope('cloud:test-session:1')
    expect(view.container.querySelector('dl')?.textContent).toContain('Outcome unknown')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it.each(['cloud', 'native'] as const)('supports guarded Ctrl/Cmd+Enter in the %s test form', async (transport) => {
    const view = await render(transport)
    await view.recipient()
    await view.key({ ctrlKey: true, isComposing: true })
    await view.key({ metaKey: true, repeat: true })
    await view.key({})
    expect(view.sendCalls()).toHaveLength(0)
    await view.key({ ctrlKey: true })
    expect(view.sendCalls()).toHaveLength(1)
    await view.recipient()
    await view.key({ metaKey: true })
    expect(view.sendCalls()).toHaveLength(2)
  })

  it('labels stale evidence and offers a local read-only recovery action', async () => {
    const view = await render()
    await view.recipient(); await view.send()
    view.setPage([message('test-send-id', { from_me: true, status: 'read' }), message('fresh-reply')])
    await view.refresh()
    view.failList(new Error('Archive unavailable'))
    await view.refresh()
    expect(view.container.textContent).toContain('Last known test results')
    expect(view.container.textContent).toContain('last known until the next successful check')
    const summary = view.container.querySelector('.wa-ui-test-summary')!
    const warning = Array.from(summary.querySelectorAll('[role="alert"]')).find((node) => node.textContent?.includes('Could not verify results'))!
    expect(warning).toBeTruthy()
    expect(warning.compareDocumentPosition(summary.querySelector('dl')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(view.container.querySelector('dl')?.textContent).toContain('DeliveryRead')
    expect(view.container.querySelector('dl')?.textContent).toContain('Incoming messageCaptured')
    const retry = Array.from(view.container.querySelectorAll('button')).find((button) => button.textContent === 'Check again')!
    expect(retry).toBeDefined()
    view.restoreList()
    await act(async () => retry.click())
    expect(view.container.textContent).not.toContain('Last known test results')
    expect(view.container.textContent).not.toContain('Could not verify results')
    expect(view.sendCalls()).toHaveLength(1)
  })

  it('does not dispatch after leaving while the baseline is pending', async () => {
    const view = await render()
    let resolve!: (page: MessagePage) => void
    view.deferBaseline(new Promise((done) => { resolve = done }))
    await view.recipient(); await view.send()
    view.unmount()
    await act(async () => resolve({ messages: [], next_cursor: null, coverage }))
    expect(view.sendCalls()).toHaveLength(0)
  })

  it('shows failed delivery independently of acceptance', () => {
    const run = { message_id: 'test-send-id', chat_id: '+15551234567', sentText: 'Test payload', startedAt: Date.now(), previousIncoming: new Set<string>() }
    expect(testEvidence(run, [message('test-send-id', { from_me: true, status: 'failed' })])).toMatchObject({ failed: true, delivered: false, read: false })
  })
})
