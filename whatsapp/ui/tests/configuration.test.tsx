// @vitest-environment jsdom

import type { Host } from '@iii-dev/console-ui'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { RuntimeStatus } from '../src/configuration'
import { DEFAULTS, type ConfigStatus } from '../src/model'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children, variant: _variant, ...props }: ComponentProps<'button'> & { variant?: string }) => <button {...props}>{children}</button>,
  StatusPanel: ({ headline, detail, action }: { headline: ReactNode; detail?: ReactNode; action?: ReactNode }) => <section>{headline}{detail}{action}</section>,
}))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const applied: ConfigStatus = {
  id: 'test-whatsapp', source: 'central', last_outcome: 'applied', last_error: null,
  rejected_reloads: 0, restart_required_fields: [], active: { ...DEFAULTS },
}
const overrides = { 'cloud.access_token': 'WHATSAPP_CLOUD_ACCESS_TOKEN', 'cloud.enabled': 'WHATSAPP_CLOUD_ENABLED' }
const cleanups: Array<() => void> = []

async function render(initial: ConfigStatus) {
  let backend = initial
  const trigger = vi.fn(async () => backend)
  const host = { iii: { trigger, on: () => () => {}, registerTrigger: () => () => {}, browserId: 'test' } } as unknown as Host
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  await act(async () => root.render(<RuntimeStatus host={host} quietWhenApplied />))
  cleanups.push(() => { act(() => root.unmount()); container.remove() })
  return { container, trigger, setBackend(next: ConfigStatus) { backend = next } }
}

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
  vi.useRealTimers()
})

describe('configuration environment overrides', () => {
  it('keeps active overrides visible in the quiet workspace status', async () => {
    const view = await render({ ...applied, environment_overrides: overrides })
    expect(view.container.textContent).toContain('Cloud API uses environment overrides')
    expect(view.container.textContent).toContain('Saved changes do not affect these fields')
    expect(view.container.textContent).toContain('restart the worker')
    expect(view.container.querySelector('summary')?.textContent).toBe('View overridden fields (2)')
    expect(view.container.textContent).toContain('Access token: WHATSAPP_CLOUD_ACCESS_TOKEN')
    expect(view.container.textContent).toContain('Enable Cloud API: WHATSAPP_CLOUD_ENABLED')
    expect(view.trigger).toHaveBeenCalledWith('whatsapp::config-status')
  })

  it.each([undefined, {}])('keeps quiet status empty when there are no reported overrides: %s', async (environment_overrides) => {
    const view = await render({ ...applied, environment_overrides })
    expect(view.container.textContent).toBe('')
  })

  it.each([
    { ...applied, last_outcome: 'rejected' as const, rejected_reloads: 1 },
    { ...applied, last_outcome: 'restart_required' as const, restart_required_fields: ['data_dir'] },
    { ...applied, source: 'local' as const },
  ])('preserves the runtime warning alongside overrides: $last_outcome/$source', async (status) => {
    const view = await render({ ...status, environment_overrides: overrides })
    expect(view.container.querySelectorAll('section')).toHaveLength(2)
    expect(view.container.textContent).toContain('Cloud API uses environment overrides')
    const warning = status.source === 'local' ? 'Local configuration is active'
      : status.last_outcome === 'rejected' ? 'Latest configuration rejected' : 'Some changes need a worker restart'
    expect(view.container.textContent).toContain(warning)
  })

  it('removes the override notice when a restarted worker reports none', async () => {
    vi.useFakeTimers()
    const view = await render({ ...applied, environment_overrides: overrides })
    view.setBackend(applied)
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(view.container.textContent).toBe('')
  })
})
