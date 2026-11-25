// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProviderListEntry } from '@/lib/models-catalog'
import { resetProviderInstalls } from '@/lib/provider-installs'
import type { RegistryWorker } from '@/lib/workers-registry'
import { AddProviderPanel, AUTO_CONFIGURE_TIMEOUT_MS } from './AddProviderPanel'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

// A live backend whose compose::add the test controls; the provider then
// registers with the router when the test says so.
const { composeCall } = vi.hoisted(() => ({
  composeCall: vi.fn<() => Promise<unknown>>(),
}))
vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtxOptional: () => ({
    backend: { id: 'real' },
    presentProviders: [],
  }),
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: composeCall }),
}))
vi.mock('@/pages/Workers/api/workers', () => ({
  fetchEngineWorkersList: async () => ({ workers: [] }),
}))
vi.mock('@/hooks/use-workers-changed', () => ({
  useWorkersChanged: () => undefined,
}))

function worker(name: string): RegistryWorker {
  return {
    name,
    description: `${name} worker`,
    version: '1.0.0',
    tags: ['provider'],
    totalDownloads: 1,
    authorName: 'iii',
    authorVerified: true,
  }
}
const REGISTRY = [
  worker('provider-zai'),
  worker('provider-xai'),
  worker('provider-elevenlabs'),
]
const ZAI: ProviderListEntry = {
  id: 'zai',
  display_name: 'Z.ai',
  supports_model_listing: true,
  configured: false,
  available: true,
}

let container: HTMLDivElement
let root: Root
const onConfigureProvider = vi.fn()
const onBusyChange = vi.fn()

function render(providers: ProviderListEntry[]) {
  act(() =>
    root.render(
      <AddProviderPanel
        registryWorkers={REGISTRY}
        installedWorkerNames={[]}
        providers={providers}
        onConfigureProvider={onConfigureProvider}
        onBusyChange={onBusyChange}
      />,
    ),
  )
}
const button = (label: string) =>
  container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)

/** An install that lands only when the test calls the returned function. */
function holdInstall(): () => Promise<void> {
  let finish: () => void = () => undefined
  composeCall.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = () => resolve({})
      }),
  )
  return async () => {
    await act(async () => finish())
  }
}

beforeEach(() => {
  vi.useFakeTimers()
  onConfigureProvider.mockReset()
  onBusyChange.mockReset()
  composeCall.mockReset()
  composeCall.mockImplementation(async () => ({}))
  resetProviderInstalls()
  container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
})
afterEach(() => {
  act(() => root.unmount())
  container.remove()
  vi.useRealTimers()
})

async function clickAdd(label: string) {
  const add = button(label)
  if (!add) throw new Error(`${label} missing`)
  add.focus()
  await act(async () => {
    add.click()
  })
  return add
}

describe('AddProviderPanel add flow', () => {
  it('keeps the focus on the row being added, so the list never jumps', async () => {
    holdInstall()
    render([])
    const add = await clickAdd('Add Z.ai')
    // The same button, now pending, still holds the focus: a menu's focus
    // trap had sent it to the last button, scrolling the row off screen.
    expect(button('Adding Z.ai')).toBe(add)
    expect(document.activeElement).toBe(add)
    expect(add.getAttribute('aria-busy')).toBe('true')
    // Everything else waits meanwhile.
    expect(onBusyChange).toHaveBeenLastCalledWith(true)
    expect(button('Add xAI')?.getAttribute('aria-disabled')).toBe('true')
  })

  it('opens the provider settings the moment it registers', async () => {
    render([])
    await clickAdd('Add Z.ai')
    // Installed, waiting for the provider to register.
    expect(button('Starting Z.ai')?.getAttribute('aria-busy')).toBe('true')
    expect(onConfigureProvider).not.toHaveBeenCalled()
    render([ZAI])
    expect(onConfigureProvider).toHaveBeenCalledWith('zai')
    expect(onBusyChange).toHaveBeenLastCalledWith(false)
  })

  it('lets go after the timeout and offers Configure instead', async () => {
    render([])
    await clickAdd('Add Z.ai')
    act(() => {
      vi.advanceTimersByTime(AUTO_CONFIGURE_TIMEOUT_MS)
    })
    expect(onBusyChange).toHaveBeenLastCalledWith(false)
    expect(button('Add xAI')?.getAttribute('aria-disabled')).toBeNull()
    // Installed but not registered yet: Configure, not an endless spinner.
    const configure = button('Configure Z.ai')
    expect(configure?.getAttribute('aria-busy')).toBeNull()
    expect(configure?.getAttribute('aria-disabled')).toBeNull()
    act(() => configure?.click())
    expect(onConfigureProvider).toHaveBeenCalledWith('zai')
    // Registering after the wait no longer navigates by itself.
    onConfigureProvider.mockReset()
    render([ZAI])
    expect(onConfigureProvider).not.toHaveBeenCalled()
    expect(button('Configure Z.ai')).not.toBeNull()
  })

  it('keeps an install running across a remount, never starting it twice', async () => {
    const finish = holdInstall()
    render([])
    await clickAdd('Add Z.ai')
    expect(composeCall).toHaveBeenCalledTimes(1)
    // The panel goes away (a cleared search) and comes back.
    act(() => root.unmount())
    root = createRoot(container)
    render([])
    const pending = button('Adding Z.ai')
    expect(pending?.getAttribute('aria-busy')).toBe('true')
    await act(async () => {
      pending?.click()
    })
    expect(composeCall).toHaveBeenCalledTimes(1)
    await finish()
    expect(button('Configure Z.ai')).not.toBeNull()
  })
})
