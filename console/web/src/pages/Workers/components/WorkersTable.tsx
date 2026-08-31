import { ChevronRight, Play, RotateCw, Settings, Square } from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { EmptyState } from '@/components/ui/EmptyState'
import { IconButton } from '@/components/ui/IconButton'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { cn } from '@/lib/utils'
import type { PendingComposeAction } from '../hooks/useWorkersLive'
import {
  type ComposeAction,
  composeActions,
  MANAGEMENT_LABEL,
  type WorkerManagementKind,
  type WorkerRow,
} from '../types'
import { WorkerSurface } from './WorkerSurface'

interface WorkersTableProps {
  rows: WorkerRow[]
  isLoading?: boolean
  stoppingName?: string | null
  pendingCompose?: PendingComposeAction | null
  onStop?: (name: string) => void
  onComposeAction?: (action: ComposeAction, container: string) => void
  onConfigure?: (configurationId: string) => void
  className?: string
}

const MANAGEMENT_VARIANT: Record<
  WorkerManagementKind,
  'default' | 'accent' | 'warn'
> = {
  compose: 'accent',
  config: 'default',
  supervisor: 'accent',
  standalone: 'warn',
  internal: 'default',
}

const STATUS_STYLE: Record<
  WorkerRow['status'],
  {
    tone: 'accent' | 'alert' | 'warn' | 'ink'
    badge: 'default' | 'warn' | 'alert'
  }
> = {
  connected: { tone: 'accent', badge: 'warn' },
  starting: { tone: 'warn', badge: 'warn' },
  failed: { tone: 'alert', badge: 'alert' },
  disconnected: { tone: 'warn', badge: 'warn' },
  stopped: { tone: 'ink', badge: 'default' },
}

function formatCell(value: string | number | null | undefined): string {
  if (value === null || value === undefined || value === '') return '—'
  return String(value)
}

interface ComposeConfirm {
  action: Exclude<ComposeAction, 'start'>
  container: string
}

export function WorkersTable({
  rows,
  isLoading,
  stoppingName,
  pendingCompose,
  onStop,
  onComposeAction,
  onConfigure,
  className,
}: WorkersTableProps) {
  if (isLoading) {
    return <WorkersTableSkeleton className={className} />
  }

  if (rows.length === 0) {
    return (
      <EmptyState
        icon={Square}
        title="no workers"
        description="no workers match the current filters. workers appear here when they connect to the engine, when compose declares them, or when the supervisor installs them."
      />
    )
  }

  return (
    <WorkersTableBody
      {...{
        rows,
        stoppingName,
        pendingCompose,
        onStop,
        onComposeAction,
        onConfigure,
        className,
      }}
    />
  )
}

function WorkersTableBody({
  rows,
  stoppingName,
  pendingCompose,
  onStop,
  onComposeAction,
  onConfigure,
  className,
}: Omit<WorkersTableProps, 'isLoading'>) {
  // Only connected workers have a surface to show: `engine::workers::info`
  // answers for the live bus, not for a stopped supervisor entry.
  const [expanded, setExpanded] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<ComposeConfirm | null>(null)

  return (
    <div
      className={cn(
        '-mx-4 -my-2 overflow-x-auto whitespace-nowrap sm:-mx-6 lg:-mx-8',
        className,
      )}
    >
      <div className="inline-block min-w-full px-4 py-2 align-middle sm:px-6 lg:px-8">
        <table className="w-full">
          <thead>
            <tr className="border-b border-rule-2">
              <th className="py-2 pr-4 text-left font-mono text-[11px] font-medium text-ink-faint whitespace-nowrap">
                Name
              </th>
              <th className="py-2 pr-4 text-left font-mono text-[11px] font-medium text-ink-faint whitespace-nowrap">
                Runtime
              </th>
              <th className="py-2 pr-4 text-left font-mono text-[11px] font-medium text-ink-faint whitespace-nowrap">
                IP address
              </th>
              <th className="py-2 pr-4 text-left font-mono text-[11px] font-medium text-ink-faint whitespace-nowrap">
                Version
              </th>
              <th className="py-2 pr-4 text-left font-mono text-[11px] font-medium text-ink-faint whitespace-nowrap">
                PID
              </th>
              <th className="py-2 pr-4 text-left font-mono text-[11px] font-medium text-ink-faint whitespace-nowrap">
                Management
              </th>
              <th className="py-2 pr-4 text-left font-mono text-[11px] font-medium text-ink-faint whitespace-nowrap">
                Tag
              </th>
              <th className="py-2 text-right font-mono text-[11px] font-medium text-ink-faint whitespace-nowrap">
                Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <WorkerTableRow
                key={row.id}
                row={row}
                stopping={stoppingName === row.name}
                pendingAction={
                  pendingCompose?.container === row.name
                    ? pendingCompose.action
                    : null
                }
                expanded={expanded === row.name}
                onToggle={() =>
                  setExpanded((prev) => (prev === row.name ? null : row.name))
                }
                onStop={onStop}
                onComposeAction={
                  onComposeAction
                    ? (action) => {
                        if (action === 'start')
                          onComposeAction(action, row.name)
                        else setConfirm({ action, container: row.name })
                      }
                    : undefined
                }
                onConfigure={onConfigure}
              />
            ))}
          </tbody>
        </table>
      </div>
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(open) => {
          if (!open) setConfirm(null)
        }}
        title={
          confirm?.action === 'restart'
            ? `Restart ${confirm.container}?`
            : `Stop ${confirm?.container ?? ''}?`
        }
        description={
          confirm?.action === 'restart'
            ? 'Compose restarts this container in place without touching its dependency graph.'
            : 'Compose stops this container and every container that depends on it, in reverse dependency order.'
        }
        confirmLabel={confirm?.action === 'restart' ? 'Restart' : 'Stop'}
        onConfirm={() => {
          if (confirm && onComposeAction) {
            onComposeAction(confirm.action, confirm.container)
          }
          setConfirm(null)
        }}
      />
    </div>
  )
}

interface WorkerTableRowProps {
  row: WorkerRow
  stopping: boolean
  pendingAction: ComposeAction | null
  expanded: boolean
  onToggle: () => void
  onStop?: (name: string) => void
  onComposeAction?: (action: ComposeAction) => void
  onConfigure?: (configurationId: string) => void
}

const COMPOSE_ICON: Record<ComposeAction, typeof Play> = {
  start: Play,
  stop: Square,
  restart: RotateCw,
}

function WorkerTableRow({
  row,
  stopping,
  pendingAction,
  expanded,
  onToggle,
  onStop,
  onComposeAction,
  onConfigure,
}: WorkerTableRowProps) {
  const expandable = row.status === 'connected'
  const composeVerbs = composeActions(row)
  const configureButton = row.configurationId ? (
    <Button
      variant="icon"
      size="icon"
      type="button"
      aria-label={`configure ${row.name}`}
      title={`configure ${row.name}`}
      disabled={!onConfigure}
      onClick={() => {
        if (row.configurationId) onConfigure?.(row.configurationId)
      }}
    >
      <Settings className="size-4" aria-hidden />
    </Button>
  ) : null

  const composeButtons =
    composeVerbs.length > 0
      ? composeVerbs.map((action) => {
          const Icon = COMPOSE_ICON[action]
          const busy = pendingAction !== null
          return (
            <IconButton
              key={action}
              variant="icon"
              label={`${action} ${row.name}`}
              disabled={busy || !onComposeAction}
              onClick={() => onComposeAction?.(action)}
            >
              <Icon
                className={cn(
                  'size-4',
                  pendingAction === action &&
                    action === 'restart' &&
                    'animate-spin',
                )}
                aria-hidden
              />
            </IconButton>
          )
        })
      : null

  const stopButton = (
    <Button
      variant="ghost"
      size="sm"
      disabled={!row.stopEnabled || stopping || !onStop}
      onClick={() => onStop?.(row.name)}
      className="text-alert border-alert/40 hover:bg-alert/10 hover:text-alert disabled:opacity-40"
    >
      {stopping ? 'stopping…' : 'stop'}
    </Button>
  )

  const fallbackStop =
    composeVerbs.length > 0 ? null : row.stopEnabled ||
      !row.stopDisabledReason ? (
      stopButton
    ) : (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-block">{stopButton}</span>
        </TooltipTrigger>
        <TooltipContent>{row.stopDisabledReason}</TooltipContent>
      </Tooltip>
    )

  // A span, not a div: this nests inside the expand <button>, which only
  // allows phrasing content.
  const nameCell = (
    <span className="flex items-center gap-2">
      <StatusDot
        tone={STATUS_STYLE[row.status].tone}
        pulse={row.status === 'connected' || row.status === 'starting'}
      />
      <span className="font-mono text-[13px] text-ink lowercase">
        {row.name}
      </span>
      {pendingAction ? (
        <Badge
          variant={STATUS_STYLE[row.status].badge}
        >{`${pendingAction}…`}</Badge>
      ) : row.status !== 'connected' ? (
        <Badge variant={STATUS_STYLE[row.status].badge}>{row.status}</Badge>
      ) : null}
    </span>
  )
  const detailId = `worker-detail-${row.name}`

  return (
    <>
      <tr className="border-b border-rule-2">
        <td className="py-2.5 pr-4">
          {expandable ? (
            <button
              type="button"
              onClick={onToggle}
              aria-expanded={expanded}
              aria-controls={detailId}
              aria-label={`${expanded ? 'hide' : 'show'} ${row.name} functions and triggers`}
              className="flex items-center gap-1.5 text-left hover:text-ink"
            >
              <ChevronRight
                className={cn(
                  'size-4 shrink-0 text-ink-faint transition-transform',
                  expanded && 'rotate-90',
                )}
                aria-hidden
              />
              {nameCell}
            </button>
          ) : (
            <div className="pl-[18px]">{nameCell}</div>
          )}
          {row.lastError ? (
            <p
              className="mt-1 max-w-[32rem] whitespace-normal pl-[18px] font-mono text-[11.5px] leading-relaxed text-alert"
              title={row.lastError}
            >
              {row.lastError}
            </p>
          ) : null}
        </td>
        <td className="py-2.5 pr-4 font-mono text-[13px] text-ink-faint lowercase">
          {formatCell(row.runtime)}
        </td>
        <td className="py-2.5 pr-4 font-mono text-[13px] text-ink-faint tabular-nums">
          {formatCell(row.ipAddress)}
        </td>
        <td className="py-2.5 pr-4 font-mono text-[13px] text-ink-faint tabular-nums">
          {formatCell(row.version)}
        </td>
        <td className="py-2.5 pr-4 font-mono text-[13px] text-ink-faint tabular-nums">
          {formatCell(row.pid)}
        </td>
        <td className="py-2.5 pr-4">
          <Badge variant={MANAGEMENT_VARIANT[row.managementKind]}>
            {MANAGEMENT_LABEL[row.managementKind]}
          </Badge>
        </td>
        <td className="py-2.5 pr-4 font-mono text-[13px] text-ink-faint lowercase">
          {formatCell(row.tag)}
        </td>
        <td className="py-2.5 text-right">
          <div className="flex items-center justify-end gap-1">
            {configureButton}
            {composeButtons}
            {fallbackStop}
          </div>
        </td>
      </tr>
      {expanded ? (
        <tr className="border-b border-rule-2 bg-paper-2/40">
          {/* The table wrapper is `whitespace-nowrap` so the columns never
              wrap mid-row; the surface below is prose, so it opts back out. */}
          <td id={detailId} colSpan={8} className="whitespace-normal px-6 pb-2">
            <WorkerSurface name={row.name} />
          </td>
        </tr>
      ) : null}
    </>
  )
}

function WorkersTableSkeleton({ className }: { className?: string }) {
  return (
    <div className={cn('space-y-2 px-4 sm:px-6 lg:px-8', className)}>
      {[0, 1, 2, 3].map((i) => (
        <Skeleton key={i} className="h-10 w-full" />
      ))}
    </div>
  )
}
