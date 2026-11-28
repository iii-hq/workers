/** The image-resize page without React: the call over streams, the options and the page state. */

import type { ExtensionIii } from '@iii-dev/console-ui'

export const RESIZE_FN = 'image_resize::resize'
/** The worker's input cap (image-resize/src/handler.rs MAX_INPUT_BYTES). */
export const MAX_INPUT_BYTES = 64 * 1024 * 1024

export type Format = 'jpeg' | 'png' | 'webp'
export type Strategy = 'scale-to-fit' | 'crop-to-fit'

/** What `image_resize::resize` answers: the output, and the source after EXIF orientation. */
export interface ResizeResult {
  format: Format
  width: number
  height: number
  source_width: number
  source_height: number
  strategy: Strategy
  /** JPEG and WebP only: the quality used, which `max_bytes` can lower. */
  quality?: number
}

/** The call's `metadata`: only the keys the user set, so the rest fall back to the worker's configuration. */
export interface Metadata {
  target_width?: number
  target_height?: number
  strategy?: Strategy
  output_format?: Format
  quality?: number
}

/** The options as typed; an empty field means the worker's configured default. */
export interface Options {
  width: string
  height: string
  strategy?: Strategy
  format?: Format
  quality: string
}

export type FieldErrors = Partial<Record<'width' | 'height' | 'quality', string>>

/** The image stream did not open or was cut off, or the host's SDK never took the streams. */
export class StreamUnavailableError extends Error {}

/**
 * Resizes `file` over two streams: the SDK sends the File as `input_channel`
 * and pumps `output_channel` into a local sink, and the call resolves once
 * every output byte is in.
 */
export async function resizeStream(
  iii: Pick<ExtensionIii, 'call'>,
  file: File,
  metadata: Metadata,
  { timeoutMs = 60_000, graceMs = 3_000 } = {},
): Promise<{ result: ResizeResult; blob: Blob }> {
  const chunks: BlobPart[] = []
  let sinkAborted!: () => void
  const aborted = new Promise<void>((resolve) => {
    sinkAborted = resolve
  })
  const output = new WritableStream<BlobPart>({
    write(chunk) {
      chunks.push(chunk)
    },
    abort() {
      sinkAborted()
    },
  })
  // shortcut: the SDK call takes no AbortSignal, so a call the page abandons keeps buffering its output
  // (at most the result) until it settles, within timeoutMs; cancel it once the SDK can.
  const call = iii.call<ResizeResult>(
    RESIZE_FN,
    { input_channel: file, output_channel: output, metadata },
    { timeoutMs },
  )
  let timer: ReturnType<typeof setTimeout> | undefined
  // The worker aborts its output just before it answers with its own error; an abort with no answer
  // means the stream itself failed (an ADE that does not relay stream sockets), which the SDK would
  // only report at timeoutMs.
  const unavailable = aborted.then(
    () =>
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new StreamUnavailableError()), graceMs)
      }),
  )
  try {
    const result = await Promise.race([call, unavailable])
    return { result, blob: new Blob(chunks, { type: `image/${result.format}` }) }
  } catch (error) {
    // An SDK that takes no stream values (ADEs up to v1.10.19 bundle 0.26) sends the File and the sink as
    // `{}` and never locks the sink; the worker's error then only names the payload it could not read.
    throw output.locked ? error : new StreamUnavailableError()
  } finally {
    clearTimeout(timer)
  }
}

const BOX_ERROR = 'Enter a whole number of pixels, 1 or more.'
const QUALITY_ERROR = 'Enter a quality from 1 to 100.'

/** `undefined` when empty, `null` when not a whole number from 1 to `max`. */
function parseWhole(raw: string, max: number): number | null | undefined {
  const text = raw.trim()
  if (!text) return undefined
  const n = Number(text)
  return /^\d+$/.test(text) && n >= 1 && n <= max ? n : null
}

/** PNG is lossless: quality does not apply when the output is PNG, chosen or kept from the input. */
export const isPngOutput = (options: Options, sourceType: string) =>
  options.format === 'png' || (!options.format && sourceType === 'image/png')

/** The call's metadata, or one message per invalid field. */
export function buildMetadata(options: Options, sourceType: string): { metadata: Metadata } | { errors: FieldErrors } {
  const metadata: Metadata = {}
  const errors: FieldErrors = {}
  const width = parseWhole(options.width, 0xffff_ffff)
  if (width === null) errors.width = BOX_ERROR
  else if (width !== undefined) metadata.target_width = width
  const height = parseWhole(options.height, 0xffff_ffff)
  if (height === null) errors.height = BOX_ERROR
  else if (height !== undefined) metadata.target_height = height
  if (options.strategy) metadata.strategy = options.strategy
  if (options.format) metadata.output_format = options.format
  if (!isPngOutput(options, sourceType)) {
    const quality = parseWhole(options.quality, 100)
    if (quality === null) errors.quality = QUALITY_ERROR
    else if (quality !== undefined) metadata.quality = quality
  }
  return Object.keys(errors).length > 0 ? { errors } : { metadata }
}

/** `photo.jpeg` resized to 200 x 150 WebP is `photo_200x150.webp`. */
export function downloadName(sourceName: string, result: Pick<ResizeResult, 'format' | 'width' | 'height'>) {
  const base = sourceName.replace(/\.[^.]+$/, '')
  return `${base}_${result.width}x${result.height}.${result.format === 'jpeg' ? 'jpg' : result.format}`
}

/** The output size against the input's: the output is always re-encoded and can grow. */
export function sizeChange(before: number, after: number): { text: string; tone: 'ok' | 'warn' | 'ink' } {
  if (after === before) return { text: 'same size', tone: 'ink' }
  const smaller = after < before
  const rounded = Math.round((Math.abs(after - before) / before) * 100)
  // An output that exists never shrank by 100%.
  const percent = smaller ? Math.min(rounded, 99) : rounded
  return { text: `${smaller ? '-' : '+'}${percent < 1 ? '<1' : percent}%`, tone: smaller ? 'ok' : 'warn' }
}

/* ── page state ─────────────────────────────────────────────────────── */

export type Preview = 'ok' | 'failed'

export interface Source {
  file: File
  preview: Preview
  /** The decoded preview's size, until the worker reports the source's. */
  width?: number
  height?: number
}

export type Notice = { kind: 'too-large'; name: string; size: number } | { kind: 'no-file' }

export type Run =
  | { phase: 'idle' }
  | { phase: 'running' }
  | { phase: 'done'; result: ResizeResult; blob: Blob; preview: Preview }
  | { phase: 'failed'; error: unknown }

/** Holds Files and Blobs, never object URLs: the page derives those and revokes them. */
export interface State {
  /** The pick, remove or submit the state reflects; a call that settles for an older one is dropped. */
  generation: number
  source: Source | null
  notice: Notice | null
  run: Run
}

export type Action =
  | { type: 'pick'; generation: number; file?: File }
  | { type: 'remove'; generation: number }
  | { type: 'previewLoaded'; width: number; height: number }
  | { type: 'previewFailed'; target: 'source' | 'result' }
  | { type: 'submit'; generation: number }
  | { type: 'settled'; generation: number; result: ResizeResult; blob: Blob }
  | { type: 'settled'; generation: number; error: unknown }

export const initialState: State = { generation: 0, source: null, notice: null, run: { phase: 'idle' } }

/** Why a pick is refused, if it is. */
export function noticeFor(file: File | undefined): Notice | null {
  if (!file) return { kind: 'no-file' }
  return file.size > MAX_INPUT_BYTES ? { kind: 'too-large', name: file.name, size: file.size } : null
}

export const canSubmit = (state: State) => state.source !== null && state.run.phase !== 'running'

export function reduce(state: State, action: Action): State {
  switch (action.type) {
    case 'pick': {
      const notice = noticeFor(action.file)
      // A drop with no file (a link, an image dragged from a page) leaves the source and any call alone.
      if (!action.file) return { ...state, notice }
      const source: Source | null = notice ? null : { file: action.file, preview: 'ok' }
      return { generation: action.generation, source, notice, run: { phase: 'idle' } }
    }
    case 'remove':
      return { generation: action.generation, source: null, notice: null, run: { phase: 'idle' } }
    case 'previewLoaded':
      return state.source
        ? { ...state, source: { ...state.source, width: action.width, height: action.height } }
        : state
    case 'previewFailed':
      if (action.target === 'source') {
        return state.source ? { ...state, source: { ...state.source, preview: 'failed' } } : state
      }
      return state.run.phase === 'done' ? { ...state, run: { ...state.run, preview: 'failed' } } : state
    case 'submit':
      return canSubmit(state)
        ? { ...state, generation: action.generation, notice: null, run: { phase: 'running' } }
        : state
    case 'settled':
      if (action.generation !== state.generation) return state
      return {
        ...state,
        run:
          'error' in action
            ? { phase: 'failed', error: action.error }
            : { phase: 'done', result: action.result, blob: action.blob, preview: 'ok' },
      }
  }
}
