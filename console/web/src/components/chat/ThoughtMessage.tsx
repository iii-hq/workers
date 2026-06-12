import { Caret } from '@/components/ui/Caret'
import { cn } from '@/lib/utils'
import type { ThoughtMessage as ThoughtMessageType } from '@/types/chat'

interface ThoughtMessageProps {
  message: ThoughtMessageType
  /** Force the panel open. Used by the examples showcase. */
  defaultOpen?: boolean
}

function thoughtLabel(durationMs: number): string {
  if (durationMs < 1500) return 'briefly'
  return `for ${(durationMs / 1000).toFixed(1)}s`
}

export function ThoughtMessage({ message, defaultOpen }: ThoughtMessageProps) {
  const streaming = !!message.streaming
  // Auto-open while the thought streams so reasoning is visible in real
  // time; the flip back when streaming ends collapses it to its summary.
  return (
    <details
      className="iii-details group/thought"
      open={defaultOpen || streaming}
    >
      <summary
        className={cn(
          'inline-flex items-center gap-2 font-mono text-[12px] text-ink-faint hover:text-ink transition-colors lowercase select-none',
        )}
      >
        <span
          aria-hidden
          className="iii-chev text-ink-ghost w-[8px] inline-block"
        >
          ▸
        </span>
        {streaming ? (
          <span className="flex items-center gap-1.5">
            <span className="thinking-shimmer">thought…</span>
            <Caret className="h-[10px] w-[5px]" />
          </span>
        ) : (
          <span>
            thought{' '}
            <span className="text-ink-ghost">
              {thoughtLabel(message.durationMs)}
            </span>
          </span>
        )}
      </summary>
      <div
        className={cn(
          'mt-2 ml-2 pl-3 border-l border-rule-2',
          'font-mono text-[13px] leading-[1.7] text-ink-faint italic whitespace-pre-wrap break-words',
        )}
      >
        {message.content || (
          <span className="text-ink-ghost not-italic">no content yet…</span>
        )}
      </div>
    </details>
  )
}
