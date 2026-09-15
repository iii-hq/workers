import type { Host } from '@iii-dev/console-ui'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  capture: vi.fn(),
  start: vi.fn(),
  stop: vi.fn(),
}))
vi.mock('./capture', () => ({ startCapture: mocks.capture }))
vi.mock('./client', () => ({ dictationStart: mocks.start, dictationStop: mocks.stop }))

import { DictationController } from './dictation'
import type { TranscriptEvent } from './types'

function setup(supported = true) {
  const release = vi.fn()
  const keepAwake = vi.fn(() => release)
  const captureStop = vi.fn(async () => undefined)
  let event!: (value: TranscriptEvent) => void
  mocks.start.mockResolvedValue({ session_id: 'dictation' })
  mocks.capture.mockResolvedValue({ stop: captureStop })
  mocks.stop.mockResolvedValue({ text: 'hello' })
  const host = {
    iii: {
      browserId: 'test',
      on: vi.fn((_id, handler) => { event = handler; return vi.fn() }),
    },
    ...(supported ? { screen: { keepAwake } } : {}),
  } as unknown as Host
  return {
    controller: new DictationController(host), keepAwake, release, captureStop,
    emit(kind: 'closed' | 'error') {
      event({ session_id: 'dictation', seq: 1, kind, text: '', segment: 0, timestamp_ms: 1 })
    },
  }
}

beforeEach(() => { vi.resetAllMocks() })

describe('dictation screen lease', () => {
  it('holds through start/listening/final transcription and releases after stop', async () => {
    const { controller, keepAwake, release } = setup()
    const start = controller.start()
    expect(keepAwake).toHaveBeenCalledTimes(1)
    await start
    expect(release).not.toHaveBeenCalled()
    let finish!: (value: { text: string }) => void
    mocks.stop.mockReturnValue(new Promise((resolve) => { finish = resolve }))
    const stopped = controller.stop()
    expect(release).not.toHaveBeenCalled()
    finish({ text: 'hello' })
    await expect(stopped).resolves.toBe('hello')
    expect(keepAwake).toHaveBeenCalledTimes(1)
    expect(release).toHaveBeenCalledTimes(1)
  })

  it('releases on cancellation', async () => {
    const { controller, release, captureStop } = setup()
    await controller.start()
    await controller.cancel()
    expect(captureStop).toHaveBeenCalledTimes(1)
    expect(release).toHaveBeenCalledTimes(1)
  })

  it.each(['start', 'capture', 'stop'] as const)('releases when %s fails', async (phase) => {
    const { controller, release } = setup()
    mocks[phase].mockRejectedValue(new Error('failed'))
    await controller.start()
    if (phase === 'stop') await controller.stop()
    expect(controller.getState().status).toBe('error')
    expect(release).toHaveBeenCalledTimes(1)
  })

  it.each(['closed', 'error'] as const)('releases on a terminal %s event', async (kind) => {
    const { controller, release, emit } = setup()
    await controller.start()
    emit(kind)
    expect(release).toHaveBeenCalledTimes(1)
    await controller.cancel()
    expect(release).toHaveBeenCalledTimes(1)
  })

  it('still works on older consoles without the optional screen API', async () => {
    const { controller, keepAwake } = setup(false)
    await controller.start()
    await expect(controller.stop()).resolves.toBe('hello')
    expect(keepAwake).not.toHaveBeenCalled()
  })
})
