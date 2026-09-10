import {
  Bot,
  CheckCircle2,
  CircleStop,
  CircleX,
  ClipboardCheck,
  Code2,
  Database,
  FileText,
  FlaskConical,
  type LucideIcon,
  MoreHorizontal,
  Palette,
  Search,
  Terminal as TerminalIcon,
} from 'lucide-react'
import { useMemo } from 'react'
import type { IIIConnectionState } from '@/lib/iii-client'
import { cn } from '@/lib/utils'
import type { Conversation, SubagentIcon } from '@/types/chat'
import {
  type ActiveSubagentStatus,
  buildActiveSubagentChipModel,
  type TerminalSubagentSummary,
} from './active-subagents'
import './ActiveSubagentChips.css'

export const SUBAGENT_ICON_COMPONENTS: Record<SubagentIcon, LucideIcon> = {
  agent: Bot,
  code: Code2,
  search: Search,
  terminal: TerminalIcon,
  database: Database,
  test: FlaskConical,
  review: ClipboardCheck,
  docs: FileText,
  design: Palette,
}

const STATUS_LABELS: Record<ActiveSubagentStatus, string> = {
  queued: 'queued',
  working: 'working',
  waiting: 'waiting',
  disconnected: 'disconnected',
}

export interface ActiveSubagentChipsProps {
  conversations: readonly Conversation[]
  rootSessionId: string
  connectionState: IIIConnectionState
  /** Opens the selected child session in its own chat panel. */
  onOpen: (sessionId: string) => void
  maxVisible?: number
  className?: string
}

/** Compact, persistent navigation for a root session's active descendants. */
export function ActiveSubagentChips({
  conversations,
  rootSessionId,
  connectionState,
  onOpen,
  maxVisible,
  className,
}: ActiveSubagentChipsProps) {
  const model = useMemo(
    () =>
      buildActiveSubagentChipModel(
        conversations,
        rootSessionId,
        connectionState,
        { maxVisible },
      ),
    [connectionState, conversations, maxVisible, rootSessionId],
  )

  if (
    model.active.length === 0 &&
    (model.terminal.total === 0 ||
      (!model.truncated && model.terminal.total === model.terminal.completed))
  ) {
    return null
  }

  return (
    <section
      aria-label={subagentRegionLabel(
        model.active.length + model.omittedActive,
        model.terminal,
      )}
      className={cn(
        'flex min-w-0 max-w-full flex-nowrap items-center gap-1.5 overflow-x-auto overscroll-x-contain pb-1',
        className,
      )}
      data-active-subagent-chips=""
    >
      {model.active.map(({ sessionId, appearance, status }) => {
        const Icon = SUBAGENT_ICON_COMPONENTS[appearance.icon]
        const statusLabel = STATUS_LABELS[status]
        return (
          <button
            type="button"
            key={sessionId}
            aria-label={`Open ${appearance.name} sub-agent in a new panel (${statusLabel})`}
            className="active-subagent-chip inline-flex h-11 max-w-44 shrink-0 items-center gap-1.5 rounded-md px-2 font-sans text-[11px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent sm:h-7"
            data-color={appearance.color}
            data-session-id={sessionId}
            data-status={status}
            onClick={() => onOpen(sessionId)}
            title={`${appearance.name} · ${statusLabel}`}
          >
            <Icon
              aria-hidden="true"
              focusable="false"
              size={16}
              strokeWidth={1.8}
            />
            <span className="truncate">{appearance.name}</span>
            <span
              aria-hidden="true"
              className={cn(
                'active-subagent-chip__status size-1.5 shrink-0 rounded-full',
                status === 'working' && 'motion-safe:animate-pulse',
              )}
            />
            <span className="sr-only">{statusLabel}</span>
          </button>
        )
      })}

      {model.omittedActive > 0 ? (
        <span
          className="inline-flex h-7 items-center rounded-md bg-surface px-2 font-mono text-[10px] text-ink-faint"
          title={`${model.omittedActive} more active sub-agents`}
        >
          +{model.omittedActive} active
        </span>
      ) : null}

      <TerminalSummary summary={model.terminal} />

      {model.truncated ? (
        <span
          className="inline-flex size-7 items-center justify-center text-ink-ghost"
          title="Sub-agent list limited for performance"
        >
          <MoreHorizontal aria-hidden="true" size={16} />
          <span className="sr-only">Sub-agent list limited</span>
        </span>
      ) : null}
    </section>
  )
}

function TerminalSummary({ summary }: { summary: TerminalSubagentSummary }) {
  if (summary.total === 0 || summary.total === summary.completed) return null
  return (
    <span
      aria-label={terminalSummaryLabel(summary)}
      className="inline-flex h-7 items-center gap-2 rounded-md bg-surface px-2 font-mono text-[10px] text-ink-faint"
      data-subagent-terminal-summary=""
      role="status"
    >
      {summary.completed > 0 ? (
        <span className="inline-flex items-center gap-1 text-ok">
          <CheckCircle2 aria-hidden="true" size={16} strokeWidth={1.8} />
          {summary.completed} done
        </span>
      ) : null}
      {summary.failed > 0 ? (
        <span className="inline-flex items-center gap-1 text-alert">
          <CircleX aria-hidden="true" size={16} strokeWidth={1.8} />
          {summary.failed} failed
        </span>
      ) : null}
      {summary.stopped > 0 ? (
        <span className="inline-flex items-center gap-1 text-warn">
          <CircleStop aria-hidden="true" size={16} strokeWidth={1.8} />
          {summary.stopped} stopped
        </span>
      ) : null}
    </span>
  )
}

function subagentRegionLabel(
  active: number,
  terminal: TerminalSubagentSummary,
): string {
  const parts = [`Sub-agents: ${active} active`]
  if (terminal.total > 0) parts.push(terminalSummaryLabel(terminal))
  return parts.join('. ')
}

function terminalSummaryLabel(summary: TerminalSubagentSummary): string {
  const parts = []
  if (summary.completed > 0) parts.push(`${summary.completed} completed`)
  if (summary.failed > 0) parts.push(`${summary.failed} failed`)
  if (summary.stopped > 0) parts.push(`${summary.stopped} stopped`)
  return parts.join(', ')
}
