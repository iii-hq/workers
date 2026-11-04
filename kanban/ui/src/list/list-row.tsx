import { ShadcnCheckbox } from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { CornerDownRight, FolderOpen, ListTree, Milestone } from 'lucide-react'
import { memo } from 'react'
import { type AgentProfile, type Column, Pills, type TicketSummary } from '../shared'
import { LabelList } from '../ui/label-badge'
import { swallow } from '../ui/option-menu'
import { AssigneePicker, PriorityMenu, StatusMenu } from '../ui/properties'
import type { Tone } from '../ui/tone'

/** Where a ticket sits in the plan, resolved by the list: the row only draws it. */
export type RowPlan = {
  parentKey: string | null
  /** True when the ticket is a subtask whose parent the board does not hold. */
  orphan: boolean
  subs: { done: number; total: number } | undefined
  release: string | null
  project: { name: string; color: Tone | null } | null
}

export type ListRowProps = {
  ticket: TicketSummary
  plan: RowPlan
  selected: boolean
  changed: boolean
  now: number
  busy: boolean
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  priorities: string[]
  agents: AgentProfile[]
  highlight: string[]
  onToggle: (id: string) => void
  onOpen: (key: string) => void
  onEdit: (ticket: TicketSummary, patch: Record<string, unknown>) => void
}

/**
 * One 40px row, Linear-style: a checkbox that shows on hover or once anything is selected, the priority bars, the
 * key, the status glyph, the title with where it sits in the plan, labels, project and release, when it changed,
 * and the assignee. Priority, status and assignee are pickers that edit this ticket alone; a click anywhere
 * else opens the ticket.
 */
export const ListRow = memo(function ListRow({
  ticket,
  plan,
  selected,
  changed,
  now,
  busy,
  columns,
  doneStatus,
  canceledStatus,
  priorities,
  agents,
  highlight,
  onToggle,
  onOpen,
  onEdit,
}: ListRowProps) {
  return (
    <li
      className="kb-lrow"
      data-changed={changed ? 'true' : undefined}
      data-selected={selected ? 'true' : undefined}
      onClick={() => onOpen(ticket.key)}
    >
      {/* biome-ignore lint/a11y/noStaticElementInteractions: a click guard around the checkbox, not a control */}
      <span className="kb-lrow__check" onClick={swallow} onKeyDown={swallow}>
        <ShadcnCheckbox aria-label={`Select ${ticket.key}`} checked={selected} onCheckedChange={() => onToggle(ticket.id)} />
      </span>
      <span className="kb-lrow__priority">
        <PriorityMenu
          disabled={busy}
          iconOnly
          onChange={(priority) => onEdit(ticket, { priority })}
          priorities={priorities}
          priority={ticket.priority}
        />
      </span>
      <span className="kb-lrow__key">{ticket.key}</span>
      <span className="kb-lrow__status">
        <StatusMenu
          canceledStatus={canceledStatus}
          columns={columns}
          disabled={busy}
          doneStatus={doneStatus}
          iconOnly
          onChange={(status) => onEdit(ticket, { status })}
          status={ticket.status}
        />
      </span>
      <span className="kb-lrow__main">
        <button
          className="kb-lrow__title"
          onClick={(event) => {
            event.stopPropagation()
            onOpen(ticket.key)
          }}
          title={ticket.title}
          type="button"
        >
          {ticket.title}
        </button>
        {plan.parentKey || plan.orphan ? (
          <span className="kb-lrow__fact" title={plan.parentKey ? `Subtask of ${plan.parentKey}` : 'Subtask'}>
            <CornerDownRight size={16} />
            {plan.parentKey ?? 'Subtask'}
          </span>
        ) : null}
        {plan.subs ? (
          <span
            aria-label={`${plan.subs.done} of ${plan.subs.total} subtasks done`}
            className="kb-lrow__fact"
            role="img"
          >
            <ListTree size={16} />
            {plan.subs.done}/{plan.subs.total}
          </span>
        ) : null}
      </span>
      <span className="kb-lrow__labels">
        <LabelList highlight={highlight} labels={ticket.labels} max={2} />
        <Pills pills={ticket.pills} />
      </span>
      <span className="kb-lrow__plan">
        {plan.project ? (
          <span className="kb-lrow__chip" title={`Project: ${plan.project.name}`}>
            <span className="kb-project-dot" data-tone={plan.project.color ?? undefined}>
              <FolderOpen size={16} />
            </span>
            <span className="kb-lrow__chip-text">{plan.project.name}</span>
          </span>
        ) : null}
        {plan.release ? (
          <span className="kb-lrow__chip" title={`Release: ${plan.release}`}>
            <Milestone size={16} />
            <span className="kb-lrow__chip-text">{plan.release}</span>
          </span>
        ) : null}
      </span>
      <span className="kb-lrow__time">
        {changed ? (
          <span aria-label="Changed since your last visit" className="kb-card__changed-dot" role="img" title="Changed since your last visit" />
        ) : null}
        <time dateTime={ticket.updated_at} title={new Date(ticket.updated_at).toLocaleString()}>
          {formatRelative(ticket.updated_at, now)}
        </time>
      </span>
      <span className="kb-lrow__assignee">
        <AssigneePicker
          agents={agents}
          disabled={busy}
          iconOnly
          onChange={(assignee) => onEdit(ticket, { assignee })}
          value={ticket.assignee}
        />
      </span>
    </li>
  )
})
