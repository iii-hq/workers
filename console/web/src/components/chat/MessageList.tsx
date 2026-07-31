import { type ReactNode, useEffect, useMemo, useRef } from 'react'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import {
  assistantCopyText,
  functionTriggersByAssistant,
} from '@/lib/function-trigger-copy'
import { cn } from '@/lib/utils'
import type {
  FunctionTriggerMessage as FunctionTriggerMessageType,
  Message as MessageType,
} from '@/types/chat'
import { EmptyState, type EmptyStateProps } from './EmptyState'
import { FunctionTriggerGroup } from './FunctionTriggerGroup'
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
}

type RenderItem =
  | { kind: 'message'; key: string; message: MessageType }
  | { kind: 'fcall-group'; key: string; messages: FunctionTriggerMessageType[] }

/**
 * Collapse runs of consecutive `function-trigger` messages into a single
 * `fcall-group` item. Single-call runs stay rendered as a standalone
 * `Message` so happy-agent, pending-approval, and error-on-fcall look
 * identical to today. Only runs of 2+ get the group accordion.
 *
 * The group's key is anchored to the first call's id so the React tree
 * stays stable as later calls land in the same run.
 */
function groupConsecutiveFcalls(messages: MessageType[]): RenderItem[] {
  const out: RenderItem[] = []
  let buffer: FunctionTriggerMessageType[] = []

  const flush = () => {
    if (buffer.length === 0) return
    if (buffer.length === 1) {
      const only = buffer[0]
      out.push({ kind: 'message', key: only.id, message: only })
    } else {
      out.push({
        kind: 'fcall-group',
        key: `fcall-group:${buffer[0].id}`,
        messages: buffer,
      })
    }
    buffer = []
  }

  for (const m of messages) {
    if (m.role === 'function-trigger') {
      buffer.push(m)
    } else {
      flush()
      out.push({ kind: 'message', key: m.id, message: m })
    }
  }
  flush()

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
}: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const lastPendingIdRef = useRef<string | null>(null)

  const items = useMemo(() => groupConsecutiveFcalls(messages), [messages])
  const fcallsByAssistant = useMemo(
    () => functionTriggersByAssistant(messages),
    [messages],
  )

  // Read optionally so isolated renders (Storybook) still work without the
  // ConversationsProvider; the empty state falls back to `ready` there.
  const ctx = useConversationsCtxOptional()

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
      <div className="mx-auto max-w-[760px] flex flex-col gap-y-8">
        {header}
        {items.map((item) => {
          if (item.kind === 'fcall-group') {
            return (
              <FunctionTriggerGroup
                key={item.key}
                messages={item.messages}
                defaultOpen={defaultOpenCalls}
                onResolveApproval={onResolveApproval}
                onAlwaysAllow={onAlwaysAllow}
                onResolveFilesystemAccess={onResolveFilesystemAccess}
                onManageFilesystemAccess={onManageFilesystemAccess}
                workingDir={workingDir}
              />
            )
          }
          const m = item.message
          // Assistant turns copy their prose plus the calls that follow them;
          // the thunk defers building that string until the copy click. Left
          // undefined when the turn has nothing to copy (no prose, no calls)
          // so the header shows no copy affordance.
          const calls =
            m.role === 'assistant' ? fcallsByAssistant.get(m.id) : undefined
          const copyText =
            m.role === 'assistant' && (m.content || calls?.length)
              ? () => assistantCopyText(m.content, calls ?? [])
              : undefined
          return (
            <Message
              key={item.key}
              message={m}
              copyText={copyText}
              defaultOpenCalls={defaultOpenCalls}
              onResolveApproval={onResolveApproval}
              onAlwaysAllow={onAlwaysAllow}
              onResolveFilesystemAccess={onResolveFilesystemAccess}
              onManageFilesystemAccess={onManageFilesystemAccess}
              workingDir={workingDir}
            />
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
    harnessState: harnessStatus.state,
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
