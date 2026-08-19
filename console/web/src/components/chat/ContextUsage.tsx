import { useEffect, useMemo, useRef, useState } from 'react'
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
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const tokens = useMemo(() => estimateConversationTokens(messages), [messages])

  useEffect(() => {
    if (!open) return

    const closeOnOutsideClick = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false)
    }

    document.addEventListener('mousedown', closeOnOutsideClick)
    document.addEventListener('keydown', closeOnEscape)
    return () => {
      document.removeEventListener('mousedown', closeOnOutsideClick)
      document.removeEventListener('keydown', closeOnEscape)
    }
  }, [open])

  const hasContextWindow = contextWindow !== undefined && contextWindow > 0
  const ratio = hasContextWindow ? Math.min(1, tokens / contextWindow) : 0
  const pct = Math.round(ratio * 100)
  const available = hasContextWindow
    ? Math.max(0, contextWindow - tokens)
    : null

  let tone: 'normal' | 'warn' | 'danger' = 'normal'
  if (ratio >= DANGER_THRESHOLD) tone = 'danger'
  else if (ratio >= WARN_THRESHOLD) tone = 'warn'

  const fillClass = cn(
    'block h-full transition-[width,background-color] duration-200',
    tone === 'normal' && 'bg-accent',
    tone === 'warn' && 'bg-warn',
    tone === 'danger' && 'bg-danger',
  )

  const labelToneClass = cn(
    tone === 'normal' && 'text-ink',
    tone === 'warn' && 'text-warn',
    tone === 'danger' && 'text-danger',
  )

  return (
    <div ref={rootRef} className="relative flex self-stretch items-center">
      <button
        type="button"
        aria-label={`context: approximately ${tokens.toLocaleString()} tokens${hasContextWindow ? ` of ${contextWindow.toLocaleString()} (${pct}%)` : ''} — click for details`}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        className="flex self-stretch items-center gap-1.5 rounded-sm font-sans text-sm text-ink-faint hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
      >
        <span>ctx</span>
        {hasContextWindow ? (
          <>
            <span
              // `surface-active`, not `surface`: the header group this sits in
              // is itself `bg-surface`, so the track needs added contrast.
              className="relative h-[6px] w-14 overflow-hidden bg-surface-active"
              role="progressbar"
              aria-label="context window usage"
              aria-valuenow={pct}
              aria-valuemin={0}
              aria-valuemax={100}
            >
              <span className={fillClass} style={{ width: `${pct}%` }} />
            </span>
            <span className={cn('tabular-nums', labelToneClass)}>{pct}%</span>
            <span className="text-ink-faint">
              {formatTokenCount(tokens)}/{formatTokenCount(contextWindow)}
            </span>
          </>
        ) : (
          <span className="tabular-nums text-ink">
            {formatTokenCount(tokens)}
          </span>
        )}
      </button>

      {open ? (
        <div
          role="dialog"
          aria-label="context details"
          className="absolute top-full right-0 z-50 mt-1 w-64 rounded-md border border-rule-2 bg-panel-raised p-3 font-sans text-sm normal-case tracking-normal text-ink shadow-floating"
        >
          <div className="flex items-baseline justify-between gap-3">
            <span className="font-medium">Context</span>
            <span className="text-xs text-ink-faint">estimated</span>
          </div>
          <div className="mt-3 space-y-2">
            <ContextDetailRow label="Conversation" value={tokens} />
            {hasContextWindow && available !== null ? (
              <>
                <ContextDetailRow label="Available" value={available} />
                <ContextDetailRow label="Window" value={contextWindow} />
              </>
            ) : null}
          </div>
          <p className="mt-3 border-t border-rule-2 pt-2 text-xs leading-relaxed text-ink-faint">
            {hasContextWindow
              ? 'An estimate based on the messages loaded in this chat.'
              : 'The selected model did not report a context-window limit.'}
          </p>
        </div>
      ) : null}
    </div>
  )
}

function ContextDetailRow({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-ink-faint">{label}</span>
      <span className="tabular-nums">{formatTokenCount(value)}</span>
    </div>
  )
}
