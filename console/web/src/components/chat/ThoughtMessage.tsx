import { Caret } from '@/components/ui/Caret'
import type { ThoughtMessage as ThoughtMessageType } from '@/types/chat'

interface ThoughtMessageProps {
  message: ThoughtMessageType
}

export function ThoughtMessage({ message }: ThoughtMessageProps) {
  // Reasoning is an ephemeral live-progress surface. Once its stream closes,
  // render null so React removes the entire thought subtree from the DOM.
  if (!message.streaming) return null

  return (
    <details className="iii-details group/thought" open>
      <summary className="inline-flex items-center gap-2 font-sans text-[12px] text-ink-faint hover:text-ink transition-colors select-none">
        <span
          aria-hidden
          className="iii-chev text-ink-ghost w-[8px] inline-block"
        >
          ▸
        </span>
        <span className="flex items-center gap-1.5">
          <span className="thinking-shimmer">Thought…</span>
          <Caret className="h-[10px] w-[5px]" />
        </span>
      </summary>
      <div className="mt-2 ml-2 pl-3 border-l border-rule-2 font-sans text-[13px] leading-[1.7] text-ink-faint italic whitespace-pre-wrap break-words">
        {message.content || (
          <span className="text-ink-ghost not-italic">No content yet…</span>
        )}
      </div>
    </details>
  )
}
