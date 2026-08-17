/**
 * Office-document expansion for the composer send path.
 *
 * A `.docx` or a `.pptx` attached in the composer used to reach the agent as
 * nothing at all: the send path forwards text blocks, and a ZIP of XML is not
 * text. The agent then answered as though no document had been given to it —
 * the same hole the PDF path closed, for every other format people attach.
 *
 * At send time each one is converted by the `document` worker on the machine
 * and appended as an `<attached-file …>` text block, the same envelope
 * `#file(<path>)` mentions and PDFs already use.
 *
 * Conversion is one call, not two. Unlike a PDF there is no classification
 * step: an office document either parses or it does not, and the format is
 * read from the bytes. What the block does carry is the count of images the
 * markdown could not represent, because a deck built out of diagrams converts
 * to a page of titles and would otherwise read as a document with little to
 * say.
 */

import type { Attachment } from '@/types/chat'
import {
  ATTACHED_FILE_PREFIX,
  type AttachmentFailure,
  type AttachmentReadSummary,
  describeWorkerFailure,
  escapeAttr,
  extensionOf,
  failureBlock,
  fileToBase64,
  reportDropped,
  type TriggerFn,
  triggerOr,
} from './shared'

export const TO_MARKDOWN_FUNCTION_ID = 'document::to-markdown'

/** Max documents converted per send; extras are reported, never dropped. */
export const MAX_DOCUMENTS_PER_SEND = 4

/**
 * Characters of markdown inlined per document. A long report would otherwise
 * consume the context the question needed. The block says when it stops short,
 * and the agent can call `document::to-markdown` itself for the rest.
 */
export const MAX_MARKDOWN_CHARS = 20_000

/**
 * Largest document read from the composer. Encoding happens in the browser, so
 * an enormous file is a frozen tab before the worker ever sees it, and the
 * worker's own ceiling would reject it anyway. Refuse it here with an
 * explanation instead.
 */
export const MAX_DOCUMENT_BYTES = 64 * 1024 * 1024

/**
 * Every extension the `document` worker converts, minus `pdf`, which has its
 * own worker and its own path through the send.
 *
 * Extension rather than MIME type on purpose: browsers report office documents
 * inconsistently (a `.csv` arrives as `application/vnd.ms-excel`, a file
 * dragged from an archive often arrives as `application/octet-stream` or with
 * no type at all), and the name is the one thing that survives every route into
 * the composer.
 */
export const DOCUMENT_EXTENSIONS = new Set([
  'doc',
  'docx',
  'docm',
  'ppt',
  'pps',
  'pot',
  'pptx',
  'pptm',
  'ppsx',
  'ppsm',
  'xls',
  'xlsx',
  'xlsm',
  'xlsb',
  'odt',
  'ods',
  'odp',
  'rtf',
  'epub',
  'csv',
])

export interface ExpandedDocuments {
  /** One `<attached-file …>` block per document, in input order. */
  blocks: string[]
  /** One entry per document actually converted, for the message chips. */
  read: AttachmentReadSummary[]
  failures: AttachmentFailure[]
}

/** Whether an attachment is an office document this worker converts. */
export function isDocumentAttachment(attachment: Attachment): boolean {
  return DOCUMENT_EXTENSIONS.has(extensionOf(attachment.name))
}

// --- wire subset of the document worker -----------------------------------

interface MarkdownWire {
  format?: string
  family?: string
  detected_from?: 'requested' | 'content' | 'extension'
  body?: {
    text?: string
    chars?: number
    total_chars?: number
    truncated?: boolean
  }
  asset_count?: number
  elapsed_ms?: number
}

/**
 * Convert every attached office document through the `document` worker.
 *
 * Attachments without their underlying `File` are skipped silently: a
 * conversation reloaded from history carries the chip metadata but not the
 * bytes, and re-reading a document attached in a previous session is not this
 * function's job.
 */
export async function expandDocumentAttachments(
  attachments: Attachment[],
  trigger?: TriggerFn,
): Promise<ExpandedDocuments> {
  const documents = attachments.filter((a) => isDocumentAttachment(a) && a.file)
  if (documents.length === 0) return { blocks: [], read: [], failures: [] }

  const call = triggerOr(trigger)

  const blocks: string[] = []
  const read: AttachmentReadSummary[] = []
  const failures: AttachmentFailure[] = []

  for (const attachment of documents.slice(0, MAX_DOCUMENTS_PER_SEND)) {
    if (attachment.size > MAX_DOCUMENT_BYTES) {
      const mb = Math.round(MAX_DOCUMENT_BYTES / (1024 * 1024))
      const reason = `larger than the ${mb} MB limit for reading a document in the composer`
      blocks.push(failureBlock(attachment.name, reason))
      failures.push({ name: attachment.name, reason })
      continue
    }
    try {
      const outcome = await expandOne(attachment, call)
      blocks.push(outcome.block)
      read.push(outcome.summary)
    } catch (err) {
      const reason = describeWorkerFailure(err, 'document')
      blocks.push(failureBlock(attachment.name, reason))
      failures.push({ name: attachment.name, reason })
    }
  }

  reportDropped(
    documents.slice(MAX_DOCUMENTS_PER_SEND),
    `only ${MAX_DOCUMENTS_PER_SEND} documents are read per message`,
    { blocks, failures },
  )

  return { blocks, read, failures }
}

interface ExpandOutcome {
  block: string
  summary: AttachmentReadSummary
}

async function expandOne(
  attachment: Attachment,
  call: TriggerFn,
): Promise<ExpandOutcome> {
  const bytes_base64 = await fileToBase64(attachment.file as File)

  const converted = (await call(TO_MARKDOWN_FUNCTION_ID, {
    bytes_base64,
    // A CSV carries no signature of its own; without the name the worker
    // cannot recognise it and would refuse a file it reads perfectly well.
    file_name: attachment.name,
    max_chars: MAX_MARKDOWN_CHARS,
  })) as MarkdownWire

  const body = converted.body ?? {}
  const text = body.text ?? ''
  const assets = converted.asset_count ?? 0
  const elapsedMs = converted.elapsed_ms ?? 0
  const chars = body.total_chars ?? text.length

  const attrs = [
    `path="${escapeAttr(attachment.name)}"`,
    `size="${attachment.size}"`,
    `format="${escapeAttr(converted.format ?? extensionOf(attachment.name))}-markdown"`,
  ]
  if (body.truncated) {
    attrs.push('truncated="true"')
    attrs.push(`total-chars="${chars}"`)
  }
  if (assets > 0) attrs.push(`embedded-images="${assets}"`)

  const notes: string[] = []
  if (body.truncated) {
    notes.push(
      `This is the first ${body.chars ?? text.length} of ${chars} characters. Call ${TO_MARKDOWN_FUNCTION_ID} with max_chars 0 for the rest.`,
    )
  }
  if (assets > 0) {
    notes.push(
      `${assets} embedded image${assets === 1 ? '' : 's'} could not be represented as markdown. Call document::extract-assets for their bytes.`,
    )
  }
  // The empty conversion is the case worth spelling out. A deck of diagrams
  // and a genuinely blank file both come back with no text, and the model has
  // no way to tell them apart unless the block says which happened.
  if (text.trim().length === 0) {
    notes.push(
      assets > 0
        ? 'This document holds no text an agent can read — its content is the images above. Do not call it empty.'
        : 'This document converted to nothing: it holds no text and no images.',
    )
  }

  const preamble = notes.length > 0 ? `${notes.join(' ')}\n\n` : ''
  return {
    block: `${ATTACHED_FILE_PREFIX}${attrs.join(' ')}>\n${preamble}${text}\n</attached-file>`,
    summary: {
      id: attachment.id,
      label: chipLabel(attachment.name, {
        format: converted.format,
        chars,
        truncated: body.truncated === true,
        assets,
        elapsedMs,
      }),
    },
  }
}

/**
 * One line for the chip on the sent message: what the worker made of the
 * document, and how fast. This is the only place a person can see that the
 * document was read at all, because the conversion happens before the model is
 * called and so never appears as a function call in the transcript.
 */
function chipLabel(
  name: string,
  summary: {
    format?: string
    chars: number
    truncated: boolean
    assets: number
    elapsedMs: number
  },
): string {
  const parts: string[] = []
  if (summary.format) parts.push(summary.format)
  parts.push(
    summary.chars > 0
      ? `${summary.chars.toLocaleString('en-US')}${summary.truncated ? '+' : ''} chars`
      : 'no text',
  )
  if (summary.assets > 0) {
    parts.push(`${summary.assets} image${summary.assets === 1 ? '' : 's'}`)
  }
  parts.push(`${summary.elapsedMs} ms`)
  return `${name} · ${parts.join(' · ')}`
}
