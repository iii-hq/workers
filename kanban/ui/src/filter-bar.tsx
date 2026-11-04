import {
  ShadcnBadge,
  ShadcnButton,
  ShadcnDialog,
  ShadcnDialogContent,
  ShadcnDialogDescription,
  ShadcnDialogFooter,
  ShadcnDialogHeader,
  ShadcnDialogTitle,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuCheckboxItem,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuLabel,
  ShadcnDropdownMenuRadioGroup,
  ShadcnDropdownMenuRadioItem,
  ShadcnDropdownMenuSeparator,
  ShadcnDropdownMenuTrigger,
  ShadcnInput,
  ShadcnTabs,
  ShadcnTabsList,
  ShadcnTabsTrigger,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import {
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ListFilter,
  SlidersHorizontal,
  Clock,
  Columns3,
  FolderOpen,
  Layers,
  ListTree,
  List as ListIcon,
  type LucideIcon,
  Milestone,
  Search,
  SignalHigh,
  Tag,
  User,
  X,
} from 'lucide-react'
import { type KeyboardEvent, type ReactElement, type ReactNode, useId, useRef, useState } from 'react'
import { coerceGroup, GROUPS, type GroupBy } from './board-state'
import {
  ALL_VIEW,
  allLabels,
  EMPTY_FILTERS,
  type Filters,
  facetCounts,
  isEmpty,
  type MatchContext,
  type SavedView,
  UPDATED_WINDOWS,
  type UpdatedWindow,
} from './filters'
import { type AgentProfile, type ProjectView, priorityLabel, type ReleaseView, type TicketSummary } from './shared'
import { AssigneeAvatar } from './ui/avatar'
import { Field } from './ui/feedback'
import { PriorityIcon } from './ui/glyphs'
import { SearchPicker } from './ui/search-picker'
import { humanize, toneOf } from './ui/tone'

type FacetId = 'projects' | 'releases' | 'labels' | 'assignees' | 'priorities' | 'updated'

type Opt = {
  value: string
  label: string
  /** Tickets this choice would keep; a choice that is not a filter has none. */
  count?: number
  lead?: ReactNode
  /** Quiet word after the name ("Archived"). */
  description?: string
  /** The heading of the group this row belongs to. */
  group?: string
  disabled?: boolean
}

/** One filter menu: what its trigger says, what it lists, and how a choice lands in the filters. */
type Facet = {
  id: FacetId
  name: string
  heading: string
  Icon: LucideIcon
  /** Shown after the name on the trigger; empty while the filter is off. */
  value: string
  opts: Opt[]
  selected: string[]
  /** One choice (radio) instead of a set (checkboxes). */
  single?: boolean
  /** Fixed content above the list (the label match mode). */
  head?: ReactNode
  note?: string
  empty?: string
  pick: (value: string) => void
  clear: () => void
}

const flip = (list: string[], value: string) =>
  list.includes(value) ? list.filter((v) => v !== value) : [...list, value]

/** `phase-2 +1`: the first choice, then how many more. */
const summary = (values: string[], name: (value: string) => string) =>
  values.length === 0 ? '' : name(values[0]) + (values.length > 1 ? ` +${values.length - 1}` : '')

/** Chosen options first, the rest in their own order. */
const selectedFirst = (opts: Opt[], selected: string[]) => [
  ...opts.filter((opt) => selected.includes(opt.value)),
  ...opts.filter((opt) => !selected.includes(opt.value)),
]

const plural = (n: number) => `${n} ${n === 1 ? 'ticket' : 'tickets'}`

export type FilterBarProps = {
  /** Every live ticket on the board, unfiltered. */
  tickets: TicketSummary[]
  /** Tickets left after all filters. */
  shownCount: number
  filters: Filters
  onChange: (next: Filters) => void
  ctx: MatchContext
  agents: AgentProfile[]
  /** Configured order, e.g. `['low', 'medium', 'high', 'urgent']`. */
  priorities: string[]
  /** The pane is narrow (< ~760px). */
  narrow: boolean
  /** The current filters differ from the active view. */
  canSaveView: boolean
  onSaveView: (name: string) => void
  /** The board's projects (archived ones included) and releases, for names. */
  projects: ProjectView[]
  releases: ReleaseView[]
  /** Group-by control; absent where the layout has no swimlanes. */
  group?: GroupControl
  /** Drawn last in the tools group (the Board / List switch). */
  end?: ReactNode
}

export type GroupControl = {
  value: GroupBy
  onChange: (next: GroupBy) => void
  showEmpty: boolean
  onShowEmptyChange: (next: boolean) => void
}

export function FilterBar({
  tickets,
  shownCount,
  filters,
  onChange,
  ctx,
  agents,
  priorities,
  narrow,
  canSaveView,
  onSaveView,
  projects,
  releases,
  group,
  end,
}: FilterBarProps) {
  const set = (patch: Partial<Filters>) => onChange({ ...filters, ...patch })
  const counts = (key: FacetId) => facetCounts(tickets, filters, ctx, key)

  const projectCounts = counts('projects')
  const projectNames = new Map(projects.map((project) => [project.id, project.name]))
  const projectLead = (color: ProjectView['color']) => (
    <span aria-hidden="true" className="kb-project-dot" data-tone={color ?? undefined}>
      <FolderOpen size={16} />
    </span>
  )
  const projectOpts = selectedFirst(
    [
      { value: '', label: 'No project', count: projectCounts[''] ?? 0, lead: projectLead(null) },
      ...[...projects]
        .sort((a, b) => Number(Boolean(a.archived_at)) - Number(Boolean(b.archived_at)))
        .map((project) => ({
          value: project.id,
          label: project.name,
          count: projectCounts[project.id] ?? 0,
          lead: projectLead(project.color),
          description: project.archived_at ? 'Archived' : undefined,
        })),
      // A saved view can name a project this board no longer has: still a row, so it can be unticked.
      ...filters.projects
        .filter((id) => id !== '' && !projectNames.has(id))
        .map((id) => ({ value: id, label: 'Unknown project', count: projectCounts[id] ?? 0, lead: projectLead(null) })),
    ],
    filters.projects,
  )

  const releaseCounts = counts('releases')
  const releaseNames = new Map(releases.map((release) => [release.id, release.name]))
  const releasedIds = new Set(releases.filter((release) => release.released_at).map((release) => release.id))
  const releaseOpts = selectedFirst(
    [
      { value: '', label: 'No release', count: releaseCounts[''] ?? 0 },
      ...releases
        .filter((release) => !releasedIds.has(release.id))
        .map((release) => ({ value: release.id, label: release.name, count: releaseCounts[release.id] ?? 0 })),
      ...releases
        .filter((release) => releasedIds.has(release.id))
        .map((release) => ({ value: release.id, label: release.name, count: releaseCounts[release.id] ?? 0 })),
      ...filters.releases
        .filter((id) => id !== '' && !releaseNames.has(id))
        .map((id) => ({ value: id, label: 'Unknown release', count: releaseCounts[id] ?? 0 })),
    ],
    filters.releases,
  )
  const firstReleased = releaseOpts.findIndex(
    (opt) => releasedIds.has(opt.value) && !filters.releases.includes(opt.value),
  )
  if (firstReleased >= 0) {
    for (let i = firstReleased; i < releaseOpts.length; i += 1) releaseOpts[i] = { ...releaseOpts[i], group: 'Released' }
  }

  const labelCounts = counts('labels')
  const labelOpts: Opt[] = [...new Set([...allLabels(tickets), ...filters.labels])]
    .map((label) => ({
      value: label,
      label,
      count: labelCounts[label] ?? 0,
      lead: <span aria-hidden="true" className="kb-label__dot" data-tone={toneOf(label)} />,
    }))
    .sort(
      (a, b) =>
        Number(filters.labels.includes(b.value)) - Number(filters.labels.includes(a.value)) ||
        b.count - a.count ||
        a.label.localeCompare(b.label),
    )

  const assigneeCounts = counts('assignees')
  const agentNames = new Map(agents.map((agent) => [agent.id, agent.name]))
  // Ids with no profile (the API failed, a removed agent, `user`) still need a row to filter by.
  const strayAssignees = [
    ...new Set(tickets.flatMap((t) => (t.assignee && !agentNames.has(t.assignee) ? [t.assignee] : []))),
  ].sort()
  const assigneeOpts: Opt[] = [
    { value: '', label: 'Unassigned', count: assigneeCounts[''] ?? 0, lead: <AssigneeAvatar agents={agents} id={null} /> },
    ...[...agents]
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((agent) => ({
        value: agent.id,
        label: agent.name,
        count: assigneeCounts[agent.id] ?? 0,
        lead: <AssigneeAvatar agents={agents} id={agent.id} />,
      })),
    ...strayAssignees.map((id) => ({
      value: id,
      label: humanize(id),
      count: assigneeCounts[id] ?? 0,
      lead: <AssigneeAvatar agents={agents} id={id} />,
    })),
  ].sort(
    // Stable: ties keep Unassigned first, then names A–Z.
    (a, b) =>
      Number(filters.assignees.includes(b.value)) - Number(filters.assignees.includes(a.value)) || b.count - a.count,
  )

  const priorityCounts = counts('priorities')
  const priorityOpt = (priority: string): Opt => ({
    value: priority,
    label: priorityLabel(priority),
    count: priorityCounts[priority] ?? 0,
    lead: <PriorityIcon priorities={priorities} priority={priority} />,
  })
  const strayPriorities = [...new Set(tickets.map((t) => t.priority))]
    .filter((p) => p && !priorities.includes(p))
    .sort()
  const priorityOpts: Opt[] = [
    ...[...priorities].reverse().map((priority) => priorityOpt(priority)),
    ...strayPriorities.map((priority) => priorityOpt(priority)),
  ]

  const updatedCounts = counts('updated')
  const updatedOpts: Opt[] = UPDATED_WINDOWS.map((window) => ({
    value: window.id,
    label: window.label,
    count: updatedCounts[window.id] ?? 0,
    disabled: window.id === 'visit' && !ctx.lastVisit,
  }))
  const ago = ctx.lastVisit ? formatRelative(ctx.lastVisit, ctx.now) : ''
  const changed = updatedCounts.visit ?? 0

  const facets: Facet[] = [
    {
      id: 'projects',
      name: 'Project',
      heading: 'Filter by project',
      Icon: FolderOpen,
      value: summary(filters.projects, (id) =>
        id === '' ? 'No project' : (projectNames.get(id) ?? 'Unknown project'),
      ),
      opts: projectOpts,
      selected: filters.projects,
      empty: 'No projects on this board yet.',
      pick: (id) => set({ projects: flip(filters.projects, id) }),
      clear: () => set({ projects: [] }),
    },
    {
      id: 'releases',
      name: 'Release',
      heading: 'Filter by release',
      Icon: Milestone,
      value: summary(filters.releases, (id) =>
        id === '' ? 'No release' : (releaseNames.get(id) ?? 'Unknown release'),
      ),
      opts: releaseOpts,
      selected: filters.releases,
      empty: 'No releases on this board yet.',
      pick: (id) => set({ releases: flip(filters.releases, id) }),
      clear: () => set({ releases: [] }),
    },
    {
      id: 'labels',
      name: 'Label',
      heading: 'Filter by label',
      Icon: Tag,
      value: summary(filters.labels, (label) => label),
      opts: labelOpts,
      selected: filters.labels,
      head: (
        <div className="kb-facet__mode">
          <span>Ticket has</span>
          <ShadcnTabs onValueChange={(labelMode) => set({ labelMode: labelMode === 'all' ? 'all' : 'any' })} value={filters.labelMode}>
            <ShadcnTabsList aria-label="Label match">
              <ShadcnTabsTrigger value="any">any of these</ShadcnTabsTrigger>
              <ShadcnTabsTrigger value="all">all of these</ShadcnTabsTrigger>
            </ShadcnTabsList>
          </ShadcnTabs>
        </div>
      ),
      empty: 'No labels on this board yet.',
      pick: (label) => set({ labels: flip(filters.labels, label) }),
      clear: () => set({ labels: [] }),
    },
    {
      id: 'assignees',
      name: 'Assignee',
      heading: 'Filter by assignee',
      Icon: User,
      value: summary(filters.assignees, (id) => (id === '' ? 'Unassigned' : (agentNames.get(id) ?? humanize(id)))),
      opts: assigneeOpts,
      selected: filters.assignees,
      pick: (id) => set({ assignees: flip(filters.assignees, id) }),
      clear: () => set({ assignees: [] }),
    },
    {
      id: 'priorities',
      name: 'Priority',
      heading: 'Filter by priority',
      Icon: SignalHigh,
      value: summary(filters.priorities, priorityLabel),
      opts: priorityOpts,
      selected: filters.priorities,
      pick: (priority) => set({ priorities: flip(filters.priorities, priority) }),
      clear: () => set({ priorities: [] }),
    },
    {
      id: 'updated',
      name: 'Updated',
      heading: 'Filter by last update',
      Icon: Clock,
      value: UPDATED_WINDOWS.find((window) => window.id === filters.updated)?.short ?? '',
      opts: updatedOpts,
      selected: [filters.updated],
      single: true,
      note: ctx.lastVisit
        ? `Your last visit: ${ago === 'just now' ? ago : `${ago} ago`}`
        : 'First visit in this browser',
      pick: (window) => set({ updated: window as UpdatedWindow }),
      clear: () => set({ updated: 'any' }),
    },
  ]

  const active = facets.filter((facet) => facet.value)

  return (
    <div aria-label="Filter tickets" className="kb-controls" data-narrow={narrow ? 'true' : undefined} role="toolbar">
      {active.length > 0 ? (
        <div className="kb-controls__chips">
          {active.map((facet) => (
            <ActiveChip f={facet} key={facet.id} />
          ))}
          <ShadcnButton className="kb-tool kb-tool--quiet" onClick={() => onChange(EMPTY_FILTERS)} size="sm" type="button" variant="ghost">
            Clear
          </ShadcnButton>
        </div>
      ) : null}
      <div className="kb-controls__tools">
        {ctx.lastVisit && filters.updated !== 'visit' && changed > 0 ? (
          <ShadcnButton className="kb-controls__changed" onClick={() => set({ updated: 'visit' })} size="sm" type="button" variant="ghost">
            <span aria-hidden="true" className="kb-card__changed-dot" />
            {changed} new
          </ShadcnButton>
        ) : null}
        <span className="kb-controls__total" role="status">
          {isEmpty(filters) ? plural(tickets.length) : `${shownCount} of ${tickets.length}`}
        </span>
        <SearchBox onChange={(q) => set({ q })} value={filters.q} />
        <FilterMenu active={active.length} facets={facets} />
        {group ? <DisplayMenu {...group} /> : null}
        {end}
      </div>
    </div>
  )
}

/**
 * Linear's Filter menu: one button; the popover lists the facets, a facet opens its options (with a way back),
 * and typing searches every option of every facet at once.
 */
function FilterMenu({ facets, active }: { facets: Facet[]; active: number }) {
  const [open, setOpen] = useState(false)
  const [facetId, setFacetId] = useState<FacetId | null>(null)
  const [query, setQuery] = useState('')
  const search = useRef<HTMLInputElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const facet = facets.find((candidate) => candidate.id === facetId) ?? null
  const q = query.trim().toLowerCase()
  const change = (next: boolean) => {
    setOpen(next)
    if (!next) {
      setFacetId(null)
      setQuery('')
    }
  }
  const enter = (id: FacetId | null) => {
    setFacetId(id)
    setQuery('')
    requestAnimationFrame(() => search.current?.focus())
  }
  const onKey = (event: KeyboardEvent<HTMLElement>) => {
    event.stopPropagation()
    if (event.key === 'Backspace' && facet && query === '' && event.target === search.current) {
      event.preventDefault()
      enter(null)
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const rows = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[data-picker-row]:not(:disabled)') ?? [])
    const at = rows.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'ArrowDown' ? Math.min(rows.length - 1, at + 1) : at - 1
    if (next < 0) search.current?.focus()
    else rows[next]?.focus()
  }
  const optionRow = (f: Facet, opt: Opt, withFacet = false) => {
    const on = f.selected.includes(opt.value)
    return (
      <button
        aria-selected={on}
        className="kb-picker__row"
        data-checkbox={f.single ? undefined : 'true'}
        data-picker-row=""
        data-zero={opt.count === 0 ? 'true' : undefined}
        disabled={opt.disabled}
        key={`${f.id}:${opt.value}`}
        onClick={() => f.pick(opt.value)}
        role="option"
        type="button"
      >
        <span aria-hidden="true" className="kb-picker__check" data-on={on ? 'true' : undefined}>
          {on ? <Check size={16} /> : null}
        </span>
        {opt.lead}
        <span className="kb-picker__label">{opt.label}</span>
        {withFacet ? <span className="kb-picker__description">{f.name}</span> : null}
        {opt.count === undefined ? null : <span className="kb-picker__count">{opt.count}</span>}
      </button>
    )
  }
  const matches = (opt: Opt) => opt.label.toLowerCase().includes(q)
  return (
    <Popover onOpenChange={change} open={open}>
      <PopoverTrigger
        render={
          <ShadcnButton aria-label="Filter" className="kb-tool" data-active={active > 0 ? 'true' : undefined} size="sm" type="button" variant="ghost" />
        }
      >
        <ListFilter size={16} />
        <span className="kb-tool__text">Filter</span>
        {active > 0 ? <span className="kb-tool__count">{active}</span> : null}
      </PopoverTrigger>
      <PopoverContent align="end" className="kb-picker kb-filter-menu" initialFocus={search} onKeyDown={onKey}>
        <div className="kb-picker__search">
          {facet ? (
            <button aria-label="All filters" className="kb-filter-menu__back" onClick={() => enter(null)} type="button">
              <ChevronLeft size={16} />
            </button>
          ) : (
            <Search size={16} />
          )}
          <input
            aria-label={facet ? `Filter by ${facet.name.toLowerCase()}` : 'Filter…'}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={facet ? `${facet.name}…` : 'Filter…'}
            ref={search}
            value={query}
          />
        </div>
        {facet?.head ? <div className="kb-picker__head">{facet.head}</div> : null}
        <div className="kb-picker__list" ref={list} role="listbox">
          {facet ? (
            facet.opts.filter(matches).length === 0 ? (
              <p className="kb-picker__empty">{q ? `Nothing matches “${query.trim()}”.` : (facet.empty ?? 'Nothing to filter by yet.')}</p>
            ) : (
              facet.opts.filter(matches).map((opt) => optionRow(facet, opt))
            )
          ) : q ? (
            facets.flatMap((f) => f.opts.filter(matches).map((opt) => optionRow(f, opt, true))).slice(0, 60)
          ) : (
            facets.map((f) => (
              <button className="kb-picker__row" data-picker-row="" key={f.id} onClick={() => enter(f.id)} type="button">
                <f.Icon className="kb-filter-menu__icon" size={16} />
                <span className="kb-picker__label">{f.name}</span>
                {f.value ? <span className="kb-picker__description">{f.value}</span> : null}
                <ChevronRight className="kb-filter-menu__chevron" size={16} />
              </button>
            ))
          )}
        </div>
        {facet?.note ? <p className="kb-picker__note">{facet.note}</p> : null}
        {facet && !facet.single && facet.selected.length > 0 ? (
          <div className="kb-picker__foot">
            <span>{facet.selected.length} selected</span>
            <button className="kb-picker__clear" onClick={facet.clear} type="button">
              Clear
            </button>
          </div>
        ) : null}
      </PopoverContent>
    </Popover>
  )
}

/** An applied filter as a chip: `Label` `is any of` `design +1` ×. The value reopens that facet's picker. */
function ActiveChip({ f }: { f: Facet }) {
  const op = f.single ? 'is' : f.selected.length > 1 ? (f.id === 'labels' ? 'has' : 'is any of') : 'is'
  return (
    <span className="kb-chip">
      <span className="kb-chip__part">
        <f.Icon size={16} />
        {f.name}
      </span>
      <span className="kb-chip__op">{op}</span>
      <FacetPicker f={f} trigger={<ShadcnButton className="kb-chip__value" size="xs" type="button" variant="ghost">{f.value}</ShadcnButton>} />
      <button aria-label={`Remove ${f.name.toLowerCase()} filter`} className="kb-chip__remove" onClick={f.clear} type="button">
        <X size={16} />
      </button>
    </span>
  )
}

/** The search field: a magnifier, the query, and a clear button once something is typed. */
function SearchBox({ value, onChange }: { value: string; onChange: (q: string) => void }) {
  return (
    <div className="kb-search">
      <Search aria-hidden="true" size={16} />
      <ShadcnInput
     
        aria-label="Search tickets by key or title"
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && value) {
            event.stopPropagation()
            onChange('')
          }
        }}
        placeholder="Search key or title"
        value={value}
      />
      {value ? (
        <button aria-label="Clear search" className="kb-search__clear" onClick={() => onChange('')} type="button">
          <X size={16} />
        </button>
      ) : null}
    </div>
  )
}

/** A facet's trigger and picker: name, the chosen value as a badge, and a searchable checklist with counts. */
function FacetPicker({ f, trigger: custom }: { f: Facet; trigger?: ReactElement }) {
  const trigger = custom ?? (
    <ShadcnButton className="kb-facet" data-active={f.value ? 'true' : undefined} size="sm" type="button" variant="outline">
      <f.Icon size={16} />
      {f.name}
      {f.value ? <ShadcnBadge className="kb-facet__value" variant="secondary">{f.value}</ShadcnBadge> : null}
      <ChevronDown className="kb-facet__chevron" size={16} />
    </ShadcnButton>
  )
  const options = f.opts.map((opt) => ({
    value: opt.value,
    label: opt.label,
    icon: opt.lead,
    count: opt.count,
    description: opt.description,
    disabled: opt.disabled,
    group: opt.group,
  }))
  if (f.single) {
    return (
      <SearchPicker
        empty={f.empty}
        label={f.heading}
        note={f.note}
        onChange={f.pick}
        options={options}
        trigger={trigger}
        value={f.selected[0] ?? null}
      />
    )
  }
  return (
    <SearchPicker
      empty={f.empty}
      head={f.head}
      label={f.heading}
      multiple
      onChange={f.pick}
      onClear={f.clear}
      options={options}
      searchPlaceholder={`Filter by ${f.name.toLowerCase()}…`}
      trigger={trigger}
      value={f.selected}
    />
  )
}

/** The glyph each grouping wears elsewhere: release milestone, project folder, parent tree. */
function GroupIcon({ id }: { id: GroupBy }) {
  const Icon = id === 'release' ? Milestone : id === 'project' ? FolderOpen : id === 'parent' ? ListTree : Columns3
  return <Icon className="kb-menu__icon" size={16} />
}

/** Display: how the board is drawn (group by, empty swimlanes) — a view option, not a filter. */
function DisplayMenu({ value, onChange, showEmpty, onShowEmptyChange }: GroupControl) {
  const grouped = value !== 'none'
  return (
    <ShadcnDropdownMenu>
      <ShadcnDropdownMenuTrigger
        render={<ShadcnButton aria-label="Display" className="kb-tool" data-active={grouped ? 'true' : undefined} size="sm" type="button" variant="ghost" />}
      >
        <SlidersHorizontal size={16} />
        <span className="kb-tool__text">Display</span>
      </ShadcnDropdownMenuTrigger>
      <ShadcnDropdownMenuContent align="end" className="kb-menu">
        <ShadcnDropdownMenuGroup>
          <ShadcnDropdownMenuLabel>Group by</ShadcnDropdownMenuLabel>
          <ShadcnDropdownMenuRadioGroup onValueChange={(next) => onChange(coerceGroup(next))} value={value}>
            {GROUPS.map((group) => (
              <ShadcnDropdownMenuRadioItem key={group.id} value={group.id}>
                <GroupIcon id={group.id} />
                {group.id === 'none' ? 'No grouping' : group.label}
              </ShadcnDropdownMenuRadioItem>
            ))}
          </ShadcnDropdownMenuRadioGroup>
        </ShadcnDropdownMenuGroup>
        <ShadcnDropdownMenuSeparator />
        <ShadcnDropdownMenuCheckboxItem checked={showEmpty} disabled={!grouped} onCheckedChange={onShowEmptyChange}>
          Show empty swimlanes
        </ShadcnDropdownMenuCheckboxItem>
      </ShadcnDropdownMenuContent>
    </ShadcnDropdownMenu>
  )
}

/** The Save view dialog on its own, for the header's ⋯ menu. */
export function SaveViewDialog({
  open,
  onOpenChange,
  onSave,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSave: (name: string) => void
}) {
  return (
    <ShadcnDialog onOpenChange={onOpenChange} open={open}>
      <ShadcnDialogContent className="kb-dialog">
        {open ? (
          <SaveViewForm
            onCancel={() => onOpenChange(false)}
            onSave={(name) => {
              onOpenChange(false)
              onSave(name)
            }}
          />
        ) : null}
      </ShadcnDialogContent>
    </ShadcnDialog>
  )
}

/** Mounted with the dialog content, so every opening starts with an empty name. Same anatomy as the New ticket dialog. */
function SaveViewForm({ onCancel, onSave }: { onCancel: () => void; onSave: (name: string) => void }) {
  const [name, setName] = useState('')
  const id = useId()
  return (
    <form
      className="kb-dialog__form"
      onSubmit={(event) => {
        event.preventDefault()
        if (name.trim()) onSave(name.trim())
      }}
    >
      <ShadcnDialogHeader>
        <ShadcnDialogTitle>Save view</ShadcnDialogTitle>
        <ShadcnDialogDescription>Saved in this browser only.</ShadcnDialogDescription>
      </ShadcnDialogHeader>
      <Field htmlFor={id} label="View name">
        <ShadcnInput autoFocus id={id} onChange={(event) => setName(event.target.value)} placeholder="Bugs to triage" value={name} />
      </Field>
      <ShadcnDialogFooter>
        <ShadcnButton onClick={onCancel} type="button" variant="outline">
          Cancel
        </ShadcnButton>
        <ShadcnButton disabled={!name.trim()} type="submit">
          Save view
        </ShadcnButton>
      </ShadcnDialogFooter>
    </form>
  )
}

export type ViewTabsProps = {
  /** `ALL_VIEW` first, then the user's views. */
  views: SavedView[]
  activeId: string
  /** The active view's filters were changed. */
  edited: boolean
  /** View id -> tickets it matches. */
  counts: Record<string, number>
  onSelect: (view: SavedView) => void
}

/** Saved views as small pills in the header; nothing until the user has saved one. */
export function ViewTabs({ views, activeId, edited, counts, onSelect }: ViewTabsProps) {
  if (views.length < 2) return null
  return (
    <ShadcnTabs
      className="kb-views__tabs tw:min-w-0"
      onValueChange={(id) => {
        const view = views.find((candidate) => candidate.id === id)
        if (view) onSelect(view)
      }}
      value={activeId}
    >
      <ShadcnTabsList aria-label="Saved views" className="kb-views__list">
        {views.map((view) => (
          <ShadcnTabsTrigger
            key={view.id}
            // Tabs report a change only: re-clicking the active tab is how an edited view goes back to its own filters.
            onClick={edited && view.id === activeId ? () => onSelect(view) : undefined}
            value={view.id}
          >
            {view.name}
            <span className="kb-views__count">{counts[view.id] ?? 0}</span>
            {edited && view.id === activeId ? <span className="kb-views__edited">Edited</span> : null}
          </ShadcnTabsTrigger>
        ))}
      </ShadcnTabsList>
    </ShadcnTabs>
  )
}

/** Board / List, as two icon tabs. */
export function LayoutToggle({ layout, onChange }: { layout: 'board' | 'list'; onChange: (layout: 'board' | 'list') => void }) {
  return (
    <ShadcnTabs className="kb-layout" onValueChange={(next) => onChange(next === 'list' ? 'list' : 'board')} value={layout}>
      <ShadcnTabsList aria-label="Layout">
        <ShadcnTabsTrigger aria-label="Board" title="Board" value="board">
          <Columns3 size={16} />
        </ShadcnTabsTrigger>
        <ShadcnTabsTrigger aria-label="List" title="List" value="list">
          <ListIcon size={16} />
        </ShadcnTabsTrigger>
      </ShadcnTabsList>
    </ShadcnTabs>
  )
}
