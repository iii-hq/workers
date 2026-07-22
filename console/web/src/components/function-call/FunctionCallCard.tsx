import { Check, Copy, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import {
  BrowserFunctionIdLabel,
  BrowserToolView,
} from '@/components/chat/browser'
import { CopyMessageButton } from '@/components/chat/CopyMessageButton'
import { CoderFunctionIdLabel, CoderToolView } from '@/components/chat/coder'
import {
  DirectoryFunctionIdLabel,
  DirectoryToolView,
} from '@/components/chat/directory'
import { EngineFunctionIdLabel, EngineToolView } from '@/components/chat/engine'
import { FpFunctionIdLabel, FpToolView } from '@/components/chat/fp'
import {
  HarnessFunctionIdLabel,
  HarnessToolView,
} from '@/components/chat/harness'
import { RouterFunctionIdLabel, RouterToolView } from '@/components/chat/router'
import {
  SandboxFunctionIdLabel,
  SandboxToolView,
} from '@/components/chat/sandbox'
import {
  ScraplingFunctionIdLabel,
  ScraplingToolView,
} from '@/components/chat/scrapling'
import { ShellFunctionIdLabel, ShellToolView } from '@/components/chat/shell'
import { StateFunctionIdLabel, StateToolView } from '@/components/chat/state'
import { WebFunctionIdLabel, WebToolView } from '@/components/chat/web'
import { WorkerFunctionIdLabel, WorkerToolView } from '@/components/chat/worker'
import {
  WorkflowFunctionIdLabel,
  WorkflowToolView,
} from '@/components/chat/workflow'
import { AlwaysAllowButton } from '@/components/permissions/AlwaysAllowButton'
import {
  type FilesystemAccessAction,
  FilesystemAccessPrompt,
} from '@/components/permissions/FilesystemAccessPrompt'
import { Button } from '@/components/ui/Button'
import { StatusDot } from '@/components/ui/StatusDot'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { copyTextToClipboard } from '@/lib/clipboard'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type { FunctionCallMessage as FunctionCallMessageType } from '@/types/chat'

/**
 * FunctionCallCard — the canonical rendering of one iii function call:
 * collapsible header (status dot, ƒ id, duration), request/response panes,
 * per-family tool views, and the optional approval bar.
 *
 * Location-agnostic by design: it is props-only (no chat store, no session
 * context). Chat renders it from live session messages; TracesV2 renders it
 * in the span info tab by synthesizing a `FunctionCallMessage` from an OTel
 * span (`pages/TracesV2/lib/functionCallFromSpan.ts`); any other surface can
 * do the same — `FunctionCallMessage` (types/chat.ts) is a plain data shape
 * whose base is just `{ id, createdAt }`.
 */
interface FunctionCallCardProps {
  message: FunctionCallMessageType
  defaultOpen?: boolean
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
  /**
   * When true, render without the outer `border border-rule bg-bg` chrome
   * so the parent (typically a `FunctionCallGroup`) can frame the stack.
   * The internal layout — header, body, pending bar — stays identical.
   */
  embedded?: boolean
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
 * Branch the function-id label across registered renderer families. New
 * families slot in here — no other change to FCM is needed. The default
 * (unbranded) span keeps unknown ids readable.
 */
function FunctionIdLabel({ functionId }: { functionId: string }) {
  if (DirectoryToolView.isDirectoryFunction(functionId)) {
    return <DirectoryFunctionIdLabel functionId={functionId} />
  }
  if (EngineToolView.isEngineListFunction(functionId)) {
    return <EngineFunctionIdLabel functionId={functionId} />
  }
  if (WorkerToolView.isWorkerFunction(functionId)) {
    return <WorkerFunctionIdLabel functionId={functionId} />
  }
  if (WebToolView.isWebFunction(functionId)) {
    return <WebFunctionIdLabel functionId={functionId} />
  }
  if (CoderToolView.isCoderFunction(functionId)) {
    return <CoderFunctionIdLabel functionId={functionId} />
  }
  if (SandboxToolView.isSandboxFunction(functionId)) {
    return <SandboxFunctionIdLabel functionId={functionId} />
  }
  if (ScraplingToolView.isScraplingFunction(functionId)) {
    return <ScraplingFunctionIdLabel functionId={functionId} />
  }
  if (ShellToolView.isShellFunction(functionId)) {
    return <ShellFunctionIdLabel functionId={functionId} />
  }
  if (WorkflowToolView.isWorkflowFunction(functionId)) {
    return <WorkflowFunctionIdLabel functionId={functionId} />
  }
  if (RouterToolView.isRouterFunction(functionId)) {
    return <RouterFunctionIdLabel functionId={functionId} />
  }
  if (HarnessToolView.isHarnessFunction(functionId)) {
    return <HarnessFunctionIdLabel functionId={functionId} />
  }
  if (FpToolView.isFpFunction(functionId)) {
    return <FpFunctionIdLabel functionId={functionId} />
  }
  if (StateToolView.isStateFunction(functionId)) {
    return <StateFunctionIdLabel functionId={functionId} />
  }
  if (BrowserToolView.isBrowserFunction(functionId)) {
    return <BrowserFunctionIdLabel functionId={functionId} />
  }
  return <span className="text-ink">{functionId}</span>
}

export function FunctionCallCard({
  message,
  defaultOpen,
  onApprove,
  onDeny,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
  embedded,
}: FunctionCallCardProps) {
  const pending = !!message.pendingApproval
  const running = !!message.running
  // Raw in-flight arguments tail (`_streaming`, injected by the harness
  // while a call's arguments are still forming) — rendered as a live pane.
  const streamingTail =
    running &&
    message.input &&
    typeof message.input === 'object' &&
    typeof (message.input as { _streaming?: unknown })._streaming === 'string'
      ? (message.input as { _streaming: string })._streaming
      : undefined
  const filesystemAccess = pending ? message.filesystemAccess : undefined
  const [open, setOpen] = useState(!!defaultOpen || pending)
  const [tab, setTab] = useState<'terminal' | 'json'>('terminal')
  const [submitting, setSubmitting] = useState<
    'approve' | 'deny' | 'always_allow' | null
  >(null)
  const [submitError, setSubmitError] = useState<string | null>(null)

  const customPreview =
    SandboxToolView.tryRenderPreview(message) ??
    EngineToolView.tryRenderPreview(message) ??
    DirectoryToolView.tryRenderPreview(message) ??
    WorkerToolView.tryRenderPreview(message) ??
    WebToolView.tryRenderPreview(message) ??
    CoderToolView.tryRenderPreview(message) ??
    ScraplingToolView.tryRenderPreview(message) ??
    ShellToolView.tryRenderPreview(message) ??
    WorkflowToolView.tryRenderPreview(message) ??
    RouterToolView.tryRenderPreview(message) ??
    HarnessToolView.tryRenderPreview(message) ??
    FpToolView.tryRenderPreview(message) ??
    StateToolView.tryRenderPreview(message) ??
    BrowserToolView.tryRenderPreview(message)
  const customTerminal = !pending
    ? (SandboxToolView.tryRender(message) ??
      EngineToolView.tryRender(message) ??
      DirectoryToolView.tryRender(message) ??
      WorkerToolView.tryRender(message) ??
      WebToolView.tryRender(message) ??
      CoderToolView.tryRender(message) ??
      ScraplingToolView.tryRender(message) ??
      ShellToolView.tryRender(message) ??
      WorkflowToolView.tryRender(message) ??
      RouterToolView.tryRender(message) ??
      HarnessToolView.tryRender(message) ??
      FpToolView.tryRender(message) ??
      StateToolView.tryRender(message) ??
      BrowserToolView.tryRender(message))
    : null
  const hasCustomTerminal = customTerminal != null
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
  // "triggered" is an execution claim, so a settled card only makes it when
  // the call actually ran. Denials are recognized by the gate's envelope in
  // the error details (see isDeniedOutput) — a plain run error keeps the
  // verb. A card with neither output nor duration (deny/abort cleared the
  // approval but no result paired in yet) stays verb-less too.
  const ran =
    !isDeniedOutput(message.output) &&
    (message.output !== undefined || typeof message.durationMs === 'number')

  return (
    <div
      className={cn(
        'function-call-surface',
        !embedded && 'border border-rule bg-bg',
      )}
      data-message-id={message.id}
    >
      {/* The copy affordance must be a sibling of the collapse toggle
          (nested buttons are invalid HTML): the labeled toggle carries the
          title with the copy icon in flow right after it, and the caret
          strip is a pointer-only duplicate target so clicking anywhere on
          the row still collapses. */}
      <div className="group/fchdr flex items-center gap-2 hover:bg-paper-2 transition-colors">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          className="min-w-0 flex items-center gap-2 pl-3 py-2 cursor-pointer text-left"
        >
          <span className="flex items-center gap-2 min-w-0">
            {pending || running ? (
              <StatusDot
                tone={pending ? 'warn' : 'accent'}
                pulse={running}
                className="shrink-0"
              />
            ) : errored ? (
              <X
                aria-hidden
                strokeWidth={2.5}
                className="size-3.5 shrink-0 text-alert"
              />
            ) : (
              <Check
                aria-hidden
                strokeWidth={2.5}
                className="size-3.5 shrink-0 text-ok"
              />
            )}
            <span className="font-mono text-[13px] text-ink truncate">
              {pending ? (
                <>
                  <span>
                    {filesystemAccess
                      ? 'needs filesystem access to run'
                      : 'waiting for your approval to run'}
                  </span>{' '}
                </>
              ) : message.identityInherited ? null : running ? (
                <>triggering </>
              ) : ran ? (
                <>triggered </>
              ) : null}
              <span className="text-accent italic font-semibold">ƒ</span>{' '}
              {running && message.unresolvedTarget ? (
                <span className="text-ink-faint">…</span>
              ) : (
                <FunctionIdLabel functionId={message.functionId} />
              )}
              {!pending &&
              !running &&
              typeof message.durationMs === 'number' ? (
                <span className="text-ink-faint">
                  {' '}
                  for <span className="tabular-nums">{message.durationMs}</span>
                  ms
                </span>
              ) : null}
            </span>
          </span>
        </button>
        {!(running && message.unresolvedTarget) ? (
          <CopyMessageButton
            text={message.functionId}
            label="copy function id"
            className="opacity-0 group-hover/fchdr:opacity-100 group-focus-within/fchdr:opacity-100 transition-opacity"
          />
        ) : null}
        <button
          type="button"
          aria-hidden="true"
          tabIndex={-1}
          onClick={() => setOpen((v) => !v)}
          className="flex-1 self-stretch flex items-center justify-end pr-3 cursor-pointer"
        >
          <span
            className={cn(
              'text-ink-ghost shrink-0 transition-transform duration-150 inline-block',
              open && 'rotate-90',
            )}
          >
            ▸
          </span>
        </button>
      </div>

      {open ? (
        <div className="border-t border-rule-2">
          {pending && customPreview ? (
            <div className="border-b border-rule-2">{customPreview}</div>
          ) : showRequestPaneAbove ? (
            <ValuePane label="request" value={message.input} />
          ) : null}
          {running && !pending ? (
            streamingTail !== undefined ? (
              <StreamingArgsPane text={streamingTail} />
            ) : hasCustomTerminal ? (
              <div className="border-t border-rule-2">{customTerminal}</div>
            ) : (
              <ValuePane label="response" value={message.output} bordered />
            )
          ) : null}
          {!pending && !running ? (
            hasCustomTerminal ? (
              <Tabs
                value={tab}
                onValueChange={(v) => setTab(v as 'terminal' | 'json')}
                className="border-t border-rule-2"
              >
                <TabsList className="px-3">
                  <TabsTrigger value="terminal">terminal</TabsTrigger>
                  <TabsTrigger value="json">raw json</TabsTrigger>
                </TabsList>
                <TabsContent value="terminal">{customTerminal}</TabsContent>
                <TabsContent value="json">
                  <ValuePane label="request" value={message.input} />
                  <ValuePane label="response" value={message.output} bordered />
                </TabsContent>
              </Tabs>
            ) : (
              <>
                <ValuePane label="request" value={message.input} />
                <ValuePane label="response" value={message.output} bordered />
              </>
            )
          ) : null}
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
        <div className="border-t border-rule-2 px-3 py-2 flex flex-col gap-2">
          <p className="font-mono text-[12px] text-ink-faint">
            execution is paused until you approve or deny this call.
          </p>
          <div className="flex items-center gap-2">
            <Button
              variant="primary"
              size="sm"
              onClick={() => void runResolve('approve')}
              disabled={!onApprove || !!submitting}
            >
              {submitting === 'approve' ? 'approving…' : 'approve'}
            </Button>
            <Button
              variant="pill"
              size="sm"
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
              <span className="font-mono text-[12px] text-ink-faint">
                waiting for the agent to resume…
              </span>
            ) : null}
          </div>
          {submitError ? (
            <div className="font-mono text-[12px] text-warn">{submitError}</div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

interface ValuePaneProps {
  label: string
  value: unknown
  bordered?: boolean
}

const TEXT_PRE_CLS =
  'bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words'

/** Rendered lines above which a pane collapses behind a "show all" footer. */
const CLAMP_LINES = 24
/** Collapsed body height — ~16 code lines, enough to identify the payload. */
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
  bordered,
  children,
}: {
  label: string
  hints?: string[]
  copyText: string
  lineCount: number
  bordered?: boolean
  children: React.ReactNode
}) {
  const clampable = lineCount > CLAMP_LINES
  const [expanded, setExpanded] = useState(false)
  const [copied, setCopied] = useState(false)

  const copy = () => {
    void copyTextToClipboard(copyText).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }

  return (
    <div className={cn(bordered && 'border-t border-rule-2')}>
      <div className="flex items-center gap-2 bg-paper-2 px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        <span className="min-w-0 flex-1 truncate">
          {label}
          {(hints ?? []).map((hint) => (
            <span
              key={hint}
              className="text-ink-ghost normal-case tracking-normal"
            >
              {' '}
              · {hint}
            </span>
          ))}
        </span>
        <button
          type="button"
          onClick={copy}
          className="shrink-0 cursor-pointer text-ink-ghost hover:text-ink transition-colors"
          aria-label={copied ? 'copied' : `copy ${label}`}
          title={copied ? 'copied' : 'copy'}
        >
          {copied ? (
            <Check size={12} aria-hidden />
          ) : (
            <Copy size={12} aria-hidden />
          )}
        </button>
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
          className="w-full border-t border-rule-2 bg-paper-2 px-3 py-1 text-center font-mono text-[11px] lowercase text-ink-faint hover:text-ink transition-colors"
        >
          {expanded ? '▴ collapse' : `▾ show all · ${lineCount} lines`}
        </button>
      ) : null}
    </div>
  )
}

/**
 * Live view of a call's arguments while they stream — the harness rides the
 * raw tail on `_streaming` (providers degrade the incomplete JSON itself).
 * Column-reverse pins the newest text to the bottom, terminal-style.
 */
function StreamingArgsPane({ text }: { text: string }) {
  return (
    <div>
      <div className="bg-paper-2 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        request
        <span className="text-ink-ghost normal-case tracking-normal">
          {' '}
          · streaming…
        </span>
      </div>
      <div className="max-h-48 overflow-y-auto flex flex-col-reverse">
        <pre className="px-3 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap break-all text-ink-faint">
          {text}
        </pre>
      </div>
    </div>
  )
}

function ValuePane({ label, value, bordered }: ValuePaneProps) {
  const empty = isEmptyValue(value)
  const primitive = !empty && isPrimitive(value)
  const single = !empty && !primitive ? singlePrimitiveField(value) : null
  const envelope =
    !empty && !primitive && !single ? resultEnvelope(value) : null
  // A string payload that is itself JSON (double-encoded): render the parsed
  // structure instead of an escaped one-liner, and say so in the header.
  const embedded =
    primitive && typeof value === 'string'
      ? parseEmbeddedJson(value)
      : single && typeof single.value === 'string'
        ? parseEmbeddedJson(single.value)
        : undefined

  if (empty) {
    return (
      <div className={cn(bordered && 'border-t border-rule-2')}>
        <div className="bg-paper-2 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          {label}
          <span className="text-ink-ghost normal-case tracking-normal">
            {' '}
            · empty
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
        copyText={formatJson(value)}
        lineCount={lineCount}
        bordered={bordered}
      >
        {blocks.map((b) =>
          b.parsed !== undefined ? (
            <JsonHighlight key={b.raw} code={formatJson(b.parsed)} />
          ) : (
            <pre key={b.raw} className={TEXT_PRE_CLS}>
              <code>{b.raw}</code>
            </pre>
          ),
        )}
        {detailsJson ? (
          <>
            <div className="bg-paper-2 px-3 py-1.5 border-y border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              details
            </div>
            <JsonHighlight code={detailsJson} />
          </>
        ) : null}
      </PaneShell>
    )
  }

  const body =
    embedded !== undefined
      ? formatJson(embedded)
      : primitive
        ? formatPrimitive(value)
        : single
          ? formatPrimitive(single.value)
          : formatJson(value)
  const hints = [
    ...(single ? [single.key] : []),
    ...(embedded !== undefined ? ['json string'] : []),
  ]

  return (
    <PaneShell
      label={label}
      hints={hints}
      copyText={body}
      lineCount={countLines(body)}
      bordered={bordered}
    >
      {embedded !== undefined ? (
        <JsonHighlight code={body} />
      ) : primitive || single ? (
        <pre className={TEXT_PRE_CLS}>
          <code>{body}</code>
        </pre>
      ) : (
        <JsonHighlight code={body} />
      )}
    </PaneShell>
  )
}
