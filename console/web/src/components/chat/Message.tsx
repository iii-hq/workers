import { Bell, Check, Copy, Zap } from 'lucide-react'
import { useState } from 'react'
import { RegisterTriggerView } from '@/components/chat/engine/RegisterTriggerView'
import { FilterChip } from '@/components/chat/engine/shared'
import { MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { FunctionTriggerCard } from '@/components/function-trigger/FunctionTriggerCard'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { Caret } from '@/components/ui/Caret'
import { Prompt } from '@/components/ui/Prompt'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { deliveryOf } from '@/lib/backend/triggers'
import { copyTextToClipboard } from '@/lib/clipboard'
import { Markdown } from '@/lib/markdown'
import { triggerFiredName } from '@/lib/sessions/entry-mapper'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type {
  AssistantMessage as AssistantMessageType,
  Message as MessageType,
  SystemMessage as SystemMessageType,
  TriggerFiredData,
  UserMessage as UserMessageType,
} from '@/types/chat'
import { AttachmentChip } from './AttachmentChip'
import { CopyMessageButton } from './CopyMessageButton'
import { MemoryChip } from './MemoryChip'
import type { TriggerRegistration } from './MessageList'
import { ThoughtMessage } from './ThoughtMessage'

interface MessageProps {
  message: MessageType
  onResolveApproval?: (
    sessionId: string,
    functionTriggerId: string,
    decision: 'allow' | 'deny',
  ) => Promise<void>
  onAlwaysAllow?: (
    sessionId: string,
    functionTriggerId: string,
    functionId: string,
  ) => Promise<void>
  onResolveFilesystemAccess?: (
    sessionId: string,
    functionTriggerId: string,
    action: FilesystemAccessAction,
  ) => Promise<void>
  onManageFilesystemAccess?: () => void
  workingDir?: string | null
  /** Copy payload for an assistant turn (prose + its function calls). Lazy so
      the string is built on click, not on every streaming re-render. */
  copyText?: string | (() => string)
  /** Render function-call cards already expanded (showcase surfaces). */
  defaultOpenCalls?: boolean
  /** Registration detail for a trigger-fired or notification message
      (resolved in MessageList). */
  registration?: TriggerRegistration
}

export function Message({
  message,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
  copyText,
  defaultOpenCalls,
  registration,
}: MessageProps) {
  switch (message.role) {
    case 'user':
      return message.notification ? (
        <NotificationMessage message={message} registration={registration} />
      ) : message.reaction ? (
        <ReactionTaskMessage message={message} />
      ) : message.spawn ? (
        <SpawnTaskMessage message={message} />
      ) : message.validation ? (
        <ValidationNudgeMessage message={message} />
      ) : (
        <UserMessage message={message} />
      )
    case 'assistant':
      return <AssistantMessage message={message} copyText={copyText} />
    case 'thought':
      return <ThoughtMessage message={message} />
    case 'function-trigger': {
      const sessionId = message.sessionId
      const functionTriggerId = message.functionTriggerId
      let onApprove: (() => Promise<void>) | undefined
      let onDeny: (() => Promise<void>) | undefined
      let onAlwaysAllowHandler: (() => Promise<void>) | undefined
      let onResolveFilesystemAccessHandler:
        | ((action: FilesystemAccessAction) => Promise<void>)
        | undefined
      if (onResolveApproval && sessionId && functionTriggerId) {
        onApprove = () =>
          onResolveApproval(sessionId, functionTriggerId, 'allow')
        onDeny = () => onResolveApproval(sessionId, functionTriggerId, 'deny')
      }
      if (onAlwaysAllow && sessionId && functionTriggerId) {
        onAlwaysAllowHandler = () =>
          onAlwaysAllow(sessionId, functionTriggerId, message.functionId)
      }
      if (onResolveFilesystemAccess && sessionId && functionTriggerId) {
        onResolveFilesystemAccessHandler = (action) =>
          onResolveFilesystemAccess(sessionId, functionTriggerId, action)
      }
      return (
        <FunctionTriggerCard
          message={message}
          defaultOpen={defaultOpenCalls}
          onApprove={onApprove}
          onDeny={onDeny}
          onAlwaysAllow={onAlwaysAllowHandler}
          onResolveFilesystemAccess={onResolveFilesystemAccessHandler}
          onManageFilesystemAccess={onManageFilesystemAccess}
          workingDir={workingDir}
        />
      )
    }
    case 'system':
      return message.kind === 'compaction' ? (
        <CompactionMarker message={message} />
      ) : message.kind === 'trigger-fired' ? (
        <TriggerFiredNotice message={message} registration={registration} />
      ) : (
        <SystemNotice message={message} />
      )
  }
}

function SystemNotice({ message }: { message: SystemMessageType }) {
  const tone = message.tone ?? 'info'
  const toneCls =
    tone === 'error'
      ? 'border-l-danger text-danger'
      : tone === 'warn'
        ? 'border-l-warn text-warn'
        : 'border-l-rule text-ink-faint'
  return (
    <article
      className={cn(
        'border-l-2 pl-3 py-1 font-mono text-[12px] uppercase tracking-[0.04em]',
        toneCls,
      )}
    >
      {message.content}
    </article>
  )
}

function CompactionMarker({ message }: { message: SystemMessageType }) {
  const tokens = message.tokensBefore ?? 0
  const summary = message.summaryText ?? ''
  return (
    <article className="my-2">
      <details className="group">
        <summary className="flex items-center gap-3 cursor-pointer list-none select-none">
          <span className="flex-1 h-px bg-rule" aria-hidden="true" />
          <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint flex items-center gap-2 group-hover:text-ink transition-colors">
            <span>compacted</span>
            {tokens > 0 ? (
              <>
                <span className="text-ink-ghost">·</span>
                <span className="tabular-nums">
                  {tokens.toLocaleString()} tokens
                </span>
              </>
            ) : null}
            <span className="text-ink-ghost normal-case tracking-normal text-[10px]">
              show summary
            </span>
          </span>
          <span className="flex-1 h-px bg-rule" aria-hidden="true" />
        </summary>
        {summary ? (
          <pre className="mt-3 mx-9 p-3 bg-panel border border-rule-2 font-mono text-[11px] leading-relaxed text-ink-faint whitespace-pre-wrap">
            {summary}
          </pre>
        ) : null}
      </details>
    </article>
  )
}

/** Split `[notification] <name>: {json}` into its name and payload. */
export function parseNotification(
  content: string,
): { name: string; payload: Record<string, unknown> } | null {
  const m = /^\[notification\]\s*([^:]+):\s*(\{[\s\S]*\})\s*$/.exec(content)
  if (!m) return null
  try {
    const payload = JSON.parse(m[2]) as unknown
    if (!payload || typeof payload !== 'object' || Array.isArray(payload))
      return null
    return { name: m[1].trim(), payload: payload as Record<string, unknown> }
  } catch {
    return null
  }
}

/** The shared pane header strip (label + optional dim hint + copy). */
function PaneHeader({
  label,
  hint,
  copyText,
}: {
  label: string
  hint?: string
  /** When set, a copy affordance rides the strip (mirrors PaneShell's). */
  copyText?: string
}) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="flex items-center gap-2 bg-paper-2 px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
      <span className="min-w-0 flex-1 truncate">
        {label}
        {hint ? (
          <span className="text-ink-ghost normal-case tracking-normal">
            {' '}
            · {hint}
          </span>
        ) : null}
      </span>
      {copyText !== undefined ? (
        <button
          type="button"
          onClick={() => {
            void copyTextToClipboard(copyText).then((ok) => {
              if (!ok) return
              setCopied(true)
              window.setTimeout(() => setCopied(false), 1200)
            })
          }}
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
      ) : null}
    </div>
  )
}

const isScalar = (v: unknown) =>
  v === null ||
  typeof v === 'string' ||
  typeof v === 'number' ||
  typeof v === 'boolean'

/**
 * The friendly tab of a notification: the delivering binding as chips, the
 * event's scalar fields as chips, nested values as a compact JSON block, and
 * the recovered registration through the same WHEN/THEN view the register
 * call renders (RegisterTriggerView falls back to raw JSON when the shape is
 * not a register request).
 */
/**
 * Whether the recovered registration declares gating conditions. On a fire
 * or delivery card this implies they PASSED: the harness evaluates
 * conditions before writing the fired record — a failing gate takes the
 * skip path and never produces one (trigger_deliver.rs: resolve → stale →
 * conditions → claim → dispatch → record).
 */
function hasConditions(registration?: TriggerRegistration): boolean {
  const d = registration?.detail
  if (!d || typeof d !== 'object') return false
  const c = (d as { conditions?: unknown }).conditions
  return Array.isArray(c) && c.length > 0
}

/**
 * Friendly registration block shared by the fired/notification terminals:
 * the same WHEN/THEN view the register call renders. Row-sourced
 * registrations carry the trigger type as the summary; the register-call
 * fallback is already a full request. Either way RegisterTriggerView parses
 * what it can and JSON-dumps what it cannot.
 */
function RegistrationTerminal({
  registration,
}: {
  registration: TriggerRegistration
}) {
  const regInput =
    registration.summary &&
    registration.summary !== 'from register call' &&
    registration.detail &&
    typeof registration.detail === 'object'
      ? { trigger_type: registration.summary, ...registration.detail }
      : registration.detail
  return (
    <div data-function-pane="registration">
      <PaneHeader
        label="registration"
        hint={registration.summary}
        copyText={JSON.stringify(registration.detail, null, 2)}
      />
      <RegisterTriggerView input={regInput} output={undefined} />
    </div>
  )
}

function NotificationTerminal({
  name,
  payload,
  registration,
}: {
  name: string
  payload: Record<string, unknown>
  registration?: TriggerRegistration
}) {
  const entries = Object.entries(payload).filter(([k]) => !k.startsWith('_'))
  const scalars = entries.filter(([, v]) => isScalar(v))
  const rest = entries.filter(([, v]) => !isScalar(v))
  return (
    <div className="bg-bg">
      <MetaRow>
        <StatusPill label="notification" variant="accent" />
        <FilterChip label="from" value={name} />
        {hasConditions(registration) ? (
          <FilterChip label="conditions" value="met" />
        ) : null}
      </MetaRow>
      <PaneHeader label="event" />
      <div className="px-3 py-2 border-b border-rule-2 flex flex-wrap items-center gap-1.5">
        {scalars.length > 0 ? (
          scalars.map(([k, v]) => (
            <FilterChip key={k} label={k} value={String(v)} />
          ))
        ) : (
          <span className="font-mono text-[11px] text-ink-ghost">
            · no scalar fields
          </span>
        )}
      </div>
      {rest.length > 0 ? (
        <div data-function-pane="event-data">
          <PaneHeader
            label="data"
            copyText={JSON.stringify(Object.fromEntries(rest), null, 2)}
          />
          <div className="max-h-64 overflow-auto">
            <JsonHighlight
              code={JSON.stringify(Object.fromEntries(rest), null, 2)}
              wrap
            />
          </div>
        </div>
      ) : null}
      {registration ? (
        <RegistrationTerminal registration={registration} />
      ) : null}
    </div>
  )
}

/**
 * The friendly tab of a trigger-fired notice: lifecycle chips, the delivery
 * (call target or session wake) as a THEN block, and the recovered
 * registration through the shared WHEN/THEN view.
 */
function TriggerFiredTerminal({
  t,
  registration,
}: {
  t: TriggerFiredData
  registration?: TriggerRegistration
}) {
  const delivery = deliveryOf(t.target)
  return (
    <div className="bg-bg">
      <MetaRow>
        <StatusPill label="trigger fired" variant="accent" />
        <FilterChip label="label" value={triggerFiredName(t)} />
        <FilterChip label="mode" value={t.once ? 'one-shot' : 'persistent'} />
        {typeof t.fired_at === 'number' ? (
          <FilterChip
            label="at"
            value={new Date(t.fired_at).toLocaleString()}
          />
        ) : null}
        {t.retired ? (
          <FilterChip label="lifecycle" value="unregistered" />
        ) : null}
        {hasConditions(registration) ? (
          <FilterChip label="conditions" value="met" />
        ) : null}
      </MetaRow>
      <PaneHeader label="then" />
      <div className="px-3 py-2 border-b border-rule-2 flex items-baseline gap-2 flex-wrap">
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          {delivery.kind === 'call' ? 'call' : 'notify'}
        </span>
        {delivery.kind === 'call' ? (
          <span className="font-mono text-[12.5px] text-accent break-all">
            {delivery.functionId}
          </span>
        ) : (
          <span className="font-mono text-[12.5px] text-ink-faint italic">
            this session
          </span>
        )}
      </div>
      {t.payload !== undefined ? (
        <div data-function-pane="payload">
          <PaneHeader
            label="payload"
            copyText={JSON.stringify(t.payload, null, 2)}
          />
          <div className="max-h-64 overflow-auto">
            <JsonHighlight code={JSON.stringify(t.payload, null, 2)} wrap />
          </div>
        </div>
      ) : null}
      {registration ? (
        <RegistrationTerminal registration={registration} />
      ) : null}
    </div>
  )
}

/** The shared "registration" detail pane (trigger-fired + notification). */
function RegistrationPane({
  registration,
}: {
  registration: TriggerRegistration
}) {
  const json = JSON.stringify(registration.detail, null, 2)
  return (
    <div className="border-t border-rule-2" data-function-pane="registration">
      <PaneHeader
        label="registration"
        hint={registration.summary}
        copyText={json}
      />
      <div className="max-h-64 overflow-auto">
        <JsonHighlight code={json} wrap />
      </div>
    </div>
  )
}

/**
 * A notify-wake delivery, in the same card language as the function-call
 * and trigger-fired rows: one line naming the binding that woke this chat,
 * payload behind the expand — plus the binding's registration when the
 * resolver recovered it (see MessageList). Content that isn't the
 * `[notification] name: {json}` shape renders as-is in the same chrome.
 */
function NotificationMessage({
  message,
  registration,
}: {
  message: UserMessageType
  registration?: TriggerRegistration
}) {
  const parsed = parseNotification(message.content)
  const [tab, setTab] = useState<'terminal' | 'json'>('terminal')
  const icon = (
    <Bell
      aria-hidden
      strokeWidth={2.5}
      className="size-3.5 shrink-0 text-warn"
    />
  )
  if (!parsed) {
    // Unlabeled / non-object / truncated notices carry their meaning in the
    // text itself — wrap it in full rather than clipping to one line.
    return (
      <article
        className="function-trigger-surface border border-rule bg-bg flex items-start gap-2 px-3 py-2"
        data-message-role="notification"
      >
        {icon}
        <span className="min-w-0 font-mono text-[13px] text-ink break-words">
          {message.content}
        </span>
      </article>
    )
  }
  return (
    <article
      className="function-trigger-surface border border-rule bg-bg"
      data-message-role="notification"
    >
      <details className="group">
        <summary className="flex items-center gap-2 px-3 py-2 cursor-pointer list-none select-none hover:bg-paper-2 transition-colors">
          {icon}
          <span className="min-w-0 flex-1 font-mono text-[13px] text-ink truncate">
            <span className="text-ink">notification</span> triggered{' '}
            <span className="text-ink-faint">{parsed.name}</span>
          </span>
          <span
            aria-hidden
            className="text-ink-ghost shrink-0 transition-transform duration-150 inline-block group-open:rotate-90"
          >
            ▸
          </span>
        </summary>
        <Tabs
          value={tab}
          onValueChange={(v) => setTab(v as 'terminal' | 'json')}
          className="border-t border-rule-2"
        >
          <TabsList className="px-3">
            <TabsTrigger value="terminal">terminal</TabsTrigger>
            <TabsTrigger value="json">raw json</TabsTrigger>
          </TabsList>
          <TabsContent value="terminal">
            <NotificationTerminal
              name={parsed.name}
              payload={parsed.payload}
              registration={registration}
            />
          </TabsContent>
          <TabsContent value="json">
            <div data-function-pane="payload">
              <PaneHeader
                label="payload"
                copyText={JSON.stringify(parsed.payload, null, 2)}
              />
              <div className="max-h-64 overflow-auto">
                <JsonHighlight
                  code={JSON.stringify(parsed.payload, null, 2)}
                  wrap
                />
              </div>
            </div>
            {registration ? (
              <RegistrationPane registration={registration} />
            ) : null}
          </TabsContent>
        </Tabs>
      </details>
    </article>
  )
}

/**
 * A subscription fire, rendered in the same visual language as a
 * `FunctionTriggerCard` header — it IS a function call, just one the
 * trigger made instead of the agent. The ⚡ (in place of ✓/✗) marks the
 * autonomous origin, and the trailing faint name says which binding fired it.
 * Wake/spawn fires have no called function; they keep the summary text.
 *
 * `registration` is the binding's configuration — from the harness store
 * when the row is still known, else recovered from the transcript's
 * register call (the fire record itself carries none of it). Absent only
 * when neither source has it.
 */
function TriggerFiredNotice({
  message,
  registration,
}: {
  message: SystemMessageType
  registration?: TriggerRegistration
}) {
  const t = message.trigger
  const [tab, setTab] = useState<'terminal' | 'json'>('terminal')
  const called =
    t?.target &&
    t.target !== 'spawn' &&
    t.target !== 'notify' &&
    t.target !== 'harness::send'
      ? t.target
      : null
  const notified =
    t && (!t.target || t.target === 'notify' || t.target === 'harness::send')
  const header = (
    <>
      <Zap
        aria-hidden
        strokeWidth={2.5}
        className="size-3.5 shrink-0 text-warn"
      />
      <span className="min-w-0 flex-1 font-mono text-[13px] text-ink truncate">
        {t && (called || notified) ? (
          <>
            {called ? (
              <>
                triggered{' '}
                <span className="text-accent italic font-semibold">ƒ</span>{' '}
                <span className="text-ink">{called}</span>
              </>
            ) : (
              <>
                <span className="text-ink">notification</span> triggered
              </>
            )}
            <span className="text-ink-faint"> {triggerFiredName(t)}</span>
            {t.retired ? (
              <span className="text-ink-ghost"> · unregistered</span>
            ) : null}
          </>
        ) : (
          message.content
        )}
      </span>
    </>
  )
  if (!t) {
    return (
      <article
        className="function-trigger-surface border border-rule bg-bg flex items-center gap-2 px-3 py-2"
        data-message-role="trigger-fired"
      >
        {header}
      </article>
    )
  }
  return (
    <article
      className="function-trigger-surface border border-rule bg-bg"
      data-message-role="trigger-fired"
    >
      <details className="group">
        <summary className="flex items-center gap-2 px-3 py-2 cursor-pointer list-none select-none hover:bg-paper-2 transition-colors">
          {header}
          <span
            aria-hidden
            className="text-ink-ghost shrink-0 transition-transform duration-150 inline-block group-open:rotate-90"
          >
            ▸
          </span>
        </summary>
        <Tabs
          value={tab}
          onValueChange={(v) => setTab(v as 'terminal' | 'json')}
          className="border-t border-rule-2"
        >
          <TabsList className="px-3">
            <TabsTrigger value="terminal">terminal</TabsTrigger>
            <TabsTrigger value="json">raw json</TabsTrigger>
          </TabsList>
          <TabsContent value="terminal">
            <TriggerFiredTerminal t={t} registration={registration} />
          </TabsContent>
          <TabsContent value="json">
            {registration ? (
              <RegistrationPane registration={registration} />
            ) : null}
            <div data-function-pane="trigger">
              <PaneHeader label="fire" copyText={JSON.stringify(t, null, 2)} />
              <div className="max-h-64 overflow-auto">
                <JsonHighlight code={JSON.stringify(t, null, 2)} wrap />
              </div>
            </div>
          </TabsContent>
        </Tabs>
      </details>
    </article>
  )
}

/**
 * The one-line hint for a reaction's collapsed payload: the firing session
 * and status for an event, the predecessor keys for a join's inputs.
 */
function reactionEventHint(event: {
  label: 'event' | 'inputs'
  json: string
}): string | null {
  try {
    const v = JSON.parse(event.json) as Record<string, unknown>
    if (v === null || typeof v !== 'object') return null
    if (event.label === 'inputs') {
      const keys = Object.keys(v)
      return keys.length > 0 ? keys.join(' + ') : null
    }
    const parts = [v.session_id, v.status].filter(
      (x): x is string => typeof x === 'string',
    )
    return parts.length > 0 ? parts.join(' · ') : null
  } catch {
    return null
  }
}

/**
 * HISTORICAL transcripts only: a trigger-fired task delivered into a session
 * back when bindings could target `harness::spawn`. New runs never produce
 * these — trigger delivery no longer creates agents — but old conversations
 * must keep rendering faithfully.
 */
function ReactionTaskMessage({ message }: { message: UserMessageType }) {
  const event = message.reactionEvent
  const hint = event ? reactionEventHint(event) : null
  return (
    <article className="flex flex-col items-start gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
        <Prompt symbol="⚡">trigger · reaction task</Prompt>
      </header>
      <div className="max-w-[80%] border-l border-rule pl-4 pr-1 py-1 break-words text-ink-faint">
        <Markdown>{message.content}</Markdown>
        {event ? (
          <details className="mt-2 group">
            <summary className="cursor-pointer list-none select-none font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost group-hover:text-ink transition-colors">
              firing event
              {hint ? ` · ${hint}` : ''}
              <span className="normal-case tracking-normal text-[10px]">
                {' '}
                · show json
              </span>
            </summary>
            <div className="mt-1 max-h-64 overflow-auto border border-rule-2">
              <JsonHighlight code={event.json} wrap />
            </div>
          </details>
        ) : null}
      </div>
    </article>
  )
}

/**
 * A validation nudge (`validation: true`): the harness re-prompting the turn
 * after the output contract or a `harness::hook::post-turn` validator
 * rejected its result. Labeled and left-aligned like the other
 * machine-authored user entries so it never reads as something the human
 * typed — the loop is visible AS a loop.
 */
function ValidationNudgeMessage({ message }: { message: UserMessageType }) {
  return (
    <article className="flex flex-col items-start gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
        <Prompt symbol="⟳">validator · corrective prompt</Prompt>
      </header>
      <div className="max-w-[80%] border-l border-rule pl-4 pr-1 py-1 break-words text-ink-faint">
        <Markdown>{message.content}</Markdown>
      </div>
    </article>
  )
}

/**
 * A direct `harness::spawn` seed task (`spawn: true`): the sub-agent's opening
 * input, but sent by the PARENT agent — labeled and left-aligned like a
 * reaction task so it never reads as something the human typed.
 */
function SpawnTaskMessage({ message }: { message: UserMessageType }) {
  return (
    <article className="flex flex-col items-start gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
        <Prompt symbol="⚙">spawn · sub-agent task</Prompt>
      </header>
      <div className="max-w-[80%] border-l border-rule pl-4 pr-1 py-1 break-words text-ink-faint">
        <Markdown>{message.content}</Markdown>
      </div>
    </article>
  )
}

function UserMessage({ message }: { message: UserMessageType }) {
  return (
    <article
      className="group flex flex-col items-end gap-2"
      data-message-role="user"
    >
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost flex items-center gap-2">
        {message.content ? (
          <CopyMessageButton
            text={message.content}
            className="opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-[opacity,color]"
          />
        ) : null}
        <Prompt symbol="$">you</Prompt>
      </header>
      <div
        className={cn(
          'max-w-[80%] border-l border-rule pl-4 pr-1 py-1',
          'break-words',
        )}
      >
        <Markdown>{message.content}</Markdown>
      </div>
      {message.attachments && message.attachments.length > 0 ? (
        <div className="flex flex-wrap gap-2 justify-end max-w-[80%]">
          {message.attachments.map((a) => (
            <AttachmentChip key={a.id} attachment={a} />
          ))}
        </div>
      ) : null}
    </article>
  )
}

function AssistantMessage({
  message,
  copyText,
}: {
  message: AssistantMessageType
  copyText?: string | (() => string)
}) {
  const showCaret = !!message.streaming
  // A tool-only turn has no prose but still carries a copy payload (its
  // function calls) via copyText; direct renders without a list-provided
  // payload fall back to the prose gate.
  const copySource = copyText ?? (message.content || undefined)
  return (
    <article
      className="group flex flex-col gap-2"
      data-message-role="assistant"
    >
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost flex items-center gap-2 flex-wrap">
        <Prompt symbol=">">agent</Prompt>
        {message.model ? (
          <span className="text-ink-ghost">· {message.model}</span>
        ) : null}
        {message.mode ? (
          <span className="text-ink-ghost">· {message.mode}</span>
        ) : null}
        {message.memory ? <MemoryChip memory={message.memory} /> : null}
        {copySource !== undefined && !message.streaming ? (
          <CopyMessageButton
            text={copySource}
            className="opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-[opacity,color]"
          />
        ) : null}
      </header>
      <div className="pr-1">
        {message.content ? (
          <Markdown>{message.content}</Markdown>
        ) : (
          <div className="font-mono text-[13px] italic thinking-shimmer">
            thinking…
          </div>
        )}
        {showCaret ? <Caret className="ml-0.5" /> : null}
      </div>
    </article>
  )
}
