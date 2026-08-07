import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
} from 'react'
import { resultEnvelope } from '@/components/function-trigger/FunctionTriggerCard'
import {
  rawRedactor,
  useFunctionTriggerRenderers,
} from '@/components/function-trigger/renderer-registry'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import {
  assistantCopyText,
  functionTriggersByAssistant,
} from '@/lib/function-trigger-copy'
import { triggerFiredName } from '@/lib/sessions/entry-mapper'
import { cn } from '@/lib/utils'
import type { Message as MessageType } from '@/types/chat'
import { EmptyState, type EmptyStateProps } from './EmptyState'
import { Message } from './Message'

interface MessageListProps {
  messages: MessageType[]
  /** Show "thinking…" shimmer at the bottom while the agent is between
      visible outputs (after submit, or between fcall-end and the next
      turn's first token). */
  isThinking?: boolean
  /** Under-the-hood context shown as the waiting shimmer (e.g. "dispatching
      zai::glm-5.2" or the session's status_reason). Falls back to "thinking…"
      when absent. */
  thinkingDetail?: string
  density?: 'route' | 'dock'
  /** Rendered at the top of the transcript scroller, above the messages, so
      it scrolls away with them instead of unmounting. When set it replaces
      the `EmptyState` on an empty transcript (landing demo). */
  header?: ReactNode
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
  /**
   * Render every function-call card (and group) already expanded. Off in the
   * product, where a turn's calls collapse to one line each; on for showcase
   * surfaces whose whole point is the result renderers.
   */
  defaultOpenCalls?: boolean
  /** Registration rows by subscription id, for trigger-fired card detail. */
  triggersById?: ReadonlyMap<string, SessionTriggerInfo>
}

/** What a trigger-fired card shows under "registration". */
export interface TriggerRegistration {
  /** Short header hint (trigger type, or the recovery source). */
  summary?: string
  detail: unknown
}

/**
 * Pull `subscription_id` out of a register-call output. Outputs arrive as the
 * `{ content: [{text}], details }` result envelope (see functionResultOutput);
 * the id lives in a JSON text block — bare objects/strings are handled too.
 */
export function subscriptionIdOf(output: unknown): string | null {
  const idOf = (v: unknown): string | null => {
    if (typeof v === 'string') {
      try {
        return idOf(JSON.parse(v))
      } catch {
        return null
      }
    }
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      const id = (v as Record<string, unknown>).subscription_id
      if (typeof id === 'string') return id
    }
    return null
  }
  const envelope = resultEnvelope(output)
  if (envelope) {
    for (const text of envelope.texts) {
      const id = idOf(text)
      if (id) return id
    }
    return idOf(envelope.details)
  }
  return idOf(output)
}

/**
 * Registration detail per message id, for trigger-fired notices AND the
 * notification deliveries their notify-wakes produce. Two-tier per
 * subscription: the harness's live/seen binding row when available, else
 * recovered from the transcript — the `engine::register_trigger` call whose
 * result carries the same subscription id (its input IS the registration).
 * Notifications correlate by the subscription id embedded in their entry id
 * (`e_notify_<sub>`), with a name-match fallback for older records; both are
 * order-independent — an idle-session wake appends the notification BEFORE
 * its fire record.
 */
export function resolveRegistrations(
  messages: MessageType[],
  triggersById?: ReadonlyMap<string, SessionTriggerInfo>,
): Map<string, TriggerRegistration> {
  const fromCalls = new Map<string, unknown>()
  const notifyFireByName = new Map<string, string>()
  for (const m of messages) {
    if (
      m.role === 'function-trigger' &&
      m.functionId === 'engine::register_trigger'
    ) {
      const subId = subscriptionIdOf(m.output)
      if (subId && m.input !== undefined) fromCalls.set(subId, m.input)
    } else if (m.role === 'system' && m.kind === 'trigger-fired' && m.trigger) {
      const t = m.trigger
      if (!t.target || t.target === 'notify' || t.target === 'harness::send')
        notifyFireByName.set(triggerFiredName(t), t.subscription_id)
    }
  }
  const regFor = (subscriptionId: string): TriggerRegistration | undefined => {
    const row = triggersById?.get(subscriptionId)
    // A post-reload ghost row is thin (no config) — the register call's
    // input is the richer record then.
    if (row && row.config !== undefined) {
      return {
        summary: row.triggerType,
        detail: {
          config: row.config,
          conditions: row.conditions?.length ? row.conditions : undefined,
          once: row.once,
          label: row.label,
          // Keep the delivery target — without it the WHEN/THEN view would
          // misreport a call binding as "notify this session".
          function_id:
            row.delivery.kind === 'call' ? row.delivery.functionId : undefined,
        },
      }
    }
    const input = fromCalls.get(subscriptionId)
    return input !== undefined
      ? { summary: 'from register call', detail: input }
      : undefined
  }
  const out = new Map<string, TriggerRegistration>()
  for (const m of messages) {
    if (m.role === 'system' && m.kind === 'trigger-fired' && m.trigger) {
      const reg = regFor(m.trigger.subscription_id)
      if (reg) out.set(m.id, reg)
    } else if (m.role === 'user' && m.notification) {
      // Exact: the persisted entry id is `e_notify_<subscription_id>`.
      const fromId = /^e_notify_(sub_[A-Za-z0-9]+)/.exec(m.id)?.[1]
      const name = /^\[notification\]\s*([^:]+):/.exec(m.content)?.[1]?.trim()
      const subId = fromId ?? (name ? notifyFireByName.get(name) : undefined)
      const reg = subId ? regFor(subId) : undefined
      if (reg) out.set(m.id, reg)
    }
  }
  return out
}

export function MessageList({
  messages,
  isThinking,
  thinkingDetail,
  density = 'route',
  header,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
  defaultOpenCalls,
  triggersById,
}: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const lastPendingIdRef = useRef<string | null>(null)

  const fcallsByAssistant = useMemo(
    () => functionTriggersByAssistant(messages),
    [messages],
  )
  const registrations = useMemo(
    () => resolveRegistrations(messages, triggersById),
    [messages, triggersById],
  )
  // Same registry `FunctionTriggerCard` uses for its own raw pane: an
  // assistant-turn copy serializes each call's arguments the same way the
  // call's own card does, so a worker's `redactRaw` (e.g.
  // sandbox-code-runner's runtime_id) has to cover this exit too — see
  // function-trigger-copy.ts.
  const renderers = useFunctionTriggerRenderers()
  const redactFor = (functionId: string) => rawRedactor(renderers, functionId)

  // Read optionally so isolated renders (Storybook) still work without the
  // ConversationsProvider; the empty state falls back to `ready` there.
  const ctx = useConversationsCtxOptional()

  /* Opening a session lands on the latest exchange, not the beginning: one
     instant jump to the bottom the first time the transcript has content —
     on mount for an already-hydrated session, or when the first hydration
     batch arrives otherwise. Layout effect so the jump happens before
     paint (no flash of the top). After this, the near-bottom rule below
     owns scrolling, so reading upward is never interrupted. */
  const didInitialScrollRef = useRef(false)
  useLayoutEffect(() => {
    if (didInitialScrollRef.current || messages.length === 0) return
    didInitialScrollRef.current = true
    const c = containerRef.current
    if (c) c.scrollTop = c.scrollHeight
  }, [messages])

  /* Auto-scroll only when the user is already near the bottom. The effect body
     reads layout off refs but the trigger we care about is "messages changed"
     or "thinking flipped", so list both explicitly. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: messages and isThinking are the triggers, not values read in the body.
  useEffect(() => {
    const c = containerRef.current
    if (!c) return
    const distanceFromBottom = c.scrollHeight - c.scrollTop - c.clientHeight
    if (distanceFromBottom < 200) {
      bottomRef.current?.scrollIntoView({ block: 'end' })
    }
  }, [messages, isThinking])

  /* PR #150: a fresh approval modal demands attention even if the user
     has scrolled up reading earlier content. Find the newest message with
     pendingApproval and scroll it into view exactly once per pending id.
     (We dedupe via lastPendingIdRef so React's re-renders don't keep
     forcing the scroll while the user is reading the request body.) */
  useEffect(() => {
    // Walk backwards instead of spreading + reversing — the spread
    // copies the whole array every render, which is a real cost on
    // token-by-token re-renders during streaming.
    let newestPending: (typeof messages)[number] | null = null
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i]
      if (m.role === 'function-trigger' && m.pendingApproval === true) {
        newestPending = m
        break
      }
    }
    if (!newestPending) {
      lastPendingIdRef.current = null
      return
    }
    if (newestPending.id === lastPendingIdRef.current) return
    lastPendingIdRef.current = newestPending.id
    // defer one frame so the DOM has the new node before we scroll
    requestAnimationFrame(() => {
      const node = containerRef.current?.querySelector(
        `[data-message-id="${newestPending.id}"]`,
      )
      if (node && 'scrollIntoView' in node) {
        ;(node as HTMLElement).scrollIntoView({
          block: 'center',
          behavior: 'smooth',
        })
      } else {
        bottomRef.current?.scrollIntoView({ block: 'end', behavior: 'smooth' })
      }
    })
  }, [messages])

  if (messages.length === 0 && !header) {
    return <EmptyState {...resolveEmptyState(ctx, density)} />
  }

  const listPad = density === 'dock' ? 'px-4 py-6' : 'px-9 py-8'

  return (
    <div ref={containerRef} className={cn('flex-1 overflow-y-auto', listPad)}>
      <div className="mx-auto max-w-[720px] flex flex-col gap-y-8">
        {header}
        {messages.map((m, i) => {
          // Assistant turns copy their prose plus the calls that follow them;
          // the thunk defers building that string until the copy click. Left
          // undefined when the turn has nothing to copy (no prose, no calls)
          // so the header shows no copy affordance.
          const calls =
            m.role === 'assistant' ? fcallsByAssistant.get(m.id) : undefined
          const copyText =
            m.role === 'assistant' && (m.content || calls?.length)
              ? () => assistantCopyText(m.content, calls ?? [], redactFor)
              : undefined
          // A call that directly follows another call belongs to the same
          // burst of agent activity — pull it up against its predecessor so
          // the run reads as one tight stack instead of scattered boxes.
          // Trigger-fired notices share the card language and ride along.
          // (Safe as a conditional wrapper: a message's predecessor is fixed
          // at insert time, so `tight` never flips on an existing node.)
          const callLike = (x: MessageType | undefined) =>
            x?.role === 'function-trigger' ||
            (x?.role === 'system' && x.kind === 'trigger-fired')
          const tight = callLike(m) && callLike(messages[i - 1])
          const node = (
            <Message
              key={m.id}
              message={m}
              copyText={copyText}
              defaultOpenCalls={defaultOpenCalls}
              onResolveApproval={onResolveApproval}
              onAlwaysAllow={onAlwaysAllow}
              onResolveFilesystemAccess={onResolveFilesystemAccess}
              onManageFilesystemAccess={onManageFilesystemAccess}
              workingDir={workingDir}
              registration={registrations.get(m.id)}
            />
          )
          return tight ? (
            <div key={m.id} className="-mt-[26px]">
              {node}
            </div>
          ) : (
            node
          )
        })}
        {isThinking ? (
          <div className="font-mono text-[13px] italic thinking-shimmer text-ink-faint">
            {thinkingDetail ?? 'thinking…'}
          </div>
        ) : null}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}

type ChatCtx = ReturnType<typeof useConversationsCtxOptional>

/**
 * Map harness presence + the model catalog (from ConversationsContext) onto an
 * `EmptyState` variant. Loading flags hold the `ready` hero so the first paint
 * never flashes an install/configure prompt before the probes resolve.
 */
function resolveEmptyState(
  ctx: ChatCtx,
  density: 'route' | 'dock',
): EmptyStateProps {
  if (!ctx) return { variant: 'ready', density }

  const { harnessStatus, modelOptions, catalogLoading } = ctx
  const base: EmptyStateProps = {
    variant: 'ready',
    density,
    stages: harnessStatus.stages,
    errorMessage: harnessStatus.error,
    onInstallHarness: harnessStatus.install,
    onRetryInstall: harnessStatus.retry,
    onConfigureProvider: () => {
      window.location.hash = '#/workers/configuration/llm-router'
    },
  }

  if (harnessStatus.error) return { ...base, variant: 'install-failed' }
  if (harnessStatus.installing) return { ...base, variant: 'installing' }
  if (harnessStatus.loading) return base
  if (!harnessStatus.present) return { ...base, variant: 'no-harness' }
  if (catalogLoading) return base
  if (modelOptions.length === 0) return { ...base, variant: 'no-provider' }
  return base
}
