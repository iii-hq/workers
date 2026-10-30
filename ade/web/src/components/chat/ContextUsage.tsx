import { ChevronDown, Gauge } from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import {
  Fact,
  FactCardHeader,
  FactList,
  FactMeter,
} from '@/components/ui/FactCard'
import type { ContextUsageReport } from '@/lib/backend/types'
import {
  estimateConversationTokens,
  formatTokenCount,
} from '@/lib/token-estimate'
import { cn } from '@/lib/utils'
import type { Message } from '@/types/chat'
import { RingGauge } from './RingGauge'

interface ContextUsageProps {
  messages: readonly Message[]
  contextWindow?: number
  /**
   * The harness's own accounting for the last request: what it cost, the
   * input budget it was fit into, and what was left. The chars/4 estimate
   * over the loaded rows, against the model's raw window, only stands in
   * before the first generate: it grows with every live row and shrinks to
   * the elided tail page on reload.
   */
  reported?: ContextUsageReport
}

const WARN_THRESHOLD = 0.75
const DANGER_THRESHOLD = 0.9

type Tone = 'normal' | 'warn' | 'danger'

/** The arc takes the tone; the hero number's `-strong` holds AA as text. */
const RING_TONE: Record<Tone, string> = {
  normal: 'text-accent',
  warn: 'text-warn',
  danger: 'text-danger',
}
const TEXT_TONE: Record<Tone, string> = {
  normal: 'text-ink',
  warn: 'text-warn-strong',
  danger: 'text-alert-strong',
}
/** The chip tint and the meter fill: ink until the window gets tight. */
const ACCENT_TONE: Record<Tone, string> = {
  normal: 'var(--color-ink)',
  warn: 'var(--color-warn)',
  danger: 'var(--color-alert)',
}

/**
 * The built-in context meter: a ring gauge, `used / window` counts and a
 * caret in the chat header, opening a popover led by the share used and
 * the three numbers behind it. The harness worker's `context` chip
 * supersedes it with real per-step numbers; the two draw the same control.
 */
export function ContextUsage({
  messages,
  contextWindow,
  reported,
}: ContextUsageProps) {
  const [open, setOpen] = useState(false)
  const estimate = useMemo(
    () => estimateConversationTokens(messages),
    [messages],
  )
  const tokens = reported?.total ?? estimate
  const estimated = reported === undefined
  // The reported budget already excludes the output allocation, so it, not
  // the model's raw window, is the capacity the last request was fit into.
  const capacity = reported?.usable ?? contextWindow

  const hasCapacity = capacity !== undefined && capacity > 0
  const ratio = hasCapacity ? Math.min(1, tokens / capacity) : 0
  const pct = Math.round(ratio * 100)
  const available = reported
    ? reported.free
    : hasCapacity
      ? Math.max(0, capacity - tokens)
      : null

  let tone: Tone = 'normal'
  if (ratio >= DANGER_THRESHOLD) tone = 'danger'
  else if (ratio >= WARN_THRESHOLD) tone = 'warn'

  const label = `context: ${estimated ? 'approximately ' : ''}${tokens.toLocaleString()} tokens${hasCapacity ? ` of ${capacity.toLocaleString()} (${pct}%)` : ''} — click for details`

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        aria-label={label}
        className={cn(
          'flex self-stretch items-center gap-1.5 rounded-sm px-0.5 font-inter text-xs text-ink-faint transition-colors duration-[var(--motion-duration-control)] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus aria-expanded:text-ink',
        )}
      >
        <span
          role="progressbar"
          aria-label="context window usage"
          aria-valuenow={pct}
          aria-valuemin={0}
          aria-valuemax={100}
          className={cn('flex', RING_TONE[tone])}
        >
          <RingGauge ratio={ratio} size={16} stroke={2.5} />
        </span>
        {hasCapacity ? (
          <span
            className={cn(
              'tabular-nums whitespace-nowrap',
              tone !== 'normal' && TEXT_TONE[tone],
            )}
          >
            {formatTokenCount(tokens)}
            <span className="mx-[3px] text-ink-ghost">/</span>
            {formatTokenCount(capacity)}
          </span>
        ) : (
          <span className="tabular-nums text-ink">
            {formatTokenCount(tokens)}
          </span>
        )}
        {/* Under a 30rem pane (`@container` on the chat root) the caret
            goes; the ring and the counts are what you act on. */}
        <ChevronDown
          aria-hidden
          className={cn(
            'size-4 shrink-0 text-ink-ghost transition-transform duration-[var(--motion-duration-control)] @max-[30rem]:hidden',
            open && 'rotate-180',
          )}
        />
      </PopoverTrigger>
      <PopoverContent
        align="end"
        sideOffset={6}
        aria-label="context details"
        className="w-72 gap-3 p-4"
      >
        <FactCardHeader
          icon={Gauge}
          accent={ACCENT_TONE[tone]}
          title="Context usage"
          description={
            estimated ? 'Estimated from the loaded messages' : 'Last request'
          }
        />

        {hasCapacity ? (
          <div className="flex items-center gap-3">
            <FactMeter
              value={pct}
              accent={ACCENT_TONE[tone]}
              label="share of the context window"
              className="flex-1"
            />
            <span
              className={cn(
                'shrink-0 font-mono text-[13px] tabular-nums',
                TEXT_TONE[tone],
              )}
            >
              {estimated ? '~' : ''}
              {formatTokenCount(tokens)} / {formatTokenCount(capacity)}
              <span className="text-ink-faint"> · {pct}%</span>
            </span>
          </div>
        ) : null}

        <FactList>
          <Fact label="Conversation" mono align="end" className="font-medium">
            {formatTokenCount(tokens)}
          </Fact>
          {hasCapacity && available !== null ? (
            <>
              <Fact label="Available" mono align="end">
                {formatTokenCount(available)}
              </Fact>
              <Fact
                label={estimated ? 'Window' : 'Input budget'}
                mono
                align="end"
              >
                {formatTokenCount(capacity)}
              </Fact>
            </>
          ) : null}
        </FactList>

        {!hasCapacity ? (
          <p className="text-xs leading-4 text-ink-faint">
            The selected model did not report a context-window limit.
          </p>
        ) : null}
      </PopoverContent>
    </Popover>
  )
}
