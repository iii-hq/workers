import { useEffect, useRef } from 'react'
import { Prompt } from '@/components/ui/Prompt'
import type { Message as MessageType } from '@/types/chat'
import { Message } from './Message'

interface MessageListProps {
  messages: MessageType[]
}

export function MessageList({ messages }: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  /* Auto-scroll only when the user is already near the bottom. The effect body
     reads layout off refs but the trigger we care about is "messages changed",
     so list it explicitly. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: messages is the trigger, not a value read in the body.
  useEffect(() => {
    const c = containerRef.current
    if (!c) return
    const distanceFromBottom = c.scrollHeight - c.scrollTop - c.clientHeight
    if (distanceFromBottom < 200) {
      bottomRef.current?.scrollIntoView({ block: 'end' })
    }
  }, [messages])

  if (messages.length === 0) {
    return <EmptyState />
  }

  return (
    <div ref={containerRef} className="flex-1 overflow-y-auto px-9 py-8">
      <div className="mx-auto max-w-[760px] flex flex-col gap-y-8">
        {messages.map((m) => (
          <Message key={m.id} message={m} />
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}

function EmptyState() {
  return (
    <div className="flex-1 flex items-center justify-center px-9">
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
