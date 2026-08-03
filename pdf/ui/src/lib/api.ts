/**
 * The slice of the worker's wire surface this page uses.
 *
 * Hand-modeled against `pdf/src/functions/*.rs`. The golden schema snapshots in
 * `pdf/tests/golden/schemas/` are the source of truth; these types are the
 * console's compile-time view of them.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'

export type DocumentType =
  | 'text_based'
  | 'scanned'
  | 'image_based'
  | 'mixed'

export interface PageOcrReason {
  page: number
  reasons: string[]
}

export interface ClassifyResponse {
  document_type: DocumentType
  confidence: number
  page_count: number
  pages_sampled: number
  pages_with_text: number
  pages_needing_ocr: number[]
  ocr_reasons: PageOcrReason[]
  ocr_recommended: boolean
  title: string | null
  source: string
  elapsed_ms: number
}

export interface Body {
  text: string
  chars: number
  total_chars: number
  truncated: boolean
  preview?: string
}

export interface PageMarkdown {
  page: number
  markdown: string
  needs_ocr: boolean
  ocr_reason?: string
}

export interface MarkdownResponse {
  document_type: DocumentType
  body: Body
  page_count: number
  pages_converted: number
  pages?: PageMarkdown[]
  pages_with_tables: number[]
  pages_with_columns: number[]
  pages_needing_ocr: number[]
  ocr_reasons: PageOcrReason[]
  has_encoding_issues: boolean
  source: string
  elapsed_ms: number
}

/**
 * Read a File as base64 without building one enormous argument list.
 *
 * `String.fromCharCode(...bytes)` overflows the call stack somewhere around a
 * megabyte, which is a small PDF. Chunking keeps it linear and bounded.
 */
export async function fileToBase64(file: File): Promise<string> {
  const buffer = new Uint8Array(await file.arrayBuffer())
  const CHUNK = 0x8000
  let binary = ''
  for (let i = 0; i < buffer.length; i += CHUNK) {
    binary += String.fromCharCode(...buffer.subarray(i, i + CHUNK))
  }
  return btoa(binary)
}

/** Human-readable label for a document type. */
export function documentTypeLabel(type: DocumentType): string {
  switch (type) {
    case 'text_based':
      return 'text based'
    case 'image_based':
      return 'image based'
    default:
      return type
  }
}

/** What a document type means for the caller, in one sentence. */
export function documentTypeMeaning(type: DocumentType): string {
  switch (type) {
    case 'text_based':
      return 'Real text throughout. Extract it locally.'
    case 'scanned':
      return 'Pictures of pages. Every page needs OCR.'
    case 'image_based':
      return 'Images with little or no text layer.'
    case 'mixed':
      return 'Some pages carry text and some do not.'
  }
}

/** Plain-language expansion of a machine-readable OCR reason. */
export function ocrReasonLabel(reason: string): string {
  switch (reason) {
    case 'scanned':
      return 'a raster page'
    case 'no_text':
      return 'nothing extractable, and nothing to OCR'
    case 'vector_text':
      return 'characters drawn as outlines, not text'
    case 'suspected_garbled_text':
      return 'a text layer that decodes to nonsense'
    default:
      return reason
  }
}

export interface Inspection {
  classify: ClassifyResponse
  markdown: MarkdownResponse | null
}

/**
 * Classify first, then convert only if there is something to convert.
 *
 * This is the routing the worker's guidance asks an agent to do, made visible:
 * a scan gets classified and stops, rather than spending a second producing an
 * empty document.
 */
export async function inspect(
  iii: ExtensionIii,
  file: File,
): Promise<Inspection> {
  const bytes_base64 = await fileToBase64(file)

  const classify = await iii.trigger<ClassifyResponse>(
    'pdf::classify',
    { bytes_base64 },
    { timeoutMs: 60_000 },
  )

  if (classify.document_type === 'scanned' || classify.document_type === 'image_based') {
    return { classify, markdown: null }
  }

  const markdown = await iii.trigger<MarkdownResponse>(
    'pdf::to-markdown',
    { bytes_base64, max_chars: 0, per_page: true },
    { timeoutMs: 120_000 },
  )
  return { classify, markdown }
}
