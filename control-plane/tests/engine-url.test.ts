import { registerWorker } from 'iii-sdk'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { resolveEngineUrl } from '../src/engine-url.js'
import { registerControlPlane } from '../src/worker.js'

vi.mock('iii-sdk', () => ({ registerWorker: vi.fn() }))
vi.mock('../src/worker.js', () => ({ registerControlPlane: vi.fn() }))

describe('resolveEngineUrl', () => {
  it.each([
    ['ws://0.0.0.0:49334', 'ws://127.0.0.1:49334'],
    ['ws://0.0.0.0:80/engine?listen=0.0.0.0', 'ws://127.0.0.1:80/engine?listen=0.0.0.0'],
    ['WSS://user:pass@0.0.0.0:443/engine?b=2&a=1', 'WSS://user:pass@127.0.0.1:443/engine?b=2&a=1'],
    ['ws://0.0.0.0:49334 ', 'ws://127.0.0.1:49334 '],
    ['  ws://0.0.0.0 ', '  ws://127.0.0.1 '],
    ['ws://0.0.0.0:49334\\engine?listen=0.0.0.0', 'ws://127.0.0.1:49334\\engine?listen=0.0.0.0'],
  ])('rewrites only the wildcard host in %s', (configured, expected) => {
    expect(resolveEngineUrl(configured)).toBe(expected)
  })

  it.each([
    'wss://engine.example:443/0.0.0.0?other=ws://0.0.0.0',
    'ws://127.0.0.1:49334',
    'ws://localhost:49334',
    'ws://[::]:49334',
    'ws://0.0.0.0.example:49334',
    'ws://0.0.0.0:secret@engine.example:49334',
    'ws://0:49334',
    'http://0.0.0.0:49334',
    'not a URL',
    'ws://0.0.0.0:bad-port',
    '',
    undefined,
  ])('leaves %s unchanged', (configured) => {
    expect(resolveEngineUrl(configured)).toBe(configured)
  })
})

describe('control-plane startup', () => {
  const signals = ['SIGINT', 'SIGTERM'] as const

  afterEach(() => {
    vi.unstubAllEnvs()
    vi.clearAllMocks()
  })

  it.each([
    ['ws://0.0.0.0:49334', 'wss://other.example', 'ws://127.0.0.1:49334'],
    ['ws://0.0.0.0:49334 ', 'wss://other.example', 'ws://127.0.0.1:49334 '],
    ['wss://engine.example', 'ws://0.0.0.0:49134', 'wss://engine.example'],
    [undefined, 'ws://0.0.0.0:49134', 'ws://127.0.0.1:49134'],
    [undefined, undefined, undefined],
  ])('passes the selected URL to the SDK (III_URL=%s, III_ENGINE_URL=%s)', async (primary, fallback, expected) => {
    vi.stubEnv('III_URL', primary)
    vi.stubEnv('III_ENGINE_URL', fallback)
    vi.resetModules()

    const existing = signals.map((signal) => new Set(process.listeners(signal)))
    try {
      await import('../src/index.js')
      expect(registerWorker).toHaveBeenCalledExactlyOnceWith(expected, {
        workerName: 'control-plane',
        workerDescription: expect.any(String),
      })
      expect(registerControlPlane).toHaveBeenCalledOnce()
    } finally {
      for (const [index, signal] of signals.entries()) {
        for (const listener of process.listeners(signal)) {
          if (!existing[index].has(listener)) process.off(signal, listener)
        }
      }
    }
  })
})
