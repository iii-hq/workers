import { Check, ChevronDown, CircleAlert, Copy, Loader2 } from 'lucide-react'
import {
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { DrawnCheck } from '@/components/chat/ChatGlyphs'
import { CopyMessageButton } from '@/components/chat/CopyMessageButton'
import { TimelineActivityTrail } from '@/components/chat/TimelineActivityTrail'
import {
  firstNonNull,
  firstRendered,
  rawRedactor,
  useFunctionTriggerRenderers,
} from '@/components/function-trigger/renderer-registry'
import {
  errorSubtitle,
  formatRowDuration,
  resultSummary,
} from '@/components/function-trigger/row-summary'
import {
  ChromiumMissingAction,
  isChromiumMissingCall,
} from '@/components/onboarding/ChromiumMissingAction'
import { AlwaysAllowButton } from '@/components/permissions/AlwaysAllowButton'
import {
  type FilesystemAccessAction,
  FilesystemAccessPrompt,
} from '@/components/permissions/FilesystemAccessPrompt'
import { Button } from '@/components/ui/Button'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@/components/ui/CollapsibleCard'
import { Hint } from '@/components/ui/Hint'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { useMediaQuery } from '@/hooks/use-media-query'
import { copyTextToClipboard } from '@/lib/clipboard'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type { FunctionTriggerMessage as FunctionTriggerMessageType } from '@/types/chat'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * FunctionTriggerCard — the canonical rendering of one iii function trigger:
 * one row of a tool group (ƒ marker, title, settled facts, status glyph,
 * chevron) that opens to the per-family tool view or the request/response
 * panes, the function identity strip, and the optional approval bar.
 *
 * Location-agnostic by design: it is props-only (no chat store, no session
 * context). Chat renders it from live session messages; TracesV2 renders it
 * in the span info tab by synthesizing a `FunctionTriggerMessage` from an OTel
 * span (`pages/TracesV2/lib/functionTriggerFromSpan.ts`); any other surface can
 * do the same — `FunctionTriggerMessage` (types/chat.ts) is a plain data shape
 * whose base is just `{ id, createdAt }`.
 */
interface FunctionTriggerCardProps {
  message: FunctionTriggerMessageType
  defaultOpen?: boolean
  /**
   * Hydrate historical arguments/results when an unloaded card opens. May
   * resolve to whether the read succeeded; `false` swaps the skeleton for a
   * retry row instead of leaving the card loading forever.
   */
  onLoadDetails?: () => Promise<boolean> | undefined
  /**
   * Approve handler. May be sync or async; the component shows a
   * `submitting…` state while the promise resolves and a red error row
   * if it rejects. Wire the actual `approval::resolve` call here.
   */
  onApprove?: () => void | Promise<void>
  onDeny?: () => void | Promise<void>
  /**
   * Approve + add to per-conversation always-allow list. When provided,
   * an "always allow" button renders next to approve/deny. Destructive
   * function ids gate on a confirmation modal inside the button.
   */
  onAlwaysAllow?: () => void | Promise<void>
  /**
   * Resolve a filesystem-access grant request (see `message.filesystemAccess`).
   * When set alongside `message.filesystemAccess`, replaces the standard
   * approve/deny/always row with `FilesystemAccessPrompt`.
   */
  onResolveFilesystemAccess?: (
    action: FilesystemAccessAction,
  ) => void | Promise<void>
  /** Opens the filesystem-access management dialog (§5 of the spec). */
  onManageFilesystemAccess?: () => void
  /** Conversation's session workspace — shown as "always allowed" context. */
  workingDir?: string | null
}

/**
 * Soft failures ride on `fcall-end.output` as `{ error: { kind, ... } }`
 * (see PLAYGROUND.md "Error semantics"). The shape isn't typed beyond
 * `unknown`, so this guard stays narrow on purpose.
 */
export function isErrorOutput(v: unknown): boolean {
  return (
    !!v &&
    typeof v === 'object' &&
    !Array.isArray(v) &&
    'error' in (v as Record<string, unknown>)
  )
}

/**
 * A deny/timeout resolution from the approval gate: the call was stopped at
 * the gate and never executed. The gate's structured DenialEnvelope
 * (approval-gate `denial.rs` — `{ status: "denied", denied_by, reason, … }`)
 * rides in the result's `details`, which `functionResultOutput` preserves
 * under `error.details` on both the live pairing and the reload path — so
 * this is detectable from the output alone, unlike a genuine run error.
 */
export function isDeniedOutput(v: unknown): boolean {
  if (!isErrorOutput(v)) return false
  const details = (v as { error?: { details?: unknown } }).error?.details
  return (
    !!details &&
    typeof details === 'object' &&
    !Array.isArray(details) &&
    (details as Record<string, unknown>).status === 'denied' &&
    'denied_by' in (details as Record<string, unknown>)
  )
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

/**
 * Parse a string that IS a JSON object/array — a double-encoded payload
 * (e.g. a model passing `payload` as a stringified object). Scalars stay
 * strings on purpose; only structure benefits from re-rendering.
 */
export function parseEmbeddedJson(s: string): unknown | undefined {
  const t = s.trim()
  if (!t.startsWith('{') && !t.startsWith('[')) return undefined
  try {
    return JSON.parse(t) as unknown
  } catch {
    return undefined
  }
}

/**
 * The iii function-result envelope: `{ content: [{type:"text", text}...],
 * details? }`. Rendered raw, `content[].text` shows as an escaped one-line
 * JSON string that usually duplicates `details` — so the pane unwraps it:
 * texts render as text (or parsed JSON), details as its own section.
 * Returns null for anything else (extra keys, non-text blocks) so unknown
 * shapes keep the truthful raw rendering.
 */
export function resultEnvelope(
  v: unknown,
): { texts: string[]; details: unknown } | null {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return null
  const o = v as Record<string, unknown>
  if (!Array.isArray(o.content)) return null
  if (Object.keys(o).some((k) => k !== 'content' && k !== 'details'))
    return null
  const texts: string[] = []
  for (const block of o.content) {
    if (!block || typeof block !== 'object') return null
    const b = block as Record<string, unknown>
    if (b.type !== 'text' || typeof b.text !== 'string') return null
    texts.push(b.text)
  }
  if (texts.length === 0 && isEmptyValue(o.details)) return null
  return { texts, details: o.details }
}

type Primitive = string | number | boolean | null

function isPrimitive(v: unknown): v is Primitive {
  return (
    v === null ||
    typeof v === 'string' ||
    typeof v === 'number' ||
    typeof v === 'boolean'
  )
}

/**
 * `null`, `undefined`, `""`, `[]`, and `{}` count as empty so we can render a
 * compact "· empty" header instead of a noisy `{}` JSON block. `null` is
 * intentionally treated as empty (the user-facing concept is "no input"); the
 * primitive `null` rendering would otherwise show the literal text "null".
 */
function isEmptyValue(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'string') return v.length === 0
  if (Array.isArray(v)) return v.length === 0
  if (typeof v === 'object') {
    return Object.keys(v as Record<string, unknown>).length === 0
  }
  return false
}

function singlePrimitiveField(
  v: unknown,
): { key: string; value: Primitive } | null {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return null
  const entries = Object.entries(v as Record<string, unknown>)
  if (entries.length !== 1) return null
  const [key, value] = entries[0]
  if (!isPrimitive(value)) return null
  return { key, value }
}

function formatPrimitive(v: Primitive): string {
  if (v === null) return 'null'
  return String(v)
}

/**
 * Branch the function-id label across the renderer registry — injected
 * renderers first, then the first-party families. The default
 * (unbranded) span keeps unknown ids readable.
 */
function FunctionIdLabel({ functionId }: { functionId: string }) {
  const renderers = useFunctionTriggerRenderers()
  for (const renderer of renderers) {
    const Label = renderer.FunctionIdLabel
    if (Label && renderer.isMatch(functionId)) {
      return <Label functionId={functionId} />
    }
  }
  return <span className="text-ink">{functionId}</span>
}

type FunctionTriggerStatus = 'pending' | 'running' | 'error' | 'done'

interface RendererLayer {
  key: string
  node: ReactNode
}

interface RendererSwapState {
  outgoing: RendererLayer | null
  target: 'terminal' | 'display'
}

/**
 * A worker may expose a full running terminal and a compact settled display.
 * Keep the outgoing presentation for one state-swap while the incoming node
 * mounts in the same host-owned slot. Renderers that keep the same
 * presentation key (FileChangesCard, for example) reconcile normally and
 * preserve their local state instead of being remounted.
 */
function FunctionRendererPresence({
  children,
  presentation,
}: {
  children: ReactNode
  presentation: 'terminal' | 'display'
}) {
  const reducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')
  const committedLayerRef = useRef<RendererLayer>({
    key: presentation,
    node: children,
  })
  const [swapState, setSwapState] = useState<RendererSwapState>(() => ({
    outgoing: null,
    target: presentation,
  }))
  let renderedSwapState = swapState

  // React explicitly supports adjusting a component's own state during
  // render. Unlike mutating refs here, this update is discarded if a
  // concurrent render is abandoned; committedLayerRef always describes the
  // presentation users actually saw.
  if (swapState.target !== presentation) {
    renderedSwapState = {
      target: presentation,
      outgoing: reducedMotion ? null : committedLayerRef.current,
    }
    setSwapState(renderedSwapState)
  }

  useLayoutEffect(() => {
    committedLayerRef.current = { key: presentation, node: children }
  }, [children, presentation])

  useEffect(() => {
    if (!renderedSwapState.outgoing) return
    const swapDuration = Number.parseFloat(
      getComputedStyle(document.documentElement).getPropertyValue(
        '--duration-quick',
      ),
    )
    const timer = window.setTimeout(
      () =>
        setSwapState((current) =>
          current.target === presentation
            ? { ...current, outgoing: null }
            : current,
        ),
      reducedMotion ? 0 : Number.isFinite(swapDuration) ? swapDuration : 150,
    )
    return () => window.clearTimeout(timer)
  }, [presentation, reducedMotion, renderedSwapState.outgoing])

  return (
    <div
      className="function-renderer-presence"
      data-transitioning={Boolean(renderedSwapState.outgoing)}
    >
      {renderedSwapState.outgoing ? (
        <div
          key={renderedSwapState.outgoing.key}
          className="function-renderer-presence__layer"
          data-layer="outgoing"
          aria-hidden="true"
          inert
        >
          {renderedSwapState.outgoing.node}
        </div>
      ) : null}
      <div
        key={presentation}
        className="function-renderer-presence__layer"
        data-layer="current"
      >
        {children}
      </div>
    </div>
  )
}

/**
 * All status glyphs occupy one persistent slot. A state attribute changes
 * which layer is visible, giving CSS both the outgoing and incoming glyphs
 * for the spinner -> check/error handoff without delaying the real state.
 */
function FunctionTriggerStatusIcon({
  status,
}: {
  status: FunctionTriggerStatus
}) {
  return (
    <span
      aria-hidden="true"
      className="activity-status-icon chat-tool-row__status"
      data-status={status}
    >
      <span data-activity-status-layer="pending">
        <StatusDot tone="warn" />
      </span>
      <span data-activity-status-layer="running">
        <Loader2
          strokeWidth={2}
          className={cn(
            'size-4 stroke-ink-faint motion-reduce:animate-none',
            status === 'running' && 'animate-spin',
          )}
        />
      </span>
      <span data-activity-status-layer="error">
        <CircleAlert strokeWidth={2} className="size-4 stroke-alert" />
      </span>
      <span data-activity-status-layer="done">
        <DrawnCheck active={status === 'done'} className="size-4 text-ok" />
      </span>
    </span>
  )
}

interface FunctionTriggerRowTextProps {
  description?: string
  functionId: string
  preview: string | null
  status: FunctionTriggerStatus
  /** Why the row needs attention or failed; wins over the args digest. */
  subtitle: string | null
  unresolvedTarget: boolean
}

/**
 * The row's title and its optional second line. The title is the agent's
 * short description when it gave one, else the function id; it never
 * changes as the call settles, so only the line under it and the status at
 * the trailing edge move. The second line opens its own height track, so a
 * failure reason can arrive under a title already on screen.
 */
function FunctionTriggerRowText({
  description,
  functionId,
  preview,
  status,
  subtitle,
  unresolvedTarget,
}: FunctionTriggerRowTextProps) {
  const settled = status === 'done' || status === 'error'
  const line = subtitle ?? (!description && settled ? preview : null)
  const mono = subtitle === null && line !== null
  return (
    <div className="chat-tool-row__text">
      <div className="chat-tool-row__title">
        <span
          className={cn(status === 'running' && 'function-trigger-shimmer')}
        >
          {description ? (
            description
          ) : status === 'running' && unresolvedTarget ? (
            <span className="text-ink-faint">…</span>
          ) : (
            <FunctionIdLabel functionId={functionId} />
          )}
        </span>
      </div>
      <div
        className="chat-tool-row__subtitle"
        data-visible={line !== null}
        aria-hidden={line === null}
      >
        <span data-mono={mono || undefined} title={line ?? undefined}>
          {line}
        </span>
      </div>
    </div>
  )
}

/** `4 results · 1.2s` once a call has settled; nothing while it runs. */
function FunctionTriggerRowMeta({
  durationMs,
  output,
  status,
}: {
  durationMs?: number
  output: unknown
  status: FunctionTriggerStatus
}) {
  if (status !== 'done' && status !== 'error') return null
  const parts = [
    status === 'done' ? resultSummary(output) : null,
    typeof durationMs === 'number' ? formatRowDuration(durationMs) : null,
  ].filter((part): part is string => part !== null)
  if (parts.length === 0) return null
  return <span className="chat-tool-row__meta">{parts.join(' · ')}</span>
}

/**
 * The strip under an opened call: the concrete function identity (the row
 * title is usually the agent's description) with a copy affordance, and,
 * when a worker's own view stands in for the payload, a toggle that opens
 * the raw request and response. The raw panes stay mounted behind a
 * zero-height track so the toggle animates and nothing re-renders.
 */
function FunctionTriggerRowFooter({
  functionId,
  raw,
}: {
  functionId: string
  raw?: { input: unknown; output: unknown }
}) {
  const [showRaw, setShowRaw] = useState(false)
  const rawId = useId()
  return (
    <div className="chat-tool-row__foot" data-function-trigger-footer="">
      <div className="chat-tool-row__foot-bar">
        <span className="chat-tool-row__fn">
          <span className="text-accent italic font-semibold">ƒ</span>{' '}
          <FunctionIdLabel functionId={functionId} />
        </span>
        <CopyMessageButton
          text={functionId}
          label="copy function id"
          className="chat-tool-row__fn-copy shrink-0"
        />
        {raw ? (
          <button
            type="button"
            aria-expanded={showRaw}
            aria-controls={rawId}
            onClick={() => setShowRaw((value) => !value)}
            className="chat-tool-row__raw-toggle"
            data-message-action="toggle-raw"
          >
            {showRaw ? 'Hide raw' : 'Show raw'}
          </button>
        ) : null}
      </div>
      {raw ? (
        <div
          id={rawId}
          className="chat-tool-row__raw"
          data-visible={showRaw}
          aria-hidden={!showRaw}
          inert={showRaw ? undefined : true}
        >
          <div className="chat-tool-row__raw-inner">
            <ValuePane label="request" value={raw.input} />
            <ValuePane label="response" value={raw.output} stacked />
          </div>
        </div>
      ) : null}
    </div>
  )
}

/**
 * One-line `key: value` digest of the request args for a settled row with
 * no description — what separates three calls to the same function without
 * expanding them. Long strings truncate, nested values collapse to `[N]` /
 * `{…}`.
 */
function argsPreview(input: unknown): string | null {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return null
  const entries = Object.entries(input as Record<string, unknown>).filter(
    ([k]) => k !== '_streaming',
  )
  if (entries.length === 0) return null
  const fmt = (v: unknown): string => {
    if (typeof v === 'string')
      return JSON.stringify(v.length > 40 ? `${v.slice(0, 40)}…` : v)
    if (v === null || typeof v === 'number' || typeof v === 'boolean')
      return String(v)
    if (Array.isArray(v)) return `[${v.length}]`
    return '{…}'
  }
  return entries.map(([k, v]) => `${k}: ${fmt(v)}`).join(', ')
}

export function FunctionTriggerCard({
  message,
  defaultOpen,
  onLoadDetails,
  onApprove,
  onDeny,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
}: FunctionTriggerCardProps) {
  const pending = !!message.pendingApproval
  const running = !!message.running
  // A placeholder from a paged transcript read: the header knows the function
  // and the description, the arguments and result are still on the server.
  // No renderer is consulted — there is nothing to render yet — and the body
  // is a skeleton until the caller fetches the whole entry.
  const unloaded = !!message.unloaded && !pending && !running
  // Registry-dispatched custom panes: injected renderers first, then the
  // first-party families, then the JSON fallback below. First non-null
  // wins; null falls through.
  const renderers = useFunctionTriggerRenderers()
  // The raw request/response as this card is allowed to show them. An
  // injected renderer that claims this function id may declare `redactRaw`
  // (a runtime id is a capability, so sandbox-code-runner does) — apply it
  // ONCE here, then use `rawInput`/`rawOutput` everywhere below: every pane
  // derives both its body and its copy text from the value it is handed, so
  // redacting at the source covers the clipboard too. Card LOGIC keeps
  // reading `message.*` — redaction is a display concern, not a semantic one.
  // Memoized because `redactRaw` deep-walks the payload and this runs for
  // every card of a claimed function id, collapsed ones included.
  const { rawInput, rawOutput } = useMemo(() => {
    const redact = rawRedactor(renderers, message.functionId)
    return redact
      ? { rawInput: redact(message.input), rawOutput: redact(message.output) }
      : { rawInput: message.input, rawOutput: message.output }
  }, [renderers, message.functionId, message.input, message.output])
  // Raw in-flight arguments tail (`_streaming`, injected beside the harness's
  // incrementally reconstructed fields) — rendered as a live pane.
  const streamingTail =
    running &&
    rawInput &&
    typeof rawInput === 'object' &&
    typeof (rawInput as { _streaming?: unknown })._streaming === 'string'
      ? (rawInput as { _streaming: string })._streaming
      : undefined
  const filesystemAccess = pending ? message.filesystemAccess : undefined
  const [open, setOpen] = useState(!!defaultOpen || pending)
  const [loadFailed, setLoadFailed] = useState(false)
  // Once per open, not per parent render (the parent hands over a fresh
  // callback each render) and not per id: a hydration that lands the call
  // but not its result re-derives the block id, and that is not a retry.
  // Closing the card clears the latch, so reopening is a retry; so is the
  // failure row's button. The same path hydrates default-open cards too.
  // The latch is the request itself, so only the read this open is still
  // waiting on may report its failure.
  const requestedDetailsRef = useRef<symbol | null>(null)
  const loadDetails = () => {
    if (!onLoadDetails) return
    const request = Symbol('load-details')
    requestedDetailsRef.current = request
    setLoadFailed(false)
    Promise.resolve(onLoadDetails())
      .catch(() => false)
      .then((ok) => {
        if (ok === false && requestedDetailsRef.current === request)
          setLoadFailed(true)
      })
  }
  // biome-ignore lint/correctness/useExhaustiveDependencies: `loadDetails` reads the current callback; the latch decides whether it runs.
  useEffect(() => {
    if (!open || !unloaded) {
      requestedDetailsRef.current = null
      setLoadFailed(false)
      return
    }
    if (requestedDetailsRef.current) return
    loadDetails()
  }, [open, unloaded, onLoadDetails])
  // Closed calls read as one line of the group. Opening one reveals the
  // payload underneath; pending approvals open themselves because they
  // require action.
  const expandedSurface = open || pending
  const [submitting, setSubmitting] = useState<
    'approve' | 'deny' | 'always_allow' | null
  >(null)
  const [submitError, setSubmitError] = useState<string | null>(null)

  const customPreview = unloaded
    ? null
    : firstNonNull(renderers, (r) =>
        r.isMatch(message.functionId)
          ? (r.tryRenderPreview?.(message) ?? null)
          : null,
      )
  const terminalRender =
    !pending && !unloaded
      ? firstRendered(renderers, (r) =>
          r.isMatch(message.functionId)
            ? running
              ? (r.tryRenderRunning ?? r.tryRender)(message)
              : r.tryRender(message)
            : null,
        )
      : null
  const customTerminal = terminalRender?.node ?? null
  const hasCustomTerminal = customTerminal != null
  const displayCustomTerminal =
    !pending &&
    hasCustomTerminal &&
    terminalRender?.renderer.metadata?.display === true
  const customDisplay = displayCustomTerminal
    ? (terminalRender?.renderer.tryRenderDisplay?.(message) ?? null)
    : null
  const displayAction = terminalRender?.renderer.metadata?.displayAction
  const expandableCustomDisplay =
    displayCustomTerminal && displayAction === 'expand'
  // The top request pane renders only while the call is in flight and no
  // richer view covers it; the settled (done) branch below renders its own
  // request/response panes, so showing it there would duplicate the pane.
  const showRequestPaneAbove = pending
    ? !customPreview
    : running
      ? !hasCustomTerminal && streamingTail === undefined
      : false

  const runResolve = async (kind: 'approve' | 'deny' | 'always_allow') => {
    const handler =
      kind === 'approve' ? onApprove : kind === 'deny' ? onDeny : onAlwaysAllow
    if (!handler || submitting) return
    setSubmitError(null)
    setSubmitting(kind)
    try {
      await handler()
      // Leave `submitting` set; the message patches once the resurrected
      // execution emits real events (pendingApproval flips off, output
      // arrives), at which point this whole pending block stops rendering.
    } catch (err) {
      setSubmitting(null)
      setSubmitError(err instanceof Error ? err.message : String(err))
    }
  }

  useEffect(() => {
    if (pending) setOpen(true)
  }, [pending])

  const errored = !pending && !running && isErrorOutput(message.output)
  // A browser call that failed for want of Chromium offers the fix inline.
  const chromiumMissing =
    errored && isChromiumMissingCall(message.functionId, message.output)
  // `rawInput`, not `message.input`: the collapsed row digests the request
  // args inline, so it is a display exit like the raw pane and the clipboard —
  // a claimed card's `redactRaw` has to cover it or a secret shows up in the
  // one line that renders without anyone expanding the card.
  const preview = argsPreview(rawInput)
  const description = message.description?.trim() || undefined
  const status: FunctionTriggerStatus = pending
    ? 'pending'
    : running
      ? 'running'
      : errored
        ? 'error'
        : 'done'
  // The line under the title: what the row needs from the reader, or why
  // it failed. A gate denial reads as a denial, not as a failed execution.
  const subtitle = pending
    ? filesystemAccess
      ? 'Needs filesystem access to run'
      : 'Waiting for your approval'
    : errored
      ? errorSubtitle(rawOutput)
      : null
  const collapsedActivityKind =
    !expandedSurface && message.functionId === 'engine::register_trigger'
      ? 'trigger-registration'
      : 'function'
  // The identity strip names the concrete function. While an agent_trigger
  // wrapper's target is still streaming there is nothing concrete to name.
  const showFooter = !pending && !(running && message.unresolvedTarget)
  const rowText = (
    <FunctionTriggerRowText
      description={description}
      functionId={message.functionId}
      preview={preview}
      status={status}
      subtitle={subtitle}
      unresolvedTarget={!!message.unresolvedTarget}
    />
  )
  const rowMeta = (
    <FunctionTriggerRowMeta
      durationMs={message.durationMs}
      output={rawOutput}
      status={status}
    />
  )

  if (expandableCustomDisplay) {
    const detailsOpen = running || open

    return (
      <section
        className="function-trigger-surface chat-tool-row"
        data-message-id={message.id}
        data-message-role="function-call"
        data-function-id={message.functionId}
        data-expanded={detailsOpen}
        data-function-status={status}
      >
        <CollapsibleCard
          open={detailsOpen}
          onOpenChange={setOpen}
          disabled={running}
          className="@container function-trigger-active-collapsible chat-tool-row__collapsible"
        >
          <CollapsibleCardTrigger
            className="p-3 select-none"
            aria-label={`${detailsOpen ? 'Hide' : 'Show'} details for ${message.functionId}`}
          >
            <FunctionRendererPresence
              presentation={customDisplay != null ? 'display' : 'terminal'}
            >
              {customDisplay ?? (
                <div className="flex min-w-0 items-center gap-2.5">
                  <TimelineActivityTrail kind="function" />
                  {rowText}
                  {rowMeta}
                  <FunctionTriggerStatusIcon status={status} />
                </div>
              )}
            </FunctionRendererPresence>
          </CollapsibleCardTrigger>
          <CollapsibleCardContent>
            <div className="chat-tool-row__body">
              <div data-function-renderer-details-slot="" className="p-3">
                {customTerminal}
              </div>
              <FunctionTriggerRowFooter
                functionId={message.functionId}
                raw={{ input: rawInput, output: rawOutput }}
              />
            </div>
          </CollapsibleCardContent>
        </CollapsibleCard>
        {chromiumMissing ? <ChromiumMissingAction /> : null}
      </section>
    )
  }

  return (
    <section
      className="function-trigger-surface chat-tool-row"
      data-message-id={message.id}
      data-message-role="function-call"
      data-function-id={message.functionId}
      data-expanded={expandedSurface}
      {...(unloaded ? { 'data-unloaded': '' } : {})}
      aria-label={
        pending ? `action required for ${message.functionId}` : undefined
      }
      tabIndex={pending ? -1 : undefined}
      data-function-status={status}
    >
      {/* The row is one labeled toggle carrying the title, the settled
          facts and the status glyph; the trailing chevron is a pointer-only
          duplicate target so clicking anywhere on the line still toggles. */}
      <div className="chat-tool-row__head group/fchdr">
        <button
          type="button"
          data-message-action="toggle"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          className="chat-tool-row__main"
        >
          <TimelineActivityTrail kind={collapsedActivityKind} />
          {rowText}
          {rowMeta}
          <FunctionTriggerStatusIcon status={status} />
        </button>
        <button
          type="button"
          data-message-action="toggle-caret"
          aria-hidden="true"
          tabIndex={-1}
          onClick={() => setOpen((v) => !v)}
          className="chat-tool-row__caret"
        >
          <ChevronDown className="chat-tool-row__chevron size-4" />
        </button>
      </div>

      {open && unloaded ? (
        <div className="chat-tool-row__body">
          {loadFailed ? (
            <div
              role="alert"
              className="flex items-center gap-2 px-3 py-3 font-inter text-[13px] text-warn"
              data-function-trigger-load-failed=""
            >
              <span>Could not load call details.</span>
              <button
                type="button"
                data-message-action="retry-load"
                onClick={loadDetails}
                className="cursor-pointer underline underline-offset-2 hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
              >
                Retry
              </button>
            </div>
          ) : (
            <div
              role="status"
              className="flex flex-col gap-2 px-3 py-3"
              data-function-trigger-skeleton=""
              aria-busy="true"
              aria-label="loading call details"
            >
              <Skeleton className="h-3 w-2/3" />
              <Skeleton className="h-3 w-1/2" />
              <Skeleton className="h-3 w-5/6" />
            </div>
          )}
          {description ? (
            <FunctionTriggerRowFooter functionId={message.functionId} />
          ) : null}
        </div>
      ) : open && !displayCustomTerminal ? (
        <div className="chat-tool-row__body">
          {pending && customPreview ? (
            <div className="border-b border-edge">{customPreview}</div>
          ) : showRequestPaneAbove ? (
            <ValuePane label="request" value={rawInput} />
          ) : null}
          {running && !pending ? (
            streamingTail !== undefined ? (
              <StreamingArgsPane text={streamingTail} />
            ) : hasCustomTerminal ? (
              <div data-function-terminal="">{customTerminal}</div>
            ) : (
              <ValuePane label="response" value={rawOutput} stacked />
            )
          ) : null}
          {!pending && !running ? (
            hasCustomTerminal ? (
              <div data-function-terminal="">{customTerminal}</div>
            ) : (
              <>
                <ValuePane label="request" value={rawInput} />
                <ValuePane label="response" value={rawOutput} stacked />
              </>
            )
          ) : null}
          {showFooter ? (
            <FunctionTriggerRowFooter
              functionId={message.functionId}
              raw={
                !running && hasCustomTerminal
                  ? { input: rawInput, output: rawOutput }
                  : undefined
              }
            />
          ) : null}
        </div>
      ) : null}

      {displayCustomTerminal ? (
        <div
          className={cn('function-trigger-display-slot', !open && 'pt-1')}
          data-function-display-slot=""
          data-state={status}
        >
          <FunctionRendererPresence
            presentation={customDisplay != null ? 'display' : 'terminal'}
          >
            {customDisplay ?? customTerminal}
          </FunctionRendererPresence>
        </div>
      ) : null}
      {/* A worker's display card is the row's content; the identity strip
          and raw payload follow it rather than sitting between the header
          and the card. */}
      {open && displayCustomTerminal && showFooter ? (
        <div className="chat-tool-row__body">
          <FunctionTriggerRowFooter
            functionId={message.functionId}
            raw={{ input: rawInput, output: rawOutput }}
          />
        </div>
      ) : null}

      {pending && filesystemAccess ? (
        <FilesystemAccessPrompt
          // A held call can be re-parked with a fresh access request (same
          // function_call_id, different root) — remount so submitting/confirm
          // state from the previous round never wedges the buttons.
          key={`${filesystemAccess.requestedRoot} ${filesystemAccess.attemptedPath ?? ''}`}
          requestedRoot={filesystemAccess.requestedRoot}
          workingDir={workingDir}
          onResolve={(action) => onResolveFilesystemAccess?.(action)}
          onManage={onManageFilesystemAccess}
          disabled={!onResolveFilesystemAccess}
        />
      ) : pending ? (
        <div className="flex flex-col gap-2 border-t border-edge px-3 py-2.5">
          <p className="font-inter text-[13px] text-ink-faint">
            Execution is paused until you approve or deny this call.
          </p>
          <div className="flex items-center gap-2" data-approval-actions="">
            <Button
              variant="primary"
              size="sm"
              data-message-action="approve"
              onClick={() => void runResolve('approve')}
              disabled={!onApprove || !!submitting}
            >
              {submitting === 'approve' ? 'approving…' : 'approve'}
            </Button>
            <Button
              variant="pill"
              size="sm"
              data-message-action="deny"
              onClick={() => void runResolve('deny')}
              disabled={!onDeny || !!submitting}
            >
              {submitting === 'deny' ? 'denying…' : 'deny'}
            </Button>
            {onAlwaysAllow ? (
              <AlwaysAllowButton
                functionId={message.functionId}
                onConfirm={() => void runResolve('always_allow')}
                disabled={!!submitting}
                submitting={submitting === 'always_allow'}
              />
            ) : null}
            {submitting ? (
              <span className="font-inter text-[13px] text-ink-faint">
                waiting for the agent to resume…
              </span>
            ) : null}
          </div>
          {submitError ? (
            <div className="font-inter text-[13px] text-warn">
              {submitError}
            </div>
          ) : null}
        </div>
      ) : null}
      {chromiumMissing ? <ChromiumMissingAction /> : null}
    </section>
  )
}

interface ValuePaneProps {
  label: string
  value: unknown
  /** A pane that follows another in the same column: no top margin, so the
      two sit one pane-gap apart instead of two. */
  stacked?: boolean
}

/* Pane chrome — the same recipe as the traces page's payload panes: Inter
   labels on a hairline-framed card, the body on the editor code theme
   (`.code-theme-editor`, index.css: Geist Mono at 13px, loose leading, the
   Vesper / min-light token palette). Machine values (line counts, ids) stay
   in Geist Mono. */
const PANE_LABEL_CLS = 'font-inter text-xs font-medium text-ink'
const PANE_HINT_CLS = 'font-inter text-[11px] font-normal text-ink-faint'
const PANE_FRAME_CLS =
  'code-theme-editor min-w-0 overflow-hidden rounded-lg border border-edge bg-(--code-surface) mx-2.5 mb-2.5'
const PANE_FRAME_TOP_CLS = 'mt-2.5'
const PANE_HEADER_CLS =
  'flex h-8 items-center gap-2 border-b border-edge bg-paper-2 pr-1 pl-3'
const PANE_BODY_PAD_CLS = 'px-4 py-3'

/** `request` → `Request` for display; the data attribute keeps the raw id. */
function titleCase(label: string): string {
  return label.charAt(0).toUpperCase() + label.slice(1)
}

/* Copy → copied is an infrequent state swap: both icons stay in one grid
   cell and cross-fade (scale 0.25→1, blur 4→0, 300ms on the standard
   curve); reduced motion leaves only the opacity fade. */
const COPY_ICON_LAYER_CLS =
  '[grid-area:1/1] transition-[opacity,filter,scale] duration-300 ease-[cubic-bezier(0.2,0,0,1)] motion-reduce:scale-100 motion-reduce:blur-[0px]'
const COPY_ICON_SHOWN_CLS = 'scale-100 opacity-100 blur-[0px]'
const COPY_ICON_HIDDEN_CLS = 'scale-[0.25] opacity-0 blur-[4px]'

function PaneCopyButton({ label, text }: { label: string; text: string }) {
  const [copied, setCopied] = useState(false)
  const copy = () => {
    void copyTextToClipboard(text).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }
  return (
    <Hint label={copied ? 'Copied' : 'Copy'}>
      <button
        type="button"
        onClick={copy}
        className="inline-flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-ink-ghost transition-[background-color,color] duration-[var(--motion-duration-fast)] hover:bg-surface-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
        aria-label={copied ? 'copied' : `copy ${label}`}
      >
        <span className="grid place-items-center">
          <span
            className={cn(
              COPY_ICON_LAYER_CLS,
              copied ? COPY_ICON_SHOWN_CLS : COPY_ICON_HIDDEN_CLS,
            )}
          >
            <Check className="size-4 text-ok" aria-hidden />
          </span>
          <span
            className={cn(
              COPY_ICON_LAYER_CLS,
              copied ? COPY_ICON_HIDDEN_CLS : COPY_ICON_SHOWN_CLS,
            )}
          >
            <Copy className="size-4" aria-hidden />
          </span>
        </span>
      </button>
    </Hint>
  )
}

const TEXT_PRE_CLS = cn(
  PANE_BODY_PAD_CLS,
  'overflow-x-auto whitespace-pre-wrap break-words',
)

/** Rendered lines above which a pane collapses behind a "show all" footer. */
const CLAMP_LINES = 24
/** Collapsed body height — ~12 code lines at the editor theme's leading,
    enough to identify the payload. */
const CLAMP_MAX_H = 'max-h-[21rem]'

function countLines(s: string): number {
  let n = 1
  for (let i = 0; i < s.length; i++) if (s[i] === '\n') n++
  return n
}

/**
 * Shared chrome for one request/response pane: label row with hints and a
 * copy affordance, and a body that clamps past CLAMP_LINES behind an explicit
 * "show all · N lines" footer — big payloads stop drowning the chat flow
 * (PRODUCT.md: hide complexity in collapsible detail, not opaque summaries).
 */
function PaneShell({
  label,
  hints,
  copyText,
  lineCount,
  stacked,
  children,
}: {
  label: string
  hints?: string[]
  copyText: string
  lineCount: number
  stacked?: boolean
  children: React.ReactNode
}) {
  const clampable = lineCount > CLAMP_LINES
  const [expanded, setExpanded] = useState(false)

  return (
    <div
      className={cn(PANE_FRAME_CLS, !stacked && PANE_FRAME_TOP_CLS)}
      data-function-pane={label}
    >
      <div className={PANE_HEADER_CLS}>
        <span className={cn('min-w-0 flex-1 truncate', PANE_LABEL_CLS)}>
          {titleCase(label)}
          {(hints ?? []).map((hint) => (
            <span key={hint} className={cn('ml-2', PANE_HINT_CLS)}>
              {hint}
            </span>
          ))}
        </span>
        <span className="shrink-0 font-geist-mono text-[11px] text-ink-faint tabular-nums">
          {lineCount.toLocaleString('en-US')}{' '}
          {lineCount === 1 ? 'line' : 'lines'}
        </span>
        <PaneCopyButton label={label} text={copyText} />
      </div>
      <div
        className={cn(
          clampable && !expanded && `${CLAMP_MAX_H} overflow-hidden`,
        )}
      >
        {children}
      </div>
      {clampable ? (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          aria-expanded={expanded}
          className={cn(
            PANE_HINT_CLS,
            'w-full cursor-pointer border-t border-edge bg-paper-2 px-3 py-1.5 text-center text-xs transition-colors hover:text-ink',
          )}
        >
          {expanded ? 'Collapse' : `Show all · ${lineCount} lines`}
        </button>
      ) : null}
    </div>
  )
}

/**
 * Live view of a call's arguments while they stream — the harness rides a
 * bounded raw tail on `_streaming` beside the incrementally parsed fields.
 * Column-reverse pins the newest text to the bottom, terminal-style.
 */
function StreamingArgsPane({ text }: { text: string }) {
  return (
    <div
      className={cn(PANE_FRAME_CLS, PANE_FRAME_TOP_CLS)}
      data-function-pane="request"
    >
      <div className={PANE_HEADER_CLS}>
        <span className={PANE_LABEL_CLS}>
          Request
          <span className={cn('ml-2', PANE_HINT_CLS)}>streaming…</span>
        </span>
      </div>
      <div className="flex max-h-56 flex-col-reverse overflow-y-auto">
        <pre
          className={cn(
            PANE_BODY_PAD_CLS,
            'whitespace-pre-wrap break-all text-(--syntax-comment)',
          )}
        >
          {text}
        </pre>
      </div>
    </div>
  )
}

/**
 * The non-envelope rendering of a value: its body text, the header hints, and
 * whether the body is highlighted JSON. One derivation, shared by the pane's
 * body and by its copy button (`paneCopyText`) — the two can never disagree.
 */
function plainPane(value: unknown): {
  body: string
  hints: string[]
  json: boolean
} {
  const primitive = isPrimitive(value)
  const single = primitive ? null : singlePrimitiveField(value)
  // A string payload that is itself JSON (double-encoded): render the parsed
  // structure instead of an escaped one-liner, and say so in the header.
  const embedded =
    primitive && typeof value === 'string'
      ? parseEmbeddedJson(value)
      : single && typeof single.value === 'string'
        ? parseEmbeddedJson(single.value)
        : undefined
  const body =
    embedded !== undefined
      ? formatJson(embedded)
      : primitive
        ? formatPrimitive(value)
        : single
          ? formatPrimitive(single.value)
          : formatJson(value)
  return {
    body,
    hints: [
      ...(single ? [single.key] : []),
      ...(embedded !== undefined ? ['json string'] : []),
    ],
    json: embedded !== undefined || !(primitive || single),
  }
}

/**
 * The EXACT text a pane's copy button puts on the clipboard for `value`.
 * Both `ValuePane` branches route through this, so the value a pane is handed
 * bounds everything that can leave it — redact the value (see `rawRedactor`)
 * and the clipboard is redacted with it. A pane that renders `· empty` shows
 * no copy button, so its return value is then unused.
 */
export function paneCopyText(value: unknown): string {
  // The envelope pane drops text blocks that merely re-serialize `details`,
  // so its copy is the whole value rather than the deduplicated rendering.
  return resultEnvelope(value) ? formatJson(value) : plainPane(value).body
}

function ValuePane({ label, value, stacked }: ValuePaneProps) {
  const empty = isEmptyValue(value)
  const envelope = empty ? null : resultEnvelope(value)

  if (empty) {
    return (
      <div
        className={cn(PANE_FRAME_CLS, !stacked && PANE_FRAME_TOP_CLS)}
        data-function-pane={label}
      >
        <div className={cn(PANE_HEADER_CLS, 'border-b-0')}>
          <span className={PANE_LABEL_CLS}>
            {titleCase(label)}
            <span className={cn('ml-2', PANE_HINT_CLS)}>empty</span>
          </span>
        </div>
      </div>
    )
  }

  if (envelope) {
    const detailsEmpty = isEmptyValue(envelope.details)
    // A text block that re-serializes `details` verbatim is pure duplication
    // (the engine returns both display text and structured details) — drop it.
    const blocks = envelope.texts
      .map((raw) => ({ raw, parsed: parseEmbeddedJson(raw) }))
      .filter(
        (b) =>
          detailsEmpty ||
          b.parsed === undefined ||
          JSON.stringify(b.parsed) !== JSON.stringify(envelope.details),
      )
    const detailsJson = detailsEmpty ? null : formatJson(envelope.details)
    const rendered = blocks.map((b) =>
      b.parsed !== undefined ? formatJson(b.parsed) : b.raw,
    )
    const lineCount =
      rendered.reduce((n, s) => n + countLines(s), 0) +
      (detailsJson ? countLines(detailsJson) + 1 : 0)
    return (
      <PaneShell
        label={label}
        copyText={paneCopyText(value)}
        lineCount={lineCount}
        stacked={stacked}
      >
        {blocks.map((b) =>
          b.parsed !== undefined ? (
            <JsonHighlight
              key={b.raw}
              code={formatJson(b.parsed)}
              className={PANE_BODY_PAD_CLS}
            />
          ) : (
            <pre key={b.raw} className={TEXT_PRE_CLS}>
              <code>{b.raw}</code>
            </pre>
          ),
        )}
        {detailsJson ? (
          <>
            <div className={cn(PANE_HEADER_CLS, 'border-t')}>
              <span className={PANE_LABEL_CLS}>Details</span>
            </div>
            <JsonHighlight code={detailsJson} className={PANE_BODY_PAD_CLS} />
          </>
        ) : null}
      </PaneShell>
    )
  }

  const { body, hints, json } = plainPane(value)

  return (
    <PaneShell
      label={label}
      hints={hints}
      copyText={paneCopyText(value)}
      lineCount={countLines(body)}
      stacked={stacked}
    >
      {json ? (
        <JsonHighlight code={body} className={PANE_BODY_PAD_CLS} />
      ) : (
        <pre className={TEXT_PRE_CLS}>
          <code>{body}</code>
        </pre>
      )}
    </PaneShell>
  )
}
