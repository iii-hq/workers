import {
  BottomSheet,
  BottomSheetContent,
  Button,
  Checkbox,
  Chip,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
  EmptyState,
  type Host,
  IconButton,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  SearchField,
  Select,
  StatusDot,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { ArrowDown, ArrowUp, Bot, Check, ListFilter, MoreHorizontal, Plus, TriangleAlert, User, X } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { subtaskStats } from './board-state'
import { isChangedSinceVisit, type MatchContext } from './filters'
import {
  type AgentProfile,
  api,
  type Column,
  type ProjectView,
  priorityLabel,
  type ReleaseView,
  StatusGlyph,
  statusLabel,
  type Ticket,
  type TicketSummary,
} from './shared'

export type TicketListProps = {
  host: Host
  tickets: TicketSummary[] // already filtered by the board, any order
  columns: Column[] // board columns in order (id, label)
  priorities: string[] // configured order, e.g. ['low','medium','high','urgent']
  agents: AgentProfile[]
  labels: string[] // every label on the board, most used first
  projects: ProjectView[] // the board's, archived ones included
  releases: ReleaseView[] // the board's, in target order
  doneStatus: string // the column that counts as done: subtask progress reads it
  subtasks?: Map<string, { done: number; total: number }> // board-wide progress per parent; absent = counted from `tickets`
  highlightLabels: string[] // the active label filter: those chips go first and look selected
  ctx: MatchContext
  narrow: boolean
  onOpenTicket: (key: string) => void
  onUpdated: (tickets: Ticket[]) => void // server results, the board upserts them
  onError: (message: string) => void
}

type SortBy = 'key' | 'priority' | 'updated'
type Sort = { by: SortBy; dir: 1 | -1 }
type Job = { key: string; call: () => Promise<Ticket> }
type Field = 'status' | 'priority' | 'assignee'
/** A radio menu's two ends: what a ticket has now (`null` = nothing) and what picking a row does. */
type Choice = { get: (ticket: TicketSummary) => string | null; set: (value: string) => void }

/** The column headers are gone on narrow panes: the same orderings, one pick. Value = `by:dir`. */
const SORTS: { value: string; label: string }[] = [
  { value: 'priority:1', label: 'Priority · urgent first' },
  { value: 'priority:-1', label: 'Priority · urgent last' },
  { value: 'updated:-1', label: 'Updated · newest first' },
  { value: 'updated:1', label: 'Updated · oldest first' },
  { value: 'key:1', label: 'Key · lowest first' },
  { value: 'key:-1', label: 'Key · highest first' },
]

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

const MAX_CHIPS = 3
const COLUMN_COUNT = 7

/** The server folds ASCII case only ("Bug" is "bug", "Ação" is not "AÇÃO"), so the menu must agree or a toggle would not stick. */
const fold = (label: string) => label.replace(/[A-Z]/g, (char) => char.toLowerCase())

const MAX_LISTED = 8
/** Parent candidates rendered at once: the search narrows the rest. */
const MAX_PARENTS = 50

/** `KAN-1, KAN-2, 3 more`: the keys capped like the delete dialog's. */
function keyList(keys: string[]): string {
  const listed = keys.length > MAX_LISTED + 1 ? MAX_LISTED : keys.length
  return [...keys.slice(0, listed), ...(listed < keys.length ? [`${keys.length - listed} more`] : [])].join(', ')
}

const RELEASE_HINT = 'Released releases are not listed.'
const PROJECT_HINT = 'Archived projects are not listed.'

const PARENT_HINT = 'Subtasks go one level deep: tickets that have subtasks are skipped.'

type Note = { headline: string; detail: string }
/** One pick-one choice, drawn as a menu item or as a sheet row. */
type Opt = { value: string; node: ReactNode }
/** The More menu entries that open a sheet page on phones: a submenu beside the menu does not fit there. */
type Page = 'priority' | 'assignee' | 'release' | 'project' | 'parent'

/** 0 = most urgent: `priorities` is configured least urgent first. */
function priorityRank(priorities: string[], priority: string): number {
  const index = priorities.indexOf(priority)
  return index < 0 ? priorities.length : priorities.length - 1 - index
}

function dotTone(priorities: string[], priority: string): string {
  if (!priorities.includes(priority)) return 'ghost'
  return ['alert', 'warn', 'faint'][priorityRank(priorities, priority)] ?? 'ghost'
}

function compareTickets(a: TicketSummary, b: TicketSummary, by: SortBy, priorities: string[]): number {
  const newestFirst = Date.parse(b.updated_at) - Date.parse(a.updated_at)
  if (by === 'priority')
    return priorityRank(priorities, a.priority) - priorityRank(priorities, b.priority) || newestFirst
  if (by === 'updated') return -newestFirst
  return Number(a.key.split('-').pop()) - Number(b.key.split('-').pop()) || a.key.localeCompare(b.key)
}

export function TicketList({
  host,
  tickets,
  columns,
  priorities,
  agents,
  labels,
  projects,
  releases,
  doneStatus,
  subtasks,
  highlightLabels,
  ctx,
  narrow,
  onOpenTicket,
  onUpdated,
  onError,
}: TicketListProps) {
  const [sort, setSort] = useState<Sort>({ by: 'priority', dir: 1 })
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  // What the last bulk action left out, in words. Gone with the next action or an empty selection.
  const [note, setNote] = useState<Note | null>(null)
  // Every live ticket on the board: a parent can sit outside the filtered view, and its key is still wanted.
  const [pool, setPool] = useState<TicketSummary[]>([])
  const asked = useRef(new Set<string>())
  // Synchronous twin of `busy`: two events in the same tick both see the state as idle.
  const inFlight = useRef(false)
  const { confirm, dialog } = useConfirm()
  const selectAllRef = useRef<HTMLInputElement>(null)
  const hadBar = useRef(false)
  const groupId = useId()
  const size = narrow ? 'lg' : 'sm'
  const highlighted = new Set(highlightLabels)

  // Drop ids the board no longer shows (deleted, or filtered out).
  useEffect(() => {
    setSelected((prev) => {
      const live = new Set(tickets.map((ticket) => ticket.id))
      const next = new Set([...prev].filter((id) => live.has(id)))
      return next.size === prev.size ? prev : next
    })
  }, [tickets])

  // Best effort: without the board the picker offers what the view shows, and the server still validates.
  const loadPool = useCallback(() => {
    api
      .board(host.iii)
      .then((board) => setPool(board.columns.flatMap((column) => column.tickets)))
      .catch(() => undefined)
  }, [host.iii])

  // The view wins over the pool: it is the fresher copy.
  const known = useMemo(() => new Map([...pool, ...tickets].map((ticket) => [ticket.id, ticket])), [pool, tickets])
  // Ids that have live subtasks: they cannot become one.
  const parents = useMemo(
    () => new Set([...known.values()].flatMap((t) => (t.parent_id && !t.deleted_at ? [t.parent_id] : []))),
    [known],
  )
  // The board's count when it gives one: a filtered view would otherwise show "1/1" for a parent that has 4 subtasks.
  const progress = useMemo(() => subtasks ?? subtaskStats(tickets, doneStatus), [subtasks, tickets, doneStatus])
  const releaseNames = useMemo(() => new Map(releases.map((release) => [release.id, release.name])), [releases])
  const projectNames = useMemo(() => new Map(projects.map((project) => [project.id, project.name])), [projects])

  // A child whose parent is not in the view needs the board to say "Subtask of KEY"; ask once per parent.
  useEffect(() => {
    const loose = tickets.flatMap((t) =>
      t.parent_id && !known.has(t.parent_id) && !asked.current.has(t.parent_id) ? [t.parent_id] : [],
    )
    for (const id of loose) asked.current.add(id)
    if (loose.length > 0) loadPool()
  }, [tickets, known, loadPool])

  const groups = useMemo(
    () =>
      columns
        .map((column) => ({
          column,
          rows: tickets
            .filter((ticket) => ticket.status === column.id)
            .sort((a, b) => sort.dir * compareTickets(a, b, sort.by, priorities)),
        }))
        .filter((group) => group.rows.length > 0),
    [columns, tickets, priorities, sort],
  )

  const picked = tickets.filter((ticket) => selected.has(ticket.id))
  const allPicked = tickets.length > 0 && picked.length === tickets.length
  // What is selected NOW, for code that resumes after an await (the delete dialog).
  const pickedRef = useRef(picked)
  pickedRef.current = picked

  // The bar unmounts with the focus inside it (clear, delete, an edit that filters every picked ticket out): land on select-all, not <body>.
  useEffect(() => {
    const active = document.activeElement
    if (hadBar.current && picked.length === 0 && (!active || active === document.body)) selectAllRef.current?.focus()
    hadBar.current = picked.length > 0
    if (picked.length === 0) setNote(null)
  }, [picked.length])

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (!next.delete(id)) next.add(id)
      return next
    })

  /** Run every job, hand the saved tickets to the board, report the ones that failed. Returns the failed keys. */
  const run = async (jobs: Job[]): Promise<Set<string>> => {
    setNote(null)
    if (jobs.length === 0) return new Set()
    // One batch at a time. The bar is disabled meanwhile, but Enter in the label search is not.
    if (inFlight.current) return new Set(jobs.map((job) => job.key))
    inFlight.current = true
    setBusy(true)
    try {
      const results = await Promise.allSettled(jobs.map((job) => job.call()))
      const saved = results.flatMap((result) => (result.status === 'fulfilled' ? [result.value] : []))
      const failed = results.flatMap((result, i) =>
        result.status === 'rejected' ? [`${jobs[i].key} — ${errorMessage(result.reason)}`] : [],
      )
      const failedKeys = new Set(results.flatMap((result, i) => (result.status === 'rejected' ? [jobs[i].key] : [])))
      if (saved.length > 0) onUpdated(saved)
      if (failed.length > 0) {
        const more = failed.length > 3 ? `; and ${failed.length - 3} more` : ''
        onError(
          `${failed.length} of ${jobs.length} ${jobs.length === 1 ? 'change was' : 'changes were'} not saved: ${failed.slice(0, 3).join('; ')}${more}`,
        )
      }
      return failedKeys
    } finally {
      inFlight.current = false
      setBusy(false)
    }
  }

  const update = (ticket: TicketSummary, patch: Record<string, unknown>): Job => ({
    key: ticket.key,
    call: () => api.update(host.iii, { id: ticket.key, ...patch, actor: 'user' }),
  })

  const setField = (field: Field, value: string) => {
    const next = field === 'assignee' ? value || null : value
    void run(picked.filter((ticket) => ticket[field] !== next).map((ticket) => update(ticket, { [field]: next })))
  }

  /**
   * Remove the label from every selected ticket when all have it, else add it to those that lack it.
   * The server edits the labels the ticket has when the write runs: a whole `labels` array computed
   * from this snapshot would drop what an agent added in between.
   */
  const setLabel = (label: string) => {
    const has = (ticket: TicketSummary) => ticket.labels.some((l) => fold(l) === fold(label))
    const removing = picked.every(has)
    const patch = removing ? { remove_labels: [label] } : { add_labels: [label] }
    void run(picked.filter((ticket) => has(ticket) === removing).map((ticket) => update(ticket, patch)))
  }

  /** Release or project for every selected ticket, `''` takes them out. A server refusal reaches `onError`. */
  const setFacet = (facet: 'release' | 'project', id: string) => {
    const current = (ticket: TicketSummary) => (facet === 'release' ? ticket.release_id : ticket.project_id) ?? ''
    void run(picked.filter((ticket) => current(ticket) !== id).map((ticket) => update(ticket, { [facet]: id || null })))
  }

  /** Parent for the selection, `null` detaches. Tickets with subtasks cannot take one: named in the note, not sent. */
  const setParent = async (parent: TicketSummary | null) => {
    const changing = picked.filter((ticket) => ticket.parent_id !== (parent?.id ?? null))
    const skipped = parent ? changing.filter((ticket) => parents.has(ticket.id)) : []
    const jobs = changing
      .filter((ticket) => !skipped.includes(ticket))
      .map((ticket) => update(ticket, { parent: parent?.key ?? null }))
    const failed = await run(jobs)
    if (!parent || skipped.length === 0) return
    const sent = jobs.length - failed.size
    const many = skipped.length > 1
    const keys = keyList(skipped.map((ticket) => ticket.key))
    const done = sent === 1 ? '1 ticket is now a subtask' : `${sent} tickets are now subtasks`
    setNote({
      headline: `${skipped.length} ${many ? 'tickets were' : 'ticket was'} skipped`,
      detail: `${keys} already ${many ? 'have' : 'has'} subtasks, and subtasks go one level deep. ${
        sent > 0 ? `${done} of ${parent.key}.` : 'Nothing else changed.'
      }`,
    })
  }

  /** The quiet second line of a ticket: where it sits in the plan. */
  const planLine = (ticket: TicketSummary) => {
    const parent = ticket.parent_id ? known.get(ticket.parent_id) : undefined
    const own = progress.get(ticket.id)
    const release = ticket.release_id ? releaseNames.get(ticket.release_id) : undefined
    const project = ticket.project_id ? projectNames.get(ticket.project_id) : undefined
    const facts: { key: string; text: string; noun?: string }[] = [
      ...(ticket.parent_id ? [{ key: 'parent', text: parent ? `Subtask of ${parent.key}` : 'Subtask' }] : []),
      ...(own ? [{ key: 'subtasks', text: `${own.done}/${own.total} subtasks` }] : []),
      ...(release ? [{ key: 'release', text: release, noun: 'Release' }] : []),
      ...(project ? [{ key: 'project', text: project, noun: 'Project' }] : []),
    ]
    if (facts.length === 0) return null
    return (
      <span className="kanban-list__plan">
        {facts.map(({ key, text, noun }) => (
          <span className="kanban-list__fact" key={key}>
            {noun ? <span className="kanban-list__sr">{noun} </span> : null}
            {text}
          </span>
        ))}
      </span>
    )
  }

  const remove = async () => {
    const keys = picked.map((ticket) => ticket.key)
    const one = keys.length === 1
    const listed = keys.length > MAX_LISTED + 1 ? MAX_LISTED : keys.length
    const ok = await confirm({
      title: `Delete ${one ? '1 ticket' : `${keys.length} tickets`}?`,
      description: `${one ? 'It disappears' : 'They disappear'} from the board but ${one ? 'stays' : 'stay'} in the board file.`,
      details: [...keys.slice(0, listed), ...(listed < keys.length ? [`and ${keys.length - listed} more`] : [])],
      confirmLabel: 'Delete',
      tone: 'danger',
    })
    if (!ok) return
    // Tickets that left the view while the dialog was open (filter changed, deleted elsewhere) are not what was confirmed.
    const stillPicked = new Set(pickedRef.current.map((ticket) => ticket.id))
    const doomed = picked.filter((ticket) => stillPicked.has(ticket.id))
    const failed = await run(
      doomed.map((ticket) => ({
        key: ticket.key,
        call: () => api.remove(host.iii, { id: ticket.key, actor: 'user' }),
      })),
    )
    // The ones that failed stay selected so the user can retry.
    setSelected((prev) => {
      const next = new Set(prev)
      for (const ticket of doomed) if (!failed.has(ticket.key)) next.delete(ticket.id)
      return next
    })
  }

  const sortHead = (by: SortBy, label: string, className: string) => {
    const active = sort.by === by
    return (
      <TableHead
        aria-sort={active ? (sort.dir === 1 ? 'ascending' : 'descending') : 'none'}
        className={`kanban-list__head ${className}`}
      >
        <Button
          className="kanban-list__sort"
          data-active={active || undefined}
          onClick={() =>
            setSort((prev) =>
              prev.by === by ? { by, dir: prev.dir === 1 ? -1 : 1 } : { by, dir: by === 'updated' ? -1 : 1 },
            )
          }
          size={size}
          variant="ghost"
        >
          {label}
          {active ? sort.dir === 1 ? <ArrowUp size={16} /> : <ArrowDown size={16} /> : null}
        </Button>
      </TableHead>
    )
  }

  const selectAll = (
    <Checkbox
      aria-label="Select all shown tickets"
      checked={allPicked}
      className="kanban-list__check"
      indeterminate={picked.length > 0 && !allPicked}
      onChange={() => setSelected(allPicked ? new Set() : new Set(tickets.map((ticket) => ticket.id)))}
      ref={selectAllRef}
    />
  )

  if (tickets.length === 0) {
    return (
      <div className="kanban-list kanban-list__empty">
        <EmptyState description="No ticket matches the filters." icon={ListFilter} title="No tickets" />
        {dialog}
      </div>
    )
  }

  return (
    <div className="kanban-list" data-bar={picked.length > 0 || undefined} data-narrow={narrow || undefined}>
      {/* First in DOM order so Shift+Tab from a row reaches it without crossing every row; it is absolutely positioned, so nothing moves. */}
      {picked.length > 0 ? (
        <BulkBar
          agents={agents}
          busy={busy}
          columns={columns}
          known={known}
          labels={labels}
          narrow={narrow}
          onClear={() => setSelected(new Set())}
          onDelete={() => void remove()}
          onFacet={setFacet}
          onField={setField}
          onLabel={setLabel}
          onOpenParents={loadPool}
          onParent={(parent) => void setParent(parent)}
          parents={parents}
          picked={picked}
          priorities={priorities}
          projects={projects}
          releases={releases}
          tickets={tickets}
        />
      ) : null}
      {note ? (
        <div className="kanban-list__result">
          <StatusPanel
            action={
              <Button onClick={() => setNote(null)} size={narrow ? 'lg' : 'md'} variant="pill">
                Dismiss
              </Button>
            }
            detail={note.detail}
            headline={note.headline}
            icon={<TriangleAlert size={16} />}
            role="status"
            variant="warn"
          />
        </div>
      ) : null}
      {narrow ? (
        <>
          <div className="kanban-list__tools">
            {selectAll}
            <span aria-hidden className="kanban-list__caption">
              Sort
            </span>
            <Select
              aria-label="Sort tickets"
              className="kanban-list__sort-select"
              onChange={(value) => {
                const [by, dir] = value.split(':')
                setSort({ by: by as SortBy, dir: dir === '-1' ? -1 : 1 })
              }}
              options={SORTS}
              sheetTitle="Sort tickets"
              value={`${sort.by}:${sort.dir}`}
            />
          </div>
          <div className="kanban-list__viewport">
            {groups.map(({ column, rows }, index) => (
              <ListGroup key={column.id}>
                <ListGroupLabel id={`${groupId}-${index}`}>
                  <div className="kanban-list__box">
                    <StatusGlyph columns={columns} status={column.id} />
                    <span>{column.label}</span>
                    <span className="kanban-list__count">{rows.length}</span>
                  </div>
                </ListGroupLabel>
                <List aria-labelledby={`${groupId}-${index}`} role="list">
                  {rows.map((ticket) => (
                    <ListItem
                      as="div"
                      className="kanban-list__ticket"
                      // The checkbox carries the state for assistive tech; `selected` would put aria-selected on a plain div.
                      data-selected={selected.has(ticket.id) ? 'true' : undefined}
                      description={
                        <>
                          <span className="kanban-list__mono">{ticket.key}</span>
                          {` · ${statusLabel(columns, ticket.status)} · ${priorityLabel(ticket.priority)}`}
                          {planLine(ticket)}
                        </>
                      }
                      key={ticket.id}
                      label={
                        <button
                          className="kanban-list__title"
                          onClick={() => onOpenTicket(ticket.key)}
                          title={ticket.title}
                          type="button"
                        >
                          {ticket.title}
                        </button>
                      }
                      leading={
                        <Checkbox
                          aria-label={`Select ${ticket.key}`}
                          checked={selected.has(ticket.id)}
                          className="kanban-list__check"
                          onChange={() => toggle(ticket.id)}
                        />
                      }
                      role="listitem"
                      trailing={
                        isChangedSinceVisit(ticket, ctx) ? (
                          <>
                            <StatusDot title="Changed since your last visit" tone="accent" />
                            <span className="kanban-list__sr">Changed since your last visit.</span>
                          </>
                        ) : undefined
                      }
                    />
                  ))}
                </List>
              </ListGroup>
            ))}
          </div>
        </>
      ) : (
        <TableViewport className="kanban-list__viewport">
          <TableFrame>
            <Table aria-label="Tickets" className="kanban-list__table" inset>
              <TableHeader>
                <TableRow className="kanban-list__headrow">
                  <TableHead className="kanban-list__head kanban-list__head--select">{selectAll}</TableHead>
                  {sortHead('key', 'Key', 'kanban-list__head--key')}
                  <TableHead className="kanban-list__head">Title</TableHead>
                  <TableHead className="kanban-list__head kanban-list__head--status">Status</TableHead>
                  {sortHead('priority', 'Priority', 'kanban-list__head--priority')}
                  <TableHead className="kanban-list__head kanban-list__head--assignee">Assignee</TableHead>
                  {sortHead('updated', 'Updated', 'kanban-list__head--updated')}
                </TableRow>
              </TableHeader>
              {groups.map(({ column, rows }) => (
                <TableBody key={column.id}>
                  <TableRow>
                    <TableHead className="kanban-list__group" colSpan={COLUMN_COUNT} scope="rowgroup">
                      <div className="kanban-list__box">
                        <StatusGlyph columns={columns} status={column.id} />
                        <span>{column.label}</span>
                        <span className="kanban-list__count">{rows.length}</span>
                      </div>
                    </TableHead>
                  </TableRow>
                  {rows.map((ticket) => {
                    const ordered = [...ticket.labels].sort(
                      (a, b) => Number(highlighted.has(b)) - Number(highlighted.has(a)),
                    )
                    const rest = ordered.slice(MAX_CHIPS)
                    const agent = agents.find((a) => a.id === ticket.assignee)
                    return (
                      <TableRow className="kanban-list__row" key={ticket.id} selected={selected.has(ticket.id)}>
                        <TableCell className="kanban-list__cell kanban-list__cell--select">
                          <Checkbox
                            aria-label={`Select ${ticket.key}`}
                            checked={selected.has(ticket.id)}
                            className="kanban-list__check"
                            onChange={() => toggle(ticket.id)}
                          />
                        </TableCell>
                        <TableCell className="kanban-list__cell kanban-list__mono">{ticket.key}</TableCell>
                        <TableCell className="kanban-list__cell">
                          <div className="kanban-list__stack">
                            <div className="kanban-list__box">
                              <button
                                className="kanban-list__title"
                                onClick={() => onOpenTicket(ticket.key)}
                                title={ticket.title}
                                type="button"
                              >
                                {ticket.title}
                              </button>
                              {ordered.slice(0, MAX_CHIPS).map((label) => (
                                <Chip className="kanban-list__chip" key={label} selected={highlighted.has(label)}>
                                  {label}
                                </Chip>
                              ))}
                              {rest.length > 0 ? (
                                <Chip className="kanban-list__chip kanban-list__mono" title={rest.join(', ')}>
                                  +{rest.length}
                                </Chip>
                              ) : null}
                            </div>
                            {planLine(ticket)}
                          </div>
                        </TableCell>
                        <TableCell className="kanban-list__cell">
                          <div className="kanban-list__box">
                            <StatusGlyph columns={columns} status={ticket.status} />
                            <span className="kanban-list__text">{statusLabel(columns, ticket.status)}</span>
                          </div>
                        </TableCell>
                        <TableCell className="kanban-list__cell">
                          <div className="kanban-list__box">
                            <span className="kanban-list__dot" data-tone={dotTone(priorities, ticket.priority)} />
                            <span className="kanban-list__text">{priorityLabel(ticket.priority)}</span>
                          </div>
                        </TableCell>
                        <TableCell className="kanban-list__cell">
                          <div className="kanban-list__box">
                            {ticket.assignee ? (
                              <>
                                {agent ? <Bot size={16} /> : <User size={16} />}
                                <span className="kanban-list__text">{agent?.name ?? ticket.assignee}</span>
                              </>
                            ) : (
                              <span className="kanban-list__text kanban-list__faint">Unassigned</span>
                            )}
                          </div>
                        </TableCell>
                        <TableCell className="kanban-list__cell kanban-list__mono kanban-list__cell--end">
                          <div className="kanban-list__box kanban-list__box--end">
                            {isChangedSinceVisit(ticket, ctx) ? (
                              <>
                                <StatusDot title="Changed since your last visit" tone="accent" />
                                <span className="kanban-list__sr">Changed since your last visit.</span>
                              </>
                            ) : null}
                            <time dateTime={ticket.updated_at} title={new Date(ticket.updated_at).toLocaleString()}>
                              {formatRelative(ticket.updated_at, ctx.now)}
                            </time>
                          </div>
                        </TableCell>
                      </TableRow>
                    )
                  })}
                </TableBody>
              ))}
            </Table>
          </TableFrame>
        </TableViewport>
      )}
      {dialog}
    </div>
  )
}

type BulkBarProps = {
  picked: TicketSummary[]
  /** Every ticket shown: where assignees and priorities without a profile or config come from. */
  tickets: TicketSummary[]
  columns: Column[]
  priorities: string[]
  agents: AgentProfile[]
  labels: string[]
  projects: ProjectView[]
  releases: ReleaseView[]
  /** Every live ticket on the board (the view over the pool): where the parent candidates come from. */
  known: Map<string, TicketSummary>
  /** Ids that have subtasks. */
  parents: Set<string>
  narrow: boolean
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

function BulkBar({
  picked,
  tickets,
  columns,
  priorities,
  agents,
  labels,
  projects,
  releases,
  known,
  parents,
  narrow,
  busy,
  onField,
  onFacet,
  onParent,
  onOpenParents,
  onLabel,
  onDelete,
  onClear,
}: BulkBarProps) {
  const [query, setQuery] = useState('')
  const [parentQuery, setParentQuery] = useState('')
  const [parentOpen, setParentOpen] = useState(false)
  const [page, setPage] = useState<Page>('release')
  // Apart from `page` so the sheet keeps its words while it slides out.
  const [sheetOpen, setSheetOpen] = useState(false)
  const phone = usePhone()
  // A submenu beside the More menu does not fit a phone: the choices become pages in a sheet.
  const sheet = narrow && phone
  const moreRef = useRef<HTMLButtonElement>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const parentSearchRef = useRef<HTMLInputElement>(null)
  const lastFocus = useRef<HTMLElement | null>(null)
  const n = picked.length
  const size = narrow ? 'lg' : 'sm'
  const item = { className: 'kanban-list__item', 'data-narrow': narrow || undefined }
  const q = query.trim()
  // "Bug" and "bug" are one label to the server: one row, under the spelling most tickets use (`labels` is most used first).
  const spellings = new Map<string, string>()
  for (const label of labels) if (!spellings.has(fold(label))) spellings.set(fold(label), label)
  const existing = spellings.get(fold(q))
  const canCreate = q !== '' && !existing
  const rows = [...spellings.values()]
    .filter((label) => label.toLowerCase().includes(q.toLowerCase()))
    .map((label) => ({
      label,
      have: picked.filter((ticket) => ticket.labels.some((l) => fold(l) === fold(label))).length,
    }))
    .sort((a, b) => b.have - a.have)
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

  // A disabled trigger cannot take the focus back from its closing menu: give it back once the batch is done.
  useEffect(() => {
    const active = document.activeElement
    if (!busy && lastFocus.current?.isConnected && (!active || active === document.body)) lastFocus.current.focus()
  }, [busy])

  /** What every selected ticket shares (`''` = unassigned), else undefined: nothing is checked. */
  const shared = (pick: (ticket: TicketSummary) => string | null) => {
    const first = pick(picked[0]) ?? ''
    return picked.every((ticket) => (pick(ticket) ?? '') === first) ? first : undefined
  }

  /** Toggle the label that matches the text in any case, else create it as typed. */
  const commit = () => {
    if (busy) return
    onLabel(existing ?? q)
    setQuery('')
  }

  /** The pick-one list for a ticket field, shared by the bar's buttons and the narrow "More" submenus. */
  const radioList = (heading: string, pick: Choice, children: ReactNode, hint?: string) => (
    <>
      <DropdownMenuLabel>{heading}</DropdownMenuLabel>
      <DropdownMenuRadioGroup onValueChange={pick.set} value={shared(pick.get)}>
        {children}
      </DropdownMenuRadioGroup>
      {hint ? <p className="kanban-list__hint">{hint}</p> : null}
    </>
  )

  const radioMenu = (label: string, heading: string, pick: Choice, children: ReactNode, hint?: string) => (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button className="kanban-list__menu" disabled={busy} size={size} variant="ghost">
          {label}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top">
        {radioList(heading, pick, children, hint)}
      </DropdownMenuContent>
    </DropdownMenu>
  )

  const byStatus: Choice = { get: (ticket) => ticket.status, set: (value) => onField('status', value) }
  const byPriority: Choice = { get: (ticket) => ticket.priority, set: (value) => onField('priority', value) }
  const byAssignee: Choice = { get: (ticket) => ticket.assignee, set: (value) => onField('assignee', value) }
  const byRelease: Choice = { get: (ticket) => ticket.release_id, set: (id) => onFacet('release', id) }
  const byProject: Choice = { get: (ticket) => ticket.project_id, set: (id) => onFacet('project', id) }

  const count = `${n} ${n === 1 ? 'ticket' : 'tickets'}`
  const moveHeading = `Move ${count} to`
  const named = (name: string) => <span className="kanban-list__name">{name}</span>
  const option = (lead: ReactNode, text: string) => (
    <span className="kanban-list__option">
      {lead}
      {text}
    </span>
  )
  const menuRows = (opts: Opt[]) =>
    opts.map(({ value, node }) => (
      <DropdownMenuRadioItem key={value} value={value} {...item}>
        {node}
      </DropdownMenuRadioItem>
    ))
  const statusOpts: Opt[] = columns.map((column) => ({
    value: column.id,
    node: option(<StatusGlyph columns={columns} status={column.id} />, column.label),
  }))
  const priorityOpts: Opt[] = [...[...priorities].reverse(), ...strayPriorities].map((priority) => ({
    value: priority,
    node: option(
      <span className="kanban-list__dot" data-tone={dotTone(priorities, priority)} />,
      priorityLabel(priority),
    ),
  }))
  const assigneeOpts: Opt[] = [
    { value: '', node: option(null, 'Unassigned') },
    ...agents.map((agent) => ({ value: agent.id, node: option(<Bot size={16} />, agent.name) })),
    ...strayAssignees.map((id) => ({ value: id, node: option(<User size={16} />, id) })),
  ]
  // Released releases and archived projects are not offered: the board does not move tickets into them from here.
  const releaseOpts: Opt[] = [
    { value: '', node: named('No release') },
    ...releases
      .filter((candidate) => candidate.status !== 'released')
      .map((candidate) => ({ value: candidate.id, node: named(candidate.name) })),
  ]
  const projectOpts: Opt[] = [
    { value: '', node: named('No project') },
    ...projects
      .filter((candidate) => candidate.status === 'active')
      .map((candidate) => ({ value: candidate.id, node: named(candidate.name) })),
  ]

  // Parent candidates: live tickets without a parent of their own, not the selection. Tickets that already have subtasks first.
  const pickedIds = new Set(picked.map((ticket) => ticket.id))
  const pq = parentQuery.trim().toLowerCase()
  const candidates = [...known.values()]
    .filter((t) => !t.deleted_at && !t.parent_id && !pickedIds.has(t.id))
    .filter((t) => `${t.key} ${t.title}`.toLowerCase().includes(pq))
    .sort((a, b) => Number(parents.has(b.id)) - Number(parents.has(a.id)) || compareTickets(a, b, 'key', []))
  const pickParent = (id: string) => {
    onParent(id ? (known.get(id) ?? null) : null)
    setParentOpen(false)
  }
  const byParent: Choice = { get: (ticket) => ticket.parent_id, set: pickParent }
  const parentOpts: Opt[] = [
    { value: '', node: named('No parent') },
    ...candidates.slice(0, MAX_PARENTS).map((candidate) => ({
      value: candidate.id,
      node: (
        <>
          <span className="kanban-list__mono">{candidate.key}</span>
          {named(candidate.title)}
        </>
      ),
    })),
  ]
  const parentNote = `${candidates.length === 0 ? 'No ticket matches. ' : ''}${
    candidates.length > MAX_PARENTS ? `Showing ${MAX_PARENTS} of ${candidates.length}: type to narrow. ` : ''
  }`
  const changeParentOpen = (open: boolean) => {
    setParentOpen(open)
    // Also on open: picking closes the menu without telling it, which would leave the last search behind.
    setParentQuery('')
    if (open) onOpenParents()
  }
  const parentSearch = (
    <SearchField
      aria-label="Find a parent ticket"
      className="kanban-list__find"
      onChange={setParentQuery}
      onKeyDown={(event) => {
        // Typing must not trigger the menu's typeahead, which would steal focus.
        if (event.key !== 'Escape') event.stopPropagation()
        if (event.key === 'Enter' && pq && candidates[0]) pickParent(candidates[0].id)
        if (event.key === 'ArrowDown') {
          event.preventDefault()
          event.currentTarget
            .closest('[role="menu"], [role="dialog"]')
            ?.querySelector<HTMLElement>('[role="menuitemradio"], [role="radio"]')
            ?.focus()
        }
      }}
      placeholder="Find by key or title"
      ref={parentSearchRef}
      value={parentQuery}
    />
  )
  const parentList = (
    <>
      <DropdownMenuLabel>{`Set the parent of ${count}`}</DropdownMenuLabel>
      {parentSearch}
      <div className="kanban-list__scroll">
        <DropdownMenuRadioGroup onValueChange={pickParent} value={shared(byParent.get)}>
          {menuRows(parentOpts)}
        </DropdownMenuRadioGroup>
      </div>
      <p className="kanban-list__hint">
        {parentNote}
        {PARENT_HINT}
      </p>
    </>
  )

  const openPage = (next: Page) => {
    setPage(next)
    setSheetOpen(true)
    if (next === 'parent') {
      setParentQuery('')
      onOpenParents()
    }
  }
  // The sheet has no trigger to hand the focus back to: the More button it was opened from.
  const closeSheet = () => {
    setSheetOpen(false)
    setTimeout(() => moreRef.current?.focus(), 0)
  }
  /** One More menu entry: a page in the sheet on phones, a submenu beside the menu elsewhere. */
  const moreEntry = (
    target: Page,
    label: string,
    content: ReactNode,
    className?: string,
    onOpenChange?: (open: boolean) => void,
  ) =>
    sheet ? (
      <DropdownMenuItem {...item} onSelect={() => openPage(target)}>
        {label}
      </DropdownMenuItem>
    ) : (
      <DropdownMenuSub onOpenChange={onOpenChange}>
        <DropdownMenuSubTrigger {...item}>{label}</DropdownMenuSubTrigger>
        <DropdownMenuSubContent className={className}>{content}</DropdownMenuSubContent>
      </DropdownMenuSub>
    )
  const sheetList = (label: string, pick: Choice, opts: Opt[]) => {
    const current = shared(pick.get)
    return (
      <div aria-label={label} className="kanban-list__sheet-list" role="radiogroup">
        {opts.map(({ value, node }) => (
          // biome-ignore lint/a11y/useSemanticElements: a 48px row with a tick and free content, not a bare radio input
          <button
            aria-checked={value === current}
            className="kanban-list__pick"
            disabled={busy}
            key={value}
            onClick={() => {
              pick.set(value)
              closeSheet()
            }}
            role="radio"
            type="button"
          >
            <Check className="kanban-list__tick" size={16} />
            {node}
          </button>
        ))}
      </div>
    )
  }
  const pages: Record<Page, { heading: string; hint?: string; body: ReactNode }> = {
    priority: { heading: `Set priority of ${count}`, body: sheetList('Priority', byPriority, priorityOpts) },
    assignee: { heading: `Assign ${count} to`, body: sheetList('Assignee', byAssignee, assigneeOpts) },
    release: {
      heading: `${moveHeading} a release`,
      hint: RELEASE_HINT,
      body: sheetList('Release', byRelease, releaseOpts),
    },
    project: {
      heading: `${moveHeading} a project`,
      hint: PROJECT_HINT,
      body: sheetList('Project', byProject, projectOpts),
    },
    parent: {
      heading: `Set the parent of ${count}`,
      hint: PARENT_HINT,
      body: (
        <>
          {parentSearch}
          {sheetList('Parent', byParent, parentOpts)}
          {parentNote ? <p className="kanban-list__hint">{parentNote}</p> : null}
        </>
      ),
    },
  }

  const labelsMenu = (
    <DropdownMenu onOpenChange={(open) => !open && setQuery('')}>
      <DropdownMenuTrigger asChild>
        <Button className="kanban-list__menu" disabled={busy} size={size} variant="ghost">
          Labels…
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="kanban-list__labels"
        onOpenAutoFocus={(event) => {
          event.preventDefault()
          searchRef.current?.focus()
        }}
        side="top"
      >
        <SearchField
          aria-label="Find or create a label"
          className="kanban-list__find"
          onChange={setQuery}
          onKeyDown={(event) => {
            // Typing must not trigger the menu's typeahead, which would steal focus.
            if (event.key !== 'Escape') event.stopPropagation()
            if (event.key === 'Enter' && q) commit()
            if (event.key === 'ArrowDown') {
              event.preventDefault()
              event.currentTarget
                .closest('[role="menu"]')
                ?.querySelector<HTMLElement>('[role="menuitemcheckbox"]')
                ?.focus()
            }
          }}
          placeholder="Find or create a label"
          ref={searchRef}
          value={query}
        />
        <div className="kanban-list__scroll">
          {rows.map(({ label, have }) => (
            <DropdownMenuCheckboxItem
              checked={have === n}
              disabled={busy}
              key={label}
              onCheckedChange={() => onLabel(label)}
              onSelect={(event) => event.preventDefault()}
              {...item}
            >
              <span className="kanban-list__name">{label}</span>
              <span className="kanban-list__coverage">{have === 0 ? '' : have === n ? 'all' : `${have} of ${n}`}</span>
            </DropdownMenuCheckboxItem>
          ))}
          {canCreate ? (
            <DropdownMenuItem
              disabled={busy}
              onSelect={(event) => {
                event.preventDefault()
                commit()
              }}
              {...item}
            >
              <Plus size={16} />
              Create “{q}”
            </DropdownMenuItem>
          ) : null}
        </div>
        <p className="kanban-list__hint">
          Checked labels are on every selected ticket. Checking one adds it to all; unchecking removes it from all.
        </p>
      </DropdownMenuContent>
    </DropdownMenu>
  )

  return (
    <div
      aria-label={`Change ${n} selected ${n === 1 ? 'ticket' : 'tickets'}`}
      className="kanban-list__bar"
      // Bubbles through portals: only the bar's own buttons count, not the menu items.
      onFocus={(event) => {
        if (event.currentTarget.contains(event.target)) lastFocus.current = event.target
      }}
      role="toolbar"
    >
      <span className="kanban-list__summary">{n} selected</span>
      {radioMenu('Status…', moveHeading, byStatus, menuRows(statusOpts))}
      {narrow ? null : radioMenu('Priority…', 'Set priority', byPriority, menuRows(priorityOpts))}
      {labelsMenu}
      {narrow ? (
        // Phones keep the two most used actions in reach; the rest sits one tap away.
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <IconButton
              className="kanban-list__menu kanban-list__more"
              disabled={busy}
              label="More actions"
              ref={moreRef}
              variant="ghost"
            >
              <MoreHorizontal size={16} />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" side="top">
            {moreEntry('priority', 'Priority…', radioList('Set priority', byPriority, menuRows(priorityOpts)))}
            {moreEntry('assignee', 'Assignee…', radioList('Assign to', byAssignee, menuRows(assigneeOpts)))}
            {moreEntry('release', 'Release…', radioList(moveHeading, byRelease, menuRows(releaseOpts), RELEASE_HINT))}
            {moreEntry('project', 'Project…', radioList(moveHeading, byProject, menuRows(projectOpts), PROJECT_HINT))}
            {moreEntry('parent', 'Parent…', parentList, 'kanban-list__labels', changeParentOpen)}
            <DropdownMenuSeparator />
            <DropdownMenuItem className="kanban-list__item kanban-list__delete" data-narrow onSelect={onDelete}>
              Delete…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : (
        <>
          {radioMenu('Assignee…', 'Assign to', byAssignee, menuRows(assigneeOpts))}
          {radioMenu('Release…', moveHeading, byRelease, menuRows(releaseOpts), RELEASE_HINT)}
          {radioMenu('Project…', moveHeading, byProject, menuRows(projectOpts), PROJECT_HINT)}
          <DropdownMenu onOpenChange={changeParentOpen} open={parentOpen}>
            <DropdownMenuTrigger asChild>
              <Button className="kanban-list__menu" disabled={busy} size={size} variant="ghost">
                Parent…
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="start"
              className="kanban-list__labels"
              onOpenAutoFocus={(event) => {
                event.preventDefault()
                parentSearchRef.current?.focus()
              }}
              side="top"
            >
              {parentList}
            </DropdownMenuContent>
          </DropdownMenu>
          <Button className="kanban-list__delete" disabled={busy} onClick={onDelete} size={size} variant="ghost">
            Delete…
          </Button>
        </>
      )}
      <IconButton
        className="kanban-list__clear"
        disabled={busy}
        label="Clear selection"
        onClick={onClear}
        variant="ghost"
      >
        <X size={16} />
      </IconButton>
      {sheet ? (
        <BottomSheet onOpenChange={(open) => !open && closeSheet()} open={sheetOpen}>
          <BottomSheetContent
            className="kanban-list__sheet"
            description={pages[page].hint}
            heading={pages[page].heading}
          >
            <div className="kanban-list__sheet-body">{pages[page].body}</div>
          </BottomSheetContent>
        </BottomSheet>
      ) : null}
    </div>
  )
}
