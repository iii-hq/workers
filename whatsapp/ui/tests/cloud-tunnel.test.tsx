// @vitest-environment jsdom

import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { CloudTunnel } from '../src/cloud-tunnel'
import type { CloudTunnelStatus } from '../src/cloud-model'

vi.mock('@iii-dev/console-ui', () => ({
  Badge: ({ children }: { children?: ReactNode }) => <span>{children}</span>,
  Button: ({ children, variant: _variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props}>{children}</button>,
  IconButton: ({ children, label, ...props }: ComponentProps<'button'> & { label: string }) => <button {...props} aria-label={label}>{children}</button>,
  Input: ({ onChange: _onChange, ...props }: ComponentProps<'input'>) => <input {...props} />,
  SettingsList: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  SettingsField: ({ field, label, renderControl }: { field: string; label: string; renderControl(props: { id: string }): ReactNode }) => <div><label htmlFor={field}>{label}</label>{renderControl({ id: field })}</div>,
  StatusPanel: ({ headline, detail, action }: { headline: string; detail: string; action?: ReactNode }) => <div role="alert">{headline}: {detail}{action}</div>,
}))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const stopped: CloudTunnelStatus = { enabled: false, status: 'stopped', tunnel_id: 'webhooks', public_url: null, webhook_url: null, webhook_path: '/webhooks/whatsapp-cloud/test', error: null }
const ready: CloudTunnelStatus = { ...stopped, enabled: true, status: 'ready', public_url: 'https://first.trycloudflare.com', webhook_url: 'https://first.trycloudflare.com/webhooks/whatsapp-cloud/test' }
const mounted: Array<() => void> = []

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

async function render(initial = stopped, webhookReady = true) {
  let backend = initial
  const statuses: Array<Promise<CloudTunnelStatus>> = []
  const starts: Array<Promise<CloudTunnelStatus>> = []
  const stops: Array<Promise<CloudTunnelStatus>> = []
  const trigger = vi.fn(async (id: string) => {
    if (id.endsWith('::status')) return statuses.shift() ?? backend
    if (id.endsWith('::start')) return starts.shift() ?? (backend = ready)
    if (id.endsWith('::stop')) return stops.shift() ?? (backend = stopped)
    throw new Error(`Unexpected function: ${id}`)
  })
  const host = { iii: { trigger, on: () => () => {}, registerTrigger: () => () => {}, browserId: 'test' } } as unknown as Host
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  await act(async () => root.render(<CloudTunnel host={host} webhookReady={webhookReady} />))
  let active = true
  const unmount = () => { if (active) { act(() => root.unmount()); container.remove(); active = false } }
  mounted.push(unmount)
  return {
    container, trigger, statuses, starts, stops, unmount,
    setBackend(next: CloudTunnelStatus) { backend = next },
    async setReady(next: boolean) { await act(async () => root.render(<CloudTunnel host={host} webhookReady={next} />)) },
    button(label: string) { return Array.from(container.querySelectorAll('button')).find((button) => button.textContent === label || button.getAttribute('aria-label') === label) },
    async click(label: string) {
      const button = this.button(label)
      expect(button, `Missing button: ${label}`).toBeDefined()
      await act(async () => button!.click())
    },
  }
}

afterEach(() => {
  for (const unmount of mounted.splice(0)) unmount()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('development webhook controls', () => {
  it('only reads status on mount and requires configured webhook credentials to start', async () => {
    const view = await render(stopped, false)
    expect(view.trigger.mock.calls.map(([id]) => id)).toEqual(['whatsapp::cloud::tunnel::status'])
    expect(view.button('Start development tunnel')?.disabled).toBe(true)
    await view.setReady(true)
    await view.click('Start development tunnel')
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('::start'))).toHaveLength(1)
    expect(view.container.querySelector('input')?.value).toBe(ready.webhook_url)
    expect(view.button('Stop tunnel')).toBeDefined()
    expect(view.container.textContent).toContain('does not register it with Meta')
  })

  it('copies the full backend URL and retains Stop when Cloud API setup becomes unavailable', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const view = await render(ready)
    await view.click('Copy webhook URL')
    expect(writeText).toHaveBeenCalledWith(ready.webhook_url)
    expect(view.button('Copied')).toBeDefined()
    await view.setReady(false)
    expect(view.button('Copy webhook URL')).toBeUndefined()
    expect(view.container.querySelector('input')).toBeNull()
    expect(view.button('Stop tunnel')?.disabled).toBe(false)
    await view.click('Stop tunnel')
    expect(view.trigger.mock.calls.some(([id]) => id.endsWith('::stop'))).toBe(true)
  })

  it('hides a previously ready URL after a failed refresh and restores only the current URL', async () => {
    const view = await render(ready)
    const failed = deferred<CloudTunnelStatus>()
    view.statuses.push(failed.promise)
    await view.click('Refresh development tunnel')
    await act(async () => failed.reject(new Error('Engine disconnected')))
    expect(view.container.querySelector('input')).toBeNull()
    expect(view.button('Copy webhook URL')).toBeUndefined()
    expect(view.container.textContent).toContain('Engine disconnected')
    const next = { ...ready, public_url: 'https://second.trycloudflare.com', webhook_url: 'https://second.trycloudflare.com/webhooks/whatsapp-cloud/test' }
    view.setBackend(next)
    await view.click('Refresh status')
    expect(view.container.querySelector('input')?.value).toBe(next.webhook_url)
    expect(view.container.textContent).not.toContain('Engine disconnected')
  })

  it.each(['starting', 'reconnecting', 'failed', 'unavailable'] as const)('never exposes the previous URL while %s', async (status) => {
    const view = await render({ ...ready, status, error: status === 'failed' ? 'Tunnel connection failed' : null })
    expect(view.container.querySelector('input')).toBeNull()
    expect(view.button('Copy webhook URL')).toBeUndefined()
    expect(view.button('Stop tunnel')).toBeDefined()
    if (status === 'failed' || status === 'unavailable') expect(view.button('Retry development tunnel')?.disabled).toBe(false)
  })

  it('drops polls from before Stop and prevents duplicate commands', async () => {
    const view = await render(ready)
    const oldPoll = deferred<CloudTunnelStatus>()
    const stop = deferred<CloudTunnelStatus>()
    view.statuses.push(oldPoll.promise)
    view.stops.push(stop.promise)
    await view.click('Refresh development tunnel')
    const stopButton = view.button('Stop tunnel')!
    await act(async () => { stopButton.click(); stopButton.click() })
    expect(view.trigger.mock.calls.filter(([id]) => id.endsWith('::stop'))).toHaveLength(1)
    expect(view.container.querySelector('input')).toBeNull()
    await act(async () => oldPoll.resolve(ready))
    expect(view.container.querySelector('input')).toBeNull()
    const afterStop = deferred<CloudTunnelStatus>()
    view.statuses.push(afterStop.promise)
    await act(async () => stop.resolve(stopped))
    expect(view.container.querySelector('input')).toBeNull()
    await act(async () => afterStop.resolve(stopped))
    expect(view.button('Start development tunnel')?.disabled).toBe(false)
    expect(view.button('Stop tunnel')).toBeUndefined()
  })

  it('never restores a ready URL from a poll that completes during Stop', async () => {
    vi.useFakeTimers()
    const view = await render(ready)
    const stop = deferred<CloudTunnelStatus>()
    const duringStop = deferred<CloudTunnelStatus>()
    view.stops.push(stop.promise)
    await view.click('Stop tunnel')
    view.statuses.push(duringStop.promise)
    await act(async () => { await vi.advanceTimersByTimeAsync(4000) })
    await act(async () => duringStop.resolve(ready))
    expect(view.container.querySelector('input')).toBeNull()
    const afterStop = deferred<CloudTunnelStatus>()
    view.statuses.push(afterStop.promise)
    await act(async () => stop.resolve(stopped))
    expect(view.container.querySelector('input')).toBeNull()
    await act(async () => afterStop.resolve(stopped))
    expect(view.button('Copy webhook URL')).toBeUndefined()
    expect(view.button('Start development tunnel')?.disabled).toBe(false)
  })

  it('keeps a failed Start actionable and clears its error on an explicit retry', async () => {
    const view = await render(stopped)
    const start = deferred<CloudTunnelStatus>()
    view.starts.push(start.promise)
    await view.click('Start development tunnel')
    await act(async () => start.reject(new Error('Quick Tunnel worker unavailable')))
    expect(view.container.textContent).toContain('Quick Tunnel worker unavailable')
    expect(view.button('Start development tunnel')?.disabled).toBe(false)
    expect(view.button('Copy webhook URL')).toBeUndefined()
    await view.click('Start development tunnel')
    expect(view.container.querySelector('input')?.value).toBe(ready.webhook_url)
    expect(view.container.textContent).not.toContain('Quick Tunnel worker unavailable')
  })

  it.each(['start', 'stop'] as const)('reconciles a lost %s response once status confirms the action succeeded', async (action) => {
    const view = await render(action === 'start' ? stopped : ready)
    const response = deferred<CloudTunnelStatus>()
    const status = deferred<CloudTunnelStatus>()
    ;(action === 'start' ? view.starts : view.stops).push(response.promise)
    await view.click(action === 'start' ? 'Start development tunnel' : 'Stop tunnel')
    view.statuses.push(status.promise)
    await act(async () => response.reject(new Error('Operation response timed out')))
    expect(view.container.textContent).toContain('Operation response timed out')
    await act(async () => status.resolve(action === 'start' ? ready : stopped))
    expect(view.container.textContent).not.toContain('Operation response timed out')
    expect(view.container.querySelector('[role="alert"]')).toBeNull()
    expect(view.container.querySelector('input')?.value ?? null).toBe(action === 'start' ? ready.webhook_url : null)
  })

  it('preserves a backend failure after a lost Start response is reconciled', async () => {
    const view = await render(stopped)
    const response = deferred<CloudTunnelStatus>()
    view.starts.push(response.promise)
    await view.click('Start development tunnel')
    view.setBackend({ ...stopped, enabled: true, status: 'failed', error: 'Tunnel provider is unavailable' })
    await act(async () => response.reject(new Error('Operation response timed out')))
    expect(view.container.textContent).not.toContain('Operation response timed out')
    expect(view.container.textContent).toContain('Tunnel provider is unavailable')
    expect(view.button('Retry development tunnel')?.disabled).toBe(false)
  })

  it('polls until the tunnel is ready and ignores a mutation result after unmount', async () => {
    vi.useFakeTimers()
    const view = await render({ ...stopped, enabled: true, status: 'starting' })
    view.setBackend(ready)
    await act(async () => { await vi.advanceTimersByTimeAsync(4000) })
    expect(view.container.querySelector('input')?.value).toBe(ready.webhook_url)
    const stop = deferred<CloudTunnelStatus>()
    view.stops.push(stop.promise)
    await view.click('Stop tunnel')
    view.unmount()
    const before = view.trigger.mock.calls.length
    await act(async () => stop.resolve(stopped))
    expect(view.trigger.mock.calls).toHaveLength(before)
  })
})
