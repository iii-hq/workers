import { Square } from 'lucide-react'
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

interface WorkersTableProps {
  rows: WorkerRow[]
  isLoading?: boolean
  stoppingName?: string | null
  onStop?: (name: string) => void
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
    <div className={cn('-mx-4 -my-2 overflow-x-auto whitespace-nowrap sm:-mx-6 lg:-mx-8', className)}>
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
                onStop={onStop}
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
  onStop?: (name: string) => void
}

function WorkerTableRow({ row, stopping, onStop }: WorkerTableRowProps) {
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

  return (
    <tr className="border-b border-rule-2">
      <td className="py-2.5 pr-4">
        <div className="flex items-center gap-2">
          <StatusDot
            tone={statusTone(row.status)}
            pulse={row.status === 'connected'}
          />
          <span className="font-mono text-[13px] text-ink lowercase">
            {row.name}
          </span>
        </div>
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
      </td>
    </tr>
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
