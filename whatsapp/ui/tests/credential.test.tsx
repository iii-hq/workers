// @vitest-environment jsdom

import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { CredentialControl } from '../src/credential'
import { credentialSecretName, referenceStatus } from '../src/secrets'

vi.mock('@iii-dev/console-ui', () => ({
  Badge: ({ children }: { children: ReactNode }) => <span>{children}</span>,
  Button: ({ children, variant: _variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props}>{children}</button>,
  Input: ({ onChange, ...props }: Omit<ComponentProps<'input'>, 'onChange'> & { onChange(value: string): void }) => <input {...props} onChange={(event) => onChange(event.target.value)} />,
}))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const TOKEN = 'EAAG-plain-token-value'
const cleanups: Array<() => void> = []
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup() })

type Backend = (id: string, payload: Record<string, unknown>) => unknown
async function render(value: string | undefined, backend: Backend) {
  const trigger = vi.fn(async (id: string, payload: Record<string, unknown>) => backend(id, payload))
  const host = { iii: { trigger } } as unknown as Host
  const onChange = vi.fn()
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  await act(async () => root.render(<CredentialControl host={host} control={{ id: 'token', 'data-field': 'cloud.access_token' }} label="Access token"
    value={value} target="WHATSAPP_CLOUD_ACCESS_TOKEN" onChange={onChange} />))
  cleanups.push(() => { act(() => root.unmount()); container.remove() })
  return { container, trigger, onChange }
}
const button = (container: HTMLElement, text: string) => [...container.querySelectorAll('button')].find((b) => b.textContent === text)
const missing = () => { throw Object.assign(new Error('function secrets::get not found'), { code: 'function_not_found' }) }

describe('Cloud credential field', () => {
  it('flags a plain-text value and moves it into the secrets store for WhatsApp', async () => {
    const { container, trigger, onChange } = await render(TOKEN, (id) => id === 'secrets::get' ? { name: 'WHATSAPP_CLOUD_ACCESS_TOKEN', consumers: ['ade'] } : {})
    expect(container.textContent).toContain('Plain text')
    expect(container.innerHTML).not.toContain(TOKEN)
    await act(async () => button(container, 'Move to secrets')!.click())
    expect(trigger).toHaveBeenCalledWith('secrets::set', expect.objectContaining({ name: 'WHATSAPP_CLOUD_ACCESS_TOKEN', value: TOKEN, consumers: ['ade', 'whatsapp'] }), expect.anything())
    expect(onChange).toHaveBeenCalledWith('secret://WHATSAPP_CLOUD_ACCESS_TOKEN')
  })

  it('saves a new value to secrets instead of configuration', async () => {
    const { container, trigger, onChange } = await render(undefined, (id) => id === 'secrets::get' ? null : {})
    const input = container.querySelector('input')!
    expect(input.type).toBe('password')
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, TOKEN)
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => button(container, 'Save to secrets')!.click())
    expect(trigger).toHaveBeenCalledWith('secrets::set', expect.objectContaining({ value: TOKEN, consumers: ['whatsapp'] }), expect.anything())
    expect(onChange).toHaveBeenCalledWith('secret://WHATSAPP_CLOUD_ACCESS_TOKEN')
    expect(onChange).not.toHaveBeenCalledWith(TOKEN)
  })

  it('keeps a value inline only on an explicit choice when the secrets worker is not running', async () => {
    const { container, onChange } = await render(undefined, missing)
    expect(container.textContent).toContain('The secrets worker is not running')
    expect(button(container, 'Save to secrets')).toBeUndefined()
    const input = container.querySelector('input')!
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, TOKEN)
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(onChange).not.toHaveBeenCalled()
    await act(async () => button(container, 'Keep as plain text')!.click())
    expect(onChange).toHaveBeenCalledWith(TOKEN)
  })

  it('shows a reference and warns when the secret is not shared with WhatsApp', async () => {
    const { container } = await render('secret://WA_TOKEN', () => ({ name: 'WA_TOKEN', consumers: ['storage'], hint: 'EAAG…lue' }))
    expect(container.textContent).toContain('secret://WA_TOKEN')
    expect(container.textContent).toContain('Not shared with the WhatsApp worker')
    expect(container.textContent).not.toContain('EAAG…lue')
  })
})

describe('credential helpers', () => {
  it('names a credential after its variable, per configuration', () => {
    expect(credentialSecretName('WHATSAPP_CLOUD_ACCESS_TOKEN', 'whatsapp')).toBe('WHATSAPP_CLOUD_ACCESS_TOKEN')
    expect(credentialSecretName('WHATSAPP_CLOUD_ACCESS_TOKEN', 'whatsapp-sales:eu')).toBe('WHATSAPP_CLOUD_ACCESS_TOKEN_WHATSAPP-SALES_EU')
  })
  it('reads a stored, shared secret as fine', () => {
    expect(referenceStatus('secret', { name: 'A', consumers: ['whatsapp'], updated_at: '2026-11-02T10:00:00Z' }, true, Date.parse('2026-11-02T10:05:00Z')))
      .toEqual({ text: 'Stored in secrets · updated 5m ago', warn: false })
    expect(referenceStatus('secret', null, true).warn).toBe(true)
  })
})
