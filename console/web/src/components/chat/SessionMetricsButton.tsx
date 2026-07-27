import { Gauge } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { SessionMetricsPanel } from '@/components/metrics/SessionMetricsPanel'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import {
  fetchHarnessMetrics,
  type HarnessMetricsState,
} from '@/lib/backend/harness-metrics'
import type { SessionUsage } from '@/lib/session-usage'
import { loadShowTurnMetrics, saveShowTurnMetrics } from '@/lib/storage'
import { estimateConversationTokens } from '@/lib/token-estimate'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'

/**
 * Header trigger + dialog for session metrics. Owns every piece of data
 * access so `SessionMetricsPanel` can stay presentational.
 *
 * The dialog is a modal rather than an inline panel because the chat dock is
 * resizable down to 320px and a metrics table does not fit in it — the modal
 * portals to the viewport centre and clears the dock at any width.
 */

interface SessionMetricsButtonProps {
  conversation: Conversation
  /** Shared with the ctx widget so the rollup is computed once per render. */
  usage: SessionUsage
  contextWindow?: number
  /** Dock density: icon only, no visible label. */
  compact?: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  onShowTurnMetricsChange?: (show: boolean) => void
  className?: string
}

export function SessionMetricsButton({
  conversation,
  usage,
  contextWindow,
  compact,
  open,
  onOpenChange,
  onShowTurnMetricsChange,
  className,
}: SessionMetricsButtonProps) {
  const [tree, setTree] = useState<HarnessMetricsState | 'loading' | null>(null)
  const [showTurnChips, setShowTurnChips] = useState(loadShowTurnMetrics)
  const disabled = conversation.messages.length === 0

  const contextEstimate = useMemo(
    () => estimateConversationTokens(conversation.messages),
    [conversation.messages],
  )

  const loadTree = useCallback(() => {
    setTree('loading')
    void fetchHarnessMetrics(conversation.id).then(setTree)
  }, [conversation.id])

  // Only on first open: `harness::metrics` walks every descendant session, so
  // it must never run just because a chat is mounted.
  useEffect(() => {
    if (open && tree === null) loadTree()
  }, [open, tree, loadTree])

  const toggleTurnChips = useCallback(
    (next: boolean) => {
      setShowTurnChips(next)
      saveShowTurnMetrics(next)
      onShowTurnMetricsChange?.(next)
    },
    [onShowTurnMetricsChange],
  )

  return (
    <>
      <button
        type="button"
        data-testid="session-metrics-trigger"
        disabled={disabled}
        onClick={() => onOpenChange(true)}
        title={
          disabled
            ? 'no messages yet — nothing to measure'
            : 'what this session consumed — tokens, cost, turns'
        }
        className={cn(
          'flex items-center gap-1 text-ink-faint hover:text-ink transition-colors',
          'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
          disabled && 'opacity-50 cursor-not-allowed hover:text-ink-faint',
          className,
        )}
      >
        <Gauge className="size-3 flex-shrink-0" aria-hidden />
        {compact ? (
          <span className="sr-only">metrics</span>
        ) : (
          <span className="text-[11px] uppercase tracking-[0.06em]">
            metrics
          </span>
        )}
      </button>

      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent data-testid="session-metrics">
          <DialogTitle className="text-[11px] uppercase tracking-[0.18em] text-ink-faint">
            session metrics
          </DialogTitle>
          <DialogDescription className="mt-1 truncate">
            {conversation.id}
          </DialogDescription>
          <SessionMetricsPanel
            usage={usage}
            contextEstimate={contextEstimate}
            contextWindow={contextWindow}
            tree={tree}
            onRetryTree={loadTree}
            onViewTraces={() => {
              onOpenChange(false)
              window.location.hash = '#/traces'
            }}
            showTurnChips={showTurnChips}
            onToggleTurnChips={toggleTurnChips}
          />
        </DialogContent>
      </Dialog>
    </>
  )
}
