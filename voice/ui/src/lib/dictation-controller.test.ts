import type { Host } from '@iii-dev/console-ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { type CaptureHandle, startCapture } from './capture'
import { DictationController } from './dictation'
import type { TranscriptEvent } from './types'

vi.mock('./capture', () => ({ startCapture: vi.fn() }))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

function setup() {
  const captureStop = vi.fn().mockResolvedValue(undefined)
  vi.mocked(startCapture).mockResolvedValue({ stop: captureStop })
  let nextSession = 0
  const start = vi.fn(async () => ({ session_id: `s${++nextSession}`, model: 'test', sample_rate: 16000 }))
  const stop = vi.fn(async (_req?: unknown) => ({ session_id: `s${nextSession}`, text: 'Kept text.', segments: [], duration_secs: 1 }))
  const push = vi.fn().mockResolvedValue({ accepted: true })
  const handlers: Array<(event: TranscriptEvent) => void> = []
  const off = vi.fn()
  const host = { iii: {
    browserId: 'test-browser',
    on: vi.fn((_id: string, handler: (event: TranscriptEvent) => void) => {
      handlers.push(handler)
      return off
    }),
    trigger: vi.fn((id: string, req: unknown) => {
      if (id.endsWith('::start')) return start()
      if (id.endsWith('::stop')) return stop(req)
      if (id.endsWith('::push')) return push(req)
      throw new Error(`Unexpected function: ${id}`)
    }),
  } } as unknown as Host
  const controller = new DictationController(host)
  const emit = (patch: Partial<TranscriptEvent> = {}, handler = handlers.length - 1) => handlers[handler]({
    session_id: `s${nextSession}`, seq: 1, kind: 'partial', text: 'live', segment: 0, timestamp_ms: 0, ...patch,
  })
  return { controller, start, stop, push, captureStop, handlers, off, emit }
}

beforeEach(() => {
  vi.resetAllMocks()
  vi.stubGlobal('window', globalThis)
})
afterEach(() => vi.unstubAllGlobals())

describe('DictationController shutdown shared by Voice and chat', () => {
  it('honors Stop before the worker has returned a session', async () => {
    const rig = setup()
    const response = deferred<Awaited<ReturnType<typeof rig.start>>>()
    rig.start.mockReturnValueOnce(response.promise)
    const starting = rig.controller.start()
    const stopping = rig.controller.stop()
    expect(rig.controller.getState().status).toBe('stopping')
    await stopping
    await starting
    expect(rig.controller.getState().status).toBe('idle')
    response.resolve({ session_id: 'opening', model: 'test', sample_rate: 16000 })
    await vi.waitFor(() => expect(rig.stop).toHaveBeenCalledOnce())
    expect(startCapture).not.toHaveBeenCalled()
    expect(rig.stop).toHaveBeenCalledWith({ session_id: 'opening' })
    expect(rig.controller.getState().status).toBe('idle')
  })

  it('releases a microphone that finishes opening after Stop', async () => {
    const rig = setup()
    const microphone = deferred<CaptureHandle>()
    const requested = deferred<void>()
    vi.mocked(startCapture).mockImplementationOnce(() => {
      requested.resolve()
      return microphone.promise
    })
    const starting = rig.controller.start()
    await requested.promise
    const stopping = rig.controller.stop()
    expect(rig.controller.getState().status).toBe('stopping')
    expect(await stopping).toBe('Kept text.')
    await starting
    expect(rig.stop).toHaveBeenCalledOnce()
    expect(rig.captureStop).not.toHaveBeenCalled()
    microphone.resolve({ stop: rig.captureStop })
    await vi.waitFor(() => expect(rig.captureStop).toHaveBeenCalledOnce())
    expect(rig.controller.listening).toBe(false)
    expect(rig.controller.getState().status).toBe('idle')
  })

  it('keeps finishing rather than reopening chat listening when final text arrives', async () => {
    const rig = setup()
    const response = deferred<Awaited<ReturnType<typeof rig.stop>>>()
    const requested = deferred<void>()
    rig.stop.mockImplementationOnce(() => { requested.resolve(); return response.promise })
    await rig.controller.start()
    const stopping = rig.controller.stop()
    await requested.promise
    rig.emit({ kind: 'final', text: 'Last sentence.' })
    expect(rig.controller.getState().committed).toEqual(['Last sentence.'])
    expect(rig.controller.getState().status).toBe('stopping')
    expect(rig.controller.listening).toBe(false)
    rig.emit({ session_id: 'another-session', seq: 20, text: 'unrelated' })
    expect(rig.controller.getState().partial).toBe('')
    await rig.controller.start()
    expect(rig.start).toHaveBeenCalledOnce()
    response.resolve({ session_id: 's1', text: 'Last sentence.', segments: [], duration_secs: 1 })
    await stopping
    expect(rig.controller.getState().status).toBe('idle')
    expect(rig.off).toHaveBeenCalledOnce()
  })

  it('shares one Stop request and keeps late callbacks from reviving a stopped session', async () => {
    const rig = setup()
    await rig.controller.start()
    const first = rig.controller.stop()
    const second = rig.controller.stop()
    expect(second).toBe(first)
    await first
    rig.emit({ seq: 50, kind: 'partial', text: 'late' })
    expect(rig.controller.getState().status).toBe('idle')
    expect(rig.controller.getState().partial).toBe('')
    expect(rig.stop).toHaveBeenCalledOnce()
    await rig.controller.start()
    rig.emit({ session_id: 's1', seq: 60, text: 'old session' }, 0)
    expect(rig.controller.getState().partial).toBe('')
    rig.emit({ text: 'new session' })
    expect(rig.controller.getState().partial).toBe('new session')
    await rig.controller.cancel()
  })

  it('honors Cancel while permission for the microphone is pending', async () => {
    const rig = setup()
    const microphone = deferred<CaptureHandle>()
    const requested = deferred<void>()
    vi.mocked(startCapture).mockImplementationOnce(() => { requested.resolve(); return microphone.promise })
    const starting = rig.controller.start()
    await requested.promise
    await rig.controller.cancel()
    await starting
    expect(rig.captureStop).not.toHaveBeenCalled()
    expect(rig.controller.getState().status).toBe('idle')
    microphone.resolve({ stop: rig.captureStop })
    await vi.waitFor(() => expect(rig.captureStop).toHaveBeenCalledOnce())
    expect(rig.stop).toHaveBeenCalledWith({ session_id: 's1', discard: true })
    expect(rig.controller.getState()).toMatchObject({ status: 'idle', partial: '', committed: [] })
  })

  it('a late permission result cannot overwrite a newly started session', async () => {
    const rig = setup()
    const microphone = deferred<CaptureHandle>()
    const requested = deferred<void>()
    vi.mocked(startCapture).mockImplementationOnce(() => { requested.resolve(); return microphone.promise })
    const starting = rig.controller.start()
    await requested.promise
    await rig.controller.cancel()
    await starting
    await rig.controller.start()
    const oldStop = vi.fn().mockResolvedValue(undefined)
    microphone.resolve({ stop: oldStop })
    await vi.waitFor(() => expect(oldStop).toHaveBeenCalledOnce())
    expect(rig.captureStop).not.toHaveBeenCalled()
    expect(rig.controller.getState().status).toBe('listening')
    await rig.controller.cancel()
  })

  it('a late permission rejection cannot clear a newer session', async () => {
    const rig = setup()
    const microphone = deferred<CaptureHandle>()
    const requested = deferred<void>()
    vi.mocked(startCapture).mockImplementationOnce(() => { requested.resolve(); return microphone.promise })
    void rig.controller.start()
    await requested.promise
    await rig.controller.cancel()
    await rig.controller.start()
    microphone.reject(new Error('old permission denied'))
    await Promise.resolve(); await Promise.resolve()
    expect(rig.controller.getState().status).toBe('listening')
    expect(rig.stop).toHaveBeenCalledTimes(1)
    await rig.controller.cancel()
  })

  it('a late server start is discarded without cancelling a newer session', async () => {
    const rig = setup()
    const response = deferred<Awaited<ReturnType<typeof rig.start>>>()
    rig.start.mockReturnValueOnce(response.promise)
    const first = rig.controller.start()
    await rig.controller.cancel()
    await first
    await rig.controller.start()
    response.resolve({ session_id: 'late-old', model: 'test', sample_rate: 16000 })
    await vi.waitFor(() => expect(rig.stop).toHaveBeenCalledWith({ session_id: 'late-old', discard: true }))
    expect(rig.controller.getState().status).toBe('listening')
    await rig.controller.cancel()
  })

  it('Cancel escalates a pending Stop before the remote request', async () => {
    const rig = setup()
    const capture = deferred<void>()
    rig.captureStop.mockReturnValueOnce(capture.promise)
    await rig.controller.start()
    rig.emit({ kind: 'final', text: 'Do not keep this.' })
    const stopping = rig.controller.stop()
    const cancelling = rig.controller.cancel()
    expect(rig.controller.stop()).toBe(stopping)
    capture.resolve()
    expect(await stopping).toBe('')
    await cancelling
    expect(rig.stop).toHaveBeenCalledExactlyOnceWith({ session_id: 's1', discard: true })
    expect(rig.controller.getState().committed).toEqual([])
  })

  it('Cancel after Stop is sent still suppresses its result and clears the transcript', async () => {
    const rig = setup()
    const response = deferred<Awaited<ReturnType<typeof rig.stop>>>()
    const requested = deferred<void>()
    rig.stop.mockImplementationOnce(() => { requested.resolve(); return response.promise })
    await rig.controller.start()
    rig.emit({ kind: 'final', text: 'Discard me.' })
    const stopping = rig.controller.stop()
    await requested.promise
    const cancelling = rig.controller.cancel()
    response.resolve({ session_id: 's1', text: 'Discard me.', segments: [], duration_secs: 1 })
    expect(await stopping).toBe('')
    await cancelling
    expect(rig.controller.getState().committed).toEqual([])
    expect(rig.stop).toHaveBeenCalledOnce()
  })

  it('releases local capture and accepts a new start after the worker closes the session', async () => {
    const rig = setup()
    await rig.controller.start()
    rig.emit({ kind: 'closed', text: 'Done.' })
    expect(rig.captureStop).toHaveBeenCalledOnce()
    expect(rig.controller.getState().status).toBe('idle')
    rig.emit({ seq: 2, text: 'late' })
    expect(rig.controller.getState().status).toBe('idle')
    await rig.controller.start()
    expect(rig.start).toHaveBeenCalledTimes(2)
    await rig.controller.cancel()
  })

  it('cleans up and permits retry after a failed Stop request', async () => {
    const rig = setup()
    await rig.controller.start()
    rig.stop.mockRejectedValueOnce(new Error('connection lost'))
    await rig.controller.stop()
    expect(rig.captureStop).toHaveBeenCalledOnce()
    expect(rig.off).toHaveBeenCalledOnce()
    expect(rig.controller.getState()).toMatchObject({ status: 'error', error: 'connection lost', partial: '' })
    rig.emit({ seq: 99, text: 'late' })
    expect(rig.controller.getState().status).toBe('error')
    await rig.controller.start()
    expect(rig.controller.getState().status).toBe('listening')
    await rig.controller.cancel()
  })
})
