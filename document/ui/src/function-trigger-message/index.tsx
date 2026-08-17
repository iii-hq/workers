/**
 * How `document::*` calls render in chat and traces.
 *
 * Without this they render as raw JSON, and the one thing that matters — did
 * this file convert, and how much of it did the agent actually get — is buried
 * in it. Each renderer surfaces the decision, not the payload.
 *
 * Match narrowly and return `null` freely: `null` falls through to the
 * console's own cards, which already handle errors and pending approvals better
 * than a worker renderer should try to.
 */

import {
  Badge,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
} from '@iii-dev/console-ui'

import {
  formatLabel,
  type AssetsResponse,
  type DetectResponse,
  type MarkdownResponse,
} from '../lib/api'

const HANDLED = new Set([
  'document::detect',
  'document::to-markdown',
  'document::extract-assets',
])

export function createDocumentTriggerRenderer(_host: Host): FunctionTriggerRenderer {
  return {
    id: 'document/page.js#renderer',
    isMatch: (functionId) => HANDLED.has(functionId),
    tryRender: (message) => render(message),
    tryRenderPreview: (message) => render(message),
  }
}

/**
 * A function result reaches the console wrapped by the harness as
 * `{ content: [...], details: <the real response> }`, not as the response
 * itself. Reading the raw value looks like it works right up until every field
 * is undefined and the renderer quietly falls through to an empty card.
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
    case 'document::detect':
      return <DetectCard result={output as DetectResponse} />
    case 'document::to-markdown':
      return <MarkdownCard result={output as MarkdownResponse} />
    case 'document::extract-assets':
      return <AssetsCard result={output as AssetsResponse} />
    default:
      return null
  }
}

function DetectCard({ result }: { result: DetectResponse }) {
  if (typeof result.convertible !== 'boolean') return null
  return (
    <div className="document-trigger">
      <div className="document-trigger__row">
        <Badge variant={result.convertible ? 'accent' : 'warn'}>
          {result.format ? formatLabel(result.format) : 'unrecognised'}
        </Badge>
        <span className="document-trigger__file">{result.source}</span>
        <span className="document-trigger__meta">{result.elapsed_ms} ms</span>
      </div>
      <p className="document-trigger__line">
        {result.convertible
          ? `Convertible to markdown${result.detected_from === 'extension' ? ', recognised from the file name only' : ''}.`
          : 'Not a document this worker reads.'}
      </p>
    </div>
  )
}

function MarkdownCard({ result }: { result: MarkdownResponse }) {
  const body = result.body
  if (!body) return null
  return (
    <div className="document-trigger">
      <div className="document-trigger__row">
        <Badge variant={body.truncated ? 'warn' : 'accent'}>
          {result.format ? formatLabel(result.format) : 'document'}
        </Badge>
        <span className="document-trigger__file">{result.source}</span>
        <span className="document-trigger__meta">{result.elapsed_ms} ms</span>
      </div>
      <p className="document-trigger__line">
        {body.truncated
          ? `Returned ${count(body.chars)} of ${count(body.total_chars)} characters. The rest was not read.`
          : `${count(body.chars)} characters of markdown.`}
        {result.asset_count > 0 &&
          ` ${count(result.asset_count)} embedded image${result.asset_count === 1 ? '' : 's'} were not included; document::extract-assets returns them.`}
      </p>
    </div>
  )
}

function AssetsCard({ result }: { result: AssetsResponse }) {
  if (!Array.isArray(result.assets)) return null
  const withBytes = result.assets.filter((a) => a.bytes_base64).length
  return (
    <div className="document-trigger">
      <div className="document-trigger__row">
        <Badge variant={result.truncated ? 'warn' : 'accent'}>
          {result.assets.length} assets
        </Badge>
        <span className="document-trigger__file">{result.source}</span>
        <span className="document-trigger__meta">{result.elapsed_ms} ms</span>
      </div>
      <p className="document-trigger__line">
        {result.truncated
          ? `${count(result.total_count)} assets in the document; ${count(result.assets.length)} returned.`
          : `${count(withBytes)} of ${count(result.assets.length)} came back with their bytes.`}
      </p>
    </div>
  )
}

function count(n: number): string {
  return n.toLocaleString('en-US')
}
