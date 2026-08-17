/**
 * The slice of the worker's wire surface this page uses.
 *
 * Hand-modeled against `document/src/functions/*.rs`. The golden schema
 * snapshots in `document/tests/golden/schemas/` are the source of truth; these
 * types are the console's compile-time view of them.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'

export type Format =
  | 'doc'
  | 'docx'
  | 'odt'
  | 'rtf'
  | 'ppt'
  | 'pptx'
  | 'odp'
  | 'excel'
  | 'ods'
  | 'csv'
  | 'epub'
  | 'pdf'

export type Family = 'prose' | 'spreadsheet' | 'presentation' | 'book' | 'pdf'

export type DetectedFrom = 'requested' | 'content' | 'extension'

export interface DetectResponse {
  format: Format | null
  family?: Family
  detected_from?: DetectedFrom
  convertible: boolean
  has_assets: boolean
  size_bytes: number
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

export interface MarkdownResponse {
  format: Format
  family: Family
  detected_from: DetectedFrom
  body: Body
  asset_count: number
  source: string
  elapsed_ms: number
}

export interface Asset {
  index: number
  media_type: string
  origin_part: string
  size_bytes: number
  bytes_base64?: string
  omitted?: 'not_requested' | 'too_large'
}

export interface AssetsResponse {
  format: Format
  assets: Asset[]
  total_count: number
  truncated: boolean
  source: string
  elapsed_ms: number
}

/**
 * Read a File as base64 without building one enormous argument list.
 *
 * `String.fromCharCode(...bytes)` overflows the call stack somewhere around a
 * megabyte, which is a small document. Chunking keeps it linear and bounded.
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

/** Human-readable label for a format. */
export function formatLabel(format: Format): string {
  switch (format) {
    case 'docx':
      return 'Word'
    case 'doc':
      return 'Word 97-2003'
    case 'odt':
      return 'OpenDocument Text'
    case 'rtf':
      return 'Rich Text'
    case 'pptx':
      return 'PowerPoint'
    case 'ppt':
      return 'PowerPoint 97-2003'
    case 'odp':
      return 'OpenDocument Presentation'
    case 'excel':
      return 'Excel'
    case 'ods':
      return 'OpenDocument Spreadsheet'
    case 'csv':
      return 'CSV'
    case 'epub':
      return 'EPUB'
    case 'pdf':
      return 'PDF'
  }
}

/** What "how was this recognised" means for the reader, in one clause. */
export function detectedFromMeaning(how: DetectedFrom): string {
  switch (how) {
    case 'content':
      return 'recognised from the file content'
    case 'extension':
      return 'recognised from the file name only'
    case 'requested':
      return 'named by the caller'
  }
}

export interface Reading {
  detect: DetectResponse
  markdown: MarkdownResponse | null
  assets: AssetsResponse | null
}

/**
 * Detect first, then convert only what is convertible, then look for images.
 *
 * This is the routing the worker asks a caller to do, made visible: an
 * unreadable file stops at the verdict rather than spending a conversion to
 * produce nothing, and the image pass is skipped for formats that carry none.
 */
export async function read(iii: ExtensionIii, file: File): Promise<Reading> {
  const bytes_base64 = await fileToBase64(file)
  const file_name = file.name

  const detect = await iii.trigger<DetectResponse>(
    'document::detect',
    { bytes_base64, file_name },
    { timeoutMs: 30_000 },
  )

  if (!detect.convertible) return { detect, markdown: null, assets: null }

  const markdown = await iii.trigger<MarkdownResponse>(
    'document::to-markdown',
    { bytes_base64, file_name, max_chars: 0 },
    { timeoutMs: 120_000 },
  )

  if (!detect.has_assets || markdown.asset_count === 0) {
    return { detect, markdown, assets: null }
  }

  const assets = await iii.trigger<AssetsResponse>(
    'document::extract-assets',
    { bytes_base64, file_name, media_type_prefix: 'image/' },
    { timeoutMs: 120_000 },
  )
  return { detect, markdown, assets }
}
