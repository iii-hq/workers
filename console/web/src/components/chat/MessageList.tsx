import { ArrowDown, ChevronRight } from 'lucide-react'
import {
  type ReactNode,
  type TouchEvent,
  type UIEvent,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type WheelEvent,
} from 'react'
import { resultEnvelope } from '@/components/function-trigger/FunctionTriggerCard'
import {
  rawRedactor,
  useFunctionTriggerRenderers,
} from '@/components/function-trigger/renderer-registry'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { Button } from '@/components/ui/Button'
import { useMediaQuery } from '@/hooks/use-media-query'
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
import {
  collapsedFunctionTriggerCalls,
  functionTriggerGroups,
  type MessageListRow,
} from './function-trigger-groups'
import { Message } from './Message'
import {
  DEFAULT_SYSTEM_PROMPT_STATE,
  type SystemPromptState,
} from './system-prompt-selection'
import { TurnRail } from './TurnRail'
import {
  nextTailScrollTop,
  TAIL_GLIDE_SETTLE_DISTANCE_PX,
  TAIL_REARM_DISTANCE_PX,
  type TailScrollState,
  tailScrollTarget,
  tailStateAfterScroll,
} from './tail-scroll'

interface MessageListProps {
  messages: MessageType[]
  /**
   * False while an existing conversation is still loading its durable
   * transcript. Initializing content stays pinned instantly until this flips,
   * so opening a chat never glides through a partial history.
   */
  transcriptHydrated?: boolean
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
  /** Open the model/provider picker from the empty provider state. */
  onConfigureProvider?: () => void
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
  transcriptHydrated = true,
  isThinking,
  thinkingDetail,
  density = 'route',
  header,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  onConfigureProvider,
  workingDir,
  defaultOpenCalls,
  triggersById,
}: MessageListProps) {
  const containerRef = useRef<HTMLElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const didInitialScrollRef = useRef(false)
  const animationFrameRef = useRef<number | null>(null)
  const lastAnimationTimeRef = useRef<number | null>(null)
  const lastScrollTopRef = useRef(0)
  const lastTouchYRef = useRef<number | null>(null)
  const tailStateRef = useRef<TailScrollState>('initializing')
  const [tailState, setTailState] = useState<TailScrollState>('initializing')
  const reducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')
  const reducedMotionRef = useRef(reducedMotion)
  reducedMotionRef.current = reducedMotion

  const fcallsByAssistant = useMemo(
    () => functionTriggersByAssistant(messages),
    [messages],
  )
  const rows = useMemo(() => functionTriggerGroups(messages), [messages])
  const registrations = useMemo(
    () => resolveRegistrations(messages, triggersById),
    [messages, triggersById],
  )
  const newestPendingApprovalId = useMemo(() => {
    for (let index = messages.length - 1; index >= 0; index--) {
      const message = messages[index]
      if (
        message.role === 'function-trigger' &&
        message.pendingApproval === true
      ) {
        return message.id
      }
    }
    return null
  }, [messages])
  const hasPendingApproval = newestPendingApprovalId !== null
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

  const transitionTailState = useCallback((next: TailScrollState) => {
    if (tailStateRef.current === next) return
    tailStateRef.current = next
    setTailState(next)
  }, [])

  const cancelTailAnimation = useCallback(() => {
    if (animationFrameRef.current !== null) {
      cancelAnimationFrame(animationFrameRef.current)
      animationFrameRef.current = null
    }
    lastAnimationTimeRef.current = null
  }, [])

  const writeScrollTop = useCallback(
    (container: HTMLElement, scrollTop: number) => {
      container.scrollTop = scrollTop
      // Programmatic scroll events arrive later. Recording the clamped value
      // now keeps them from looking like manual upward movement.
      lastScrollTopRef.current = container.scrollTop
    },
    [],
  )

  /**
   * One retargetable animation follows the live tail. ResizeObserver callbacks
   * only ask for this loop; they never create competing smooth-scroll jobs.
   */
  const scheduleTailFollow = useCallback(() => {
    const container = containerRef.current
    if (!container || tailStateRef.current !== 'following') return

    const target = tailScrollTarget(container)
    if (reducedMotionRef.current) {
      cancelTailAnimation()
      writeScrollTop(container, target)
      return
    }
    if (animationFrameRef.current !== null) return

    const step = (time: number) => {
      animationFrameRef.current = null
      const currentContainer = containerRef.current
      if (!currentContainer || tailStateRef.current !== 'following') {
        lastAnimationTimeRef.current = null
        return
      }

      const nextTarget = tailScrollTarget(currentContainer)
      const current = currentContainer.scrollTop
      const remaining = nextTarget - current
      if (Math.abs(remaining) <= TAIL_GLIDE_SETTLE_DISTANCE_PX) {
        writeScrollTop(currentContainer, nextTarget)
        lastAnimationTimeRef.current = null
        return
      }

      const previousTime = lastAnimationTimeRef.current
      const elapsed = previousTime === null ? 1000 / 60 : time - previousTime
      lastAnimationTimeRef.current = time
      writeScrollTop(
        currentContainer,
        nextTailScrollTop(current, nextTarget, elapsed),
      )
      animationFrameRef.current = requestAnimationFrame(step)
    }

    animationFrameRef.current = requestAnimationFrame(step)
  }, [cancelTailAnimation, writeScrollTop])

  const hasScrollableContent = messages.length > 0 || Boolean(header)

  /* Opening a session lands on the latest exchange before paint. ChatView is
     keyed by conversation id, so every open gets a fresh initializing state;
     the first hydrated content is always an instant jump, never a glide. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: message identity and thinking are deliberate pre-paint layout triggers while a transcript hydrates.
  useLayoutEffect(() => {
    if (didInitialScrollRef.current || !hasScrollableContent) return
    if (tailStateRef.current === 'paused') return
    const container = containerRef.current
    if (!container) return
    cancelTailAnimation()
    writeScrollTop(container, tailScrollTarget(container))
    if (!transcriptHydrated) return
    didInitialScrollRef.current = true
    transitionTailState('following')
  }, [
    cancelTailAnimation,
    hasScrollableContent,
    isThinking,
    messages,
    transcriptHydrated,
    transitionTailState,
    writeScrollTop,
  ])

  /* Content height can change without a messages identity change: markdown
     wraps, images load, results expand, the shimmer mounts, or the pane
     resizes. Observe both the content and viewport, coalescing all of it into
     the single follow loop. A paused reader is never moved. */
  useEffect(() => {
    if (!hasScrollableContent || typeof ResizeObserver === 'undefined') return
    const container = containerRef.current
    const content = contentRef.current
    if (!container || !content) return

    const observer = new ResizeObserver(() => {
      if (tailStateRef.current === 'paused') return
      if (!didInitialScrollRef.current) {
        // Hydration can include late layout (images, rich cards, fonts). Keep
        // initialization pinned with direct writes; never animate partial data.
        writeScrollTop(container, tailScrollTarget(container))
        return
      }
      scheduleTailFollow()
    })
    observer.observe(container)
    observer.observe(content)
    return () => observer.disconnect()
  }, [hasScrollableContent, scheduleTailFollow, writeScrollTop])

  useEffect(() => {
    if (reducedMotion && tailStateRef.current === 'following') {
      scheduleTailFollow()
    }
  }, [reducedMotion, scheduleTailFollow])

  useEffect(() => cancelTailAnimation, [cancelTailAnimation])

  const pauseTailFollow = useCallback(() => {
    const container = containerRef.current
    if (
      !container ||
      container.scrollHeight <= container.clientHeight + TAIL_REARM_DISTANCE_PX
    ) {
      return
    }
    cancelTailAnimation()
    transitionTailState('paused')
  }, [cancelTailAnimation, transitionTailState])

  const handleScroll = useCallback(
    (event: UIEvent<HTMLElement>) => {
      const container = event.currentTarget
      const currentScrollTop = container.scrollTop
      const nextState = tailStateAfterScroll(
        tailStateRef.current,
        lastScrollTopRef.current,
        container,
      )
      if (nextState === 'paused') cancelTailAnimation()
      transitionTailState(nextState)
      lastScrollTopRef.current = currentScrollTop
    },
    [cancelTailAnimation, transitionTailState],
  )

  const handleWheel = useCallback(
    (event: WheelEvent<HTMLElement>) => {
      if (event.deltaY < 0) pauseTailFollow()
    },
    [pauseTailFollow],
  )

  const handleTouchStart = useCallback((event: TouchEvent<HTMLElement>) => {
    lastTouchYRef.current = event.touches[0]?.clientY ?? null
  }, [])

  const handleTouchMove = useCallback(
    (event: TouchEvent<HTMLElement>) => {
      const nextY = event.touches[0]?.clientY
      const previousY = lastTouchYRef.current
      if (nextY !== undefined && previousY !== null && nextY > previousY) {
        pauseTailFollow()
      }
      lastTouchYRef.current = nextY ?? null
    },
    [pauseTailFollow],
  )

  const clearTouchPosition = useCallback(() => {
    lastTouchYRef.current = null
  }, [])

  const handleJumpToLatest = useCallback(() => {
    didInitialScrollRef.current = true
    transitionTailState('following')
    containerRef.current?.focus({ preventScroll: true })
    scheduleTailFollow()
  }, [scheduleTailFollow, transitionTailState])

  const handleJumpToAction = useCallback(() => {
    const container = containerRef.current
    const content = contentRef.current
    if (!container || !content || !newestPendingApprovalId) {
      handleJumpToLatest()
      return
    }

    const approval = Array.from(
      content.querySelectorAll<HTMLElement>('[data-message-id]'),
    ).find((node) => node.dataset.messageId === newestPendingApprovalId)
    if (!approval) {
      handleJumpToLatest()
      return
    }

    cancelTailAnimation()
    const containerRect = container.getBoundingClientRect()
    const approvalRect = approval.getBoundingClientRect()
    const centeredTop =
      container.scrollTop +
      approvalRect.top -
      containerRect.top -
      (container.clientHeight - approvalRect.height) / 2
    const target = Math.max(
      0,
      Math.min(tailScrollTarget(container), centeredTop),
    )

    if (reducedMotionRef.current) writeScrollTop(container, target)
    else container.scrollTo({ top: target, behavior: 'smooth' })

    const action = approval.querySelector<HTMLElement>(
      '[data-approval-actions] button:not([disabled])',
    )
    const focusTarget = action ?? approval
    focusTarget.focus({ preventScroll: true })
  }, [
    cancelTailAnimation,
    handleJumpToLatest,
    newestPendingApprovalId,
    writeScrollTop,
  ])

  if (messages.length === 0 && !header) {
    return (
      <EmptyState {...resolveEmptyState(ctx, density, onConfigureProvider)} />
    )
  }

  const listPad =
    density === 'dock'
      ? 'px-3 py-5 sm:px-4 sm:py-6'
      : 'px-3 py-5 sm:px-6 sm:py-7 lg:px-9 lg:py-8'

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1">
      <TurnRail
        messages={messages}
        container={containerRef}
        content={contentRef}
      />
      <section
        ref={containerRef}
        data-tail-scroll={tailState}
        aria-label="conversation messages"
        tabIndex={-1}
        className={cn(
          'min-h-0 min-w-0 flex-1 overflow-y-auto focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-accent',
          listPad,
        )}
        onScroll={handleScroll}
        onWheel={handleWheel}
        onTouchStart={handleTouchStart}
        onTouchMove={handleTouchMove}
        onTouchEnd={clearTouchPosition}
        onTouchCancel={clearTouchPosition}
      >
        <div
          ref={contentRef}
          className="mx-auto flex max-w-[720px] flex-col gap-y-6 sm:gap-y-8"
        >
          {header}
          {rows.map((row, i) => {
            if (row.kind === 'function-trigger-group') {
              return (
                <FunctionTriggerGroup
                  key={row.id}
                  row={row}
                  renderers={renderers}
                  defaultOpenCalls={defaultOpenCalls}
                  onResolveApproval={onResolveApproval}
                  onAlwaysAllow={onAlwaysAllow}
                  onResolveFilesystemAccess={onResolveFilesystemAccess}
                  onManageFilesystemAccess={onManageFilesystemAccess}
                  workingDir={workingDir}
                  summaryCopyText={
                    row.summary
                      ? (() => {
                          const calls = fcallsByAssistant.get(row.summary.id)
                          return calls?.length
                            ? () =>
                                assistantCopyText(
                                  row.summary?.content ?? '',
                                  calls,
                                  redactFor,
                                )
                            : row.summary.content
                        })()
                      : undefined
                  }
                />
              )
            }

            const m = row.message
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
            // Consecutive trigger-fired notices share the same compact visual
            // language as call groups, so keep those system rows close too.
            const triggerFired = (x: MessageListRow | undefined) =>
              x?.kind === 'message' &&
              x.message.role === 'system' &&
              x.message.kind === 'trigger-fired'
            const tight = triggerFired(row) && triggerFired(rows[i - 1])
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
              <div key={m.id} className="-mt-6.5">
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
        </div>
      </section>
      {tailState === 'paused' ? (
        <Button
          type="button"
          variant="pill"
          size="sm"
          className={cn(
            'iii-ui-motion-overlay absolute bottom-3 left-1/2 z-10 h-9 -translate-x-1/2 rounded-full bg-panel-raised/80 pr-3 pl-2.5 font-medium text-base shadow-raised backdrop-blur-md sm:h-8 sm:text-[0.8125rem]',
            hasPendingApproval && 'text-warn',
          )}
          onClick={hasPendingApproval ? handleJumpToAction : handleJumpToLatest}
          aria-label={
            hasPendingApproval
              ? 'jump to action required'
              : 'jump to latest message'
          }
        >
          <span
            className="absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <ArrowDown aria-hidden="true" className="stroke-current" />
          <span>{hasPendingApproval ? 'action required' : 'latest'}</span>
        </Button>
      ) : null}
    </div>
  )
}

interface FunctionTriggerGroupProps {
  row: Extract<MessageListRow, { kind: 'function-trigger-group' }>
  renderers: ReturnType<typeof useFunctionTriggerRenderers>
  defaultOpenCalls?: boolean
  summaryCopyText?: string | (() => string)
  onResolveApproval?: MessageListProps['onResolveApproval']
  onAlwaysAllow?: MessageListProps['onAlwaysAllow']
  onResolveFilesystemAccess?: MessageListProps['onResolveFilesystemAccess']
  onManageFilesystemAccess?: MessageListProps['onManageFilesystemAccess']
  workingDir?: string | null
}

/**
 * A sequence of agent calls reads as one phase. Collapsed groups keep the
 * latest call plus any rich-display, approval, or live call visible; expanding
 * restores every call in chronological order. The assistant's intermediate
 * prose stays normal prose after the stack and doubles as the batch summary.
 */
function FunctionTriggerGroup({
  row,
  renderers,
  defaultOpenCalls,
  summaryCopyText,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
}: FunctionTriggerGroupProps) {
  const [expanded, setExpanded] = useState(!!defaultOpenCalls)
  const contentId = useId()
  const collapsedCalls = collapsedFunctionTriggerCalls(row.calls, (call) =>
    renderers.some(
      (renderer) =>
        renderer.metadata?.display === true &&
        renderer.isMatch(call.functionId),
    ),
  )
  const hiddenCount = row.calls.length - collapsedCalls.length
  const visibleCalls = expanded ? row.calls : collapsedCalls
  const canCollapse = hiddenCount > 0

  return (
    <section
      className="flex flex-col gap-y-8"
      data-function-trigger-group=""
      data-function-trigger-count={row.calls.length}
    >
      <div className="flex flex-col gap-y-8">
        {canCollapse ? (
          <button
            type="button"
            aria-expanded={expanded}
            aria-controls={contentId}
            onClick={() => setExpanded((value) => !value)}
            className="group relative flex w-fit cursor-pointer items-center gap-2 font-mono text-base text-ink-faint hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent sm:text-[0.8125rem]"
          >
            <span
              className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
            <ChevronRight
              aria-hidden="true"
              className={cn(
                'size-5 shrink-0 transition-transform duration-150 motion-reduce:transition-none sm:size-4',
                expanded && 'rotate-90',
              )}
            />
            <span className="tabular-nums">{row.calls.length} triggers</span>
            <span className="text-ink-ghost">
              · {expanded ? 'show latest' : 'show all'}
            </span>
          </button>
        ) : null}
        <div
          id={contentId}
          className={cn('flex flex-col gap-y-8', canCollapse && '-mt-6.5')}
          data-function-trigger-group-calls=""
        >
          {visibleCalls.map((call, index) => (
            <div key={call.id} className={cn(index > 0 && '-mt-6.5')}>
              <Message
                message={call}
                defaultOpenCalls={defaultOpenCalls}
                onResolveApproval={onResolveApproval}
                onAlwaysAllow={onAlwaysAllow}
                onResolveFilesystemAccess={onResolveFilesystemAccess}
                onManageFilesystemAccess={onManageFilesystemAccess}
                workingDir={workingDir}
              />
            </div>
          ))}
        </div>
      </div>
      {row.summary ? (
        <Message message={row.summary} copyText={summaryCopyText} />
      ) : null}
    </section>
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
  onConfigureProvider?: () => void,
): EmptyStateProps {
  if (!ctx) return { variant: 'ready', density }

  const { harnessStatus, modelOptions, catalogLoading, active } = ctx
  const base: EmptyStateProps = {
    variant: 'ready',
    density,
    stages: harnessStatus.stages,
    errorMessage: harnessStatus.error,
    onInstallHarness: harnessStatus.install,
    onRetryInstall: harnessStatus.retry,
    onConfigureProvider,
    /* The system prompt is chosen here and nowhere else. It persists on the
       conversation record, so it survives this view being keyed away and
       back on a chat-tab switch. */
    systemPrompt: active?.systemPrompt ?? DEFAULT_SYSTEM_PROMPT_STATE,
    onSystemPromptChange: active
      ? (next: SystemPromptState) => ctx.setSystemPrompt(active.id, next)
      : undefined,
  }

  if (harnessStatus.error) return { ...base, variant: 'install-failed' }
  if (harnessStatus.installing) return { ...base, variant: 'installing' }
  if (harnessStatus.loading) return base
  if (!harnessStatus.present) return { ...base, variant: 'no-harness' }
  if (catalogLoading) return base
  if (modelOptions.length === 0) return { ...base, variant: 'no-provider' }
  return base
}
