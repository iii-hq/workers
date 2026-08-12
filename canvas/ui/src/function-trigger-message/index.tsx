/**
 * How `canvas::*` calls render in chat and traces.
 *
 * The flagship card (create/update) renders the mermaid diagram inline —
 * the agent drew something, so show the drawing, not the JSON around it.
 * Everything else compresses to the one fact that matters: what exists
 * (get/list), what's gone (delete), what the reference says (syntax), and
 * whether the source parses (validate).
 *
 * Match narrowly and return `null` freely: `null` falls through to the
 * console's own cards, which already handle raw JSON, errors, and pending
 * approvals better than a worker renderer should try to. Empty or
 * unrecognizable payloads always fall through — never an empty card.
 *
 * `@iii-dev/console-ui` is imported type-only on purpose: its runtime entry
 * throws by design (the real module arrives via the console's import map),
 * and keeping this module value-free of it lets vitest import the renderer
 * directly. Chips, pills, and tables are plain elements styled by the
 * chat-card section of ../../styles.css.
 */

import { useEffect, useState, type ReactNode } from 'react'

import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
  Host,
} from '@iii-dev/console-ui'

import { loadMermaid, mermaidInitConfig } from '../lib/loaders'
import { CANVAS_FUNCTION_IDS, unwrapEnvelope } from '../lib/types'
import {
  CANVAS_PREFIX,
  type CanvasRecordView,
  capList,
  errorDisplay,
  formatDay,
  hasContent,
  looksFreeform,
  mergeViews,
  parseDeleteResponse,
  parseListResponse,
  parseRecordView,
  parseSyntaxFamilies,
  parseValidateResponse,
  sceneElementCount,
} from './parsers'

/** The canvas page route (`host.pages.register({id: 'canvas'})` → `#/ext/canvas`). */
const CANVAS_PAGE_HASH = '#/ext/canvas'

export const HANDLED: ReadonlySet<string> = new Set(CANVAS_FUNCTION_IDS)

export function createCanvasTriggerRenderer(
  host: Host,
): FunctionTriggerRenderer {
  const render = (
    message: FunctionTriggerMessage,
    running: boolean,
  ): ReactNode | null => {
    if (!HANDLED.has(message.functionId)) return null
    // The host card owns the approve/deny bar; the preview slot is ours.
    if (message.pendingApproval) return null
    if (!running) {
      const error = errorDisplay(message.output)
      if (error != null) return <ErrorCard message={message} error={error} />
    }
    if (running) return renderRunning(host, message)
    return renderDone(host, message)
  }
  return {
    id: 'canvas/page.js#renderer',
    isMatch: (functionId) => HANDLED.has(functionId),
    tryRender: (message) => render(message, Boolean(message.running)),
    tryRenderRunning: (message) => render(message, true),
    tryRenderPreview: (message) => renderPreview(message),
    FunctionIdLabel,
    // redactRaw stays undefined: canvas payloads carry diagram source,
    // names, and slugs only — nothing secret for the raw tab to contain.
  }
}

function op(message: FunctionTriggerMessage): string {
  return message.functionId.slice(CANVAS_PREFIX.length)
}

function renderDone(
  host: Host,
  message: FunctionTriggerMessage,
): ReactNode | null {
  const output = unwrapEnvelope(message.output)
  switch (message.functionId) {
    case 'canvas::create':
    case 'canvas::update': {
      const view = mergeViews(
        parseRecordView(output),
        parseRecordView(message.input),
      )
      if (!hasContent(view)) return null
      return looksFreeform(view) ? (
        <FreeformCard op={op(message)} view={view} />
      ) : (
        <MermaidCard host={host} op={op(message)} view={view} />
      )
    }
    case 'canvas::get': {
      const view = parseRecordView(output)
      if (!hasContent(view)) return null
      return <GetCard view={view} />
    }
    case 'canvas::list': {
      const items = parseListResponse(output)
      if (items == null) return null
      return <ListCard items={items} />
    }
    case 'canvas::delete': {
      const result = parseDeleteResponse(output)
      if (result == null) return null
      return (
        <CardShell op="delete">
          <div className="canvas-trigger__note">
            {result.deleted
              ? `deleted ${result.id}`
              : `${result.id} not found — nothing deleted`}
          </div>
        </CardShell>
      )
    }
    case 'canvas::syntax': {
      const families = parseSyntaxFamilies(output)
      if (families == null) return null
      return <SyntaxCard families={families} />
    }
    case 'canvas::validate': {
      const result = parseValidateResponse(output)
      if (result == null) return null
      return <ValidateCard result={result} />
    }
    default:
      return null
  }
}

function renderRunning(
  host: Host,
  message: FunctionTriggerMessage,
): ReactNode | null {
  const view = parseRecordView(message.input)
  switch (message.functionId) {
    case 'canvas::create':
    case 'canvas::update':
      if (!hasContent(view)) return null
      if (view.source && !looksFreeform(view)) {
        return (
          <MermaidCard host={host} op={op(message)} view={view} running />
        )
      }
      if (looksFreeform(view)) {
        return <FreeformCard op={op(message)} view={view} running />
      }
      return (
        <CardShell op={op(message)} running head={<HeadMeta view={view} />}>
          <RunningNote />
        </CardShell>
      )
    case 'canvas::get':
    case 'canvas::delete':
      if (!view.id) return null
      return (
        <CardShell op={op(message)} running>
          <div className="canvas-trigger__note">
            <span className="canvas-trigger__id">{view.id}</span>
          </div>
          <RunningNote />
        </CardShell>
      )
    default:
      // list/syntax/validate run in milliseconds and carry thin inputs;
      // the host's request pane says everything a shimmer would.
      return null
  }
}

/** Pending-approval preview: the diagram a call is about to write. */
function renderPreview(message: FunctionTriggerMessage): ReactNode | null {
  if (!HANDLED.has(message.functionId)) return null
  const view = parseRecordView(message.input)
  switch (message.functionId) {
    case 'canvas::create':
    case 'canvas::update': {
      if (!hasContent(view)) return null
      const lines = view.source ? view.source.split('\n').length : null
      return (
        <CardShell op={op(message)} head={<HeadMeta view={view} />}>
          {lines != null ? (
            <div className="canvas-trigger__note">
              {lines} line{lines === 1 ? '' : 's'} of{' '}
              {looksFreeform(view) ? 'freeform scene' : 'mermaid'} source
            </div>
          ) : null}
        </CardShell>
      )
    }
    case 'canvas::delete':
      if (!view.id) return null
      return (
        <CardShell op="delete">
          <div className="canvas-trigger__note">
            about to delete{' '}
            <span className="canvas-trigger__id">{view.id}</span>
          </div>
        </CardShell>
      )
    default:
      return null
  }
}

/* ── cards ──────────────────────────────────────────────────────────── */

function CardShell({
  op: opLabel,
  running,
  head,
  children,
}: {
  op: string
  running?: boolean
  head?: ReactNode
  children?: ReactNode
}) {
  return (
    <div className="canvas-trigger">
      <div className="canvas-trigger__head">
        <span
          className={`canvas-trigger__pill${running ? ' canvas-trigger__pill--quiet' : ''}`}
        >
          {opLabel}
        </span>
        {head}
        <span className="canvas-trigger__tag">canvas ui</span>
      </div>
      {children}
    </div>
  )
}

function HeadMeta({ view }: { view: CanvasRecordView }) {
  const chip = view.family ?? view.format
  return (
    <>
      {view.name ? (
        <span className="canvas-trigger__name">{view.name}</span>
      ) : null}
      {chip ? <span className="canvas-trigger__chip">{chip}</span> : null}
      {view.id ? <span className="canvas-trigger__id">{view.id}</span> : null}
    </>
  )
}

function OpenLink() {
  return (
    <a className="canvas-trigger__open" href={CANVAS_PAGE_HASH}>
      open in canvas
    </a>
  )
}

function RunningNote() {
  return (
    <div className="canvas-trigger__note canvas-trigger__pulse">running…</div>
  )
}

/** The flagship card: the diagram itself, rendered inline. */
function MermaidCard({
  host,
  op: opLabel,
  view,
  running,
}: {
  host: Host
  op: string
  view: CanvasRecordView
  running?: boolean
}) {
  return (
    <CardShell
      op={opLabel}
      running={running}
      head={
        <>
          <HeadMeta view={view} />
          <OpenLink />
        </>
      }
    >
      {view.source ? (
        <MermaidDiagram host={host} source={view.source} />
      ) : (
        <div className="canvas-trigger__note">no source in this payload</div>
      )}
      {running ? <RunningNote /> : null}
    </CardShell>
  )
}

let renderSeq = 0

/**
 * Lazy inline mermaid render: the vendor bundle loads on the first card,
 * strict + suppressErrorRendering so invalid source becomes a caught error
 * (shown over the source in a <pre>) instead of mermaid's bomb SVG. The
 * whole diagram is a link to the canvas page.
 */
function MermaidDiagram({ host, source }: { host: Host; source: string }) {
  const theme = host.useTheme()
  const [state, setState] = useState<
    | { status: 'loading' }
    | { status: 'done'; svg: string }
    | { status: 'error'; message: string }
  >({ status: 'loading' })

  useEffect(() => {
    let alive = true
    setState({ status: 'loading' })
    loadMermaid(host)
      .then(async ({ mermaid }) => {
        mermaid.initialize(mermaidInitConfig(theme))
        const { svg } = await mermaid.render(
          `canvas-trigger-${++renderSeq}`,
          source,
        )
        if (alive) setState({ status: 'done', svg })
      })
      .catch((error: unknown) => {
        if (alive) {
          setState({
            status: 'error',
            message: error instanceof Error ? error.message : String(error),
          })
        }
      })
    return () => {
      alive = false
    }
  }, [host, source, theme])

  if (state.status === 'loading') {
    return (
      <div className="canvas-trigger__diagram canvas-trigger__diagram--loading canvas-trigger__pulse">
        rendering…
      </div>
    )
  }
  if (state.status === 'error') {
    return (
      <div className="canvas-trigger__fail">
        <div className="canvas-trigger__error">{state.message}</div>
        <pre className="canvas-trigger__source">{source}</pre>
      </div>
    )
  }
  return (
    <a
      className="canvas-trigger__diagram"
      href={CANVAS_PAGE_HASH}
      title="open in canvas"
      // biome-ignore lint/security/noDangerouslySetInnerHtml: mermaid output under securityLevel 'strict'
      dangerouslySetInnerHTML={{ __html: state.svg }}
    />
  )
}

/** Freeform canvases stay compact in chat — excalidraw never loads here. */
function FreeformCard({
  op: opLabel,
  view,
  running,
}: {
  op: string
  view: CanvasRecordView
  running?: boolean
}) {
  const count = sceneElementCount(view.source)
  return (
    <CardShell
      op={opLabel}
      running={running}
      head={
        <>
          <HeadMeta view={view} />
          <OpenLink />
        </>
      }
    >
      <div className="canvas-trigger__note">
        freeform canvas
        {count != null ? ` · ${count} element${count === 1 ? '' : 's'}` : ''}
      </div>
      {running ? <RunningNote /> : null}
    </CardShell>
  )
}

function GetCard({ view }: { view: CanvasRecordView }) {
  const day = formatDay(view.updated_at)
  return (
    <CardShell
      op="get"
      head={
        <>
          <HeadMeta view={view} />
          <OpenLink />
        </>
      }
    >
      <div className="canvas-trigger__note">
        {view.format ?? 'canvas'}
        {view.family ? ` · ${view.family}` : ''}
        {day ? ` · updated ${day}` : ''}
      </div>
    </CardShell>
  )
}

function ListCard({ items }: { items: CanvasRecordView[] }) {
  if (items.length === 0) {
    return (
      <CardShell op="list" head={<OpenLink />}>
        <div className="canvas-trigger__note">no canvases yet</div>
      </CardShell>
    )
  }
  const { shown, hidden } = capList(items)
  return (
    <CardShell op="list" head={<OpenLink />}>
      <div className="canvas-trigger__table-wrap">
        <table className="canvas-trigger__table">
          <thead>
            <tr>
              <th>name</th>
              <th>family</th>
              <th>id</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((item, index) => (
              <tr key={item.id ?? index}>
                <td>{item.name ?? '—'}</td>
                <td>{item.family ?? item.format ?? '—'}</td>
                <td className="canvas-trigger__id">{item.id ?? '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {hidden > 0 ? (
          <div className="canvas-trigger__note">+{hidden} more</div>
        ) : null}
      </div>
    </CardShell>
  )
}

function SyntaxCard({ families }: { families: string[] }) {
  return (
    <CardShell op="syntax">
      {families.length === 0 ? (
        <div className="canvas-trigger__note">no families returned</div>
      ) : (
        <div className="canvas-trigger__chips">
          {families.map((family) => (
            <span key={family} className="canvas-trigger__chip">
              {family}
            </span>
          ))}
        </div>
      )}
    </CardShell>
  )
}

function ValidateCard({
  result,
}: {
  result: {
    valid: boolean
    family: string | null
    issues: { line: number | null; message: string }[]
  }
}) {
  return (
    <CardShell
      op="validate"
      head={
        result.family ? (
          <span className="canvas-trigger__chip">{result.family}</span>
        ) : undefined
      }
    >
      {result.valid ? (
        <div className="canvas-trigger__ok">✓ valid</div>
      ) : (
        <div className="canvas-trigger__issues">
          {result.issues.length === 0 ? (
            <div className="canvas-trigger__issue">
              <span className="canvas-trigger__issue-line">—</span>
              <span>invalid source</span>
            </div>
          ) : (
            result.issues.map((issue, index) => (
              <div key={index} className="canvas-trigger__issue">
                <span className="canvas-trigger__issue-line">
                  {issue.line != null ? `line ${issue.line}` : '—'}
                </span>
                <span>{issue.message}</span>
              </div>
            ))
          )}
        </div>
      )}
    </CardShell>
  )
}

function ErrorCard({
  message,
  error,
}: {
  message: FunctionTriggerMessage
  error: string
}) {
  const source = parseRecordView(message.input).source
  return (
    <CardShell op={op(message)}>
      <div className="canvas-trigger__error">{error}</div>
      {source ? (
        <pre className="canvas-trigger__source">{source}</pre>
      ) : null}
    </CardShell>
  )
}

function FunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith(CANVAS_PREFIX)) {
    return <span style={{ color: 'var(--color-ink)' }}>{functionId}</span>
  }
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>{CANVAS_PREFIX}</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>
        {functionId.slice(CANVAS_PREFIX.length)}
      </span>
    </>
  )
}
