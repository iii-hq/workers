import {
  Check,
  Ellipsis,
  X,
} from 'lucide-react'
import { type StatusStep, statusStep } from '../board-state'
import type { Column } from '../shared'

/**
 * Linear's status set, drawn in CSS so every step has its own shape as well as its own hue: backlog a dashed
 * ring, to do an empty ring, in progress a half-filled amber pie, in review a three-quarter green pie, done a
 * solid purple disc with a check, canceled a solid grey disc with a cross.
 */
export function StatusIcon({ step }: { step: StatusStep }) {
  return (
    <span aria-hidden="true" className="kb-status-icon" data-step={step}>
      <span className="kb-status-icon__ring">
        {step === 'progress' || step === 'review' ? <span className="kb-status-icon__fill" /> : null}
        {step === 'done' ? <Check size={16} strokeWidth={3} /> : null}
        {step === 'canceled' ? <X size={16} strokeWidth={3} /> : null}
      </span>
    </span>
  )
}

/** The status glyph of a column id on this board. */
export function ColumnStatusIcon({
  columns,
  status,
  doneStatus,
  canceledStatus,
}: {
  columns: Column[]
  status: string
  doneStatus: string
  canceledStatus: string | null
}) {
  return <StatusIcon step={statusStep(columns, status, doneStatus, canceledStatus)} />
}

/** Where a priority sits from the top of the configured list: 0 is the most urgent, -1 unknown. */
export function priorityRank(priority: string, priorities: readonly string[]): number {
  const index = priorities.indexOf(priority)
  return index < 0 ? -1 : priorities.length - 1 - index
}

/**
 * Priority as Linear's three bars: low lights one, medium two, high three, in yellow, orange and red; the most
 * urgent configured priority is a red square with "!"; a value the board does not know is an ellipsis.
 */
export function PriorityIcon({ priority, priorities }: { priority: string; priorities: readonly string[] }) {
  const rank = priorityRank(priority, priorities)
  if (rank === 0) {
    return (
      <span aria-hidden="true" className="kb-priority-icon" data-rank="top">
        <span className="kb-priority-icon__urgent">!</span>
      </span>
    )
  }
  if (rank < 0) {
    return (
      <span aria-hidden="true" className="kb-priority-icon" data-rank="none">
        <Ellipsis size={16} />
      </span>
    )
  }
  const lit = rank === 1 ? 3 : rank === 2 ? 2 : 1
  return (
    <span aria-hidden="true" className="kb-priority-icon" data-rank={String(Math.min(rank, 3))}>
      <span className="kb-priority-bars">
        {[1, 2, 3].map((bar) => (
          <span className="kb-priority-bars__bar" data-lit={bar <= lit ? 'true' : undefined} key={bar} />
        ))}
      </span>
    </span>
  )
}
