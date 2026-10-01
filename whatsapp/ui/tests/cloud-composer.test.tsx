// @vitest-environment jsdom

import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { CloudComposer } from '../src/cloud-composer'
import type { CloudSent, CloudTemplate, TemplatePage } from '../src/cloud-model'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children, variant: _variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props}>{children}</button>,
  Input: ({ onChange, ...props }: Omit<ComponentProps<'input'>, 'onChange'> & { onChange(value: string): void }) => <input {...props} onChange={(event) => onChange(event.target.value)} />,
  Select: ({ options, onChange, value, placeholder, ...props }: { options: Array<{ value: string; label: string; disabled?: boolean }>; onChange(value: string): void; value?: string; placeholder?: string; 'aria-label'?: string; disabled?: boolean }) => <select {...props} value={value ?? ''} onChange={(event) => onChange(event.target.value)}><option value="">{placeholder}</option>{options.map((option) => <option key={option.value} value={option.value} disabled={option.disabled}>{option.label}</option>)}</select>,
  CodeEditor: ({ value, onChange, ...props }: { value: string; onChange(value: string): void; 'aria-label'?: string; disabled?: boolean }) => <textarea aria-label={props['aria-label']} disabled={props.disabled} value={value} onChange={(event) => onChange(event.target.value)} />,
  SettingsList: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  SettingsField: ({ field, label, renderControl }: { field: string; label: string; renderControl(props: { id: string }): ReactNode }) => <div><label htmlFor={field}>{label}</label>{renderControl({ id: field })}</div>,
  EmptyState: ({ title, description }: { title: string; description: string }) => <p>{title} {description}</p>,
  StatusPanel: ({ headline, detail, action }: { headline: string; detail?: string; action?: ReactNode }) => <div role="alert">{headline}: {detail}{action}</div>,
}))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const approved: CloudTemplate = { id: 'approved', name: 'request_update', language: 'en_US', status: 'APPROVED', category: 'UTILITY', parameter_format: 'NAMED', components: [{ type: 'BODY', text: 'Hello {{customer}}.' }] }
const accepted: CloudSent = { message_id: 'outbound', chat_id: '+15551234567', status: 'accepted' }
const cleanups: Array<() => void> = []

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

async function render(options: { newChat?: boolean; latestIncomingAt?: number } = {}) {
  const templateResults: Array<Promise<TemplatePage>> = []
  const sendResults: Array<Promise<CloudSent>> = []
  const trigger = vi.fn(async (id: string, _input: unknown) => {
    if (id.endsWith('templates::list')) return templateResults.shift() ?? { templates: [approved], next_cursor: null }
    if (id.includes('messages::send_')) return sendResults.shift() ?? accepted
    throw new Error(`Unexpected function: ${id}`)
  })
  const host = { iii: { trigger } } as unknown as Host
  const onDirty = vi.fn()
  const onSent = vi.fn()
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  await act(async () => root.render(<CloudComposer host={host} chat={options.newChat ? null : { chat_id: accepted.chat_id, title: 'Customer', is_group: false }} ready onDirty={onDirty} onSent={onSent} latestIncomingAt={options.latestIncomingAt} />))
  let mounted = true
  const unmount = () => { if (mounted) { act(() => root.unmount()); container.remove(); mounted = false } }
  cleanups.push(unmount)
  return {
    container, trigger, onDirty, onSent, templateResults, sendResults, unmount,
    sends() { return trigger.mock.calls.filter(([id]) => id.includes('messages::send_')) },
    field(label: string) {
      const id = Array.from(container.querySelectorAll('label')).find((element) => element.textContent === label)?.htmlFor
      return (id ? container.querySelector(`[id="${id}"]`) : container.querySelector(`[aria-label="${label}"]`)) as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
    },
    async fill(label: string, value: string) {
      const field = this.field(label)
      expect(field, `Missing field: ${label}`).toBeTruthy()
      await act(async () => {
        const prototype = field.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : field.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype
        Object.getOwnPropertyDescriptor(prototype, 'value')!.set!.call(field, value)
        field.dispatchEvent(new Event(field.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }))
      })
    },
    button(label: string) { return Array.from(container.querySelectorAll('button')).find((button) => button.textContent === label) },
    async click(label: string) {
      const button = this.button(label)
      expect(button, `Missing button: ${label}`).toBeDefined()
      await act(async () => button!.click())
    },
    async key(init: KeyboardEventInit) {
      await act(async () => this.field('Cloud message text').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, ...init })))
    },
  }
}

afterEach(() => {
  cleanups.splice(0).forEach((cleanup) => cleanup())
  vi.useRealTimers()
})

describe('Cloud message composition', () => {
  it('loads templates on entering their mode once and sends named fields only on an explicit action', async () => {
    const view = await render()
    expect(view.trigger).not.toHaveBeenCalled()
    await view.fill('Cloud message type', 'template')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    await view.fill('Cloud message template', 'approved')
    await view.fill('Body: customer', 'Ada')
    expect(view.sends()).toHaveLength(0)
    await view.fill('Cloud message type', 'text')
    await view.fill('Cloud message type', 'template')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    await view.click('Send template')
    expect(view.sends()).toEqual([['whatsapp::cloud::messages::send_template', { chat_id: accepted.chat_id, name: 'request_update', language: 'en_US', components: [{ type: 'body', parameters: [{ type: 'text', parameter_name: 'customer', text: 'Ada' }] }] }]])
    expect(view.onSent).toHaveBeenCalledWith(accepted)
  })

  it('requires an explicit retry after a template load fails', async () => {
    const view = await render()
    const response = deferred<TemplatePage>()
    view.templateResults.push(response.promise)
    await view.fill('Cloud message type', 'template')
    expect(view.trigger).toHaveBeenCalledTimes(1)
    await act(async () => response.reject(new Error('Meta unavailable')))
    expect(view.container.textContent).toContain('Meta unavailable')
    await view.fill('Cloud message type', 'text')
    await view.fill('Cloud message type', 'template')
    expect(view.trigger).toHaveBeenCalledTimes(1)
    await view.click('Retry templates')
    expect(view.field('Cloud message template').textContent).toContain('request_update')
  })

  it('requires explicit JSON for dynamic buttons and preserves caller-supplied components', async () => {
    const view = await render()
    view.templateResults.push(Promise.resolve({ templates: [{ ...approved, components: [{ type: 'BUTTONS', buttons: [{ type: 'URL', text: 'Track', url: 'https://example.com/{{1}}' }] }] }], next_cursor: null }))
    await view.fill('Cloud message type', 'template')
    await view.fill('Cloud message template', 'approved')
    await view.click('Send template')
    expect(view.sends()).toHaveLength(0)
    expect(view.container.textContent).toContain('Message not sent')
    const components = [{ type: 'button', sub_type: 'url', index: '0', parameters: [{ type: 'text', text: 'order-123' }] }]
    await view.fill('Cloud template parameters', JSON.stringify(components))
    await view.click('Send template')
    expect(view.sends()).toEqual([['whatsapp::cloud::messages::send_template', { chat_id: accepted.chat_id, name: 'request_update', language: 'en_US', components }]])
  })

  it('keeps loaded templates while appending the next page and does not send pending templates', async () => {
    const view = await render()
    view.templateResults.push(Promise.resolve({ templates: [approved], next_cursor: 'page-two' }))
    await view.fill('Cloud message type', 'template')
    view.templateResults.push(Promise.resolve({ templates: [{ ...approved, id: 'pending', name: 'pending_offer', status: 'PENDING' }], next_cursor: null }))
    await view.click('Load more templates')
    expect(view.trigger).toHaveBeenLastCalledWith('whatsapp::cloud::templates::list', { limit: 50, after: 'page-two' })
    expect(view.field('Cloud message template').textContent).toContain('request_update')
    expect(view.field('Cloud message template').textContent).toContain('pending_offer')
    await view.fill('Cloud message template', 'pending')
    expect(view.button('Send template')?.disabled).toBe(true)
    expect(view.sends()).toHaveLength(0)
  })

  it('keeps invalid local drafts and never calls the send endpoint', async () => {
    const view = await render({ newChat: true })
    await view.fill('Recipient', 'not-a-phone')
    await view.fill('Cloud message text', 'Keep this draft')
    await view.click('Send')
    expect(view.container.textContent).toContain('Message not sent')
    expect(view.container.textContent).not.toContain('Check the conversation before retrying')
    expect(view.field('Cloud message text').value).toBe('Keep this draft')
    expect(view.sends()).toHaveLength(0)
  })

  it('keeps uncertain requests as drafts and never retries them automatically', async () => {
    const view = await render()
    const response = deferred<CloudSent>()
    view.sendResults.push(response.promise)
    await view.fill('Cloud message text', 'Do not duplicate this')
    await view.click('Send')
    await act(async () => response.reject(new Error('Request timed out')))
    expect(view.container.textContent).toContain('Send not confirmed')
    expect(view.container.textContent).toContain('Check the conversation before retrying')
    expect(view.field('Cloud message text').value).toBe('Do not duplicate this')
    expect(view.sends()).toHaveLength(1)
    expect(view.onSent).not.toHaveBeenCalled()
    expect(view.button('Send')?.disabled).toBe(false)
  })

  it('preserves Enter for text and ignores composing shortcuts while deduplicating explicit sends', async () => {
    const view = await render()
    const response = deferred<CloudSent>()
    view.sendResults.push(response.promise)
    await view.fill('Cloud message text', 'A deliberate message')
    await view.key({})
    await view.key({ ctrlKey: true, isComposing: true })
    expect(view.sends()).toHaveLength(0)
    await view.key({ ctrlKey: true })
    await view.key({ metaKey: true })
    expect(view.sends()).toHaveLength(1)
    await act(async () => { view.button('Sending…')!.click(); view.button('Sending…')!.click() })
    expect(view.sends()).toHaveLength(1)
    await act(async () => response.resolve(accepted))
    expect(view.field('Cloud message text').value).toBe('')
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
  })

  it('does not report a send after its conversation was unmounted', async () => {
    const view = await render()
    const response = deferred<CloudSent>()
    view.sendResults.push(response.promise)
    await view.fill('Cloud message text', 'Pending')
    await view.key({ metaKey: true })
    expect(view.sends()).toHaveLength(1)
    view.unmount()
    await act(async () => response.resolve(accepted))
    expect(view.onSent).not.toHaveBeenCalled()
  })

  it.each([undefined, Date.now() - 25 * 60 * 60 * 1000, Date.now() + 60 * 1000])('keeps the window unknown without recent valid incoming evidence (%s)', async (latestIncomingAt) => {
    const view = await render({ latestIncomingAt })
    expect(view.container.textContent).toContain('Reply window unknown')
    expect(view.container.textContent).not.toContain('window closed')
  })

  it('uses recent incoming evidence for an open window and returns to unknown on expiry', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-10-01T12:00:00Z'))
    const view = await render({ latestIncomingAt: Date.now() - 24 * 60 * 60 * 1000 + 1000 })
    expect(view.container.textContent).toContain('Reply window open')
    await act(async () => { await vi.advanceTimersByTimeAsync(1001) })
    expect(view.container.textContent).toContain('Reply window unknown')
    expect(view.container.textContent).not.toContain('window closed')
  })
})
