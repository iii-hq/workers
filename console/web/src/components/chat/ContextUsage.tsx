import { useMemo } from 'react'
import type { SessionUsage } from '@/lib/session-usage'
import {
  estimateConversationTokens,
  formatTokenCount,
} from '@/lib/token-estimate'
import { cn } from '@/lib/utils'
import type { Message } from '@/types/chat'

/**
 * How close this conversation is to overflowing its context window.
 *
 * This is deliberately an ESTIMATE (chars ÷ 4) and stays one even now that
 * real provider usage is available, because the two measure different things:
 * this is current occupancy, while `Σ usage.input` is cumulative billing that
 * counts the same prompt once per step. The obvious substitute — the last
 * call's `input + cache_read + cache_write` — is right for anthropic (whose
 * `input` excludes cached tokens) and double-counts for openai/codex (whose
 * `input` includes them), so it cannot be computed portably in the browser.
 *
 * What we do instead: mark the number `~` so it never implies precision, and
 * put the last provider-reported prompt size in the tooltip as a calibration
 * reference. A genuinely exact gauge belongs in llm-router, where provider
 * semantics are already known.
 */

interface ContextUsageProps {
  messages: readonly Message[]
  contextWindow?: number
  /** Last provider-reported call, for the tooltip cross-check. */
  lastCall?: SessionUsage['lastCall']
  /** When set the widget becomes a button that opens the metrics dialog. */
  onClick?: () => void
}

const WARN_THRESHOLD = 0.75
const DANGER_THRESHOLD = 0.9

function calibration(lastCall: ContextUsageProps['lastCall']): string {
  if (typeof lastCall?.usage.input !== 'number') return ''
  return `\nlast provider prompt: ${lastCall.usage.input.toLocaleString()} tokens (measured)`
}

export function ContextUsage({
  messages,
  contextWindow,
  lastCall,
  onClick,
}: ContextUsageProps) {
  const tokens = useMemo(() => estimateConversationTokens(messages), [messages])

  const shell = cn(
    'flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint',
    onClick &&
      'hover:text-ink transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
  )
  const openHint = onClick ? '\nclick for session metrics' : ''

  if (!contextWindow || contextWindow <= 0) {
    const body = (
      <>
        <span>ctx</span>
        <span className="tabular-nums text-ink">
          ~{formatTokenCount(tokens)}
        </span>
      </>
    )
    const title = `~${tokens.toLocaleString()} tokens estimated (context window unknown)${calibration(lastCall)}${openHint}`
    return onClick ? (
      <button type="button" onClick={onClick} title={title} className={shell}>
        {body}
      </button>
    ) : (
      <div className={shell} title={title}>
        {body}
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

  const title = `~${tokens.toLocaleString()} / ${contextWindow.toLocaleString()} tokens estimated (${pct}%)${
    hint ? ` — ${hint}` : ''
  }${calibration(lastCall)}${openHint}`

  const body = (
    <>
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
        ~{formatTokenCount(tokens)}/{formatTokenCount(contextWindow)}
      </span>
    </>
  )

  return onClick ? (
    <button type="button" onClick={onClick} title={title} className={shell}>
      {body}
    </button>
  ) : (
    <div className={shell} title={title}>
      {body}
    </div>
  )
}
