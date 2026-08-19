import {
  Badge,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  IconButton,
  StatusDot,
  TableCell,
  TableRow,
} from '@iii-dev/console-ui'
import type { SessionCronTask, SystemCronBinding } from '../lib/api'
import { describeCron, nextCronRun, untilLabel } from '../lib/cron'
import { statusView } from './status'

export function MoreIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 16 16" className={className} aria-hidden fill="currentColor">
      <circle cx="8" cy="3.5" r="1.25" />
      <circle cx="8" cy="8" r="1.25" />
      <circle cx="8" cy="12.5" r="1.25" />
    </svg>
  )
}

/** A wake has no target function: the fire notifies the owning conversation. */
export function targetLabel(task: SessionCronTask): string {
  return task.target ?? 'wakes this chat'
}

export function cadenceLabel(expression: string): string {
  return describeCron(expression) ?? expression
}

function nextRunLabel(expression: string, now: Date): string {
  const next = nextCronRun(expression, now)
  return next ? untilLabel(next, now) : 'unknown'
}

function firesLabel(task: SessionCronTask): string {
  const cap = task.once ? 1 : task.maxFires
  return cap === undefined ? `${task.fires}` : `${task.fires} / ${cap}`
}

export function TaskRow({
  task,
  now,
  selected,
  onOpen,
  onReplace,
  onRemove,
  onCopyId,
}: {
  task: SessionCronTask
  now: Date
  selected: boolean
  onOpen: () => void
  onReplace: () => void
  onRemove: () => void
  onCopyId: () => void
}) {
  const view = statusView(task, now.getTime())
  return (
    <TableRow
      data-selected={selected ? 'true' : undefined}
      aria-current={selected ? 'true' : undefined}
      onClick={onOpen}
      className="cron-ui-row"
    >
      <TableCell>
        <span className="cron-ui-status">
          <StatusDot tone={view.tone} aria-hidden />
          <span>{view.label}</span>
        </span>
      </TableCell>
      <TableCell className="cron-ui-cell-strong">
        {task.label ?? 'Untitled schedule'}
      </TableCell>
      <TableCell>
        <Badge className="cron-ui-target">{targetLabel(task)}</Badge>
      </TableCell>
      <TableCell>{cadenceLabel(task.expression)}</TableCell>
      <TableCell>{nextRunLabel(task.expression, now)}</TableCell>
      <TableCell className="cron-ui-cell-numeric">{firesLabel(task)}</TableCell>
      <TableCell className="cron-ui-cell-actions">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <IconButton
              label={`Actions for ${task.label ?? task.subscriptionId}`}
              onClick={(event: React.MouseEvent) => event.stopPropagation()}
            >
              <MoreIcon className="cron-ui-icon" />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onOpen}>Open details</DropdownMenuItem>
            <DropdownMenuItem onSelect={onReplace}>Replace…</DropdownMenuItem>
            <DropdownMenuItem onSelect={onCopyId}>
              Copy subscription id
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onRemove}>Unregister</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </TableRow>
  )
}

export function BindingRow({
  binding,
  now,
  selected,
  onOpen,
}: {
  binding: SystemCronBinding
  now: Date
  selected: boolean
  onOpen: () => void
}) {
  return (
    <TableRow
      data-selected={selected ? 'true' : undefined}
      aria-current={selected ? 'true' : undefined}
      onClick={onOpen}
      className="cron-ui-row"
    >
      <TableCell>
        <span className="cron-ui-status">
          <StatusDot tone="accent" aria-hidden />
          <span>Active</span>
        </span>
      </TableCell>
      <TableCell className="cron-ui-cell-strong">{binding.workerName}</TableCell>
      <TableCell>
        <Badge className="cron-ui-target">{binding.functionId}</Badge>
      </TableCell>
      <TableCell>{cadenceLabel(binding.expression)}</TableCell>
      <TableCell>{nextRunLabel(binding.expression, now)}</TableCell>
      <TableCell className="cron-ui-cell-numeric">—</TableCell>
      <TableCell className="cron-ui-cell-actions" />
    </TableRow>
  )
}
