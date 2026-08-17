import { useMemo } from 'react'
import {
  estimateConversationTokens,
  formatTokenCount,
} from '@/lib/token-estimate'
import { cn } from '@/lib/utils'
import type { Message } from '@/types/chat'

interface ContextUsageProps {
  messages: readonly Message[]
  contextWindow?: number
}

const WARN_THRESHOLD = 0.75
const DANGER_THRESHOLD = 0.9

export function ContextUsage({ messages, contextWindow }: ContextUsageProps) {
  const tokens = useMemo(() => estimateConversationTokens(messages), [messages])

  if (!contextWindow || contextWindow <= 0) {
    return (
      <div className="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        <span>ctx</span>
        <span className="tabular-nums text-ink">
          {formatTokenCount(tokens)}
        </span>
      </div>
    )
  }

  const ratio = Math.min(1, tokens / contextWindow)
  const pct = Math.round(ratio * 100)

  let tone: 'normal' | 'warn' | 'danger' = 'normal'
  if (ratio >= DANGER_THRESHOLD) tone = 'danger'
  else if (ratio >= WARN_THRESHOLD) tone = 'warn'

  const fillClass = cn(
    'h-full transition-[width,background-color] duration-200',
    tone === 'normal' && 'bg-accent',
    tone === 'warn' && 'bg-warn',
    tone === 'danger' && 'bg-danger',
  )

  const labelToneClass = cn(
    tone === 'normal' && 'text-ink',
    tone === 'warn' && 'text-warn',
    tone === 'danger' && 'text-danger',
  )

  /* No tooltip: the bar, the percentage and the counts already say it, and
     only the system-prompt and status read-outs carry one now. The
     threshold hint rides the tone colour instead of prose. */
  return (
    <div className="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
      <span>ctx</span>
      <div
        // `surface-active`, not `surface`: the header group this sits in is
        // itself `bg-surface`, so a same-token track would vanish into it.
        className="relative w-14 h-[6px] bg-surface-active overflow-hidden"
        role="progressbar"
        aria-label="context window usage"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div className={fillClass} style={{ width: `${pct}%` }} />
      </div>
      <span className={cn('tabular-nums', labelToneClass)}>{pct}%</span>
      {/* `ink-faint` (5.1:1), not `ink-ghost` (2.4:1 on panel-raised — under
          the 4.5:1 AA floor). Load-bearing numbers, not chrome. */}
      <span className="text-ink-faint normal-case tracking-normal">
        {formatTokenCount(tokens)}/{formatTokenCount(contextWindow)}
      </span>
    </div>
  )
}
