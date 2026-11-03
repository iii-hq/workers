import { ShadcnButton } from '@iii-dev/console-ui'
import { CircleDashed, Folder, GitBranch, Milestone } from 'lucide-react'
import { forwardRef, type ReactNode, useMemo } from 'react'
import { statusStep } from '../board-state'
import { type AgentProfile, type Brief, type Column, priorityLabel, type ProjectView, type ReleaseView, statusLabel, type TicketSummary } from '../shared'
import { parentOptions, projectOptions, releaseGroups } from '../subtask-rules'
import { AssigneeAvatar, assigneeName } from './avatar'
import { PriorityIcon, StatusIcon } from './glyphs'
import { OptionMenu } from './option-menu'
import { type PickerOption, SearchPicker } from './search-picker'

/**
 * A property as a compact outline button: icon, value, nothing else. The New ticket dialog's row of properties and
 * the ticket page's sidebar both use it; `quiet` drops the border (a value in a table cell or a sidebar row).
 */
export const PropertyButton = forwardRef<
  HTMLButtonElement,
  { children: ReactNode; quiet?: boolean; empty?: boolean; className?: string; disabled?: boolean }
>(function PropertyButton({ children, quiet, empty, className, ...rest }, ref) {
  return (
    <ShadcnButton
      className={['kb-prop', className].filter(Boolean).join(' ')}
      data-empty={empty ? 'true' : undefined}
      data-quiet={quiet ? 'true' : undefined}
      ref={ref as never}
      size="sm"
      type="button"
      variant={quiet ? 'ghost' : 'outline'}
      {...rest}
    >
      {children}
    </ShadcnButton>
  )
})

type Board = { columns: Column[]; doneStatus: string; canceledStatus: string | null }

/** Status: the step glyph and the column name; `iconOnly` is the card's corner picker. */
export function StatusMenu({
  columns,
  doneStatus,
  canceledStatus,
  status,
  onChange,
  iconOnly,
  quiet,
  disabled,
}: Board & {
  status: string
  onChange: (status: string) => void
  iconOnly?: boolean
  quiet?: boolean
  disabled?: boolean
}) {
  const step = (id: string) => statusStep(columns, id, doneStatus, canceledStatus)
  const known = columns.some((column) => column.id === status)
  const options = [
    ...columns.map((column) => ({ value: column.id, label: column.label, icon: <StatusIcon step={step(column.id)} /> })),
    ...(known ? [] : [{ value: status, label: status, icon: <StatusIcon step="unknown" />, disabled: true }]),
  ]
  const trigger = iconOnly ? (
    <ShadcnButton aria-label={`Status: ${statusLabel(columns, status)}`} className="kb-status-trigger" size="icon-xs" type="button" variant="ghost">
      <StatusIcon step={step(status)} />
    </ShadcnButton>
  ) : (
    <PropertyButton quiet={quiet}>
      <StatusIcon step={step(status)} />
      {statusLabel(columns, status)}
    </PropertyButton>
  )
  return <OptionMenu disabled={disabled} label="Status" onChange={onChange} options={options} trigger={trigger} value={status} />
}

/** Priority: signal bars and the word, most urgent first. */
export function PriorityMenu({
  priority,
  priorities,
  onChange,
  iconOnly,
  quiet,
  disabled,
}: {
  priority: string
  priorities: string[]
  onChange: (priority: string) => void
  iconOnly?: boolean
  quiet?: boolean
  disabled?: boolean
}) {
  const values = priorities.includes(priority) || !priority ? priorities : [...priorities, priority]
  const options = [...values].reverse().map((value) => ({
    value,
    label: priorityLabel(value),
    icon: <PriorityIcon priorities={priorities} priority={value} />,
  }))
  const trigger = iconOnly ? (
    <ShadcnButton aria-label={`Priority: ${priorityLabel(priority) || 'none'}`} className="kb-priority-trigger" size="icon-xs" type="button" variant="ghost">
      <PriorityIcon priorities={priorities} priority={priority} />
    </ShadcnButton>
  ) : (
    <PropertyButton quiet={quiet}>
      <PriorityIcon priorities={priorities} priority={priority} />
      {priorityLabel(priority) || 'No priority'}
    </PropertyButton>
  )
  return <OptionMenu disabled={disabled} label="Priority" onChange={onChange} options={options} trigger={trigger} value={priority} />
}

/** The assignee picker: agent profiles A–Z with their avatars, "Unassigned" first; `''` means nobody. */
export function AssigneePicker({
  agents,
  value,
  onChange,
  iconOnly,
  quiet,
  disabled,
}: {
  agents: AgentProfile[]
  value: string | null
  onChange: (assignee: string | null) => void
  iconOnly?: boolean
  quiet?: boolean
  disabled?: boolean
}) {
  const options = useMemo<PickerOption[]>(() => {
    const known = new Set(agents.map((agent) => agent.id))
    return [
      { value: '', label: 'Unassigned', icon: <AssigneeAvatar agents={agents} id={null} /> },
      ...[...agents]
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((agent) => ({ value: agent.id, label: agent.name, icon: <AssigneeAvatar agents={agents} id={agent.id} /> })),
      // An assignee with no profile (a removed agent, `user`) still shows, so it can be seen and changed.
      ...(value && !known.has(value) ? [{ value, label: assigneeName(value, agents), icon: <AssigneeAvatar agents={agents} id={value} /> }] : []),
    ]
  }, [agents, value])
  const trigger = iconOnly ? (
    <ShadcnButton aria-label={`Assignee: ${assigneeName(value, agents)}`} className="kb-assignee-trigger" size="icon-xs" type="button" variant="ghost">
      <AssigneeAvatar agents={agents} id={value} />
    </ShadcnButton>
  ) : (
    <PropertyButton empty={!value} quiet={quiet}>
      <AssigneeAvatar agents={agents} id={value} />
      {assigneeName(value, agents)}
    </PropertyButton>
  )
  return (
    <SearchPicker
      disabled={disabled}
      label="Assignee"
      onChange={(next) => onChange(next === '' ? null : next)}
      options={options}
      searchPlaceholder="Assign to…"
      trigger={trigger}
      value={value ?? ''}
    />
  )
}

/** A project (archived ones listed but closed to new tickets); `null` is no project. */
export function ProjectPicker({
  projects,
  value,
  onChange,
  quiet,
  disabled,
}: {
  projects: ProjectView[] | null
  value: string | null
  onChange: (project: string | null) => void
  quiet?: boolean
  disabled?: boolean
}) {
  const current = projects?.find((project) => project.id === value) ?? null
  const options = useMemo<PickerOption[]>(
    () => [
      { value: '', label: 'No project', icon: <CircleDashed size={16} /> },
      ...(projects ? projectOptions(projects, value) : []).map((option) => ({
        ...option,
        icon: (
          <span className="kb-project-dot" data-tone={projects?.find((p) => p.id === option.value)?.color ?? undefined}>
            <Folder size={16} />
          </span>
        ),
      })),
    ],
    [projects, value],
  )
  return (
    <SearchPicker
      disabled={disabled || !projects}
      empty="No projects on this board yet."
      label="Project"
      onChange={(next) => onChange(next === '' ? null : next)}
      options={options}
      searchPlaceholder="Find a project…"
      trigger={
        <PropertyButton empty={!value} quiet={quiet}>
          <span className="kb-project-dot" data-tone={current?.color ?? undefined}>
            <Folder size={16} />
          </span>
          {projects ? (current?.name ?? (value ? 'Unknown project' : 'Project')) : 'Loading…'}
        </PropertyButton>
      }
      value={value ?? ''}
    />
  )
}

/** A release, grouped (planned / active / released) the way the release list orders them; `null` is none. */
export function ReleasePicker({
  releases,
  value,
  onChange,
  quiet,
  disabled,
}: {
  releases: ReleaseView[] | null
  value: string | null
  onChange: (release: string | null) => void
  quiet?: boolean
  disabled?: boolean
}) {
  const current = releases?.find((release) => release.id === value) ?? null
  const options = useMemo<PickerOption[]>(
    () => [
      { value: '', label: 'No release', icon: <CircleDashed size={16} /> },
      ...(releases ? releaseGroups(releases, value) : []).flatMap((group) =>
        group.options.map((option) => ({ ...option, group: group.label, icon: <Milestone size={16} /> })),
      ),
    ],
    [releases, value],
  )
  return (
    <SearchPicker
      disabled={disabled || !releases}
      empty="No releases on this board yet."
      label="Release"
      onChange={(next) => onChange(next === '' ? null : next)}
      options={options}
      searchPlaceholder="Find a release…"
      trigger={
        <PropertyButton empty={!value} quiet={quiet}>
          <Milestone size={16} />
          {releases ? (current?.name ?? (value ? 'Unknown release' : 'Release')) : 'Loading…'}
        </PropertyButton>
      }
      value={value ?? ''}
    />
  )
}

/** The parent ticket: any ticket that can take subtasks, by key and title; `null` is a top-level ticket. */
export function ParentPicker({
  tickets,
  value,
  selfId,
  current,
  onChange,
  quiet,
  disabled,
}: {
  tickets: TicketSummary[] | null
  value: string | null
  selfId: string | null
  current: Brief | null
  onChange: (parent: string | null) => void
  quiet?: boolean
  disabled?: boolean
}) {
  const options = useMemo<PickerOption[]>(
    () => [
      { value: '', label: 'No parent', icon: <CircleDashed size={16} /> },
      ...(tickets ? parentOptions(tickets, selfId, current) : []).map((option) => ({ ...option, icon: <GitBranch size={16} /> })),
    ],
    [tickets, selfId, current],
  )
  const chosen = options.find((option) => option.value === value)
  return (
    <SearchPicker
      disabled={disabled || !tickets}
      empty="No ticket can take subtasks yet."
      label="Parent"
      onChange={(next) => onChange(next === '' ? null : next)}
      options={options}
      searchPlaceholder="Find a ticket…"
      trigger={
        <PropertyButton empty={!value} quiet={quiet}>
          <GitBranch size={16} />
          <span className="kb-prop__text">{tickets ? (value ? (chosen?.label ?? current?.key ?? 'Parent') : 'Parent') : 'Loading…'}</span>
        </PropertyButton>
      }
      value={value ?? ''}
    />
  )
}
