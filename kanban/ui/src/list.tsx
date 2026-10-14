import {
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
import { ArrowDown, ArrowUp, Bot, ListFilter, MoreHorizontal, Plus, User, X } from 'lucide-react'
import { type ReactNode, useEffect, useId, useMemo, useRef, useState } from 'react'
import { isChangedSinceVisit, type MatchContext } from './filters'
import {
  type AgentProfile,
  api,
  type Column,
  priorityLabel,
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

/** The column headers are gone on narrow panes: the same orderings, one pick. Value = `by:dir`. */
const SORTS: { value: string; label: string }[] = [
  { value: 'priority:1', label: 'Priority · urgent first' },
  { value: 'priority:-1', label: 'Priority · urgent last' },
  { value: 'updated:-1', label: 'Updated · newest first' },
  { value: 'updated:1', label: 'Updated · oldest first' },
  { value: 'key:1', label: 'Key · lowest first' },
  { value: 'key:-1', label: 'Key · highest first' },
]

const MAX_CHIPS = 3
const COLUMN_COUNT = 7

/** The server folds ASCII case only ("Bug" is "bug", "Ação" is not "AÇÃO"), so the menu must agree or a toggle would not stick. */
const fold = (label: string) => label.replace(/[A-Z]/g, (char) => char.toLowerCase())

const MAX_LISTED = 8

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
  }, [picked.length])

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (!next.delete(id)) next.add(id)
      return next
    })

  /** Run every job, hand the saved tickets to the board, report the ones that failed. Returns the failed keys. */
  const run = async (jobs: Job[]): Promise<Set<string>> => {
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
          labels={labels}
          narrow={narrow}
          onClear={() => setSelected(new Set())}
          onDelete={() => void remove()}
          onField={setField}
          onLabel={setLabel}
          picked={picked}
          priorities={priorities}
          tickets={tickets}
        />
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
  narrow: boolean
  busy: boolean
  onField: (field: Field, value: string) => void
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
  narrow,
  busy,
  onField,
  onLabel,
  onDelete,
  onClear,
}: BulkBarProps) {
  const [query, setQuery] = useState('')
  const searchRef = useRef<HTMLInputElement>(null)
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
  const radioList = (heading: string, field: Field, children: ReactNode) => (
    <>
      <DropdownMenuLabel>{heading}</DropdownMenuLabel>
      <DropdownMenuRadioGroup
        onValueChange={(value) => onField(field, value)}
        value={shared((ticket) => ticket[field])}
      >
        {children}
      </DropdownMenuRadioGroup>
    </>
  )

  const radioMenu = (label: string, heading: string, field: Field, children: ReactNode) => (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button className="kanban-list__menu" disabled={busy} size={size} variant="ghost">
          {label}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top">
        {radioList(heading, field, children)}
      </DropdownMenuContent>
    </DropdownMenu>
  )

  const statusItems = columns.map((column) => (
    <DropdownMenuRadioItem key={column.id} value={column.id} {...item}>
      <span className="kanban-list__option">
        <StatusGlyph columns={columns} status={column.id} />
        {column.label}
      </span>
    </DropdownMenuRadioItem>
  ))
  const priorityItems = [...[...priorities].reverse(), ...strayPriorities].map((priority) => (
    <DropdownMenuRadioItem key={priority} value={priority} {...item}>
      <span className="kanban-list__option">
        <span className="kanban-list__dot" data-tone={dotTone(priorities, priority)} />
        {priorityLabel(priority)}
      </span>
    </DropdownMenuRadioItem>
  ))
  const assigneeItems = (
    <>
      <DropdownMenuRadioItem value="" {...item}>
        <span className="kanban-list__option">Unassigned</span>
      </DropdownMenuRadioItem>
      {agents.map((agent) => (
        <DropdownMenuRadioItem key={agent.id} value={agent.id} {...item}>
          <span className="kanban-list__option">
            <Bot size={16} />
            {agent.name}
          </span>
        </DropdownMenuRadioItem>
      ))}
      {strayAssignees.map((id) => (
        <DropdownMenuRadioItem key={id} value={id} {...item}>
          <span className="kanban-list__option">
            <User size={16} />
            {id}
          </span>
        </DropdownMenuRadioItem>
      ))}
    </>
  )
  const statusHeading = `Move ${n} ${n === 1 ? 'ticket' : 'tickets'} to`

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
      {radioMenu('Status…', statusHeading, 'status', statusItems)}
      {narrow ? null : radioMenu('Priority…', 'Set priority', 'priority', priorityItems)}
      {labelsMenu}
      {narrow ? (
        // Phones keep the two most used actions in reach; the rest sits one tap away.
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <IconButton
              className="kanban-list__menu kanban-list__more"
              disabled={busy}
              label="More actions"
              variant="ghost"
            >
              <MoreHorizontal size={16} />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" side="top">
            <DropdownMenuSub>
              <DropdownMenuSubTrigger {...item}>Priority…</DropdownMenuSubTrigger>
              <DropdownMenuSubContent>{radioList('Set priority', 'priority', priorityItems)}</DropdownMenuSubContent>
            </DropdownMenuSub>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger {...item}>Assignee…</DropdownMenuSubTrigger>
              <DropdownMenuSubContent>{radioList('Assign to', 'assignee', assigneeItems)}</DropdownMenuSubContent>
            </DropdownMenuSub>
            <DropdownMenuSeparator />
            <DropdownMenuItem className="kanban-list__item kanban-list__delete" data-narrow onSelect={onDelete}>
              Delete…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : (
        <>
          {radioMenu('Assignee…', 'Assign to', 'assignee', assigneeItems)}
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
    </div>
  )
}
