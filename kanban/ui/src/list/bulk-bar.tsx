import { ShadcnButton } from '@iii-dev/console-ui'
import { ChevronDown, CircleDashed, Folder, GitBranch, Milestone, Tag, Trash2, X } from 'lucide-react'
import { forwardRef, type ReactNode, useEffect, useRef } from 'react'
import { type AgentProfile, type Column, priorityLabel, type ProjectView, type ReleaseView, type TicketSummary } from '../shared'
import { AssigneeAvatar, assigneeName } from '../ui/avatar'
import { PriorityIcon, StatusIcon } from '../ui/glyphs'
import { IconAction } from '../ui/icon-action'
import { OptionMenu } from '../ui/option-menu'
import { type PickerOption, SearchPicker } from '../ui/search-picker'
import { statusStep } from '../board-state'
import { fold, LabelsPicker } from './labels-picker'

export type Field = 'status' | 'priority' | 'assignee'

const RELEASE_HINT = 'Released releases are not listed.'
const PROJECT_HINT = 'Archived projects are not listed.'
const PARENT_HINT = 'Subtasks go one level deep: tickets that have subtasks are skipped.'

export type BulkBarProps = {
  picked: TicketSummary[]
  /** Every ticket shown: where assignees and priorities without a profile or config come from. */
  tickets: TicketSummary[]
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  priorities: string[]
  agents: AgentProfile[]
  labels: string[]
  projects: ProjectView[]
  releases: ReleaseView[]
  /** Every live ticket on the board (the view over the pool): where the parent candidates come from. */
  known: Map<string, TicketSummary>
  /** Ids that have subtasks. */
  parents: Set<string>
  busy: boolean
  onField: (field: Field, value: string) => void
  onFacet: (facet: 'release' | 'project', id: string) => void
  onParent: (parent: TicketSummary | null) => void
  /** The parent picker opened: refresh the pool. */
  onOpenParents: () => void
  onLabel: (label: string) => void
  onDelete: () => void
  onClear: () => void
}

/**
 * A bar action's trigger: a quiet button with the field's glyph and name. It forwards its ref and the props the
 * menu hands a trigger (click, keys, aria), so it can be rendered through Base UI's `render`.
 */
const Action = forwardRef<
  HTMLButtonElement,
  { icon: ReactNode; children: ReactNode; disabled?: boolean; 'aria-label'?: string }
>(
  // The picker names its trigger after the menu heading ("Move 2 tickets to"); the visible word is the name here.
  function Action({ icon, children, 'aria-label': _heading, ...rest }, ref) {
    return (
      <ShadcnButton className="kb-bulk__action" ref={ref} size="sm" type="button" variant="ghost" {...rest}>
        {icon}
        {children}
        <ChevronDown className="kb-facet__chevron" size={16} />
      </ShadcnButton>
    )
  },
)

/**
 * The floating bar over the list while tickets are selected: how many, then status, priority, assignee, labels,
 * release, project, parent and delete for all of them at once, and a way out.
 */
export function BulkBar({
  picked,
  tickets,
  columns,
  doneStatus,
  canceledStatus,
  priorities,
  agents,
  labels,
  projects,
  releases,
  known,
  parents,
  busy,
  onField,
  onFacet,
  onParent,
  onOpenParents,
  onLabel,
  onDelete,
  onClear,
}: BulkBarProps) {
  const lastFocus = useRef<HTMLElement | null>(null)
  const n = picked.length

  // A disabled trigger cannot take the focus back from its closing menu: give it back once the batch is done.
  useEffect(() => {
    const active = document.activeElement
    if (!busy && lastFocus.current?.isConnected && (!active || active === document.body)) lastFocus.current.focus()
  }, [busy])

  /** What every selected ticket shares (`''` = none), else undefined: nothing is checked. */
  const shared = (pick: (ticket: TicketSummary) => string | null) => {
    const first = pick(picked[0]) ?? ''
    return picked.every((ticket) => (pick(ticket) ?? '') === first) ? first : undefined
  }

  // Ids and priorities that only exist on tickets (API failed, removed agent, `user`) still need a row.
  const strayAssignees = [
    ...new Set(
      tickets.flatMap((ticket) =>
        ticket.assignee && !agents.some((agent) => agent.id === ticket.assignee) ? [ticket.assignee] : [],
      ),
    ),
  ].sort()
  const strayPriorities = [...new Set(tickets.map((ticket) => ticket.priority))]
    .filter((priority) => priority && !priorities.includes(priority))
    .sort()

  const statusOpts = columns.map((column) => ({
    value: column.id,
    label: column.label,
    icon: <StatusIcon step={statusStep(columns, column.id, doneStatus, canceledStatus)} />,
  }))
  const priorityOpts = [...[...priorities].reverse(), ...strayPriorities].map((priority) => ({
    value: priority,
    label: priorityLabel(priority),
    icon: <PriorityIcon priorities={priorities} priority={priority} />,
  }))
  const assigneeOpts: PickerOption[] = [
    { value: '', label: 'Unassigned', icon: <AssigneeAvatar agents={agents} id={null} /> },
    ...[...agents]
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((agent) => ({ value: agent.id, label: agent.name, icon: <AssigneeAvatar agents={agents} id={agent.id} /> })),
    ...strayAssignees.map((id) => ({ value: id, label: assigneeName(id, agents), icon: <AssigneeAvatar agents={agents} id={id} /> })),
  ]
  // Released releases and archived projects are not offered: the board does not move tickets into them from here.
  const releaseOpts: PickerOption[] = [
    { value: '', label: 'No release', icon: <CircleDashed size={16} /> },
    ...releases
      .filter((candidate) => candidate.status !== 'released')
      .map((candidate) => ({ value: candidate.id, label: candidate.name, icon: <Milestone size={16} /> })),
  ]
  const projectOpts: PickerOption[] = [
    { value: '', label: 'No project', icon: <CircleDashed size={16} /> },
    ...projects
      .filter((candidate) => candidate.status === 'active')
      .map((candidate) => ({
        value: candidate.id,
        label: candidate.name,
        icon: (
          <span className="kb-project-dot" data-tone={candidate.color ?? undefined}>
            <Folder size={16} />
          </span>
        ),
      })),
  ]
  // Parent candidates: live tickets without a parent of their own, not the selection. Tickets that already have subtasks first.
  const pickedIds = new Set(picked.map((ticket) => ticket.id))
  const keyNumber = (key: string) => Number(key.split('-').pop())
  const parentOpts: PickerOption[] = [
    { value: '', label: 'No parent', icon: <CircleDashed size={16} /> },
    ...[...known.values()]
      .filter((t) => !t.deleted_at && !t.parent_id && !pickedIds.has(t.id))
      .sort((a, b) => Number(parents.has(b.id)) - Number(parents.has(a.id)) || keyNumber(a.key) - keyNumber(b.key))
      .map((candidate) => ({
        value: candidate.id,
        label: candidate.key,
        description: candidate.title,
        icon: <GitBranch size={16} />,
      })),
  ]

  const coverage = (label: string) => picked.filter((ticket) => ticket.labels.some((l) => fold(l) === fold(label))).length

  return (
    <div
      aria-label={`Change ${n} selected ${n === 1 ? 'ticket' : 'tickets'}`}
      className="kb-bulk"
      // Bubbles through portals: only the bar's own buttons count, not the menu items.
      onFocus={(event) => {
        if (event.currentTarget.contains(event.target)) lastFocus.current = event.target
      }}
      role="toolbar"
    >
      <span className="kb-bulk__count">
        <span className="kb-bulk__number">{n}</span> selected
      </span>
      <span aria-hidden="true" className="kb-bulk__rule" />
      <OptionMenu
        disabled={busy}
        label={`Move ${n === 1 ? '1 ticket' : `${n} tickets`} to`}
        onChange={(value) => onField('status', value)}
        options={statusOpts}
        trigger={<Action icon={<StatusIcon step="ready" />}>Status</Action>}
        value={shared((ticket) => ticket.status) ?? ''}
      />
      <OptionMenu
        disabled={busy}
        label="Set priority"
        onChange={(value) => onField('priority', value)}
        options={priorityOpts}
        trigger={<Action icon={<PriorityIcon priorities={priorities} priority={priorities[priorities.length - 2] ?? ''} />}>Priority</Action>}
        value={shared((ticket) => ticket.priority) ?? ''}
      />
      <SearchPicker
        disabled={busy}
        label="Assign to"
        onChange={(value) => onField('assignee', value)}
        options={assigneeOpts}
        searchPlaceholder="Assign to…"
        trigger={<Action icon={<AssigneeAvatar agents={agents} id={null} />}>Assignee</Action>}
        value={shared((ticket) => ticket.assignee) ?? null}
      />
      <LabelsPicker
        busy={busy}
        coverage={coverage}
        labels={labels}
        onToggle={onLabel}
        total={n}
        trigger={<Action icon={<Tag size={16} />}>Labels</Action>}
      />
      <SearchPicker
        disabled={busy}
        label="Move to a release"
        note={RELEASE_HINT}
        onChange={(id) => onFacet('release', id)}
        options={releaseOpts}
        trigger={<Action icon={<Milestone size={16} />}>Release</Action>}
        value={shared((ticket) => ticket.release_id) ?? null}
      />
      <SearchPicker
        disabled={busy}
        label="Move to a project"
        note={PROJECT_HINT}
        onChange={(id) => onFacet('project', id)}
        options={projectOpts}
        trigger={<Action icon={<Folder size={16} />}>Project</Action>}
        value={shared((ticket) => ticket.project_id) ?? null}
      />
      <SearchPicker
        disabled={busy}
        label={`Set the parent of ${n === 1 ? '1 ticket' : `${n} tickets`}`}
        note={PARENT_HINT}
        onChange={(id) => onParent(id ? (known.get(id) ?? null) : null)}
        onOpenChange={(open) => {
          if (open) onOpenParents()
        }}
        options={parentOpts}
        searchPlaceholder="Find by key or title…"
        trigger={<Action icon={<GitBranch size={16} />}>Parent</Action>}
        value={shared((ticket) => ticket.parent_id) ?? null}
      />
      <span aria-hidden="true" className="kb-bulk__rule" />
      <ShadcnButton className="kb-bulk__delete" disabled={busy} onClick={onDelete} size="sm" type="button" variant="ghost">
        <Trash2 size={16} />
        Delete…
      </ShadcnButton>
      <IconAction disabled={busy} label="Clear selection" onClick={onClear} side="top">
        <X size={16} />
      </IconAction>
    </div>
  )
}
