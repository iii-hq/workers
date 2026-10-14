import {
  BottomSheet,
  BottomSheetContent,
  BottomSheetTrigger,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
  IconButton,
  Input,
  SearchField,
  SegmentedControl,
  StatusDot,
  Tabs,
  TabsList,
  TabsTrigger,
  Toolbar,
  useConfirm,
} from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import {
  Bot,
  Check,
  ChevronDown,
  CircleDashed,
  Clock,
  Columns3,
  List as ListIcon,
  type LucideIcon,
  MoreHorizontal,
  SignalHigh,
  Tag,
  User,
} from 'lucide-react'
import { type ReactNode, useEffect, useId, useRef, useState, useSyncExternalStore } from 'react'
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
import { type AgentProfile, priorityLabel, type TicketSummary } from './shared'

type FacetId = 'labels' | 'assignees' | 'priorities' | 'updated'

type Opt = { value: string; label: string; count: number; lead?: ReactNode; disabled?: boolean }

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
  /** Fixed content above the list (the label search). */
  head?: ReactNode
  note?: string
  empty?: string
  pick: (value: string) => void
  clear: () => void
}

/**
 * The console's BottomSheet is `md:hidden` (viewport under 768px), so a narrow
 * pane on a wide screen keeps the popover: the sheet needs a phone viewport too.
 */
const PHONE = '(max-width: 767px)'
const watchPhone = (notify: () => void) => {
  const query = window.matchMedia(PHONE)
  query.addEventListener('change', notify)
  return () => query.removeEventListener('change', notify)
}
const usePhone = () => useSyncExternalStore(watchPhone, () => window.matchMedia(PHONE).matches)

const flip = (list: string[], value: string) =>
  list.includes(value) ? list.filter((v) => v !== value) : [...list, value]

/** `phase-2 +1`: the first choice, then how many more. */
const summary = (values: string[], name: (value: string) => string) =>
  values.length === 0 ? '' : name(values[0]) + (values.length > 1 ? ` +${values.length - 1}` : '')

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
}: FilterBarProps) {
  const [open, setOpen] = useState<FacetId | null>(null)
  const [labelQuery, setLabelQuery] = useState('')
  const [saving, setSaving] = useState(false)
  const phone = usePhone()
  const set = (patch: Partial<Filters>) => onChange({ ...filters, ...patch })
  const counts = (key: FacetId) => facetCounts(tickets, filters, ctx, key)

  const labelCounts = counts('labels')
  const query = labelQuery.trim().toLowerCase()
  const labelOpts: Opt[] = [...new Set([...allLabels(tickets), ...filters.labels])]
    .filter((label) => label.toLowerCase().includes(query))
    .map((label) => ({ value: label, label, count: labelCounts[label] ?? 0 }))
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
    { value: '', label: 'Unassigned', count: assigneeCounts[''] ?? 0, lead: <CircleDashed size={16} /> },
    ...[...agents]
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((agent) => ({
        value: agent.id,
        label: agent.name,
        count: assigneeCounts[agent.id] ?? 0,
        lead: <Bot size={16} />,
      })),
    ...strayAssignees.map((id) => ({
      value: id,
      label: id,
      count: assigneeCounts[id] ?? 0,
      lead: <User size={16} />,
    })),
  ].sort(
    // Stable: ties keep Unassigned first, then names A–Z.
    (a, b) =>
      Number(filters.assignees.includes(b.value)) - Number(filters.assignees.includes(a.value)) || b.count - a.count,
  )

  const priorityCounts = counts('priorities')
  const priorityOpt = (priority: string, tone?: 'alert' | 'warn'): Opt => ({
    value: priority,
    label: priorityLabel(priority),
    count: priorityCounts[priority] ?? 0,
    lead: <span className="kanban-filter__dot" data-tone={tone} />,
  })
  const strayPriorities = [...new Set(tickets.map((t) => t.priority))]
    .filter((p) => p && !priorities.includes(p))
    .sort()
  const priorityOpts: Opt[] = [
    ...[...priorities]
      .reverse()
      .map((priority, index) => priorityOpt(priority, index === 0 ? 'alert' : index === 1 ? 'warn' : undefined)),
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
      id: 'labels',
      name: 'Label',
      heading: 'Filter by label',
      Icon: Tag,
      value: summary(filters.labels, (label) => label),
      opts: labelOpts,
      selected: filters.labels,
      head: (
        <>
          <SearchField
            aria-label="Find a label"
            onChange={setLabelQuery}
            placeholder="Find a label"
            value={labelQuery}
          />
          <div className="kanban-filter__mode">
            <span>Ticket has</span>
            <SegmentedControl
              aria-label="Label match"
              onChange={(labelMode) => set({ labelMode })}
              options={[
                { value: 'any', label: 'any of these' },
                { value: 'all', label: 'all of these' },
              ]}
              value={filters.labelMode}
              variant="radio"
            />
          </div>
        </>
      ),
      empty: query ? `No label matches “${labelQuery.trim()}”.` : 'No labels on this board yet.',
      pick: (label) => set({ labels: flip(filters.labels, label) }),
      clear: () => set({ labels: [] }),
    },
    {
      id: 'assignees',
      name: 'Assignee',
      heading: 'Filter by assignee',
      Icon: User,
      value: summary(filters.assignees, (id) => (id === '' ? 'Unassigned' : (agentNames.get(id) ?? id))),
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

  return (
    <Toolbar aria-label="Filter tickets" className="kanban-filter" data-narrow={narrow ? 'true' : undefined}>
      <SearchField
        aria-label="Search tickets by key or title"
        className="kanban-filter__search"
        onChange={(q) => set({ q })}
        placeholder="Search key or title"
        value={filters.q}
      />
      <div className="kanban-filter__triggers">
        {facets.map((facet) => (
          <FacetMenu
            f={facet}
            key={facet.id}
            narrow={narrow}
            onOpenChange={(next) => {
              setOpen(next ? facet.id : null)
              if (!next) setLabelQuery('')
            }}
            open={open === facet.id}
            sheet={narrow && phone}
            shown={shownCount}
          />
        ))}
      </div>
      <div className="kanban-filter__end">
        {ctx.lastVisit && filters.updated !== 'visit' && changed > 0 ? (
          <Button
            className="kanban-filter__action"
            data-strong="true"
            onClick={() => set({ updated: 'visit' })}
            size="sm"
            variant="ghost"
          >
            <StatusDot tone="accent" />
            {changed} changed since your last visit
          </Button>
        ) : null}
        <span className="kanban-filter__total" role="status">
          {isEmpty(filters) ? plural(tickets.length) : `${shownCount} of ${tickets.length}`}
        </span>
        {isEmpty(filters) ? null : (
          <Button className="kanban-filter__action" onClick={() => onChange(EMPTY_FILTERS)} size="sm" variant="ghost">
            Clear filters
          </Button>
        )}
        {canSaveView ? (
          <Button
            className="kanban-filter__action"
            data-strong="true"
            onClick={() => setSaving(true)}
            size="sm"
            variant="ghost"
          >
            Save as view…
          </Button>
        ) : null}
      </div>
      <Dialog onOpenChange={setSaving} open={saving}>
        <DialogContent className="kanban-dialog">
          <SaveViewForm
            onCancel={() => setSaving(false)}
            onSave={(name) => {
              setSaving(false)
              onSaveView(name)
            }}
          />
        </DialogContent>
      </Dialog>
    </Toolbar>
  )
}

function FacetMenu({
  f,
  sheet,
  narrow,
  shown,
  open,
  onOpenChange,
}: {
  f: Facet
  sheet: boolean
  narrow: boolean
  shown: number
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const headRef = useRef<HTMLDivElement>(null)
  const picked = f.selected.length
  const pickedText = picked === 0 ? 'None selected' : `${picked} selected`
  const trigger = (
    <Button className="kanban-filter__trigger" data-active={f.value ? 'true' : undefined} size="sm" variant="ghost">
      <f.Icon size={16} />
      {f.name}
      {f.value ? <span className="kanban-filter__value">{f.value}</span> : null}
      <ChevronDown size={16} />
    </Button>
  )
  const rows = f.opts.map((opt) => (
    <Row
      checked={f.selected.includes(opt.value)}
      key={opt.value}
      onPick={f.pick}
      opt={opt}
      sheet={sheet}
      single={f.single}
    />
  ))
  const body = rows.length > 0 ? rows : <p className="kanban-filter__empty">{f.empty}</p>

  if (sheet) {
    return (
      <BottomSheet onOpenChange={onOpenChange} open={open}>
        <BottomSheetTrigger asChild>{trigger}</BottomSheetTrigger>
        <BottomSheetContent
          className="kanban-filter__sheet"
          description={f.single ? f.note : pickedText}
          heading={f.heading}
        >
          <div className="kanban-filter__sheet-body">
            {f.head ? <div className="kanban-filter__head">{f.head}</div> : null}
            {/* biome-ignore lint/a11y/useAriaPropsSupportedByRole: the role is always radiogroup or group, both name themselves */}
            <div aria-label={f.heading} className="kanban-filter__list" role={f.single ? 'radiogroup' : 'group'}>
              {body}
            </div>
          </div>
          <div className="kanban-filter__sheet-foot">
            {f.single ? null : (
              <Button disabled={picked === 0} onClick={f.clear} variant="ghost">
                Clear
              </Button>
            )}
            <Button className="kanban-filter__show" onClick={() => onOpenChange(false)}>
              Show {plural(shown)}
            </Button>
          </div>
        </BottomSheetContent>
      </BottomSheet>
    )
  }

  return (
    <DropdownMenu onOpenChange={onOpenChange} open={open}>
      <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="kanban-filter__menu"
        data-facet={f.id}
        data-narrow={narrow ? 'true' : undefined}
        onOpenAutoFocus={
          f.head
            ? (event) => {
                event.preventDefault()
                headRef.current?.querySelector('input')?.focus()
              }
            : undefined
        }
      >
        {f.head ? (
          // biome-ignore lint/a11y/noStaticElementInteractions: a key guard around the real controls (search, match mode), not a control
          <div
            className="kanban-filter__head"
            onKeyDown={(event) => {
              // The menu would turn typing into item typeahead and swallow Tab: keep both out of the field.
              if (event.key === 'Tab' || event.key.length === 1) event.stopPropagation()
              else if (event.key === 'ArrowDown') {
                event.preventDefault()
                event.currentTarget.parentElement?.querySelector<HTMLElement>('[role^="menuitem"]')?.focus()
              }
            }}
            ref={headRef}
          >
            {f.head}
          </div>
        ) : null}
        {f.single ? (
          <DropdownMenuRadioGroup className="kanban-filter__list" onValueChange={f.pick} value={f.selected[0]}>
            {body}
          </DropdownMenuRadioGroup>
        ) : (
          <div className="kanban-filter__list">{body}</div>
        )}
        {f.single ? (
          <p className="kanban-filter__note">{f.note}</p>
        ) : (
          <div className="kanban-filter__foot">
            <span className="kanban-filter__hint">{pickedText}</span>
            <DropdownMenuItem
              className="kanban-filter__clear"
              disabled={picked === 0}
              onSelect={(event) => {
                event.preventDefault()
                f.clear()
              }}
            >
              Clear
            </DropdownMenuItem>
          </div>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function Row({
  opt,
  sheet,
  single,
  checked,
  onPick,
}: {
  opt: Opt
  sheet: boolean
  single?: boolean
  checked: boolean
  onPick: (value: string) => void
}) {
  const zero = opt.count === 0 ? 'true' : undefined
  const content = (
    <>
      {opt.lead}
      <span className="kanban-filter__name">{opt.label}</span>
      <span className="kanban-filter__count">{opt.count}</span>
    </>
  )
  if (sheet) {
    return (
      // biome-ignore lint/a11y/useAriaPropsSupportedByRole: the role is always radio or checkbox, both carry aria-checked
      <button
        aria-checked={checked}
        className="kanban-filter__row"
        data-zero={zero}
        disabled={opt.disabled}
        onClick={() => onPick(opt.value)}
        role={single ? 'radio' : 'checkbox'}
        type="button"
      >
        <Check className="kanban-filter__tick" size={16} />
        {content}
      </button>
    )
  }
  if (single) {
    return (
      <DropdownMenuRadioItem data-zero={zero} disabled={opt.disabled} value={opt.value}>
        {content}
      </DropdownMenuRadioItem>
    )
  }
  return (
    <DropdownMenuCheckboxItem
      checked={checked}
      data-zero={zero}
      onCheckedChange={() => onPick(opt.value)}
      onSelect={(event) => event.preventDefault()}
    >
      {content}
    </DropdownMenuCheckboxItem>
  )
}

/** Mounted with the dialog content, so every opening starts with an empty name. Same anatomy as the New ticket dialog. */
function SaveViewForm({ onCancel, onSave }: { onCancel: () => void; onSave: (name: string) => void }) {
  const [name, setName] = useState('')
  const id = useId()
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault()
        if (name.trim()) onSave(name.trim())
      }}
    >
      <DialogTitle>Save view</DialogTitle>
      <DialogDescription>Saved in this browser only.</DialogDescription>
      <div className="kanban-dialog__form">
        <label className="kanban-field" htmlFor={id}>
          <span className="kanban-field__label">View name</span>
          <Input autoFocus id={id} onChange={setName} placeholder="Bugs to triage" value={name} />
        </label>
      </div>
      <div className="kanban-dialog__foot">
        <Button onClick={onCancel} type="button" variant="ghost">
          Cancel
        </Button>
        <Button disabled={!name.trim()} type="submit" variant="primary">
          Save view
        </Button>
      </div>
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
  /** User views only, never `ALL_VIEW`. */
  onDelete: (id: string) => void
  layout: 'board' | 'list'
  onLayoutChange: (layout: 'board' | 'list') => void
  /** The pane is narrow (< ~760px): tabs, layout switch and menu get 44px targets. */
  narrow: boolean
}

export function ViewTabs({
  views,
  activeId,
  edited,
  counts,
  onSelect,
  onDelete,
  layout,
  onLayoutChange,
  narrow,
}: ViewTabsProps) {
  const { confirm, dialog } = useConfirm()
  const root = useRef<HTMLDivElement>(null)
  const seen = useRef(views.length)
  const refocus = useRef(false)
  const own = views.filter((view) => view.id !== ALL_VIEW.id)
  /** After the dialog's own focus return (a timer), so this one wins. */
  const focusActiveTab = () =>
    setTimeout(() => root.current?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus(), 0)
  const remove = async (view: SavedView) => {
    const ok = await confirm({
      title: `Delete view “${view.name}”?`,
      description: "Saved in this browser only; this can't be undone.",
      confirmLabel: 'Delete view',
      tone: 'danger',
    })
    if (!ok) return
    // Deleting the last one unmounts Manage views, where the dialog would return the focus.
    refocus.current = own.length === 1
    onDelete(view.id)
  }
  // A view saved from this pane becomes the active one; the Save button that opened the dialog is gone by then.
  useEffect(() => {
    const saved = views.length > seen.current && views[views.length - 1]?.id === activeId
    seen.current = views.length
    if (saved || refocus.current) focusActiveTab()
    refocus.current = false
  }, [views, activeId])
  return (
    <div className="kanban-views" data-narrow={narrow ? 'true' : undefined} ref={root}>
      <Tabs
        className="kanban-views__tabs"
        onValueChange={(id) => {
          const view = views.find((candidate) => candidate.id === id)
          if (view) onSelect(view)
        }}
        value={activeId}
      >
        <TabsList aria-label="Saved views" variant="line">
          {views.map((view) => (
            <TabsTrigger
              icon={false}
              key={view.id}
              // Radix reports a change only: re-clicking the active tab is how an edited view goes back to its own filters.
              onClick={edited && view.id === activeId ? () => onSelect(view) : undefined}
              value={view.id}
            >
              {view.name}
              <span className="kanban-views__count">{counts[view.id] ?? 0}</span>
              {edited && view.id === activeId ? <span className="kanban-views__edited">edited</span> : null}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      {own.length > 0 ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <IconButton className="kanban-views__manage" label="Manage views" variant="ghost">
              <MoreHorizontal size={16} />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="kanban-views__menu">
            {own.map((view) => (
              <DropdownMenuItem key={view.id} onSelect={() => void remove(view)}>
                <span className="kanban-views__delete">Delete view “{view.name}”…</span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
      <SegmentedControl
        aria-label="Layout"
        onChange={onLayoutChange}
        options={[
          { value: 'board', label: 'Board', icon: <Columns3 size={16} /> },
          { value: 'list', label: 'List', icon: <ListIcon size={16} /> },
        ]}
        value={layout}
        variant="radio"
      />
      {dialog}
    </div>
  )
}
