import { useEffect, useMemo, useRef } from 'react'
import { Prompt } from '@/components/ui/Prompt'
import { cn } from '@/lib/utils'
import type {
  FunctionCallMessage as FunctionCallMessageType,
  Message as MessageType,
} from '@/types/chat'
import { FunctionCallGroup } from './FunctionCallGroup'
import { Message } from './Message'

interface MessageListProps {
  messages: MessageType[]
  /** Show "thinking…" shimmer at the bottom while the agent is between
      visible outputs (after submit, or between fcall-end and the next
      turn's first token). */
  isThinking?: boolean
  density?: 'route' | 'dock'
  onResolveApproval?: (
    sessionId: string,
    functionCallId: string,
    decision: 'allow' | 'deny',
  ) => Promise<void>
  onAlwaysAllow?: (
    sessionId: string,
    functionCallId: string,
    functionId: string,
  ) => Promise<void>
}

type RenderItem =
  | { kind: 'message'; key: string; message: MessageType }
  | { kind: 'fcall-group'; key: string; messages: FunctionCallMessageType[] }

/**
 * Collapse runs of consecutive `function-call` messages into a single
 * `fcall-group` item. Single-call runs stay rendered as a standalone
 * `Message` so happy-agent, pending-approval, and error-on-fcall look
 * identical to today. Only runs of 2+ get the group accordion.
 *
 * The group's key is anchored to the first call's id so the React tree
 * stays stable as later calls land in the same run.
 */
function groupConsecutiveFcalls(messages: MessageType[]): RenderItem[] {
  const out: RenderItem[] = []
  let buffer: FunctionCallMessageType[] = []

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
    if (m.role === 'function-call') {
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
  density = 'route',
  onResolveApproval,
  onAlwaysAllow,
}: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const lastPendingIdRef = useRef<string | null>(null)

  const items = useMemo(() => groupConsecutiveFcalls(messages), [messages])

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
      if (m.role === 'function-call' && m.pendingApproval === true) {
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

  if (messages.length === 0) {
    return <EmptyState density={density} />
  }

  const listPad = density === 'dock' ? 'px-4 py-6' : 'px-9 py-8'

  return (
    <div ref={containerRef} className={cn('flex-1 overflow-y-auto', listPad)}>
      <div className="mx-auto max-w-[760px] flex flex-col gap-y-8">
        {items.map((item) =>
          item.kind === 'message' ? (
            <Message
              key={item.key}
              message={item.message}
              onResolveApproval={onResolveApproval}
              onAlwaysAllow={onAlwaysAllow}
            />
          ) : (
            <FunctionCallGroup
              key={item.key}
              messages={item.messages}
              onResolveApproval={onResolveApproval}
              onAlwaysAllow={onAlwaysAllow}
            />
          ),
        )}
        {isThinking ? (
          <div className="font-mono text-[13px] italic thinking-shimmer text-ink-faint">
            thinking…
          </div>
        ) : null}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}

function EmptyState({ density }: { density: 'route' | 'dock' }) {
  const emptyPad = density === 'dock' ? 'px-4' : 'px-9'
  return (
    <div className={cn('flex-1 flex items-center justify-center', emptyPad)}>
      <div className="max-w-[520px] w-full flex flex-col gap-6">
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          <Prompt symbol="$">new session</Prompt>
        </div>
        <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase">
          how can i help.
        </h1>
        <p className="font-mono text-[14px] leading-[1.7] text-ink-faint lowercase">
          pick a mode and a model, attach files if you need to, then send a
          message. responses are mocked locally — swap{' '}
          <code className="font-mono text-[12.5px] border border-rule-2 bg-paper-2 text-ink px-1">
            lib/backend/real.ts
          </code>{' '}
          for a real provider when you're ready.
        </p>
        <ul className="font-mono text-[13px] leading-[1.7] text-ink-faint flex flex-col gap-1">
          <li>
            · <span className="text-ink">plan</span> — outline an approach
            before doing.
          </li>
          <li>
            · <span className="text-ink">ask</span> — answer a question with
            context.
          </li>
          <li>
            · <span className="text-ink">agent</span> — take action and report
            back.
          </li>
        </ul>
      </div>
    </div>
  )
}
