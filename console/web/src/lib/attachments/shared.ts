/**
 * The pieces every attachment kind shares: the envelope a block is written in,
 * how bytes get to a worker, and how a failure is phrased.
 *
 * One envelope for every kind is the point. `<attached-file …>` is what
 * `#file(...)` mentions already use, so the transcript, the chip renderer and
 * the model all see a shape they know, whether the content came from a PDF, a
 * spreadsheet, or a text file the browser read directly.
 */

import { getIiiClient } from '@/lib/iii-client'

/** Opening of an attachment block; the header ends at the first `>`. */
export const ATTACHED_FILE_PREFIX = '<attached-file '

export interface AttachmentFailure {
  name: string
  reason: string
}

/**
 * A native image content block on the outgoing user message.
 *
 * Wire-identical to the harness's `ContentBlock::Image` (`{ type, mime, data }`
 * — see `harness/src/types/content.rs`), which the Anthropic and OpenAI
 * providers already map onto their own image shapes. Text blocks describe a
 * document; this one IS the picture.
 */
export interface AttachmentImageBlock {
  type: 'image'
  mime: string
  data: string
}

/** What one attachment turned into, for the chip on the sent message. */
export interface AttachmentReadSummary {
  /** Attachment id, so the caller can match this back to its chip. */
  id: string
  /** The chip's new label, already assembled. */
  label: string
}

/**
 * Invoke a worker function. Injectable so a test drives an expansion without a
 * live engine, which every expander needs and none should re-declare.
 */
export type TriggerFn = (
  functionId: string,
  payload: Record<string, unknown>,
) => Promise<unknown>

/** The live trigger, or the caller's stand-in. */
export function triggerOr(trigger: TriggerFn | undefined): TriggerFn {
  return (
    trigger ??
    (async (functionId, payload) => {
      const client = await getIiiClient()
      return client.trigger(functionId, payload)
    })
  )
}

/**
 * Report the attachments a per-send ceiling cut off.
 *
 * Every kind has a ceiling and every kind has to say what it dropped: an
 * attachment that silently disappears is the failure this whole path exists to
 * prevent, and a person who attached six documents deserves to know two of them
 * did not go.
 */
export function reportDropped(
  dropped: readonly { name: string }[],
  reason: string,
  into: { blocks: string[]; failures: AttachmentFailure[] },
): void {
  for (const attachment of dropped) {
    into.blocks.push(failureBlock(attachment.name, reason))
    into.failures.push({ name: attachment.name, reason })
  }
}

/**
 * Base64 without building one enormous argument list.
 *
 * `String.fromCharCode(...bytes)` overflows the call stack somewhere around a
 * megabyte, which is a small document. Chunking keeps it linear and bounded.
 */
export async function fileToBase64(file: Blob): Promise<string> {
  return bytesToBase64(new Uint8Array(await file.arrayBuffer()))
}

export function bytesToBase64(bytes: Uint8Array): string {
  const CHUNK = 0x8000
  let binary = ''
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK))
  }
  return btoa(binary)
}

/**
 * `>` has to be escaped as well as `&` and `"`. The header is parsed by finding
 * the first `>`, so a file name containing one would cut the header short and
 * lose every attribute after it.
 */
export function escapeAttr(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('"', '&quot;')
    .replaceAll('>', '&gt;')
}

/** A self-closing block saying a named file could not be read, and why. */
export function failureBlock(name: string, reason: string): string {
  return `${ATTACHED_FILE_PREFIX}path="${escapeAttr(name)}" error="${escapeAttr(reason)}" />`
}

/**
 * The one failure worth naming precisely: the worker is not installed. Anything
 * else surfaces as-is, trimmed.
 */
export function describeWorkerFailure(err: unknown, worker: string): string {
  const message = messageOf(err)
  if (/not registered|NOT_FOUND|function .* not found/i.test(message)) {
    return `the ${worker} worker is not running — install it with \`iii worker add ${worker}\``
  }
  return message.length > 160 ? `${message.slice(0, 157)}…` : message
}

/**
 * The readable half of whatever a rejection carried.
 *
 * The browser SDK rejects with a plain object, not an `Error`, so `String(err)`
 * produces the literal text `[object Object]` — which is what the chip and the
 * warn notice showed the first time a document failed to convert. Dig for the
 * fields a bus error actually carries, and fall back to JSON rather than to a
 * sentence that says nothing.
 */
function messageOf(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  if (err && typeof err === 'object') {
    const record = err as Record<string, unknown>
    for (const key of ['message', 'error', 'reason', 'detail']) {
      const value = record[key]
      if (typeof value === 'string' && value.length > 0) return value
      if (value && typeof value === 'object') {
        const nested = (value as Record<string, unknown>).message
        if (typeof nested === 'string' && nested.length > 0) return nested
      }
    }
    if (typeof record.code === 'string') return record.code
    try {
      return JSON.stringify(err)
    } catch {
      return 'the worker returned an error with no message'
    }
  }
  return String(err)
}

/** The file's extension, lowercased, without the dot. Empty when it has none. */
export function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.')
  if (dot <= 0 || dot === name.length - 1) return ''
  return name.slice(dot + 1).toLowerCase()
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} b`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} kb`
  return `${(bytes / (1024 * 1024)).toFixed(1)} mb`
}
