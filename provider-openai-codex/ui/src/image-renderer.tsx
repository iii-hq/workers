/**
 * Injected function-trigger renderer for `provider::openai-codex::image::*`.
 *
 * `image::generate` deliberately returns no picture bytes: base64 in a
 * function result only inflates the conversation context. The preview the
 * transcript lacks is drawn here instead — the card asks the worker for a
 * downscaled JPEG through `image::read` over the tab's own bus client, so the
 * bytes travel worker → console and never through the model. A result that
 * already carries an image block (`inline: preview|full`, or an `image::read`
 * call) is shown as is.
 */

import {
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
  IconButton,
  Skeleton,
} from '@iii-dev/console-ui'
import { errorMessage, formatBytes, unwrapEnvelope } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, Copy, ImageOff } from 'lucide-react'
import { useEffect, useState } from 'react'

const PREFIX = 'provider::openai-codex::'
export const GENERATE_ID = `${PREFIX}image::generate`
export const READ_ID = `${PREFIX}image::read`

/** The `details` half of a generate/read result, as far as the card needs it. */
export interface ImageDetails {
  path: string
  model?: string
  hostModel?: string
  mime?: string
  bytes?: number
  width?: number
  height?: number
  revisedPrompt?: string
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value ? value : undefined
}

function num(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

/**
 * The function's `details` out of what the console hands us; null for errors
 * and foreign shapes. The harness wraps every tool result as
 * `{ content, details: <the function's whole return> }`, and this worker's
 * return is itself `{ content, details }` — so the facts sit two levels down
 * (`output.details.details.path`). A flat `{ path, … }` is accepted too.
 */
export function parseImageDetails(output: unknown): ImageDetails | null {
  const envelope = asRecord(output)
  if (!envelope || 'error' in envelope) return null
  let inner = unwrapEnvelope(output)
  if (typeof inner === 'string') {
    try {
      inner = JSON.parse(inner)
    } catch {
      return null
    }
  }
  let d = asRecord(inner)
  if (d && typeof d.path !== 'string' && asRecord(d.details)) d = asRecord(d.details)
  if (!d || typeof d.path !== 'string' || !d.path) return null
  return {
    path: d.path,
    model: str(d.model),
    hostModel: str(d.host_model),
    mime: str(d.mime),
    bytes: num(d.bytes),
    width: num(d.width),
    height: num(d.height),
    revisedPrompt: str(d.revised_prompt),
  }
}

function imageBlockUrl(blocks: unknown): string | null {
  if (!Array.isArray(blocks)) return null
  for (const block of blocks) {
    const b = asRecord(block)
    if (b?.type === 'image' && typeof b.data === 'string' && b.data) {
      const mime = typeof b.mime === 'string' && b.mime ? b.mime : 'image/png'
      return `data:${mime};base64,${b.data}`
    }
  }
  return null
}

/**
 * A data URL when the result already carries an image block — in the
 * harness-normalized `content`, or in the function's own `content` one level
 * down (a bare `image::read` return over the bus has only the latter).
 */
export function inlineImageUrl(output: unknown): string | null {
  const envelope = asRecord(output)
  if (!envelope) return null
  return imageBlockUrl(envelope.content) ?? imageBlockUrl(asRecord(envelope.details)?.content)
}

type PreviewState =
  | { status: 'loading' }
  | { status: 'ready'; src: string }
  | { status: 'error'; message: string }

function usePreview(host: Host, path: string, inline: string | null): PreviewState {
  const [state, setState] = useState<PreviewState>(() =>
    inline ? { status: 'ready', src: inline } : { status: 'loading' },
  )
  useEffect(() => {
    if (inline) {
      setState({ status: 'ready', src: inline })
      return
    }
    let cancelled = false
    setState({ status: 'loading' })
    host.iii
      .trigger<unknown>(READ_ID, { path, variant: 'preview' }, { timeoutMs: 20_000 })
      .then((result) => {
        if (cancelled) return
        const src = inlineImageUrl(result)
        setState(
          src
            ? { status: 'ready', src }
            : { status: 'error', message: 'The worker returned no preview for this file.' },
        )
      })
      .catch((err: unknown) => {
        if (!cancelled) setState({ status: 'error', message: errorMessage(err) })
      })
    return () => {
      cancelled = true
    }
  }, [host, path, inline])
  return state
}

function splitPath(path: string): { dir: string; name: string } {
  const cut = path.lastIndexOf('/')
  return cut < 0 ? { dir: '', name: path } : { dir: path.slice(0, cut + 1), name: path.slice(cut + 1) }
}

function PathLine({ path }: { path: string }) {
  const { dir, name } = splitPath(path)
  const { state, copy } = useCopyFlash(path)
  return (
    <div className="oai-image-path">
      <code className="oai-image-path-text" title={path}>
        {dir ? <span className="oai-image-path-dir">{dir}</span> : null}
        <span className="oai-image-path-name">{name}</span>
      </code>
      <IconButton
        label={state === 'copied' ? 'Path copied' : 'Copy path'}
        variant="ghost"
        onClick={copy}
      >
        {state === 'copied' ? <Check size={16} aria-hidden /> : <Copy size={16} aria-hidden />}
      </IconButton>
    </div>
  )
}

function ImageCard({
  host,
  details,
  inline,
}: {
  host: Host
  details: ImageDetails
  inline: string | null
}) {
  const preview = usePreview(host, details.path, inline)
  const dims = details.width && details.height ? `${details.width}×${details.height}` : null
  const facts = [
    details.model,
    dims,
    details.bytes != null ? formatBytes(details.bytes) : null,
    details.mime?.replace(/^image\//, ''),
  ].filter((fact): fact is string => Boolean(fact))
  return (
    <figure className="oai-image-card">
      <div className="oai-image-frame" data-state={preview.status}>
        {preview.status === 'loading' ? (
          <Skeleton className="oai-image-skeleton" aria-label="Loading preview" />
        ) : preview.status === 'ready' ? (
          <img
            className="oai-image-img"
            src={preview.src}
            alt={details.revisedPrompt ?? `image generated by ${details.model ?? 'OpenAI'}`}
          />
        ) : (
          <div className="oai-image-missing" role="status">
            <ImageOff size={16} aria-hidden />
            <span>{preview.message}</span>
          </div>
        )}
      </div>
      <figcaption className="oai-image-meta">
        {facts.length > 0 ? (
          <div className="oai-image-facts">
            {facts.map((fact) => (
              <span key={fact}>{fact}</span>
            ))}
          </div>
        ) : null}
        <PathLine path={details.path} />
        {details.revisedPrompt ? <p className="oai-image-revised">{details.revisedPrompt}</p> : null}
      </figcaption>
    </figure>
  )
}

function GeneratingCard({ input }: { input: unknown }) {
  const req = asRecord(input)
  const model = str(req?.model) ?? 'OpenAI'
  const prompt = str(req?.prompt)
  return (
    <figure className="oai-image-card" aria-busy>
      <div className="oai-image-frame" data-state="loading">
        <Skeleton className="oai-image-skeleton" aria-label="Generating image" />
      </div>
      <figcaption className="oai-image-meta">
        <div className="oai-image-facts">
          <span>generating with {model}…</span>
        </div>
        {prompt ? <p className="oai-image-revised">{prompt}</p> : null}
      </figcaption>
    </figure>
  )
}

/** Dims the namespace so the op (`image::generate`) reads first. */
export function FunctionIdLabel({ functionId }: { functionId: string }) {
  const [ns, op] = functionId.startsWith(PREFIX)
    ? [PREFIX, functionId.slice(PREFIX.length)]
    : ['', functionId]
  return (
    <>
      {ns ? <span className="oai-image-fn-ns">{ns}</span> : null}
      <span className="oai-image-fn-op">{op}</span>
    </>
  )
}

export function isImageFunction(functionId: string): boolean {
  return functionId === GENERATE_ID || functionId === READ_ID
}

function renderSettled(host: Host, message: FunctionTriggerMessage): React.ReactNode | null {
  if (
    !isImageFunction(message.functionId) ||
    message.pendingApproval ||
    message.running ||
    message.output == null
  ) {
    return null
  }
  const details = parseImageDetails(message.output)
  if (!details) return null // errors and foreign shapes: the host's view wins
  return <ImageCard host={host} details={details} inline={inlineImageUrl(message.output)} />
}

function renderRunning(message: FunctionTriggerMessage): React.ReactNode | null {
  if (message.functionId !== GENERATE_ID || message.pendingApproval || !message.running) return null
  return <GeneratingCard input={message.input} />
}

export function createImageRenderer(host: Host): FunctionTriggerRenderer {
  return {
    id: 'provider-openai-codex/page.js#image',
    isMatch: isImageFunction,
    tryRender: (message) => renderSettled(host, message),
    tryRenderRunning: renderRunning,
    tryRenderPreview: () => null,
    // The picture is the artifact: keep it visible when a call group collapses.
    tryRenderDisplay: (message) => renderSettled(host, message),
    FunctionIdLabel,
    metadata: { display: true },
  }
}
