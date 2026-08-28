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
import {
  collapsedTimelineActivities,
  groupTimelineActivities,
  type TimelineActivityGroupRow,
  triggerActivityRows,
} from '@/components/trigger-activity/grouping'
import {
  registrationFromCall,
  registrationFromRow,
  type TriggerRegistration,
} from '@/components/trigger-activity/model'
import { Button } from '@/components/ui/Button'
import { ImageViewer } from '@/components/ui/ImageViewer'
import { useMediaQuery } from '@/hooks/use-media-query'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import {
  assistantCopyText,
  functionTriggersByAssistant,
} from '@/lib/function-trigger-copy'
import {
  notificationBindingId,
  triggerFiredName,
} from '@/lib/sessions/entry-mapper'
import { cn } from '@/lib/utils'
import type { Message as MessageType } from '@/types/chat'
import type { WorktreePickerOptions } from './DirectoryPicker'
import { EmptyState, type EmptyStateProps } from './EmptyState'
import { functionTriggerGroups } from './function-trigger-groups'
import { imageZoomTarget } from './image-zoom-target'
import { Message, type SpawnTaskContext } from './Message'
import { ModelWaitingIndicator } from './ModelWaitingIndicator'
import {
  DEFAULT_SYSTEM_PROMPT_STATE,
  type SkillSelection,
  type SystemPromptState,
} from './system-prompt-selection'
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
  /** Show the model-waiting indicator at the bottom while the agent is
      between visible outputs (after submit, or between fcall-end and the
      next turn's first token). */
  isThinking?: boolean
  /** Under-the-hood context shown in the waiting indicator (e.g. "dispatching
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
  /** Current child-session identity shown by direct spawn seed messages. */
  spawnContext?: SpawnTaskContext
  /** Session agent profile name shown on assistant message headers. */
  agentName?: string
  workingDir?: string | null
  onWorkingDirChange?: (next: string) => void
  workingDirError?: string | null
  defaultWorkingDir?: string | null
  worktreePicker?: WorktreePickerOptions
  /**
   * Render every function-call card (and group) already expanded. Off in the
   * product, where a turn's calls collapse to one line each; on for showcase
   * surfaces whose whole point is the result renderers.
   */
  defaultOpenCalls?: boolean
  /** Registration rows by subscription id, for trigger-fired card detail. */
  triggersById?: ReadonlyMap<string, SessionTriggerInfo>
  /**
   * External landing request (trace → "go to message"): once this row's node
   * exists, the list centers it, flashes it, and calls
   * `onFocusMessageHandled` so the owner can consume the request. A target
   * hidden behind a collapsed activity group is revealed first.
   */
  focusMessageId?: string | null
  onFocusMessageHandled?: () => void
}

/**
 * Pull `subscription_id` out of a register-call output. Outputs arrive as the
 * `{ content: [{text}], details }` result envelope (see functionResultOutput);
 * the id lives in a JSON text block — bare objects/strings are handled too.
 */
export function subscriptionIdOf(output: unknown): string | null {
  return registrationResultOf(output).subscriptionId ?? null
}

function registrationResultOf(output: unknown): {
  subscriptionId?: string
  once?: boolean
  note?: string
} {
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
  const parseRegistrationResult = (
    v: unknown,
  ): { subscriptionId?: string; once?: boolean; note?: string } => {
    if (typeof v === 'string') {
      try {
        return parseRegistrationResult(JSON.parse(v))
      } catch {
        return {}
      }
    }
    if (!v || typeof v !== 'object' || Array.isArray(v)) return {}
    const record = v as Record<string, unknown>
    return {
      ...(typeof record.subscription_id === 'string'
        ? { subscriptionId: record.subscription_id }
        : {}),
      ...(typeof record.once === 'boolean' ? { once: record.once } : {}),
      ...(typeof record.note === 'string' ? { note: record.note } : {}),
    }
  }
  const envelope = resultEnvelope(output)
  if (envelope) {
    for (const text of envelope.texts) {
      const result = parseRegistrationResult(text)
      if (result.subscriptionId) return result
    }
    return parseRegistrationResult(envelope.details)
  }
  const result = parseRegistrationResult(output)
  return result.subscriptionId
    ? result
    : { subscriptionId: idOf(output) ?? undefined }
}

/**
 * Registration detail per message id, for trigger-fired notices AND the
 * notification deliveries their notify-wakes produce. Two-tier per
 * subscription: the harness's live/seen binding row when available, else
 * recovered from the transcript — the `engine::register_trigger` call whose
 * result carries the same subscription id (its input IS the registration).
 * Notifications correlate by the binding id carried by their trusted origin
 * or recovered from the current/legacy deterministic entry id, with a
 * name-match fallback only for older records. Resolution is order-independent:
 * an idle-session wake appends the notification before its fire record.
 */
export function resolveRegistrations(
  messages: MessageType[],
  triggersById?: ReadonlyMap<string, SessionTriggerInfo>,
): Map<string, TriggerRegistration> {
  const fromCalls = new Map<string, TriggerRegistration>()
  const notifyFireByName = new Map<string, string>()
  for (const m of messages) {
    if (
      m.role === 'function-trigger' &&
      m.functionId === 'engine::register_trigger'
    ) {
      const result = registrationResultOf(m.output)
      const subId = result.subscriptionId
      if (subId && m.input !== undefined) {
        fromCalls.set(
          subId,
          registrationFromCall({
            id: m.id,
            input: m.input,
            subscriptionId: subId,
            effectiveOnce: result.once,
            note: result.note,
          }),
        )
      }
    } else if (m.role === 'system' && m.kind === 'trigger-fired' && m.trigger) {
      const t = m.trigger
      if (!t.target || t.target === 'notify' || t.target === 'harness::send')
        notifyFireByName.set(triggerFiredName(t), t.subscription_id)
    }
  }
  const regFor = (subscriptionId: string): TriggerRegistration | undefined => {
    const row = triggersById?.get(subscriptionId)
    const fromCall = fromCalls.get(subscriptionId)
    // A post-reload ghost row is reconstructed from the activity record. Even
    // when that record carries config, it does not retain registration-only
    // conditions or lifecycle limits; the transcript call is richer. The
    // activity record still overlays its current source/state in the card.
    if (row?.fired && fromCall) return fromCall
    if (row && row.config !== undefined) {
      return registrationFromRow(row)
    }
    return fromCall ?? (row ? registrationFromRow(row) : undefined)
  }
  const out = new Map<string, TriggerRegistration>()
  for (const m of messages) {
    if (m.role === 'system' && m.kind === 'trigger-fired' && m.trigger) {
      const t = m.trigger
      const reg =
        regFor(t.subscription_id) ??
        (t.trigger_type
          ? registrationFromCall({
              id: `trigger-registration:${t.subscription_id}`,
              subscriptionId: t.subscription_id,
              effectiveOnce: t.once,
              input: {
                trigger_type: t.trigger_type,
                config: t.config,
                label: t.label,
                metadata: t.action ? { action: t.action } : undefined,
                function_id:
                  t.target &&
                  t.target !== 'notify' &&
                  t.target !== 'harness::send'
                    ? t.target
                    : undefined,
              },
            })
          : undefined)
      if (reg) out.set(m.id, reg)
    } else if (m.role === 'user' && m.notification) {
      const fromId = m.triggerBindingId ?? notificationBindingId(m.id)
      const name = /^\[notification\]\s*([^:]+):/.exec(m.content)?.[1]?.trim()
      const fromName = name ? notifyFireByName.get(name) : undefined
      // Some legacy `e_notify_*` ids append an ordinal that is impossible to
      // distinguish from an underscore inside the subscription id. Trust an
      // id only when it resolves; otherwise retain the historical name match.
      const reg =
        (fromId ? regFor(fromId) : undefined) ??
        (fromName ? regFor(fromName) : undefined)
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
  spawnContext,
  agentName,
  workingDir,
  onWorkingDirChange,
  workingDirError,
  defaultWorkingDir,
  worktreePicker,
  defaultOpenCalls,
  triggersById,
  focusMessageId,
  onFocusMessageHandled,
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
  // Any content image in the transcript opens in the viewer on click,
  // whichever renderer produced it; controls keep their own behaviour.
  const [zoomedImage, setZoomedImage] = useState<{
    src: string
    alt: string
  } | null>(null)

  const reducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')
  const reducedMotionRef = useRef(reducedMotion)
  reducedMotionRef.current = reducedMotion

  const fcallsByAssistant = useMemo(
    () => functionTriggersByAssistant(messages),
    [messages],
  )
  const rows = useMemo(
    () =>
      groupTimelineActivities(
        triggerActivityRows(functionTriggerGroups(messages)),
      ),
    [messages],
  )
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
     wraps, images load, results expand, the waiting indicator mounts, or the pane
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

  /* External landing (trace → "go to message"): center the requested row
     once it exists, then hand the request back to the owner. Tail following
     is paused first so a live tail can't yank the view back down; the flash
     gives the jump a visible landmark. Gated on hydration so it never
     centers against a partial history. */
  const focusAppliedRef = useRef<string | null>(null)
  // biome-ignore lint/correctness/useExhaustiveDependencies: message arrival is the retry trigger while the target row hasn't rendered yet.
  useEffect(() => {
    if (!focusMessageId) {
      focusAppliedRef.current = null
      return
    }
    if (!transcriptHydrated || focusAppliedRef.current === focusMessageId) {
      return
    }
    const container = containerRef.current
    const content = contentRef.current
    if (!container || !content) return
    // `data-message-row` is the transcript-row identity (top-level rows,
    // group items, group summaries) — not `data-message-id`, which is the
    // card-level attribute the approval jump uses. A row that absorbed a
    // wake notification carries both entry ids, space-separated.
    const node = Array.from(
      content.querySelectorAll<HTMLElement>('[data-message-row]'),
    ).find((el) => el.dataset.messageRow?.split(' ').includes(focusMessageId))
    if (!node) return
    focusAppliedRef.current = focusMessageId
    cancelTailAnimation()
    didInitialScrollRef.current = true
    transitionTailState('paused')
    const containerRect = container.getBoundingClientRect()
    const nodeRect = node.getBoundingClientRect()
    const centeredTop =
      container.scrollTop +
      nodeRect.top -
      containerRect.top -
      (container.clientHeight - nodeRect.height) / 2
    const target = Math.max(
      0,
      Math.min(tailScrollTarget(container), centeredTop),
    )
    if (reducedMotionRef.current) writeScrollTop(container, target)
    else container.scrollTo({ top: target, behavior: 'smooth' })
    if (typeof node.animate === 'function') {
      node.animate(
        [
          {
            backgroundColor: 'color-mix(in srgb, currentColor 5%, transparent)',
          },
          { backgroundColor: 'transparent' },
        ],
        { duration: 900, easing: 'ease-out' },
      )
    }
    onFocusMessageHandled?.()
  }, [
    focusMessageId,
    transcriptHydrated,
    messages,
    onFocusMessageHandled,
    cancelTailAnimation,
    transitionTailState,
    writeScrollTop,
  ])

  if (messages.length === 0 && !header) {
    return (
      <EmptyState
        {...resolveEmptyState(ctx, density, onConfigureProvider, {
          workingDir,
          onWorkingDirChange,
          workingDirError,
          defaultWorkingDir,
          worktreePicker,
        })}
      />
    )
  }

  const listPad =
    density === 'dock'
      ? 'px-3 py-5 sm:px-4 sm:py-6'
      : 'px-3 py-5 sm:px-6 sm:py-7 lg:px-9 lg:py-8'

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1">
      <ImageViewer
        open={zoomedImage !== null}
        onOpenChange={(next) => {
          if (!next) setZoomedImage(null)
        }}
        src={zoomedImage?.src}
        alt={zoomedImage?.alt ?? 'image'}
      />
      <section
        ref={containerRef}
        data-message-list=""
        data-tail-scroll={tailState}
        aria-label="conversation messages"
        tabIndex={-1}
        className={cn(
          'min-h-0 min-w-0 flex-1 overflow-y-auto focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-accent',
          listPad,
        )}
        onScroll={handleScroll}
        onClick={(event) => {
          if (event.defaultPrevented) return
          const zoom = imageZoomTarget(event.target)
          if (zoom) setZoomedImage(zoom)
        }}
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
          {rows.map((row) => {
            if (row.kind === 'activity-group') {
              return (
                <div key={row.id} data-message-row={row.id}>
                  <FunctionTriggerGroup
                    row={row}
                    renderers={renderers}
                    registrations={registrations}
                    defaultOpenCalls={defaultOpenCalls}
                    focusMessageId={focusMessageId}
                    onResolveApproval={onResolveApproval}
                    onAlwaysAllow={onAlwaysAllow}
                    onResolveFilesystemAccess={onResolveFilesystemAccess}
                    onManageFilesystemAccess={onManageFilesystemAccess}
                    workingDir={workingDir}
                    agentName={agentName}
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
                </div>
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
            return (
              <div key={m.id} data-message-row={m.id}>
                <Message
                  message={m}
                  spawnContext={spawnContext}
                  agentName={agentName}
                  copyText={copyText}
                  defaultOpenCalls={defaultOpenCalls}
                  onResolveApproval={onResolveApproval}
                  onAlwaysAllow={onAlwaysAllow}
                  onResolveFilesystemAccess={onResolveFilesystemAccess}
                  onManageFilesystemAccess={onManageFilesystemAccess}
                  workingDir={workingDir}
                  registration={registrations.get(m.id)}
                />
              </div>
            )
          })}
          {isThinking ? <ModelWaitingIndicator label={thinkingDetail} /> : null}
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
  row: TimelineActivityGroupRow
  renderers: ReturnType<typeof useFunctionTriggerRenderers>
  registrations: ReadonlyMap<string, TriggerRegistration>
  defaultOpenCalls?: boolean
  summaryCopyText?: string | (() => string)
  onResolveApproval?: MessageListProps['onResolveApproval']
  onAlwaysAllow?: MessageListProps['onAlwaysAllow']
  onResolveFilesystemAccess?: MessageListProps['onResolveFilesystemAccess']
  onManageFilesystemAccess?: MessageListProps['onManageFilesystemAccess']
  workingDir?: string | null
  agentName?: string
  /** External landing target — a hidden matching item expands the group. */
  focusMessageId?: string | null
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
  registrations,
  defaultOpenCalls,
  summaryCopyText,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
  agentName,
  focusMessageId,
}: FunctionTriggerGroupProps) {
  const [expanded, setExpanded] = useState(!!defaultOpenCalls)
  const contentId = useId()
  const collapsedItems = collapsedTimelineActivities(row.items, (call) =>
    renderers.some(
      (renderer) =>
        renderer.metadata?.display === true &&
        renderer.isMatch(call.functionId),
    ),
  )
  // External landing (trace → "go to message"): a target hidden behind the
  // collapse must have a DOM row in the same render the request resolves, so
  // the reveal happens here, via the render-phase setState pattern. Latched
  // through `expanded` — not derived — so consuming the request doesn't
  // re-collapse the revealed row. An absorbed wake notification is this
  // row's transcript entry too, so it counts as a match.
  const ownsFocusTarget = (item: (typeof row.items)[number]) =>
    item.message.id === focusMessageId ||
    (item.kind === 'trigger-activity' &&
      item.notification?.id === focusMessageId)
  if (
    !expanded &&
    focusMessageId != null &&
    !collapsedItems.some(ownsFocusTarget) &&
    row.items.some(ownsFocusTarget)
  ) {
    setExpanded(true)
  }
  const hiddenCount = row.items.length - collapsedItems.length
  const visibleItems = expanded ? row.items : collapsedItems
  const canCollapse = hiddenCount > 0

  return (
    <section
      className="flex flex-col gap-y-8"
      data-function-trigger-group=""
      data-function-trigger-count={row.items.length}
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
            <span className="tabular-nums">{row.items.length} triggers</span>
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
          {visibleItems.map((item, index) => {
            const message = item.message
            const notification =
              item.kind === 'trigger-activity' ? item.notification : undefined
            return (
              <div
                key={item.id}
                // An item that absorbed its wake notification represents two
                // transcript entries; the row id carries both, space-separated.
                data-message-row={
                  notification ? `${message.id} ${notification.id}` : message.id
                }
                className={cn(index > 0 && '-mt-6.5')}
              >
                <Message
                  message={message}
                  agentName={agentName}
                  triggerNotification={notification}
                  registration={
                    registrations.get(message.id) ??
                    (notification
                      ? registrations.get(notification.id)
                      : undefined)
                  }
                  defaultOpenCalls={defaultOpenCalls}
                  onResolveApproval={onResolveApproval}
                  onAlwaysAllow={onAlwaysAllow}
                  onResolveFilesystemAccess={onResolveFilesystemAccess}
                  onManageFilesystemAccess={onManageFilesystemAccess}
                  workingDir={workingDir}
                />
              </div>
            )
          })}
        </div>
      </div>
      {row.summary ? (
        <div data-message-row={row.summary.id}>
          <Message
            message={row.summary}
            copyText={summaryCopyText}
            agentName={agentName}
          />
        </div>
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
  directory?: Pick<
    EmptyStateProps,
    | 'workingDir'
    | 'onWorkingDirChange'
    | 'workingDirError'
    | 'defaultWorkingDir'
    | 'worktreePicker'
  >,
): EmptyStateProps {
  if (!ctx) return { variant: 'ready', density, ...directory }

  const { harnessStatus, modelOptions, catalogLoading, active } = ctx
  const base: EmptyStateProps = {
    variant: 'ready',
    density,
    stages: harnessStatus.stages,
    errorMessage: harnessStatus.error,
    onInstallHarness: harnessStatus.install,
    onRetryInstall: harnessStatus.retry,
    onConfigureProvider,
    systemPrompt: active?.systemPrompt ?? DEFAULT_SYSTEM_PROMPT_STATE,
    onSystemPromptChange: active
      ? (next: SystemPromptState) => ctx.setSystemPrompt(active.id, next)
      : undefined,
    agentProfile: active?.agentProfile,
    onAgentProfileChange: active
      ? (next) => ctx.setAgentProfile(active.id, next)
      : undefined,
    skills: active?.skills,
    onSkillsChange: active
      ? (next: SkillSelection) => ctx.setSkills(active.id, next)
      : undefined,
    ...directory,
  }

  if (harnessStatus.error) return { ...base, variant: 'install-failed' }
  if (harnessStatus.installing) return { ...base, variant: 'installing' }
  if (harnessStatus.loading) return base
  if (!harnessStatus.present) return { ...base, variant: 'no-harness' }
  if (catalogLoading) return base
  if (modelOptions.length === 0) return { ...base, variant: 'no-provider' }
  return base
}
