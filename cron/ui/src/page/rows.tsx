import {
  Badge,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  IconButton,
  ListItem,
  StatusDot,
  TableCell,
  TableRow,
  uiClasses,
} from '@iii-dev/console-ui'
import type { SessionCronTask, SystemCronBinding } from '../lib/api'
import { describeCron, nextCronRun, untilLabel } from '../lib/cron'
import { statusView } from './status'

export function MoreIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 16 16" className={className} aria-hidden="true" fill="currentColor">
      <circle cx="8" cy="3.5" r="1.25" />
      <circle cx="8" cy="8" r="1.25" />
      <circle cx="8" cy="12.5" r="1.25" />
    </svg>
  )
}

/** A wake has no target function: the fire notifies the session that owns the
    schedule, which for anything created here is a session of its own. */
export function targetLabel(task: SessionCronTask): string {
  return task.target ?? (task.sessionTitle ? `wakes ${task.sessionTitle}` : 'wakes its session')
}

const cadenceCache = new Map<string, string>()

export function cadenceLabel(expression: string): string {
  const cached = cadenceCache.get(expression)
  if (cached !== undefined) return cached
  const label = describeCron(expression) ?? expression
  cadenceCache.set(expression, label)
  return label
}

function nextRunLabel(expression: string, now: Date): string {
  const next = nextCronRun(expression, now)
  return next ? untilLabel(next, now) : 'unknown'
}

function runsLabel(task: SessionCronTask): string {
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
  onOpenConversation,
}: {
  task: SessionCronTask
  now: Date
  selected: boolean
  onOpen: () => void
  onReplace: () => void
  onRemove: () => void
  onCopyId: () => void
  onOpenConversation: () => void
}) {
  const view = statusView(task, now.getTime())
  return (
    <TableRow interactive selected={selected} aria-current={selected ? 'true' : undefined} onClick={onOpen}>
      <TableCell>
        <span className="cron-ui-status">
          <StatusDot tone={view.tone} aria-hidden />
          <span>{view.label}</span>
        </span>
      </TableCell>
      <TableCell className="cron-ui-cell-strong">{task.label ?? 'Untitled schedule'}</TableCell>
      <TableCell>
        <Badge className="cron-ui-target">{targetLabel(task)}</Badge>
      </TableCell>
      <TableCell>{cadenceLabel(task.expression)}</TableCell>
      <TableCell>{nextRunLabel(task.expression, now)}</TableCell>
      <TableCell className="cron-ui-cell-numeric">{runsLabel(task)}</TableCell>
      <TableCell className="cron-ui-cell-actions">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <IconButton
              label={`Actions for ${task.label ?? task.subscriptionId}`}
              onClick={(event: React.MouseEvent) => event.stopPropagation()}
            >
              <MoreIcon className={uiClasses.icon} />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onOpen}>Open details</DropdownMenuItem>
            <DropdownMenuItem onSelect={onOpenConversation}>Open conversation</DropdownMenuItem>
            <DropdownMenuItem onSelect={onReplace}>Replace…</DropdownMenuItem>
            <DropdownMenuItem onSelect={onCopyId}>Copy subscription id</DropdownMenuItem>
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
    <TableRow interactive selected={selected} aria-current={selected ? 'true' : undefined} onClick={onOpen}>
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

/** Narrow panes get a list, not a squeezed table: the name stays readable and
    the rest collapses into one supporting line. */
export function TaskListRow({
  task,
  now,
  selected,
  onOpen,
}: {
  task: SessionCronTask
  now: Date
  selected: boolean
  onOpen: () => void
}) {
  const view = statusView(task, now.getTime())
  return (
    <ListItem
      selected={selected}
      onClick={onOpen}
      leading={<StatusDot tone={view.tone} aria-hidden />}
      label={task.label ?? 'Untitled schedule'}
      description={`${cadenceLabel(task.expression)} · ${nextRunLabel(task.expression, now)}`}
      trailing={<span className="cron-ui-list-meta">{runsLabel(task)}</span>}
    />
  )
}

export function BindingListRow({
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
    <ListItem
      selected={selected}
      onClick={onOpen}
      leading={<StatusDot tone="accent" aria-hidden />}
      label={binding.functionId}
      description={`${binding.workerName} · ${cadenceLabel(binding.expression)} · ${nextRunLabel(binding.expression, now)}`}
    />
  )
}
