import { ChevronRight, Settings, Square } from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { cn } from '@/lib/utils'
import type { WorkerManagementKind, WorkerRow } from '../types'
import { WorkerSurface } from './WorkerSurface'

interface WorkersTableProps {
  rows: WorkerRow[]
  isLoading?: boolean
  stoppingName?: string | null
  onStop?: (name: string) => void
  onConfigure?: (configurationId: string) => void
  className?: string
}

const MANAGEMENT_LABEL: Record<WorkerManagementKind, string> = {
  config: 'config',
  supervisor: 'managed',
  standalone: 'standalone',
  internal: 'internal',
}

const MANAGEMENT_VARIANT: Record<
  WorkerManagementKind,
  'default' | 'accent' | 'warn'
> = {
  config: 'default',
  supervisor: 'accent',
  standalone: 'warn',
  internal: 'default',
}

function statusTone(
  status: WorkerRow['status'],
): 'accent' | 'alert' | 'warn' | 'ink' {
  if (status === 'connected') return 'accent'
  if (status === 'stopped') return 'ink'
  return 'warn'
}

function formatCell(value: string | number | null | undefined): string {
  if (value === null || value === undefined || value === '') return '—'
  return String(value)
}

export function WorkersTable({
  rows,
  isLoading,
  stoppingName,
  onStop,
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
        description="no workers match the current filters. workers appear here when they connect to the engine or are installed via the supervisor."
      />
    )
  }

  return (
    <WorkersTableBody
      {...{ rows, stoppingName, onStop, onConfigure, className }}
    />
  )
}

function WorkersTableBody({
  rows,
  stoppingName,
  onStop,
  onConfigure,
  className,
}: Omit<WorkersTableProps, 'isLoading'>) {
  // Only connected workers have a surface to show: `engine::workers::info`
  // answers for the live bus, not for a stopped supervisor entry.
  const [expanded, setExpanded] = useState<string | null>(null)

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
                expanded={expanded === row.name}
                onToggle={() =>
                  setExpanded((prev) => (prev === row.name ? null : row.name))
                }
                onStop={onStop}
                onConfigure={onConfigure}
              />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

interface WorkerTableRowProps {
  row: WorkerRow
  stopping: boolean
  expanded: boolean
  onToggle: () => void
  onStop?: (name: string) => void
  onConfigure?: (configurationId: string) => void
}

function WorkerTableRow({
  row,
  stopping,
  expanded,
  onToggle,
  onStop,
  onConfigure,
}: WorkerTableRowProps) {
  const expandable = row.status === 'connected'
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
      <Settings className="h-3.5 w-3.5" aria-hidden />
    </Button>
  ) : null

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

  const nameCell = (
    <div className="flex items-center gap-2">
      <StatusDot
        tone={statusTone(row.status)}
        pulse={row.status === 'connected'}
      />
      <span className="font-mono text-[13px] text-ink lowercase">
        {row.name}
      </span>
    </div>
  )

  return (
    <>
      <tr className="border-b border-rule-2">
        <td className="py-2.5 pr-4">
          {expandable ? (
            <button
              type="button"
              onClick={onToggle}
              aria-expanded={expanded}
              aria-label={`${expanded ? 'hide' : 'show'} ${row.name} functions and triggers`}
              className="flex items-center gap-1.5 text-left hover:text-ink"
            >
              <ChevronRight
                className={cn(
                  'h-3 w-3 shrink-0 text-ink-faint transition-transform',
                  expanded && 'rotate-90',
                )}
                aria-hidden
              />
              {nameCell}
            </button>
          ) : (
            <div className="pl-[18px]">{nameCell}</div>
          )}
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
          <div className="flex items-center justify-end gap-2">
            {configureButton}
            {row.stopEnabled || !row.stopDisabledReason ? (
              stopButton
            ) : (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="inline-block">{stopButton}</span>
                </TooltipTrigger>
                <TooltipContent>{row.stopDisabledReason}</TooltipContent>
              </Tooltip>
            )}
          </div>
        </td>
      </tr>
      {expanded ? (
        <tr className="border-b border-rule-2 bg-paper-2/40">
          {/* The table wrapper is `whitespace-nowrap` so the columns never
              wrap mid-row; the surface below is prose, so it opts back out. */}
          <td colSpan={8} className="whitespace-normal px-6 pb-2">
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
