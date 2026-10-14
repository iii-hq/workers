// @vitest-environment jsdom

import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { CloudComposer } from '../src/cloud-composer'
import { DraftProvider, DraftScope, useDraftMemory } from '../src/drafts'
import type { CloudSent, CloudTemplate, TemplatePage } from '../src/cloud-model'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children, variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button data-variant={variant ?? 'default'} {...props}>{children}</button>,
  Input: ({ onChange, ...props }: Omit<ComponentProps<'input'>, 'onChange'> & { onChange(value: string): void }) => <input {...props} onChange={(event) => onChange(event.target.value)} />,
  Select: ({ options, onChange, value, placeholder, ...props }: { options: Array<{ value: string; label: string; disabled?: boolean }>; onChange(value: string): void; value?: string; placeholder?: string; 'aria-label'?: string; disabled?: boolean }) => <select {...props} value={value ?? ''} onChange={(event) => onChange(event.target.value)}><option value="">{placeholder}</option>{options.map((option) => <option key={option.value} value={option.value} disabled={option.disabled}>{option.label}</option>)}</select>,
  CodeEditor: ({ value, onChange, ...props }: { value: string; onChange(value: string): void; 'aria-label'?: string; disabled?: boolean }) => <textarea aria-label={props['aria-label']} disabled={props.disabled} value={value} onChange={(event) => onChange(event.target.value)} />,
  SettingsList: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  SettingsField: ({ field, label, renderControl }: { field: string; label: string; renderControl(props: { id: string }): ReactNode }) => <div><label htmlFor={field}>{label}</label>{renderControl({ id: field })}</div>,
  EmptyState: ({ title, description }: { title: string; description: string }) => <p>{title} {description}</p>,
  StatusPanel: ({ headline, detail, action }: { headline: ReactNode; detail?: ReactNode; action?: ReactNode }) => <div role="alert">{headline}: {detail}{action}</div>,
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

async function render(options: { newChat?: boolean; latestIncomingAt?: number; group?: boolean; maxMediaBytes?: number; initialMode?: 'text' | 'template' | 'attachment'; initialSent?: CloudSent; ready?: boolean } = {}) {
  const templateResults: Array<Promise<TemplatePage>> = []
  const sendResults: Array<Promise<CloudSent>> = []
  const uploadResults: Array<Promise<unknown>> = []
  const trigger = vi.fn(async (id: string, _input: unknown) => {
    if (id === 'whatsapp::config-status') return { active: { max_media_bytes: options.maxMediaBytes ?? 16 * 1024 * 1024 } }
    if (id === 'whatsapp::cloud::media::upload') return uploadResults.shift() ?? { media_id: '123456789', media_type: 'document', file_name: 'report.txt', mimetype: 'text/plain', file_length: 6 }
    if (id.endsWith('templates::list')) return templateResults.shift() ?? { templates: [approved], next_cursor: null }
    if (id.includes('messages::send_')) return sendResults.shift() ?? accepted
    throw new Error(`Unexpected function: ${id}`)
  })
  const host = { iii: { trigger } } as unknown as Host
  const onDirty = vi.fn()
  const onSent = vi.fn()
  const onCheckConversation = vi.fn()
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  let memory: ReturnType<typeof useDraftMemory> = null
  const MemoryProbe = () => { memory = useDraftMemory(); return null }
  let account = 'cloud:waba:phone', retained = true
  let chat = options.newChat ? null : { chat_id: options.group ? 'group:123456' : accepted.chat_id, title: 'Customer', is_group: options.group ?? false }
  let props = { ready: options.ready ?? true, initialMode: options.initialMode, initialSent: options.initialSent, latestIncomingAt: options.latestIncomingAt }
  const renderComposer = () => root.render(<DraftProvider><MemoryProbe /><DraftScope id={account}>{retained && <CloudComposer key={`${account}:${chat?.chat_id ?? 'new'}`} host={host} chat={chat} onDirty={onDirty} onSent={onSent} onCheckConversation={onCheckConversation} {...props} />}</DraftScope></DraftProvider>)
  await act(async () => renderComposer())
  let mounted = true
  const unmount = () => { if (mounted) { act(() => root.unmount()); container.remove(); mounted = false } }
  cleanups.push(unmount)
  return {
    container, trigger, onDirty, onSent, onCheckConversation, templateResults, sendResults, uploadResults, unmount,
    draftCount() { return memory?.values.size ?? 0 },
    async hide() { retained = false; await act(async () => renderComposer()) },
    async restore() { retained = true; await act(async () => renderComposer()) },
    async conversation(next: typeof chat) { chat = next; await act(async () => renderComposer()) },
    async account(next: string) { account = next; await act(async () => renderComposer()) },
    async update(next: Partial<typeof props>) { props = { ...props, ...next }; await act(async () => renderComposer()) },
    async attach(file: File) {
      const field = container.querySelector('input[type="file"]')!
      Object.defineProperty(field, 'files', { configurable: true, value: [file] })
      await act(async () => field.dispatchEvent(new Event('change', { bubbles: true })))
    },
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
  vi.restoreAllMocks()
  vi.useRealTimers()
})

describe('Cloud message composition', () => {
  it('keeps the next recipient and selected template beside the send action while required fields are edited', async () => {
    const view = await render({ newChat: true, initialMode: 'template' })
    await view.fill('Recipient', accepted.chat_id)
    await view.fill('Cloud message template', 'approved')
    await view.fill('Body parameter customer — {{customer}}', 'Ada')
    const footer = view.button('Send template')!.closest('.wa-ui-cloud-send')!
    const recap = footer.querySelector('[aria-label="Next message details"]')!
    expect(recap.textContent).toContain(`Next recipient: ${accepted.chat_id}`)
    expect(recap.textContent).toContain('Template: request_update · en_US')
    await view.fill('Recipient', '+15557654321')
    expect(recap.textContent).toContain('Next recipient: +15557654321')
    expect(view.sends()).toHaveLength(0)
  })

  it('distinguishes an uncertain attempted recipient from the next recipient without retrying', async () => {
    const view = await render({ newChat: true })
    await view.fill('Recipient', accepted.chat_id)
    await view.fill('Cloud message text', 'First attempt')
    view.sendResults.push(Promise.reject(new Error('Request timed out')))
    await view.click('Send')
    await view.fill('Recipient', '+15557654321')
    expect(view.container.querySelector('[aria-label="Cloud send error"]')?.textContent).toContain(`Attempted recipient: ${accepted.chat_id}`)
    expect(view.container.querySelector('[aria-label="Next message details"]')?.textContent).toContain('Next recipient: +15557654321')
    await view.key({ ctrlKey: true })
    await view.click('Check this conversation')
    expect(view.onCheckConversation).toHaveBeenCalledWith(expect.objectContaining({ chat_id: accepted.chat_id }))
    expect(view.sends()).toHaveLength(1)
  })

  it('keeps the successful template choice while clearing values and leaving accepted evidence clean across a new conversation and remount', async () => {
    const view = await render({ newChat: true, initialMode: 'template' })
    await view.fill('Recipient', accepted.chat_id)
    await view.fill('Cloud message template', 'approved')
    await view.fill('Body parameter customer — {{customer}}', 'Ada')
    await view.click('Send template')
    expect(view.field('Cloud message template').value).toBe('approved')
    expect(view.field('Body parameter customer — {{customer}}').value).toBe('')
    expect(view.field('Cloud template parameters').value).toBe('')
    expect(view.button('Send template')?.disabled).toBe(true)
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    expect(view.draftCount()).toBe(0)
    await view.update({ initialSent: accepted })
    await view.conversation({ chat_id: accepted.chat_id, title: accepted.chat_id, is_group: false })
    expect(view.field('Cloud message template').value).toBe('approved')
    expect(view.container.textContent).toContain('Accepted by Meta')
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    await view.hide(); await view.restore()
    expect(view.field('Cloud message template').value).toBe('approved')
    expect(view.field('Body parameter customer — {{customer}}').value).toBe('')
    expect(view.container.textContent).toContain('Accepted by Meta')
    expect(view.button('Send template')?.disabled).toBe(true)
    expect(view.sends()).toHaveLength(1)
    await view.account('cloud:another-account:another-phone')
    expect(view.field('Cloud message template').value).toBe('')
    expect(view.sends()).toHaveLength(1)
  })

  it.each(['text', 'template', 'attachment'] as const)('validates a new %s recipient on blur, links the error and preserves it through a tool remount without sending', async (initialMode) => {
    const view = await render({ newChat: true, initialMode })
    const recipient = view.field('Recipient') as HTMLInputElement
    await view.fill('Recipient', 'not-a-phone')
    expect(recipient.getAttribute('aria-invalid')).not.toBe('true')
    await act(async () => { recipient.focus(); recipient.blur() })
    expect(recipient.getAttribute('aria-invalid')).toBe('true')
    const description = recipient.getAttribute('aria-describedby')!.split(' ').map((id) => document.getElementById(id)?.textContent).join(' ')
    expect(description).toContain('Native WhatsApp JIDs are not supported here.')
    expect(view.sends()).toHaveLength(0)
    await view.hide(); await view.restore()
    expect(view.field('Recipient').getAttribute('aria-invalid')).toBe('true')
    await view.fill('Recipient', '+15551234567')
    expect(view.field('Recipient').getAttribute('aria-invalid')).not.toBe('true')
    expect(view.sends()).toHaveLength(0)
  })

  it('retains unknown attempt evidence with an empty clean draft and never restores an older acceptance', async () => {
    const view = await render({ initialSent: accepted })
    await view.fill('Cloud message text', 'Next attempt')
    view.sendResults.push(Promise.reject(new Error('Request timed out')))
    await view.click('Send')
    await view.fill('Cloud message text', '')
    expect(view.draftCount()).toBe(0)
    expect(view.onDirty).toHaveBeenLastCalledWith(false)
    await view.hide()
    await view.restore()
    expect(view.container.textContent).toContain('Send not confirmed')
    expect(view.container.textContent).not.toContain('Accepted by Meta')
    expect(view.draftCount()).toBe(0)
    expect(view.button('Send again')?.disabled).toBe(true)
    await view.click('Check this conversation')
    expect(view.sends()).toHaveLength(1)
  })

  it('starts oversized feedback at its headline and exposes the read-only recovery beside a secondary repeat', async () => {
    const view = await render()
    const fields = view.container.querySelector<HTMLDivElement>('.wa-ui-cloud-composer-fields')!
    const original = HTMLElement.prototype.getBoundingClientRect
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      const top = this === fields ? 500 : 550
      if (this === fields || this.getAttribute('aria-label') === 'Cloud send error') return { top, bottom: top + (this === fields ? 124 : 200), height: this === fields ? 124 : 200, left: 0, right: 340, width: 340, x: 0, y: top, toJSON() {} }
      return original.call(this)
    })
    await view.fill('Cloud message text', 'Inspect before repeating')
    view.sendResults.push(Promise.reject(new Error('Request timed out')))
    await view.click('Send')
    expect(fields.scrollTop).toBe(50)
    expect(document.activeElement?.getAttribute('aria-label')).toBe('Cloud send error')
    expect(view.button('Check this conversation')?.closest('.wa-ui-cloud-send')).toBeTruthy()
    expect(view.button('Check this conversation')?.dataset.variant).toBe('default')
    expect(view.button('Send again')?.dataset.variant).toBe('ghost')
    await view.key({ ctrlKey: true })
    await view.fill('Cloud message text', 'Changed next attempt')
    await view.key({ metaKey: true })
    expect(view.sends()).toHaveLength(1)
    await view.click('Check this conversation')
    expect(view.sends()).toHaveLength(1)
    await view.click('Send again')
    expect(view.sends()).toHaveLength(2)
  })

  it('reveals a clipped acceptance within the composer without scrolling the conversation', async () => {
    const view = await render()
    const fields = view.container.querySelector<HTMLDivElement>('.wa-ui-cloud-composer-fields')!
    const rect = (top: number, bottom: number) => ({ top, bottom, height: bottom - top, left: 0, right: 340, width: 340, x: 0, y: top, toJSON() {} })
    const original = HTMLElement.prototype.getBoundingClientRect
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this === fields) return rect(500, 730)
      if (this.getAttribute('aria-label') === 'Cloud send result') return rect(750, 930)
      return original.call(this)
    })
    await view.fill('Cloud message text', 'Explicit acceptance')
    await view.click('Send')
    const receipt = view.container.querySelector('[aria-label="Cloud send result"]')!
    expect(fields.scrollTop).toBe(200)
    expect(document.activeElement).toBe(receipt)
    expect(receipt.textContent).toContain('Acceptance does not confirm delivery')
    expect(view.field('Message ID').value).toBe(accepted.message_id)
    expect(view.sends()).toHaveLength(1)
    // Passive status changes retain the user's new scroll/focus position.
    fields.scrollTop = 47
    view.field('Cloud message text').focus()
    await view.update({ latestIncomingAt: Date.now(), ready: false })
    expect(fields.scrollTop).toBe(47)
    expect(document.activeElement).toBe(view.field('Cloud message text'))
    expect(view.sends()).toHaveLength(1)
  })

  it('keeps an already visible receipt and editor focus stable', async () => {
    const view = await render()
    const fields = view.container.querySelector<HTMLDivElement>('.wa-ui-cloud-composer-fields')!
    const original = HTMLElement.prototype.getBoundingClientRect
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this === fields || this.getAttribute('aria-label') === 'Cloud send result') return { top: 500, bottom: 700, height: 200, left: 0, right: 340, width: 340, x: 0, y: 500, toJSON() {} }
      return original.call(this)
    })
    await view.fill('Cloud message text', 'Visible acceptance')
    view.field('Cloud message text').focus()
    fields.scrollTop = 31
    await view.click('Send')
    expect(fields.scrollTop).toBe(31)
    expect(document.activeElement).toBe(view.field('Cloud message text'))
    expect(view.field('Message ID').value).toBe(accepted.message_id)
  })

  it('opens an explicitly requested template composer and loads choices without sending', async () => {
    const view = await render({ newChat: true, initialMode: 'template' })
    expect(view.field('Cloud message type').value).toBe('template')
    expect(view.field('Cloud message template').textContent).toContain('request_update')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    expect(view.button('Send template')?.disabled).toBe(true)
    expect(view.sends()).toHaveLength(0)
  })

  it('uses initial mode only once and preserves a chosen text draft through status changes', async () => {
    const view = await render({ newChat: true, initialMode: 'template' })
    await view.fill('Recipient', 'US.ENT.Customer123')
    await view.fill('Cloud message type', 'text')
    await view.fill('Cloud message text', 'Keep this reply draft')
    const textarea = view.field('Cloud message text')
    await view.update({ ready: false, initialMode: 'attachment' })
    expect(view.field('Cloud message type').value).toBe('text')
    expect(view.field('Cloud message text')).toBe(textarea)
    expect(textarea.value).toBe('Keep this reply draft')
    expect(view.field('Recipient').value).toBe('US.ENT.Customer123')
    expect(view.button('Send')?.disabled).toBe(true)
    await view.update({ ready: true, initialMode: 'template', latestIncomingAt: Date.now() })
    expect(view.field('Cloud message type').value).toBe('text')
    expect(view.field('Cloud message text').value).toBe('Keep this reply draft')
    expect(view.button('Send')?.disabled).toBe(false)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    expect(view.sends()).toHaveLength(0)
  })

  it('routes text from a Cloud group to the official group send function', async () => {
    const view = await render({ group: true })
    await view.fill('Cloud message text', 'Hello group')
    await view.click('Send')
    expect(view.sends()).toEqual([['whatsapp::cloud::groups::messages::send_text', { group_id: '123456', text: 'Hello group' }]])
  })

  it('uploads a browser-selected document and sends its media ID after explicit Send', async () => {
    const view = await render()
    await view.fill('Cloud message type', 'attachment')
    await view.attach(new File(['Report'], 'report.txt', { type: 'text/plain' }))
    expect(view.sends()).toHaveLength(0)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('media::upload'))).toHaveLength(0)
    await view.fill('Caption (optional)', 'Monthly report')
    await view.click('Send attachment')
    await act(async () => { await vi.waitFor(() => expect(view.sends()).toHaveLength(1)) })
    expect(view.trigger).toHaveBeenCalledWith('whatsapp::cloud::media::upload', { media_type: 'document', data_base64: btoa('Report'), mimetype: 'text/plain', file_name: 'report.txt' })
    expect(view.sends()).toEqual([['whatsapp::cloud::messages::send_document', { chat_id: accepted.chat_id, media_id: '123456789', file_name: 'report.txt', caption: 'Monthly report' }]])
  })

  it('keeps an attachment and caption in memory across view switches and clears it after acceptance', async () => {
    const view = await render()
    await view.fill('Cloud message type', 'attachment')
    await view.attach(new File(['Report'], 'report.txt', { type: 'text/plain' }))
    await view.fill('Caption (optional)', 'Kept attachment caption')
    await view.hide()
    await view.restore()
    expect(view.field('Cloud message type').value).toBe('attachment')
    expect(view.field('Caption (optional)').value).toBe('Kept attachment caption')
    expect(view.container.textContent).toContain('report.txt')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('media::upload'))).toHaveLength(0)
    expect(view.sends()).toHaveLength(0)
    await view.click('Send attachment')
    await act(async () => { await vi.waitFor(() => expect(view.sends()).toHaveLength(1)) })
    await view.hide()
    await view.restore()
    expect(view.container.textContent).not.toContain('report.txt')
    expect(view.sends()).toHaveLength(1)
  })

  it('restores an ambiguous send warning with the existing upload without automatically retrying', async () => {
    const view = await render()
    await view.fill('Cloud message type', 'attachment')
    await view.attach(new File(['Report'], 'report.txt', { type: 'text/plain' }))
    const failed = deferred<CloudSent>()
    view.sendResults.push(failed.promise)
    await view.click('Send attachment')
    await act(async () => { await vi.waitFor(() => expect(view.sends()).toHaveLength(1)) })
    await act(async () => failed.reject(new Error('Request timed out; the outcome is unknown.')))
    await view.hide()
    await view.restore()
    expect(view.container.textContent).toContain('Request timed out; the outcome is unknown.')
    expect(view.container.textContent).toContain('Check the conversation before sending again')
    expect(view.sends()).toHaveLength(1)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('media::upload'))).toHaveLength(1)
    await view.click('Send again')
    expect(view.sends()).toHaveLength(2)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('media::upload'))).toHaveLength(1)
  })

  it('keeps Cloud drafts separate for different configured business phones', async () => {
    const view = await render()
    await view.fill('Cloud message text', 'Draft for the first business phone')
    await view.account('cloud:waba:another-phone')
    expect(view.field('Cloud message text').value).toBe('')
    await view.fill('Cloud message text', 'Draft for the other business phone')
    await view.account('cloud:waba:phone')
    expect(view.field('Cloud message text').value).toBe('Draft for the first business phone')
    expect(view.sends()).toHaveLength(0)
  })

  it('rejects Markdown and oversized files without reading or uploading them', async () => {
    const view = await render({ maxMediaBytes: 4 })
    await view.fill('Cloud message type', 'attachment')
    await view.attach(new File(['# Report'], 'report.md', { type: 'text/markdown' }))
    expect(view.container.textContent).toContain('Choose a TXT or PDF file')
    expect(view.button('Send attachment')?.disabled).toBe(true)
    await view.attach(new File(['Report'], 'report.txt', { type: 'text/plain' }))
    expect(view.container.textContent).toContain('exceeds')
    expect(view.button('Send attachment')?.disabled).toBe(true)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('media::upload'))).toHaveLength(0)
  })

  it('retains uploaded media after an ambiguous send and reuses it only on explicit resend', async () => {
    const view = await render()
    const response = deferred<CloudSent>()
    view.sendResults.push(response.promise)
    await view.fill('Cloud message type', 'attachment')
    await view.attach(new File(['Report'], 'report.txt', { type: 'text/plain' }))
    await view.click('Send attachment')
    await act(async () => { await vi.waitFor(() => expect(view.sends()).toHaveLength(1)) })
    await act(async () => response.reject(new Error('Send timed out')))
    expect(view.container.textContent).toContain('Check the conversation before sending again')
    expect(view.container.textContent).toContain('report.txt')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('media::upload'))).toHaveLength(1)
    await view.click('Send again')
    expect(view.sends()).toHaveLength(2)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('media::upload'))).toHaveLength(1)
  })

  it('does not send an attachment when its upload completes after unmount', async () => {
    const view = await render()
    const upload = deferred<unknown>()
    view.uploadResults.push(upload.promise)
    await view.fill('Cloud message type', 'attachment')
    await view.attach(new File(['Report'], 'report.txt', { type: 'text/plain' }))
    await view.click('Send attachment')
    await act(async () => { await vi.waitFor(() => expect(view.trigger.mock.calls.some(([id]) => id.endsWith('media::upload'))).toBe(true)) })
    view.unmount()
    await act(async () => upload.resolve({ media_id: '123456789' }))
    expect(view.sends()).toHaveLength(0)
    expect(view.onSent).not.toHaveBeenCalled()
  })

  it('routes an uploaded group attachment through the official group media contract', async () => {
    const view = await render({ group: true })
    await view.fill('Cloud message type', 'attachment')
    await view.attach(new File(['Report'], 'report.txt', { type: 'text/plain' }))
    await view.click('Send attachment')
    await act(async () => { await vi.waitFor(() => expect(view.sends()).toHaveLength(1)) })
    expect(view.sends()).toEqual([['whatsapp::cloud::groups::messages::send_media', { group_id: '123456', media_type: 'document', media_id: '123456789', file_name: 'report.txt' }]])
  })

  it('stops before sending when upload fails and keeps the selected file', async () => {
    const view = await render()
    const upload = deferred<unknown>()
    view.uploadResults.push(upload.promise)
    await view.fill('Cloud message type', 'attachment')
    await view.attach(new File(['Report'], 'report.txt', { type: 'text/plain' }))
    await view.click('Send attachment')
    await act(async () => { await vi.waitFor(() => expect(view.trigger.mock.calls.some(([id]) => id.endsWith('media::upload'))).toBe(true)) })
    await act(async () => upload.reject(new Error('Upload timed out')))
    expect(view.container.textContent).toContain('No message was sent')
    expect(view.container.textContent).toContain('report.txt')
    expect(view.sends()).toHaveLength(0)
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('media::upload'))).toHaveLength(1)
  })
  it('loads templates on entering their mode once and sends named fields only on an explicit action', async () => {
    const view = await render()
    expect(view.trigger).not.toHaveBeenCalled()
    await view.fill('Cloud message type', 'template')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    await view.fill('Cloud message template', 'approved')
    await view.fill('Body parameter customer — {{customer}}', 'Ada')
    expect(view.sends()).toHaveLength(0)
    await view.fill('Cloud message type', 'text')
    await view.fill('Cloud message type', 'template')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('templates::list'))).toHaveLength(1)
    await view.click('Send template')
    expect(view.sends()).toEqual([['whatsapp::cloud::messages::send_template', { chat_id: accepted.chat_id, name: 'request_update', language: 'en_US', components: [{ type: 'body', parameters: [{ type: 'text', parameter_name: 'customer', text: 'Ada' }] }] }]])
    expect(view.onSent).toHaveBeenCalledWith(accepted, 'template')
  })

  it('communicates required fields quietly and disables template sends until every value is present', async () => {
    const view = await render()
    view.templateResults.push(Promise.resolve({ templates: [{ ...approved, parameter_format: 'POSITIONAL', components: [{ type: 'BODY', text: 'Hello {{1}}, your order is {{2}}.' }] }], next_cursor: null }))
    await view.fill('Cloud message type', 'template')
    await view.fill('Cloud message template', 'approved')
    const field = view.field('Body parameter 1 — {{1}}') as HTMLInputElement
    expect(field.required).toBe(true)
    expect(field.getAttribute('aria-invalid')).not.toBe('true')
    expect(document.getElementById(field.getAttribute('aria-describedby')!)?.textContent).toContain('Required')
    expect(view.container.textContent).toContain('Complete the required template fields')
    expect(view.container.textContent).not.toContain('Message not sent')
    expect(view.button('Send template')?.disabled).toBe(true)
    await view.fill('Body parameter 1 — {{1}}', 'Ada')
    await view.fill('Body parameter 2 — {{2}}', '   ')
    expect(view.button('Send template')?.disabled).toBe(true)
    await view.click('Send template')
    expect(view.sends()).toHaveLength(0)
    await view.fill('Body parameter 2 — {{2}}', 'ready')
    expect(view.button('Send template')?.disabled).toBe(false)
    expect(view.sends()).toHaveLength(0)
  })

  it('marks an empty required field only after leaving it and clears that feedback as it is completed', async () => {
    const view = await render({ initialMode: 'template' })
    await view.fill('Cloud message template', 'approved')
    const field = view.field('Body parameter customer — {{customer}}') as HTMLInputElement
    await act(async () => { field.focus(); field.blur() })
    expect(field.getAttribute('aria-invalid')).toBe('true')
    expect(document.getElementById(field.getAttribute('aria-describedby')!)?.textContent).toContain('Enter a value for {{customer}}')
    await view.fill('Body parameter customer — {{customer}}', 'Ada')
    expect(field.getAttribute('aria-invalid')).not.toBe('true')
    expect(view.button('Send template')?.disabled).toBe(false)
  })

  it('permits a static approved template without inventing required fields', async () => {
    const view = await render()
    view.templateResults.push(Promise.resolve({ templates: [{ ...approved, components: [{ type: 'BODY', text: 'Your report is ready.' }] }], next_cursor: null }))
    await view.fill('Cloud message type', 'template')
    await view.fill('Cloud message template', 'approved')
    expect(view.button('Send template')?.disabled).toBe(false)
    await view.click('Send template')
    expect(view.sends()).toEqual([['whatsapp::cloud::messages::send_template', { chat_id: accepted.chat_id, name: 'request_update', language: 'en_US', components: undefined }]])
  })

  it('preserves an explicit Advanced JSON override and disables sends while its local structure is invalid', async () => {
    const view = await render({ initialMode: 'template' })
    await view.fill('Cloud message template', 'approved')
    const components = [{ type: 'body', parameters: [{ type: 'currency', parameter_name: 'customer', currency: { fallback_value: '$12.00', code: 'USD', amount_1000: 12000 } }] }]
    await view.fill('Cloud template parameters', '{')
    expect(view.container.textContent).toContain('Template parameters must be valid JSON')
    expect(view.button('Send template')?.disabled).toBe(true)
    await view.fill('Cloud template parameters', '{}')
    expect(view.container.textContent).toContain('JSON array of component objects')
    expect(view.button('Send template')?.disabled).toBe(true)
    await view.fill('Cloud template parameters', '[]')
    expect(view.container.textContent).toContain('required component objects')
    expect(view.button('Send template')?.disabled).toBe(true)
    await view.fill('Cloud template parameters', JSON.stringify(components))
    expect(view.button('Send template')?.disabled).toBe(false)
    expect(view.field('Body parameter customer — {{customer}}').disabled).toBe(true)
    expect(view.sends()).toHaveLength(0)
    await view.click('Send template')
    expect(view.sends()).toEqual([['whatsapp::cloud::messages::send_template', { chat_id: accepted.chat_id, name: 'request_update', language: 'en_US', components }]])
  })

  it('blocks empty component objects and missing known named parameters before a template send', async () => {
    const view = await render({ initialMode: 'template' })
    await view.fill('Cloud message template', 'approved')
    for (const raw of ['[{}]', '[{"type":"body","parameters":[]}]', '[{"type":"body","parameters":[{"type":"text","text":"Ada"}]}]']) {
      await view.fill('Cloud template parameters', raw)
      expect(view.button('Send template')?.disabled).toBe(true)
      expect(view.container.textContent).toContain('Fix the template components to send.')
      expect(view.sends()).toHaveLength(0)
    }
  })

  it('shows a template-specific read-only example and official payload reference without populating or sending it', async () => {
    const view = await render({ initialMode: 'template' })
    await view.fill('Cloud message template', 'approved')
    const example = view.container.querySelector('[aria-label="Selected template component example"]')!
    expect(JSON.parse(example.textContent!)).toEqual([{ type: 'body', parameters: [{ type: 'text', parameter_name: 'customer', text: '' }] }])
    const link = view.container.querySelector<HTMLAnchorElement>('a')!
    expect(link.href).toBe('https://developers.facebook.com/documentation/business-messaging/whatsapp/templates/overview')
    expect(link.rel).toBe('noopener noreferrer')
    expect(view.field('Cloud template parameters').value).toBe('')
    expect(view.button('Send template')?.disabled).toBe(true)
    expect(view.sends()).toHaveLength(0)
  })

  it('keeps guided values and invalid JSON drafts through readiness changes without any send attempt', async () => {
    const view = await render({ initialMode: 'template' })
    await view.fill('Cloud message template', 'approved')
    await view.fill('Body parameter customer — {{customer}}', 'Ada')
    const raw = JSON.stringify([{ text: 'x'.repeat(65536) }])
    await view.fill('Cloud template parameters', raw)
    expect(view.container.textContent).toContain('64 KiB')
    expect(view.button('Send template')?.disabled).toBe(true)
    const description = view.container.querySelector('[aria-label="Advanced template components"]')?.getAttribute('aria-describedby')?.split(' ').map((id) => document.getElementById(id)?.textContent).join(' ')
    expect(description).toContain('64 KiB')
    await view.update({ ready: false })
    await view.update({ ready: true })
    expect(view.field('Cloud template parameters').value).toBe(raw)
    expect(view.field('Body parameter customer — {{customer}}').value).toBe('Ada')
    await view.fill('Cloud template parameters', '')
    expect(view.field('Body parameter customer — {{customer}}').disabled).toBe(false)
    expect(view.button('Send template')?.disabled).toBe(false)
    expect(view.sends()).toHaveLength(0)
  })

  it('seeds an accepted receipt once and keeps its identifier selectable and copyable until the next edit', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    const originalClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    try {
      const view = await render({ initialSent: accepted })
      const receipt = view.field('Message ID') as HTMLInputElement
      expect(receipt.readOnly).toBe(true)
      expect(receipt.value).toBe(accepted.message_id)
      expect(view.container.textContent).toContain('Acceptance does not confirm delivery')
      const copy = view.container.querySelector<HTMLButtonElement>('[aria-label="Copy message ID"]')!
      await act(async () => copy.click())
      expect(writeText).toHaveBeenCalledWith(accepted.message_id)
      await view.update({ initialSent: { ...accepted, message_id: 'replacement' }, ready: false })
      expect(view.field('Message ID').value).toBe(accepted.message_id)
      await view.fill('Cloud message text', 'Next draft')
      expect(view.field('Message ID')).toBeNull()
      await view.update({ initialSent: accepted, ready: true })
      expect(view.field('Message ID')).toBeNull()
      expect(view.sends()).toHaveLength(0)
    } finally {
      if (originalClipboard) Object.defineProperty(navigator, 'clipboard', originalClipboard)
      else Reflect.deleteProperty(navigator, 'clipboard')
    }
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

  it('names positional fields with their placeholders and sends them in component order', async () => {
    const view = await render()
    view.templateResults.push(Promise.resolve({ templates: [{ ...approved, parameter_format: 'POSITIONAL', components: [{ type: 'HEADER', format: 'TEXT', text: 'Order {{1}}' }, { type: 'BODY', text: 'Hello {{1}}, your order is {{2}}.' }] }], next_cursor: null }))
    await view.fill('Cloud message type', 'template')
    await view.fill('Cloud message template', 'approved')
    await view.fill('Header parameter 1 — {{1}}', '123')
    await view.fill('Body parameter 1 — {{1}}', 'Ada')
    await view.fill('Body parameter 2 — {{2}}', 'ready')
    expect(view.sends()).toHaveLength(0)
    await view.click('Send template')
    expect(view.sends()).toEqual([['whatsapp::cloud::messages::send_template', { chat_id: accepted.chat_id, name: 'request_update', language: 'en_US', components: [{ type: 'header', parameters: [{ type: 'text', text: '123' }] }, { type: 'body', parameters: [{ type: 'text', text: 'Ada' }, { type: 'text', text: 'ready' }] }] }]])
  })

  it('requires explicit JSON for dynamic buttons and preserves caller-supplied components', async () => {
    const view = await render()
    view.templateResults.push(Promise.resolve({ templates: [{ ...approved, components: [{ type: 'BUTTONS', buttons: [{ type: 'URL', text: 'Track', url: 'https://example.com/{{1}}' }] }] }], next_cursor: null }))
    await view.fill('Cloud message type', 'template')
    await view.fill('Cloud message template', 'approved')
    expect(view.button('Send template')?.disabled).toBe(true)
    await view.click('Send template')
    expect(view.sends()).toHaveLength(0)
    expect(view.container.textContent).not.toContain('Message not sent')
    await view.fill('Cloud template parameters', '[]')
    expect(view.container.textContent).toContain('required component objects')
    expect(view.button('Send template')?.disabled).toBe(true)
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
    expect(view.button('Send')?.disabled).toBe(true)
    expect(view.container.textContent).toContain('Enter a valid recipient to send.')
    expect(view.container.textContent).not.toContain('Check the conversation before sending again')
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
    expect(view.container.textContent).toContain('Sending again may duplicate the previous message')
    expect(view.field('Cloud message text').value).toBe('Do not duplicate this')
    expect(view.sends()).toHaveLength(1)
    expect(view.onSent).not.toHaveBeenCalled()
    expect(view.button('Send')).toBeUndefined()
    expect(view.button('Send again')?.disabled).toBe(false)
    await view.click('Check this conversation')
    expect(view.onCheckConversation).toHaveBeenCalledWith({ chat_id: accepted.chat_id, title: 'Customer', is_group: false })
    expect(view.sends()).toHaveLength(1)
    await view.click('Send again')
    expect(view.sends()).toHaveLength(2)
  })

  it('keeps uncertain recovery tied to the attempted recipient when the next draft is edited', async () => {
    const view = await render({ newChat: true })
    const response = deferred<CloudSent>()
    view.sendResults.push(response.promise)
    await view.fill('Recipient', accepted.chat_id)
    await view.fill('Cloud message text', 'Original attempt')
    await view.click('Send')
    await act(async () => response.reject(new Error('Request timed out')))
    await view.fill('Recipient', '+15557654321')
    await view.fill('Cloud message text', 'Edited next attempt')
    expect(view.container.textContent).toContain('Send not confirmed')
    await view.hide()
    await view.restore()
    await view.click('Check this conversation')
    expect(view.onCheckConversation).toHaveBeenCalledWith({ chat_id: accepted.chat_id, title: accepted.chat_id, is_group: false })
    expect(view.field('Recipient').value).toBe('+15557654321')
    expect(view.button('Send again')?.disabled).toBe(false)
    expect(view.sends()).toHaveLength(1)
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
    expect(view.container.querySelector('[aria-label="Reply window unknown"] span')?.textContent).toBe('Window unknown')
    expect(view.container.textContent).not.toContain('window closed')
  })

  it('uses recent incoming evidence for an open window and returns to unknown on expiry', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-10-01T12:00:00Z'))
    const view = await render({ latestIncomingAt: Date.now() - 24 * 60 * 60 * 1000 + 1000 })
    expect(view.container.querySelector('[aria-label="Reply window open"] span')?.textContent).toBe('Window open')
    await act(async () => { await vi.advanceTimersByTimeAsync(1001) })
    expect(view.container.querySelector('[aria-label="Reply window unknown"] span')?.textContent).toBe('Window unknown')
    expect(view.container.textContent).not.toContain('window closed')
  })
})
