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
import { EllipsisVertical } from 'lucide-react'
import type { SessionCronTask, SystemCronObserver } from '../lib/api'
import { describeCron, nextCronRun, untilLabel } from '../lib/cron'
import { statusView } from './status'

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

/** An observer's engine state, as the status column shows it. */
export function observerStateView(state: string): { label: string; tone: 'accent' | 'warn' | 'alert' | 'ink' } {
  switch (state) {
    case 'active':
      return { label: 'Active', tone: 'accent' }
    case 'pending':
      return { label: 'Pending', tone: 'warn' }
    case 'suspended':
      return { label: 'Suspended', tone: 'warn' }
    case 'error':
      return { label: 'Error', tone: 'alert' }
    default:
      return { label: state.charAt(0).toUpperCase() + state.slice(1), tone: 'ink' }
  }
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
              label={`Actions for ${task.label ?? task.observerId}`}
              onClick={(event: React.MouseEvent) => event.stopPropagation()}
            >
              <EllipsisVertical className={uiClasses.icon} aria-hidden />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onOpen}>Open details</DropdownMenuItem>
            <DropdownMenuItem onSelect={onOpenConversation}>Open conversation</DropdownMenuItem>
            <DropdownMenuItem onSelect={onReplace}>Replace…</DropdownMenuItem>
            <DropdownMenuItem onSelect={onCopyId}>Copy observer id</DropdownMenuItem>
            <DropdownMenuItem onSelect={onRemove}>Remove</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </TableRow>
  )
}

export function ObserverRow({
  observer,
  now,
  selected,
  onOpen,
}: {
  observer: SystemCronObserver
  now: Date
  selected: boolean
  onOpen: () => void
}) {
  const view = observerStateView(observer.state)
  return (
    <TableRow interactive selected={selected} aria-current={selected ? 'true' : undefined} onClick={onOpen}>
      <TableCell>
        <span className="cron-ui-status">
          <StatusDot tone={view.tone} aria-hidden />
          <span>{view.label}</span>
        </span>
      </TableCell>
      <TableCell className="cron-ui-cell-strong">{observer.workerName}</TableCell>
      <TableCell>
        <Badge className="cron-ui-target">{observer.functionId}</Badge>
      </TableCell>
      <TableCell>{cadenceLabel(observer.expression)}</TableCell>
      <TableCell>{nextRunLabel(observer.expression, now)}</TableCell>
      <TableCell className="cron-ui-cell-numeric">{observer.fires}</TableCell>
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

export function ObserverListRow({
  observer,
  now,
  selected,
  onOpen,
}: {
  observer: SystemCronObserver
  now: Date
  selected: boolean
  onOpen: () => void
}) {
  return (
    <ListItem
      selected={selected}
      onClick={onOpen}
      leading={<StatusDot tone={observerStateView(observer.state).tone} aria-hidden />}
      label={observer.functionId}
      description={`${observer.workerName} · ${cadenceLabel(observer.expression)} · ${nextRunLabel(observer.expression, now)}`}
      trailing={<span className="cron-ui-list-meta">{observer.fires}</span>}
    />
  )
}
