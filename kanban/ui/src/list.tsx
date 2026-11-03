import { type Host, ShadcnButton, ShadcnCheckbox, useConfirm } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ArrowDownUp, ListFilter, Plus } from 'lucide-react'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { subtaskStats } from './board-state'
import { isChangedSinceVisit, type MatchContext } from './filters'
import { BulkBar, type Field } from './list/bulk-bar'
import { fold } from './list/labels-picker'
import { ListRow, type RowPlan } from './list/list-row'
import { type AgentProfile, api, type Column, type ProjectView, type ReleaseView, type Ticket, type TicketSummary } from './shared'
import { Empty, Notice } from './ui/feedback'
import { ColumnStatusIcon } from './ui/glyphs'
import { IconAction } from './ui/icon-action'
import { OptionMenu } from './ui/option-menu'

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
  canceledStatus: string | null // the column that counts as canceled, out of subtask totals; null = none
  subtasks?: Map<string, { done: number; total: number }> // board-wide progress per parent; absent = counted from `tickets`
  highlightLabels: string[] // the active label filter: those badges go first and look selected
  ctx: MatchContext
  narrow: boolean
  onOpenTicket: (key: string) => void
  onUpdated: (tickets: Ticket[]) => void // server results, the board upserts them
  onError: (message: string) => void
  /** The group header's add: a new ticket in that status. Absent = no add button. */
  onAdd?: (status: string) => void
}

type SortBy = 'key' | 'priority' | 'updated'
type Sort = { by: SortBy; dir: 1 | -1 }
type Job = { key: string; call: () => Promise<Ticket> }

/** The orderings, one pick. Value = `by:dir`. */
const SORTS: { value: string; label: string }[] = [
  { value: 'priority:1', label: 'Priority · urgent first' },
  { value: 'priority:-1', label: 'Priority · urgent last' },
  { value: 'updated:-1', label: 'Updated · newest first' },
  { value: 'updated:1', label: 'Updated · oldest first' },
  { value: 'key:1', label: 'Key · lowest first' },
  { value: 'key:-1', label: 'Key · highest first' },
]

const MAX_LISTED = 8

/** `KAN-1, KAN-2, 3 more`: the keys capped. */
function keyList(keys: string[]): string {
  const listed = keys.length > MAX_LISTED + 1 ? MAX_LISTED : keys.length
  return [...keys.slice(0, listed), ...(listed < keys.length ? [`${keys.length - listed} more`] : [])].join(', ')
}

type Note = { headline: string; detail: string }

/** 0 = most urgent: `priorities` is configured least urgent first. */
function priorityRank(priorities: string[], priority: string): number {
  const index = priorities.indexOf(priority)
  return index < 0 ? priorities.length : priorities.length - 1 - index
}

function compareTickets(a: TicketSummary, b: TicketSummary, by: SortBy, priorities: string[]): number {
  const newestFirst = Date.parse(b.updated_at) - Date.parse(a.updated_at)
  if (by === 'priority')
    return priorityRank(priorities, a.priority) - priorityRank(priorities, b.priority) || newestFirst
  if (by === 'updated') return -newestFirst
  return Number(a.key.split('-').pop()) - Number(b.key.split('-').pop()) || a.key.localeCompare(b.key)
}

/**
 * The List layout: the filtered tickets grouped by status, Linear-style. Each group has a sticky header (status
 * glyph, name, count, add); each ticket is one dense row whose priority, status and assignee edit in place. A
 * selection opens a floating bar that edits every selected ticket at once.
 */
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
  canceledStatus,
  subtasks,
  highlightLabels,
  ctx,
  narrow,
  onOpenTicket,
  onUpdated,
  onError,
  onAdd,
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
  const selectAllRef = useRef<HTMLSpanElement>(null)
  const hadBar = useRef(false)
  const groupId = useId()

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
  const progress = useMemo(
    () => subtasks ?? subtaskStats(tickets, doneStatus, canceledStatus),
    [subtasks, tickets, doneStatus, canceledStatus],
  )
  const releaseNames = useMemo(() => new Map(releases.map((release) => [release.id, release.name])), [releases])

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
    if (hadBar.current && picked.length === 0 && (!active || active === document.body))
      selectAllRef.current?.querySelector<HTMLElement>('[role="checkbox"], button, input')?.focus()
    hadBar.current = picked.length > 0
    if (picked.length === 0) setNote(null)
  }, [picked.length])

  const toggle = useCallback(
    (id: string) =>
      setSelected((prev) => {
        const next = new Set(prev)
        if (!next.delete(id)) next.add(id)
        return next
      }),
    [],
  )

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
  const runRef = useRef(run)
  runRef.current = run

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

  const projectById = useMemo(() => new Map(projects.map((project) => [project.id, project])), [projects])
  /** Where a ticket sits in the plan, in names: the row draws it. */
  const planOf = (ticket: TicketSummary): RowPlan => {
    const parent = ticket.parent_id ? known.get(ticket.parent_id) : undefined
    const project = ticket.project_id ? projectById.get(ticket.project_id) : undefined
    return {
      parentKey: parent?.key ?? null,
      orphan: Boolean(ticket.parent_id) && !parent,
      subs: progress.get(ticket.id),
      release: (ticket.release_id ? releaseNames.get(ticket.release_id) : undefined) ?? null,
      project: project ? { name: project.name, color: project.color } : null,
    }
  }

  /** One ticket edited from its own row (priority, status, assignee). */
  const edit = useCallback(
    (ticket: TicketSummary, patch: Record<string, unknown>) => {
      void runRef.current([update(ticket, patch)])
    },
    // `update` and `run` read fresh state through refs.
    [],
  )

  const remove = async () => {
    const keys = picked.map((ticket) => ticket.key)
    const one = keys.length === 1
    const listed = keys.length > MAX_LISTED + 1 ? MAX_LISTED : keys.length
    const ok = await confirm({
      title: `Delete ${one ? '1 ticket' : `${keys.length} tickets`}?`,
      description: `${one ? 'It disappears' : 'They disappear'} from the board but ${one ? 'stays' : 'stay'} in the board file.`,
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

  const sortValue = `${sort.by}:${sort.dir}`
  const sortLabel = SORTS.find((option) => option.value === sortValue)?.label ?? 'Order'

  if (tickets.length === 0) {
    return (
      <div className="kb-list">
        <Empty description="No ticket matches the filters." icon={ListFilter} title="No tickets" />
        {dialog}
      </div>
    )
  }

  return (
    <div className="kb-list" data-bar={picked.length > 0 || undefined} data-narrow={narrow || undefined}>
      <div className="kb-list__tools">
        <span className="kb-list__select-all" ref={selectAllRef}>
          <ShadcnCheckbox
            aria-label="Select all shown tickets"
            checked={allPicked}
            indeterminate={picked.length > 0 && !allPicked}
            onCheckedChange={() => setSelected(allPicked ? new Set() : new Set(tickets.map((ticket) => ticket.id)))}
          />
        </span>
        <span className="kb-list__total">
          {picked.length > 0 ? `${picked.length} of ${tickets.length} selected` : `${tickets.length} ${tickets.length === 1 ? 'ticket' : 'tickets'}`}
        </span>
        <span className="kb-list__spacer" />
        <OptionMenu
          align="end"
          label="Order by"
          onChange={(value) => {
            const [by, dir] = value.split(':')
            setSort({ by: by as SortBy, dir: dir === '-1' ? -1 : 1 })
          }}
          options={SORTS.map((option) => ({ value: option.value, label: option.label }))}
          trigger={
            <ShadcnButton className="kb-list__order" size="sm" type="button" variant="ghost">
              <ArrowDownUp size={16} />
              {sortLabel}
            </ShadcnButton>
          }
          value={sortValue}
        />
      </div>
      {note ? (
        <Notice actionLabel="Dismiss" detail={note.detail} onAction={() => setNote(null)} title={note.headline} />
      ) : null}
      <div className="kb-list__viewport">
        {groups.map(({ column, rows }, index) => (
          <section aria-labelledby={`${groupId}-${index}`} className="kb-list__group" key={column.id}>
            <header className="kb-list__group-head">
              <ColumnStatusIcon canceledStatus={canceledStatus} columns={columns} doneStatus={doneStatus} status={column.id} />
              <h3 className="kb-list__group-title" id={`${groupId}-${index}`}>
                {column.label}
              </h3>
              <span className="kb-list__group-count">{rows.length}</span>
              <span className="kb-list__spacer" />
              {onAdd ? (
                <IconAction label={`New ticket in ${column.label}`} onClick={() => onAdd(column.id)} size="icon-xs">
                  <Plus size={16} />
                </IconAction>
              ) : null}
            </header>
            <ul aria-labelledby={`${groupId}-${index}`} className="kb-list__rows">
              {rows.map((ticket) => (
                <ListRow
                  agents={agents}
                  busy={busy}
                  canceledStatus={canceledStatus}
                  changed={isChangedSinceVisit(ticket, ctx)}
                  columns={columns}
                  doneStatus={doneStatus}
                  highlight={highlightLabels}
                  key={ticket.id}
                  now={ctx.now}
                  onEdit={edit}
                  onOpen={onOpenTicket}
                  onToggle={toggle}
                  plan={planOf(ticket)}
                  priorities={priorities}
                  selected={selected.has(ticket.id)}
                  ticket={ticket}
                />
              ))}
            </ul>
          </section>
        ))}
      </div>
      {picked.length > 0 ? (
        <BulkBar
          agents={agents}
          busy={busy}
          canceledStatus={canceledStatus}
          columns={columns}
          doneStatus={doneStatus}
          known={known}
          labels={labels}
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
      {dialog}
    </div>
  )
}
