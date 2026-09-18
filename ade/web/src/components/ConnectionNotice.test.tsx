// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient, type IIIConnectionState } from '@/lib/iii-client'
import {
  CONNECTION_NOTICE_TIMEOUT_MS,
  ConnectionNotice,
} from './ConnectionNotice'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/backend', () => ({ getDefaultBackend: () => ({ id: 'real' }) }))

let root: Root
let host: HTMLDivElement
let changeConnection: (state: IIIConnectionState) => void
const off = vi.fn()

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.useFakeTimers()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  Object.defineProperty(navigator, 'onLine', {
    configurable: true,
    value: true,
  })
  vi.clearAllMocks()
  vi.mocked(getIiiClient).mockResolvedValue({
    addConnectionStateListener: (listener: typeof changeConnection) => {
      changeConnection = listener
      listener('connecting')
      return off
    },
  } as never)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

async function mount() {
  await act(async () => root.render(<ConnectionNotice />))
}

describe('connection feedback', () => {
  it('shows bootstrap rejection instead of leaving an empty surface', async () => {
    vi.mocked(getIiiClient).mockRejectedValue(new Error('runtime unavailable'))
    await mount()
    expect(host.querySelector('[role="alert"]')?.textContent).toContain(
      'Unable to connect',
    )
    expect(host.querySelector('button')?.textContent).toBe('Reload app')
  })

  it('bounds the wait even across repeated SDK reconnect attempts', async () => {
    await mount()
    expect(host.textContent).toContain('Connecting…')
    await act(async () =>
      vi.advanceTimersByTime(CONNECTION_NOTICE_TIMEOUT_MS / 2),
    )
    await act(async () => changeConnection('reconnecting'))
    await act(async () =>
      vi.advanceTimersByTime(CONNECTION_NOTICE_TIMEOUT_MS / 2),
    )
    expect(host.textContent).toContain('Unable to connect')
    await act(async () => changeConnection('connected'))
    expect(host.textContent).toBe('')
    await act(async () => changeConnection('reconnecting'))
    expect(host.textContent).toContain('Connection lost. Reconnecting…')
    expect(host.textContent).not.toContain('Unable to connect')
  })

  it('shows offline even before the socket detects a break and clears on recovery', async () => {
    await mount()
    await act(async () => changeConnection('connected'))
    await act(async () => {
      Object.defineProperty(navigator, 'onLine', {
        configurable: true,
        value: false,
      })
      window.dispatchEvent(new Event('offline'))
    })
    expect(host.textContent).toContain('You are offline')
    await act(async () => {
      Object.defineProperty(navigator, 'onLine', {
        configurable: true,
        value: true,
      })
      window.dispatchEvent(new Event('online'))
    })
    expect(host.textContent).toBe('')
  })

  it('does not probe the engine for the demo backend and releases its listener', async () => {
    await act(async () => root.render(<ConnectionNotice enabled={false} />))
    expect(host.textContent).toBe('')
    expect(getIiiClient).not.toHaveBeenCalled()
    await mount()
    await act(async () => root.render(<ConnectionNotice enabled={false} />))
    expect(off).toHaveBeenCalledOnce()
  })
})
