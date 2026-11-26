import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  type Action,
  buildMetadata,
  canSubmit,
  downloadName,
  initialState,
  MAX_INPUT_BYTES,
  type Options,
  RESIZE_FN,
  type ResizeResult,
  reduce,
  resizeStream,
  type State,
  StreamUnavailableError,
  sizeChange,
} from './resize'

const RESULT: ResizeResult = {
  format: 'webp',
  width: 200,
  height: 150,
  source_width: 1600,
  source_height: 1200,
  strategy: 'scale-to-fit',
}

type Payload = { input_channel: File; output_channel: WritableStream<BlobPart>; metadata: unknown }
type Call = (id: string, payload: Payload, options: { timeoutMs?: number }) => Promise<unknown>

function fakeIii(call: Call) {
  const spy = vi.fn(call)
  return { spy, iii: { call: spy as unknown as <T>(...args: Parameters<Call>) => Promise<T> } }
}

const image = () => new File([new Uint8Array([1, 2, 3])], 'photo.jpg', { type: 'image/jpeg' })

// Node's process, typed here: the worker tsconfig loads no Node types.
const node = (
  globalThis as unknown as {
    process: { on(event: string, f: () => void): void; off(event: string, f: () => void): void }
  }
).process

afterEach(() => {
  vi.useRealTimers()
})

describe('resizeStream', () => {
  it('sends the File itself and collects the output bytes in order', async () => {
    const file = image()
    const { spy, iii } = fakeIii(async (_id, payload) => {
      const writer = payload.output_channel.getWriter()
      await writer.write(new Uint8Array([10, 11]))
      await writer.write(new Uint8Array([12]))
      await writer.close()
      return RESULT
    })

    const { result, blob } = await resizeStream(iii, file, { target_width: 200 }, { timeoutMs: 5_000 })

    expect(result).toEqual(RESULT)
    expect(blob.type).toBe('image/webp')
    expect([...new Uint8Array(await blob.arrayBuffer())]).toEqual([10, 11, 12])
    const [id, payload, options] = spy.mock.calls[0]
    expect(id).toBe(RESIZE_FN)
    expect(payload.input_channel).toBe(file)
    expect(payload.metadata).toEqual({ target_width: 200 })
    expect(options).toEqual({ timeoutMs: 5_000 })
  })

  it('rejects with StreamUnavailableError once the grace passes after an abort with no answer', async () => {
    vi.useFakeTimers()
    const { iii } = fakeIii(async (_id, payload) => {
      await payload.output_channel.getWriter().abort('the connection failed')
      return new Promise(() => {})
    })
    let outcome: unknown = 'pending'
    resizeStream(iii, image(), {}, { graceMs: 3_000 }).then(
      () => {
        outcome = 'resolved'
      },
      (err) => {
        outcome = err
      },
    )

    await vi.advanceTimersByTimeAsync(2_999)
    expect(outcome).toBe('pending')
    await vi.advanceTimersByTimeAsync(1)
    expect(outcome).toBeInstanceOf(StreamUnavailableError)
  })

  it("shows the worker's own error when it aborts the output and then answers", async () => {
    vi.useFakeTimers()
    const unhandled = vi.fn()
    node.on('unhandledRejection', unhandled)
    const fault = { code: 'INVALID_ARGS', message: 'input is not a recognized image; send jpeg, png or webp' }
    const { iii } = fakeIii(async (_id, payload) => {
      await payload.output_channel.getWriter().abort(fault.message)
      throw fault
    })

    await expect(resizeStream(iii, image(), {}, { graceMs: 3_000 })).rejects.toBe(fault)
    await vi.advanceTimersByTimeAsync(10_000)
    vi.useRealTimers()
    await new Promise((resolve) => setTimeout(resolve, 0))
    node.off('unhandledRejection', unhandled)
    expect(unhandled).not.toHaveBeenCalled()
  })

  it('passes a call error through when the output was never aborted', async () => {
    const failure = new Error('Invocation timeout after 60000ms')
    const { iii } = fakeIii(async () => {
      throw failure
    })
    await expect(resizeStream(iii, image(), {})).rejects.toBe(failure)
  })
})

const NO_OPTIONS: Options = { width: '', height: '', quality: '' }

describe('buildMetadata', () => {
  it('sends nothing when no option is set', () => {
    expect(buildMetadata(NO_OPTIONS, 'image/jpeg')).toStrictEqual({ metadata: {} })
  })

  it('sends only the set keys, under the names the worker reads', () => {
    expect(buildMetadata({ ...NO_OPTIONS, width: '320' }, 'image/jpeg')).toStrictEqual({
      metadata: { target_width: 320 },
    })
    expect(
      buildMetadata(
        { width: ' 320 ', height: '200', strategy: 'crop-to-fit', format: 'webp', quality: '70' },
        'image/jpeg',
      ),
    ).toStrictEqual({
      metadata: { target_width: 320, target_height: 200, strategy: 'crop-to-fit', output_format: 'webp', quality: 70 },
    })
  })

  it('rejects a box that is not a whole number of pixels', () => {
    for (const bad of ['0', '-4', '1.5', '12px']) {
      expect(buildMetadata({ ...NO_OPTIONS, width: bad, height: bad }, 'image/jpeg')).toStrictEqual({
        errors: {
          width: 'Enter a whole number of pixels, 1 or more.',
          height: 'Enter a whole number of pixels, 1 or more.',
        },
      })
    }
  })

  it('rejects a quality outside 1 to 100', () => {
    for (const bad of ['0', '101', '7.5']) {
      expect(buildMetadata({ ...NO_OPTIONS, quality: bad }, 'image/jpeg')).toStrictEqual({
        errors: { quality: 'Enter a quality from 1 to 100.' },
      })
    }
    expect(buildMetadata({ ...NO_OPTIONS, quality: '100' }, 'image/jpeg')).toStrictEqual({
      metadata: { quality: 100 },
    })
  })

  it('omits quality, valid or not, when the output is PNG', () => {
    expect(buildMetadata({ ...NO_OPTIONS, format: 'png', quality: '70' }, 'image/jpeg')).toStrictEqual({
      metadata: { output_format: 'png' },
    })
    expect(buildMetadata({ ...NO_OPTIONS, quality: '101' }, 'image/png')).toStrictEqual({ metadata: {} })
  })
})

describe('downloadName', () => {
  it('names the file after the source, the output size and the output format', () => {
    expect(downloadName('photo.jpeg', { ...RESULT, format: 'jpeg' })).toBe('photo_200x150.jpg')
    expect(downloadName('scan.final.png', RESULT)).toBe('scan.final_200x150.webp')
    expect(downloadName('README', { ...RESULT, format: 'png' })).toBe('README_200x150.png')
  })
})

describe('sizeChange', () => {
  it('reads smaller as ok, larger as a warning and equal as the same size', () => {
    expect(sizeChange(1000, 180)).toEqual({ text: '-82%', tone: 'ok' })
    expect(sizeChange(1000, 1120)).toEqual({ text: '+12%', tone: 'warn' })
    expect(sizeChange(1000, 1000)).toEqual({ text: 'same size', tone: 'ink' })
    expect(sizeChange(100_000, 100_001)).toEqual({ text: '+<1%', tone: 'warn' })
  })

  it('never reads an image as shrunk by 100%', () => {
    expect(sizeChange(1_848_667, 4_886)).toEqual({ text: '-99%', tone: 'ok' })
  })
})

const file = (name: string, size = 1024) => ({ name, size, type: 'image/png' }) as File
const run = (state: State, ...actions: Action[]) => actions.reduce(reduce, state)
const done = (generation: number): Action => ({
  type: 'settled',
  generation,
  result: RESULT,
  blob: new Blob([new Uint8Array([1])], { type: 'image/webp' }),
})

describe('reduce', () => {
  it('refuses an image over the cap with a notice and no source', () => {
    const state = run(
      initialState,
      { type: 'pick', generation: 1, file: file('small.png') },
      { type: 'pick', generation: 2, file: file('huge.png', MAX_INPUT_BYTES + 1) },
    )
    expect(state.notice).toEqual({ kind: 'too-large', name: 'huge.png', size: MAX_INPUT_BYTES + 1 })
    expect(state.source).toBeNull()
    expect(canSubmit(state)).toBe(false)
    expect(run(state, { type: 'submit', generation: 3 }).run).toEqual({ phase: 'idle' })
  })

  it('keeps the source, and a running call, when a drop carries no file', () => {
    const before = run(
      initialState,
      { type: 'pick', generation: 1, file: file('a.png') },
      { type: 'submit', generation: 2 },
    )
    const state = run(before, { type: 'pick', generation: 3 })
    expect(state.notice).toEqual({ kind: 'no-file' })
    expect(state.source?.file.name).toBe('a.png')
    expect(state.run).toEqual({ phase: 'running' })
    expect(run(state, done(2)).run.phase).toBe('done')
  })

  it('ignores a result that settles after a newer pick, remove or submit', () => {
    const running = run(
      initialState,
      { type: 'pick', generation: 1, file: file('a.png') },
      { type: 'submit', generation: 2 },
    )
    const picked = run(running, { type: 'pick', generation: 3, file: file('b.png') }, done(2))
    expect(picked.run).toEqual({ phase: 'idle' })
    expect(picked.source?.file.name).toBe('b.png')

    const removed = run(running, { type: 'remove', generation: 3 }, done(2))
    expect(removed.run).toEqual({ phase: 'idle' })

    const resubmitted = run(
      running,
      { type: 'settled', generation: 2, error: new Error('boom') },
      { type: 'submit', generation: 3 },
      done(2),
    )
    expect(resubmitted.run).toEqual({ phase: 'running' })
    expect(run(resubmitted, done(3)).run.phase).toBe('done')
  })

  it('ends idle with no source when the image is removed during a call', () => {
    const state = run(
      initialState,
      { type: 'pick', generation: 1, file: file('a.png') },
      { type: 'submit', generation: 2 },
      { type: 'remove', generation: 3 },
    )
    expect(state.source).toBeNull()
    expect(state.run).toEqual({ phase: 'idle' })
    expect(canSubmit(state)).toBe(false)
  })

  it('ignores a submit while a call runs', () => {
    const running = run(
      initialState,
      { type: 'pick', generation: 1, file: file('a.png') },
      { type: 'submit', generation: 2 },
    )
    expect(canSubmit(running)).toBe(false)
    expect(run(running, { type: 'submit', generation: 3 })).toBe(running)
  })

  it('still lets an image the browser cannot preview be resized', () => {
    const state = run(
      initialState,
      { type: 'pick', generation: 1, file: file('a.png') },
      { type: 'previewFailed', target: 'source' },
    )
    expect(state.source?.preview).toBe('failed')
    expect(canSubmit(state)).toBe(true)
  })

  it('clears the notice and the last result on a good pick', () => {
    const state = run(
      initialState,
      { type: 'pick', generation: 1, file: file('a.png') },
      { type: 'submit', generation: 2 },
      done(2),
      { type: 'pick', generation: 3 },
      { type: 'pick', generation: 4, file: file('b.png') },
    )
    expect(state.notice).toBeNull()
    expect(state.run).toEqual({ phase: 'idle' })
    expect(state.source).toEqual({ file: file('b.png'), preview: 'ok' })
  })
})
