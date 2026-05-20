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
      <div
        className="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint"
        title={`${tokens.toLocaleString()} tokens (context window unknown)`}
      >
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

  const hint =
    tone === 'normal'
      ? null
      : tone === 'warn'
        ? 'consider /compact'
        : 'pre-flight compaction imminent'

  return (
    <div
      className="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint"
      title={`${tokens.toLocaleString()} / ${contextWindow.toLocaleString()} tokens (${pct}%)${
        hint ? ` — ${hint}` : ''
      }`}
    >
      <span>ctx</span>
      <div
        className="relative w-14 h-[6px] bg-rule-2 border border-rule overflow-hidden"
        role="progressbar"
        aria-label="context window usage"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div className={fillClass} style={{ width: `${pct}%` }} />
      </div>
      <span className={cn('tabular-nums', labelToneClass)}>{pct}%</span>
      <span className="text-ink-ghost normal-case tracking-normal">
        {formatTokenCount(tokens)}/{formatTokenCount(contextWindow)}
      </span>
    </div>
  )
}
