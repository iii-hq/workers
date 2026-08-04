/**
 * How `pdf::*` calls render in chat and traces.
 *
 * Without this they render as raw JSON, and the one number that matters — is
 * this document readable, and how much of it did the agent actually get — is
 * buried in it. Each renderer surfaces the decision, not the payload.
 *
 * Match narrowly and return `null` freely: `null` falls through to the console's
 * own cards, which already handle errors and pending approvals better than a
 * worker renderer should try to.
 */

import { Badge, type FunctionTriggerMessage, type FunctionTriggerRenderer, type Host } from '@iii-dev/console-ui'

import {
  documentTypeLabel,
  ocrReasonLabel,
  type ClassifyResponse,
  type MarkdownResponse,
} from '../lib/api'

const HANDLED = new Set([
  'pdf::classify',
  'pdf::to-markdown',
  'pdf::extract-text',
  'pdf::extract-items',
  'pdf::extract-regions',
])

export function createPdfTriggerRenderer(_host: Host): FunctionTriggerRenderer {
  return {
    id: 'pdf/page.js#renderer',
    isMatch: (functionId) => HANDLED.has(functionId),
    tryRender: (message) => render(message),
    tryRenderPreview: (message) => render(message),
  }
}

/**
 * A function result reaches the console wrapped by the harness as
 * `{ content: [...], details: <the real response> }`, not as the response
 * itself. Reading the raw value looks like it works right up until every field
 * is undefined and the renderer quietly falls through to an empty card, which
 * is exactly what happened the first time this shipped.
 *
 * The console has its own `unwrapEnvelope`, but injected assets can only import
 * from `@iii-dev/console-ui`, so the same two-line rule lives here.
 */
function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}

function render(message: FunctionTriggerMessage) {
  const output = unwrapEnvelope(message.output)
  if (!output || typeof output !== 'object') return null

  switch (message.functionId) {
    case 'pdf::classify':
      return <ClassifyCard result={output as ClassifyResponse} />
    case 'pdf::to-markdown':
      return <MarkdownCard result={output as MarkdownResponse} />
    case 'pdf::extract-text':
      return <BodyCard result={output as { body?: BodyLike; source?: string }} label="text" />
    case 'pdf::extract-items':
      return <ItemsCard result={output as ItemsLike} />
    case 'pdf::extract-regions':
      return <RegionsCard result={output as RegionsLike} />
    default:
      return null
  }
}

interface BodyLike {
  chars: number
  total_chars: number
  truncated: boolean
}

interface ItemsLike {
  count?: number
  total_count?: number
  truncated?: boolean
  source?: string
}

interface RegionsLike {
  region_count?: number
  regions_needing_ocr?: number
  source?: string
}

function ClassifyCard({ result }: { result: ClassifyResponse }) {
  if (!result.document_type) return null
  const needing = result.pages_needing_ocr?.length ?? 0
  const reasons = new Set(
    (result.ocr_reasons ?? []).flatMap((r) => r.reasons).map(ocrReasonLabel),
  )
  return (
    <div className="pdf-trigger">
      <div className="pdf-trigger__row">
        <Badge variant={needing === 0 ? 'accent' : needing >= result.page_count ? 'alert' : 'warn'}>
          {documentTypeLabel(result.document_type)}
        </Badge>
        <span className="pdf-trigger__file">{result.source}</span>
        <span className="pdf-trigger__meta">
          {result.page_count} pages · {Math.round((result.confidence ?? 0) * 100)}% confident ·{' '}
          {result.elapsed_ms} ms
        </span>
      </div>
      <p className="pdf-trigger__line">
        {needing === 0
          ? 'Every page is readable without OCR.'
          : `${needing} of ${result.page_count} pages need OCR${
              reasons.size > 0 ? `: ${[...reasons].join('; ')}` : ''
            }.`}
      </p>
    </div>
  )
}

function MarkdownCard({ result }: { result: MarkdownResponse }) {
  const body = result.body
  if (!body) return null
  return (
    <div className="pdf-trigger">
      <div className="pdf-trigger__row">
        <Badge variant={body.truncated ? 'warn' : 'accent'}>
          {body.truncated ? 'truncated' : 'complete'}
        </Badge>
        <span className="pdf-trigger__file">{result.source}</span>
        <span className="pdf-trigger__meta">
          {result.pages_converted} of {result.page_count} pages · {result.elapsed_ms} ms
        </span>
      </div>
      <p className="pdf-trigger__line">
        {body.truncated
          ? `Returned ${format(body.chars)} of ${format(body.total_chars)} characters. The rest was not read.`
          : `${format(body.chars)} characters of markdown.`}
        {result.has_encoding_issues && ' Font encodings decoded badly; do not trust this text.'}
      </p>
    </div>
  )
}

function BodyCard({
  result,
  label,
}: {
  result: { body?: BodyLike; source?: string }
  label: string
}) {
  const body = result.body
  if (!body) return null
  return (
    <div className="pdf-trigger">
      <div className="pdf-trigger__row">
        <Badge variant={body.truncated ? 'warn' : 'accent'}>
          {body.truncated ? 'truncated' : 'complete'}
        </Badge>
        <span className="pdf-trigger__file">{result.source}</span>
      </div>
      <p className="pdf-trigger__line">
        {body.truncated
          ? `Returned ${format(body.chars)} of ${format(body.total_chars)} characters of ${label}.`
          : `${format(body.chars)} characters of ${label}.`}
      </p>
    </div>
  )
}

function ItemsCard({ result }: { result: ItemsLike }) {
  if (typeof result.count !== 'number') return null
  return (
    <div className="pdf-trigger">
      <div className="pdf-trigger__row">
        <Badge variant={result.truncated ? 'warn' : 'accent'}>
          {result.count} items
        </Badge>
        <span className="pdf-trigger__file">{result.source}</span>
      </div>
      {result.truncated && (
        <p className="pdf-trigger__line">
          {format(result.total_count ?? 0)} items on those pages; the rest were not returned.
        </p>
      )}
    </div>
  )
}

function RegionsCard({ result }: { result: RegionsLike }) {
  if (typeof result.region_count !== 'number') return null
  const suspect = result.regions_needing_ocr ?? 0
  return (
    <div className="pdf-trigger">
      <div className="pdf-trigger__row">
        <Badge variant={suspect === 0 ? 'accent' : 'warn'}>
          {result.region_count} regions
        </Badge>
        <span className="pdf-trigger__file">{result.source}</span>
      </div>
      {suspect > 0 && (
        <p className="pdf-trigger__line">
          {suspect} of {result.region_count} regions came back unreliable and would need OCR.
        </p>
      )}
    </div>
  )
}

function format(n: number): string {
  return n.toLocaleString('en-US')
}
