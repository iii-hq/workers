/**
 * PDF attachment expansion for the composer send path.
 *
 * A PDF attached in the composer used to reach the agent as nothing at all:
 * `AttachmentButton` builds a preview only for small text and image files, and
 * the only thing the send path forwards is text blocks. The agent then answered
 * as though no document had been given to it.
 *
 * At send time each attached PDF is read by the `pdf` worker on the machine and
 * appended as an `<attached-file …>` text block — the same envelope
 * `#file(<path>)` mentions use, so the transcript, the chip renderer, and the
 * model all see a shape they already understand.
 *
 * Classification comes first because it decides whether extraction is worth
 * doing: a scan has no text to extract, and a block saying so is far more useful
 * to the model than an empty one. Failures never block the send.
 */

import { getIiiClient } from '@/lib/iii-client'
import type { Attachment } from '@/types/chat'

export const CLASSIFY_FUNCTION_ID = 'pdf::classify'
export const TO_MARKDOWN_FUNCTION_ID = 'pdf::to-markdown'

/** Max PDFs expanded per send; extras are reported, never silently dropped. */
export const MAX_PDFS_PER_SEND = 4

/**
 * Characters of markdown inlined per document. A long report would otherwise
 * consume the context the question needed. The block says when it stops short,
 * and the agent can call `pdf::to-markdown` itself with a page filter.
 */
export const MAX_MARKDOWN_CHARS = 20_000

/**
 * Largest document read from the composer. Encoding happens in the browser, so
 * an enormous file is a frozen tab before the worker ever sees it, and its own
 * ceiling would reject it anyway. Refuse it here with an explanation instead.
 */
export const MAX_PDF_BYTES = 64 * 1024 * 1024

const ATTACHED_FILE_PREFIX = '<attached-file '

export interface PdfExpansionFailure {
  name: string
  reason: string
}

/**
 * What one document turned into, for the chip on the sent message.
 *
 * Without this the composer chip reads "report.pdf 32kb" whether the document
 * was parsed, skipped or failed, which leaves a person with no way to tell
 * that the PDF reached the agent at all.
 */
export interface PdfReadSummary {
  /** Attachment id, so the caller can match this back to its chip. */
  id: string
  pages: number
  /** Absent when the document held no extractable text. */
  chars?: number
  /** Classify plus extract, as the worker reported them. */
  elapsedMs: number
  needsOcr: boolean
  truncated: boolean
}

export interface ExpandedPdfs {
  /** One `<attached-file …>` block per expanded document, in input order. */
  blocks: string[]
  /** One entry per document actually read, for the message chips. */
  read: PdfReadSummary[]
  failures: PdfExpansionFailure[]
}

/** Whether an attachment is a PDF, by declared type or by extension. */
export function isPdfAttachment(attachment: Attachment): boolean {
  return (
    attachment.type === 'application/pdf' ||
    attachment.name.toLowerCase().endsWith('.pdf')
  )
}

// --- wire subset of the pdf worker ---------------------------------------

interface ClassifyWire {
  document_type?: 'text_based' | 'scanned' | 'image_based' | 'mixed'
  page_count?: number
  pages_needing_ocr?: number[]
  ocr_reasons?: Array<{ page: number; reasons: string[] }>
  elapsed_ms?: number
}

interface MarkdownWire {
  body?: {
    text?: string
    chars?: number
    total_chars?: number
    truncated?: boolean
  }
  page_count?: number
  pages_with_tables?: number[]
  has_encoding_issues?: boolean
  elapsed_ms?: number
}

type TriggerFn = (
  functionId: string,
  payload: Record<string, unknown>,
) => Promise<unknown>

/**
 * Base64 without building one enormous argument list.
 *
 * `String.fromCharCode(...bytes)` overflows the call stack somewhere around a
 * megabyte, which is a small PDF. Chunking keeps it linear and bounded.
 */
async function fileToBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer())
  const CHUNK = 0x8000
  let binary = ''
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK))
  }
  return btoa(binary)
}

/**
 * Read every attached PDF through the `pdf` worker and format the blocks.
 *
 * Attachments without their underlying `File` are skipped silently: a
 * conversation reloaded from history carries the chip metadata but not the
 * bytes, and re-reading a document the user attached in a previous session is
 * not this function's job.
 */
export async function expandPdfAttachments(
  attachments: Attachment[],
  trigger?: TriggerFn,
): Promise<ExpandedPdfs> {
  const pdfs = attachments.filter((a) => isPdfAttachment(a) && a.file)
  if (pdfs.length === 0) return { blocks: [], read: [], failures: [] }

  const call =
    trigger ??
    (async (functionId: string, payload: Record<string, unknown>) => {
      const client = await getIiiClient()
      return client.trigger(functionId, payload)
    })

  const blocks: string[] = []
  const read: PdfReadSummary[] = []
  const failures: PdfExpansionFailure[] = []

  for (const attachment of pdfs.slice(0, MAX_PDFS_PER_SEND)) {
    if (attachment.size > MAX_PDF_BYTES) {
      const mb = Math.round(MAX_PDF_BYTES / (1024 * 1024))
      const reason = `larger than the ${mb} MB limit for reading a PDF in the composer`
      blocks.push(failureBlock(attachment.name, reason))
      failures.push({ name: attachment.name, reason })
      continue
    }
    try {
      const outcome = await expandOne(attachment, call)
      blocks.push(outcome.block)
      read.push(outcome.summary)
    } catch (err) {
      const reason = describeFailure(err)
      blocks.push(failureBlock(attachment.name, reason))
      failures.push({ name: attachment.name, reason })
    }
  }

  for (const dropped of pdfs.slice(MAX_PDFS_PER_SEND)) {
    const reason = `only ${MAX_PDFS_PER_SEND} PDFs are read per message`
    blocks.push(failureBlock(dropped.name, reason))
    failures.push({ name: dropped.name, reason })
  }

  return { blocks, read, failures }
}

interface ExpandOutcome {
  block: string
  summary: PdfReadSummary
}

async function expandOne(
  attachment: Attachment,
  call: TriggerFn,
): Promise<ExpandOutcome> {
  const bytes_base64 = await fileToBase64(attachment.file as File)

  const classified = (await call(CLASSIFY_FUNCTION_ID, {
    bytes_base64,
  })) as ClassifyWire
  const type = classified.document_type ?? 'mixed'
  const pages = classified.page_count ?? 0
  const classifyMs = classified.elapsed_ms ?? 0

  const unreadable = (): ExpandOutcome => ({
    block: scannedBlock(attachment.name, pages, classified),
    summary: {
      id: attachment.id,
      pages,
      elapsedMs: classifyMs,
      needsOcr: true,
      truncated: false,
    },
  })

  // A scan has nothing to extract. Say so in the block: the model needs to know
  // the document was read and found unreadable, not that it was never given one.
  if (type === 'scanned' || type === 'image_based') return unreadable()

  const converted = (await call(TO_MARKDOWN_FUNCTION_ID, {
    bytes_base64,
    max_chars: MAX_MARKDOWN_CHARS,
  })) as MarkdownWire

  const body = converted.body ?? {}
  const text = body.text ?? ''
  if (text.trim().length === 0) return unreadable()

  const attrs = [
    `path="${escapeAttr(attachment.name)}"`,
    `size="${attachment.size}"`,
    `pages="${converted.page_count ?? pages}"`,
    'format="pdf-markdown"',
  ]
  if (body.truncated) {
    attrs.push('truncated="true"')
    attrs.push(`total-chars="${body.total_chars ?? text.length}"`)
  }
  if (converted.has_encoding_issues) attrs.push('encoding-issues="true"')

  const needsOcr = classified.pages_needing_ocr ?? []
  if (needsOcr.length > 0) {
    attrs.push(`pages-needing-ocr="${needsOcr.join(',')}"`)
  }

  const notes: string[] = []
  if (body.truncated) {
    notes.push(
      `This is the first ${body.chars ?? text.length} of ${
        body.total_chars ?? '?'
      } characters. Call ${TO_MARKDOWN_FUNCTION_ID} with a pages filter for the rest.`,
    )
  }
  if (needsOcr.length > 0) {
    notes.push(
      `Pages ${needsOcr.join(', ')} hold no readable text and are missing from this extract.`,
    )
  }
  if (converted.has_encoding_issues) {
    notes.push('Font encodings decoded badly; treat this text as unreliable.')
  }

  const preamble = notes.length > 0 ? `${notes.join(' ')}\n\n` : ''
  return {
    block: `${ATTACHED_FILE_PREFIX}${attrs.join(' ')}>\n${preamble}${text}\n</attached-file>`,
    summary: {
      id: attachment.id,
      pages: converted.page_count ?? pages,
      chars: body.total_chars ?? text.length,
      elapsedMs: classifyMs + (converted.elapsed_ms ?? 0),
      needsOcr: needsOcr.length > 0,
      truncated: body.truncated === true,
    },
  }
}

/**
 * One line for the chip on the sent message: what the worker made of the
 * document, and how fast. This is the only place a person can see that the PDF
 * was read at all, because the expansion happens before the model is called and
 * so never appears as a function call in the transcript.
 */
export function summaryLabel(name: string, summary: PdfReadSummary): string {
  const parts = [`${summary.pages} page${summary.pages === 1 ? '' : 's'}`]
  if (summary.needsOcr && summary.chars === undefined) {
    parts.push('no readable text')
  } else if (summary.chars !== undefined) {
    parts.push(
      `${summary.chars.toLocaleString('en-US')}${summary.truncated ? '+' : ''} chars`,
    )
  }
  parts.push(`${summary.elapsedMs} ms`)
  return `${name} · ${parts.join(' · ')}`
}

function scannedBlock(
  name: string,
  pages: number,
  classified: ClassifyWire,
): string {
  const reasons = [
    ...new Set((classified.ocr_reasons ?? []).flatMap((r) => r.reasons)),
  ]
  const why = reasons.length > 0 ? ` (${reasons.join(', ')})` : ''
  const attrs = [
    `path="${escapeAttr(name)}"`,
    `pages="${pages}"`,
    'format="pdf-markdown"',
    'needs-ocr="true"',
  ]
  return (
    `${ATTACHED_FILE_PREFIX}${attrs.join(' ')}>\n` +
    `This PDF holds no extractable text${why}. It is a scan or an image, so ` +
    `every one of its ${pages} pages would need OCR. Nothing was extracted. Do ` +
    `not claim the document is empty — say it could not be read as text.\n` +
    `</attached-file>`
  )
}

function failureBlock(name: string, reason: string): string {
  return `${ATTACHED_FILE_PREFIX}path="${escapeAttr(name)}" error="${escapeAttr(reason)}" />`
}

/**
 * The one failure worth naming precisely: the worker is not installed. Anything
 * else surfaces as-is, trimmed.
 */
function describeFailure(err: unknown): string {
  const message = err instanceof Error ? err.message : String(err)
  if (/not registered|NOT_FOUND|function .* not found/i.test(message)) {
    return 'the pdf worker is not running — install it with `iii worker add pdf`'
  }
  return message.length > 160 ? `${message.slice(0, 157)}…` : message
}

/**
 * `>` has to be escaped as well as `&` and `"`. The header is parsed by finding
 * the first `>`, so a file name containing one would cut the header short and
 * lose every attribute after it.
 */
function escapeAttr(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('"', '&quot;')
    .replaceAll('>', '&gt;')
}
