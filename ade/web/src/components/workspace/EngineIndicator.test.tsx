// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { copyTextToClipboard } from '@/lib/clipboard'
import { getIiiClient, type IIIConnectionState } from '@/lib/iii-client'
import {
  connectionStatusLabel,
  EngineIndicator,
  engineAddress,
  engineHostLabel,
} from './EngineIndicator'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/clipboard', () => ({
  copyTextToClipboard: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('@/lib/backend', () => ({ getDefaultBackend: () => ({ id: 'real' }) }))

describe('engineHostLabel', () => {
  it('names an engine by host and port, without scheme or path', () => {
    expect(engineHostLabel('ws://127.0.0.1:49134')).toBe('127.0.0.1:49134')
    expect(engineHostLabel('wss://iii.example.com:1234/path')).toBe(
      'iii.example.com:1234',
    )
    expect(engineHostLabel('wss://iii.example.com/ws')).toBe('iii.example.com')
    expect(engineHostLabel('not a url')).toBe('not a url')
  })

  it('prefers the engine the ADE worker reported over the page proxy', () => {
    expect(
      engineAddress({
        wsUrl: 'ws://localhost:5180/ws',
        engineUrl: 'ws://127.0.0.1:49134',
      }),
    ).toBe('ws://127.0.0.1:49134')
    expect(engineAddress({ wsUrl: 'ws://localhost:5180/ws' })).toBe(
      'ws://localhost:5180/ws',
    )
  })

  it('spells every connection state', () => {
    expect(connectionStatusLabel('connected')).toBe('Connected')
    expect(connectionStatusLabel('connecting')).toBe('Connecting…')
    expect(connectionStatusLabel('reconnecting')).toBe('Reconnecting…')
    expect(connectionStatusLabel('failed')).toBe('Unable to connect')
  })
})

describe('EngineIndicator', () => {
  let root: Root
  let host: HTMLDivElement
  let changeConnection: (state: IIIConnectionState) => void
  const off = vi.fn()

  beforeEach(() => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    host = document.createElement('div')
    document.body.append(host)
    root = createRoot(host)
    vi.clearAllMocks()
    vi.mocked(getIiiClient).mockResolvedValue({
      connection: {
        wsUrl: 'ws://localhost:5180/ws',
        namespace: 'project-a',
        engineUrl: 'ws://127.0.0.1:49134',
      },
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
    vi.unstubAllGlobals()
  })

  it('names the engine and its state in the header pill', async () => {
    await act(async () => {
      root.render(<EngineIndicator />)
    })
    const trigger = host.querySelector('button')
    expect(trigger?.textContent).toContain('127.0.0.1:49134')
    expect(trigger?.getAttribute('aria-label')).toBe(
      'Engine: 127.0.0.1:49134, Connecting…',
    )

    await act(async () => changeConnection('connected'))
    expect(trigger?.getAttribute('aria-label')).toBe(
      'Engine: 127.0.0.1:49134, Connected',
    )
    expect(host.querySelector('.bg-ok')).not.toBeNull()
  })

  // Review on #1760: every value in the popover is something to paste.
  it('puts a copy button beside each value in the popover', async () => {
    await act(async () => {
      root.render(<EngineIndicator />)
    })
    await act(async () => host.querySelector('button')?.click())
    const copy = (name: string) =>
      document.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)
    expect(copy('Copy the engine URL')).not.toBeNull()
    expect(copy('Copy the namespace')).not.toBeNull()
    expect(copy("Copy the page's WebSocket URL")).not.toBeNull()
    await act(async () => copy('Copy the namespace')?.click())
    expect(copyTextToClipboard).toHaveBeenCalledWith('project-a')
    expect(copy('Copied the namespace')).not.toBeNull()
    await act(async () => copy('Copy the engine URL')?.click())
    expect(copyTextToClipboard).toHaveBeenLastCalledWith('ws://127.0.0.1:49134')
  })

  it('reads out the engine on the phone row', async () => {
    await act(async () => {
      root.render(<EngineIndicator variant="row" />)
    })
    expect(host.textContent).toContain('127.0.0.1:49134')
    expect(host.textContent).toContain('Connecting…')
    expect(host.querySelector('button')).toBeNull()
  })

  it('renders nothing for a backend without an engine', async () => {
    await act(async () => {
      root.render(<EngineIndicator enabled={false} />)
    })
    expect(host.innerHTML).toBe('')
    expect(getIiiClient).not.toHaveBeenCalled()
  })

  it('releases the connection listener on unmount', async () => {
    await act(async () => {
      root.render(<EngineIndicator />)
    })
    await act(async () => root.unmount())
    expect(off).toHaveBeenCalled()
    root = createRoot(host)
  })
})
