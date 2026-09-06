/**
 * Microphone capture: opens a mono `getUserMedia` stream and turns it into
 * ~100ms batches of 16kHz mono PCM16 (`CaptureChunk`), delivered to the
 * caller's `onChunk`. Prefers an `AudioWorklet` — its processor source is
 * generated at runtime from `resampleTo16kMonoInt16.toString()` and loaded
 * from a `Blob` URL, so no extra worklet asset ships — and falls back to a
 * `ScriptProcessorNode` when `AudioWorklet` is unavailable. Wire encoding
 * (base64) and the `voice::dictation::push` calls are the caller's job
 * (see dictation.ts); this module knows nothing about the worker's wire
 * protocol, which keeps it host-independent and easy to unit test.
 */

const WORKLET_NAME = 'voice-capture-processor'
const BATCH_MS = 100

/**
 * Pure PCM conversion: linearly resample `input` (at `inputSampleRate`) to
 * 16kHz mono and quantize to signed 16-bit integers, clamped to
 * [-1, 1] first. Exported for direct unit testing and reused verbatim
 * inside the AudioWorklet via `.toString()` — it must stay self-contained
 * (no closures over outer variables) for that to work.
 */
export function resampleTo16kMonoInt16(input: Float32Array, inputSampleRate: number): Int16Array {
  const outputSampleRate = 16000
  const toInt16 = (sample: number): number => {
    const clamped = Math.max(-1, Math.min(1, sample))
    return clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff
  }
  if (inputSampleRate === outputSampleRate) {
    const out = new Int16Array(input.length)
    for (let i = 0; i < input.length; i++) out[i] = toInt16(input[i])
    return out
  }
  const ratio = inputSampleRate / outputSampleRate
  const outputLength = Math.floor(input.length / ratio)
  const out = new Int16Array(outputLength)
  for (let i = 0; i < outputLength; i++) {
    const srcIndex = i * ratio
    const lo = Math.floor(srcIndex)
    const hi = Math.min(lo + 1, input.length - 1)
    const frac = srcIndex - lo
    out[i] = toInt16(input[lo] + (input[hi] - input[lo]) * frac)
  }
  return out
}

function buildWorkletModuleSource(): string {
  return [
    `const resampleTo16kMonoInt16 = ${resampleTo16kMonoInt16.toString()}`,
    '',
    'class VoiceCaptureProcessor extends AudioWorkletProcessor {',
    '  constructor() {',
    '    super()',
    '    this._chunks = []',
    '    this._bufferedSamples = 0',
    `    this._batchSamples = Math.ceil((sampleRate * ${BATCH_MS}) / 1000)`,
    '    this.port.onmessage = (event) => {',
    '      if (event.data && event.data.flush) {',
    '        this._flush()',
    '        this.port.postMessage({ flushed: true })',
    '      }',
    '    }',
    '  }',
    '  _flush() {',
    '    if (this._bufferedSamples === 0) return',
    '    const merged = new Float32Array(this._bufferedSamples)',
    '    let offset = 0',
    '    for (const chunk of this._chunks) {',
    '      merged.set(chunk, offset)',
    '      offset += chunk.length',
    '    }',
    '    this._chunks = []',
    '    this._bufferedSamples = 0',
    '    const pcm16 = resampleTo16kMonoInt16(merged, sampleRate)',
    '    this.port.postMessage({ pcm16: pcm16.buffer }, [pcm16.buffer])',
    '  }',
    '  process(inputs) {',
    '    const channel = inputs[0] && inputs[0][0]',
    '    if (channel && channel.length) {',
    '      this._chunks.push(channel.slice())',
    '      this._bufferedSamples += channel.length',
    '      if (this._bufferedSamples >= this._batchSamples) this._flush()',
    '    }',
    '    return true',
    '  }',
    '}',
    '',
    `registerProcessor('${WORKLET_NAME}', VoiceCaptureProcessor)`,
    '',
  ].join('\n')
}

function resolveAudioContextCtor(): typeof AudioContext {
  const w = window as unknown as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext }
  const ctor = w.AudioContext ?? w.webkitAudioContext
  if (!ctor) throw new Error('this browser has no Web Audio API')
  return ctor
}

/** A context running at the recognizer's rate lets the browser resample the
    microphone with its own filtered resampler; the worklet then only
    converts to int16. Browsers that refuse the rate fall back to their
    default rate and the worklet's linear resampler. */
function createContextAt16k(Ctor: typeof AudioContext): AudioContext {
  try {
    return new Ctor({ sampleRate: 16000 })
  } catch {
    return new Ctor()
  }
}

function permissionErrorMessage(err: unknown): string {
  if (err instanceof DOMException) {
    if (err.name === 'NotAllowedError' || err.name === 'PermissionDeniedError') {
      return 'microphone access was denied'
    }
    if (err.name === 'NotFoundError' || err.name === 'DevicesNotFoundError') {
      return 'no microphone was found'
    }
  }
  return 'could not access the microphone'
}

export interface CaptureChunk {
  pcm16: Int16Array
}

export interface StartCaptureOptions {
  onChunk: (chunk: CaptureChunk) => void
}

const FLUSH_TIMEOUT_MS = 300

export interface CaptureHandle {
  /** Deliver the audio still buffered, then release the track, disconnect the graph and close the context. */
  stop: () => Promise<void>
}

export async function startCapture(options: StartCaptureOptions): Promise<CaptureHandle> {
  let stream: MediaStream
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    })
  } catch (err) {
    throw new Error(permissionErrorMessage(err))
  }
  try {
    return await wireGraph(stream, options)
  } catch (err) {
    for (const track of stream.getTracks()) track.stop()
    throw err
  }
}

async function wireGraph(stream: MediaStream, options: StartCaptureOptions): Promise<CaptureHandle> {
  const audioContext = createContextAt16k(resolveAudioContextCtor())
  try {
    return await wireNodes(audioContext, stream, options)
  } catch (err) {
    audioContext.close().catch(() => {})
    throw err
  }
}

async function wireNodes(
  audioContext: AudioContext,
  stream: MediaStream,
  options: StartCaptureOptions,
): Promise<CaptureHandle> {
  const source = audioContext.createMediaStreamSource(stream)
  const supportsWorklet = typeof AudioWorkletNode !== 'undefined' && 'audioWorklet' in audioContext

  let teardown: () => void
  let flush: () => Promise<void>

  if (supportsWorklet) {
    const blob = new Blob([buildWorkletModuleSource()], { type: 'application/javascript' })
    const url = URL.createObjectURL(blob)
    try {
      await audioContext.audioWorklet.addModule(url)
    } finally {
      URL.revokeObjectURL(url)
    }
    const node = new AudioWorkletNode(audioContext, WORKLET_NAME)
    let flushed: (() => void) | null = null
    node.port.onmessage = (event: MessageEvent) => {
      const data = event.data as { pcm16?: ArrayBuffer; flushed?: boolean }
      if (data.pcm16) options.onChunk({ pcm16: new Int16Array(data.pcm16) })
      if (data.flushed) flushed?.()
    }
    source.connect(node)
    flush = () =>
      new Promise<void>((resolve) => {
        const timer = window.setTimeout(resolve, FLUSH_TIMEOUT_MS)
        flushed = () => {
          window.clearTimeout(timer)
          resolve()
        }
        node.port.postMessage({ flush: true })
      })
    teardown = () => {
      node.port.onmessage = null
      node.disconnect()
    }
  } else {
    const bufferSize = 4096
    const processor = audioContext.createScriptProcessor(bufferSize, 1, 1)
    let chunks: Float32Array[] = []
    let bufferedSamples = 0
    const batchSamples = Math.ceil((audioContext.sampleRate * BATCH_MS) / 1000)
    const emit = () => {
      if (bufferedSamples === 0) return
      const merged = new Float32Array(bufferedSamples)
      let offset = 0
      for (const chunk of chunks) {
        merged.set(chunk, offset)
        offset += chunk.length
      }
      chunks = []
      bufferedSamples = 0
      options.onChunk({ pcm16: resampleTo16kMonoInt16(merged, audioContext.sampleRate) })
    }
    processor.onaudioprocess = (event: AudioProcessingEvent) => {
      const channel = event.inputBuffer.getChannelData(0)
      chunks.push(channel.slice())
      bufferedSamples += channel.length
      if (bufferedSamples >= batchSamples) emit()
    }
    source.connect(processor)
    processor.connect(audioContext.destination)
    flush = async () => emit()
    teardown = () => {
      processor.onaudioprocess = null
      processor.disconnect()
    }
  }

  let stopped = false
  return {
    stop: async () => {
      if (stopped) return
      stopped = true
      await flush()
      teardown()
      source.disconnect()
      for (const track of stream.getTracks()) track.stop()
      audioContext.close().catch(() => {})
    },
  }
}
