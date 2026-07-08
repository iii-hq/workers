import { Check, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { CoderFunctionIdLabel, CoderToolView } from '@/components/chat/coder'
import {
  DirectoryFunctionIdLabel,
  DirectoryToolView,
} from '@/components/chat/directory'
import { EngineFunctionIdLabel, EngineToolView } from '@/components/chat/engine'
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

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
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
  if (StateToolView.isStateFunction(functionId)) {
    return <StateFunctionIdLabel functionId={functionId} />
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
    StateToolView.tryRenderPreview(message)
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
      StateToolView.tryRender(message))
    : null
  const hasCustomTerminal = customTerminal != null
  // The top request pane renders only while the call is in flight and no
  // richer view covers it; the settled (done) branch below renders its own
  // request/response panes, so showing it there would duplicate the pane.
  const showRequestPaneAbove = pending
    ? !customPreview
    : running
      ? !hasCustomTerminal
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

  return (
    <div
      className={cn(
        'function-call-surface',
        !embedded && 'border border-rule bg-bg',
      )}
      data-message-id={message.id}
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={cn(
          'w-full flex items-center justify-between gap-3 px-3 py-2 cursor-pointer text-left',
          'hover:bg-paper-2 transition-colors',
        )}
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
                    : 'permission to trigger'}
                </span>{' '}
              </>
            ) : null}
            <span className="text-accent italic font-semibold">ƒ</span>{' '}
            {running && message.unresolvedTarget ? (
              <span className="text-ink-faint">…</span>
            ) : (
              <FunctionIdLabel functionId={message.functionId} />
            )}
            {!pending && !running && typeof message.durationMs === 'number' ? (
              <span className="text-ink-faint">
                {' '}
                for <span className="tabular-nums">{message.durationMs}</span>
                ms
              </span>
            ) : null}
          </span>
        </span>
        <span
          aria-hidden
          className={cn(
            'text-ink-ghost shrink-0 transition-transform duration-150 inline-block',
            open && 'rotate-90',
          )}
        >
          ▸
        </span>
      </button>

      {open ? (
        <div className="border-t border-rule-2">
          {pending && customPreview ? (
            <div className="border-b border-rule-2">{customPreview}</div>
          ) : showRequestPaneAbove ? (
            <ValuePane label="request" value={message.input} />
          ) : null}
          {running && !pending ? (
            hasCustomTerminal ? (
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

function ValuePane({ label, value, bordered }: ValuePaneProps) {
  const empty = isEmptyValue(value)
  const primitive = !empty && isPrimitive(value)
  const single = !empty && !primitive ? singlePrimitiveField(value) : null

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

  return (
    <div className={cn(bordered && 'border-t border-rule-2')}>
      <div className="bg-paper-2 px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        {label}
        {single ? (
          <span className="text-ink-ghost normal-case tracking-normal">
            {' '}
            · {single.key}
          </span>
        ) : null}
      </div>
      {primitive ? (
        <pre className="bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
          <code>{formatPrimitive(value)}</code>
        </pre>
      ) : single ? (
        <pre className="bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
          <code>{formatPrimitive(single.value)}</code>
        </pre>
      ) : (
        <JsonHighlight code={formatJson(value)} />
      )}
    </div>
  )
}
