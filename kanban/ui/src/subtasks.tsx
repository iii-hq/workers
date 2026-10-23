import {
  Button,
  IconButton,
  Input,
  SegmentedControl,
  Selector,
  StatusPanel,
  useConfirm,
  type Host,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ChevronLeft, ChevronRight, CornerLeftUp, Link2, TriangleAlert, Unlink } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import {
  archivedProjectIn,
  attachOrder,
  attachReasons,
  openChildren,
  parentOptions,
  plural,
  prefillHint,
  progressPercent,
  projectOptions,
  releaseGroups,
  siblings,
  subtaskDefaults,
  doneWarning,
} from './subtask-rules'
import {
  PriorityChip,
  StatusGlyph,
  api,
  noteOwnWrite,
  priorityLabel,
  statusLabel,
  type AgentProfile,
  type Brief,
  type Column,
  type ProjectView,
  type ReleaseView,
  type Ticket,
  type TicketSummary,
} from './shared'

/** A ticket key that opens the ticket, in activity sentences. */
export function TicketRef({ ticketKey, onOpen }: { ticketKey: string; onOpen?: (key: string) => void }) {
  return onOpen ? (
    <button className="kanban-ref" onClick={() => onOpen(ticketKey)} type="button">
      {ticketKey}
    </button>
  ) : (
    <span className="kanban-ref">{ticketKey}</span>
  )
}

export function ProgressBar({ done, total, className }: { done: number; total: number; className?: string }) {
  return (
    <span
      aria-label={`${done} of ${total} subtasks done`}
      className={className ? `kanban-bar ${className}` : 'kanban-bar'}
      role="img"
    >
      <span style={{ width: `${progressPercent({ done, total })}%` }} />
    </span>
  )
}

/* ── project, release and parent as selectors ────────────────────────── */

type FacetProps = {
  /** Project or release id; for the parent, its key. */
  value: string | null
  onChange: (next: string | null) => void
  /** The list is about to be read: the caller refreshes what it knows. */
  onOpen?: () => void
  disabled?: boolean
  id?: string
  'aria-describedby'?: string
}

export function ProjectSelector({
  projects,
  value,
  onChange,
  onOpen,
  disabled,
  id,
  'aria-describedby': describedBy,
}: FacetProps & { projects: ProjectView[] | null }) {
  const options = useMemo(() => (projects ? projectOptions(projects, value) : []), [projects, value])
  return (
    <Selector
      allowEmpty
      aria-describedby={describedBy}
      aria-label="Project"
      className="kanban-selector"
      disabled={disabled || !projects}
      emptyLabel="No project"
      id={id}
      onChange={onChange}
      onClear={() => onChange(null)}
      onOpenChange={(open) => {
        if (open) onOpen?.()
      }}
      options={options}
      placeholder={projects ? 'No project' : 'Loading…'}
      searchPlaceholder="Find a project…"
      value={value ?? undefined}
    />
  )
}

export function ReleaseSelector({
  releases,
  value,
  onChange,
  onOpen,
  disabled,
  id,
  'aria-describedby': describedBy,
}: FacetProps & { releases: ReleaseView[] | null }) {
  const groups = useMemo(() => (releases ? releaseGroups(releases, value) : []), [releases, value])
  return (
    <Selector
      allowEmpty
      aria-describedby={describedBy}
      aria-label="Release"
      className="kanban-selector"
      disabled={disabled || !releases}
      emptyLabel="No release"
      groups={groups}
      id={id}
      onChange={onChange}
      onClear={() => onChange(null)}
      onOpenChange={(open) => {
        if (open) onOpen?.()
      }}
      placeholder={releases ? 'No release' : 'Loading…'}
      searchPlaceholder="Find a release…"
      value={value ?? undefined}
    />
  )
}

export function ParentSelector({
  tickets,
  selfId,
  current,
  value,
  onChange,
  onOpen,
  disabled,
  id,
  'aria-describedby': describedBy,
}: FacetProps & { tickets: TicketSummary[] | null; selfId: string | null; current: Brief | null }) {
  const options = useMemo(() => (tickets ? parentOptions(tickets, selfId, current) : []), [tickets, selfId, current])
  return (
    <Selector
      allowEmpty
      aria-describedby={describedBy}
      aria-label="Parent"
      className="kanban-selector"
      disabled={disabled || !tickets}
      emptyLabel="None"
      id={id}
      onChange={onChange}
      onClear={() => onChange(null)}
      onOpenChange={(open) => {
        if (open) onOpen?.()
      }}
      options={options}
      placeholder={tickets ? 'None' : 'Loading…'}
      searchPlaceholder="Find a ticket by key or title…"
      value={value ?? undefined}
    />
  )
}

/* ── the parent strip of a subtask ───────────────────────────────────── */

type ParentStripProps = {
  parent: Brief
  /** The parent as the worker returns it: its progress and its other subtasks. Null until fetched. */
  parentView: Ticket | null
  ticketId: string
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  canOpen: boolean
  onOpen: (key: string) => void
}

export function ParentStrip({
  parent,
  parentView,
  ticketId,
  columns,
  doneStatus,
  canceledStatus,
  canOpen,
  onOpen,
}: ParentStripProps) {
  const { previous, next } = parentView ? siblings(parentView.children, ticketId) : { previous: null, next: null }
  const status = statusLabel(columns, parent.status)
  const deleted = parent.deleted_at ? ' (deleted)' : ''
  const progress = parentView?.progress
  return (
    <div aria-label="Parent ticket" className="kanban-parentbar" role="group">
      <button
        aria-label={`Subtask of ${parent.key}${deleted}, ${parent.title}. ${status}${
          progress ? `, ${progress.done} of ${progress.total} subtasks done` : ''
        }. Open the parent`}
        className="kanban-parentbar__link"
        disabled={!canOpen}
        onClick={() => onOpen(parent.key)}
        type="button"
      >
        <CornerLeftUp size={16} />
        <span className="kanban-parentbar__of">
          <span className="kanban-parentbar__faint">Subtask of</span> <span className="kanban-mono">{parent.key}</span>
          {deleted} ·
        </span>
        <span className="kanban-parentbar__title" title={parent.title}>
          {parent.title}
        </span>
        <span className="kanban-parentbar__status">
          <span aria-hidden="true" className="kanban-parentbar__faint">
            ·
          </span>
          <StatusGlyph
            canceledStatus={canceledStatus}
            columns={columns}
            doneStatus={doneStatus}
            status={parent.status}
          />
          {status}
        </span>
        {progress ? (
          <span className="kanban-parentbar__progress">
            <span className="kanban-mono">
              {progress.done} of {progress.total} done
            </span>
            <ProgressBar done={progress.done} total={progress.total} />
          </span>
        ) : null}
      </button>
      {previous || next ? (
        <div className="kanban-parentbar__nav">
          {previous ? (
            <Button
              aria-label={`Previous subtask ${previous.key}`}
              className="kanban-touch"
              disabled={!canOpen}
              onClick={() => onOpen(previous.key)}
              size="sm"
              title={`Previous subtask ${previous.key}`}
              variant="ghost"
            >
              <ChevronLeft size={16} />
              <span className="kanban-mono">{previous.key}</span>
            </Button>
          ) : null}
          {next ? (
            <Button
              aria-label={`Next subtask ${next.key}`}
              className="kanban-touch"
              disabled={!canOpen}
              onClick={() => onOpen(next.key)}
              size="sm"
              title={`Next subtask ${next.key}`}
              variant="ghost"
            >
              <span className="kanban-mono">{next.key}</span>
              <ChevronRight size={16} />
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

/* ── writes ──────────────────────────────────────────────────────────── */

/** What a screen action is: it runs, shows its error, and re-reads the ticket either way. True when it worked. */
export type Act = (work: () => Promise<void>) => Promise<boolean>

/** One write per subtask, in order; the one that fails is named, the ones before it stay done. */
async function eachChild(children: Brief[], work: (child: Brief) => Promise<unknown>): Promise<void> {
  for (const child of children) {
    try {
      await work(child)
    } catch (caught) {
      throw new Error(`${child.key}: ${errorMessage(caught)}`)
    }
  }
}

const listed = (children: Brief[], max = 8): string[] => [
  ...children.slice(0, max).map((child) => `${child.key} — ${child.title}`),
  ...(children.length > max ? [`…and ${children.length - max} more`] : []),
]

/* ── Done with open subtasks ─────────────────────────────────────────── */

type DoneWarningProps = {
  host: Host
  ticket: Ticket
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  busy: boolean
  onAct: Act
}

export function DoneWarning({ host, ticket, columns, doneStatus, canceledStatus, busy, onAct }: DoneWarningProps) {
  const { confirm, dialog } = useConfirm()
  const warning = doneWarning(ticket, doneStatus, canceledStatus, columns)
  if (!warning) return null
  const { open, back } = warning
  const doneLabel = statusLabel(columns, doneStatus)
  const count = open.length
  const total = ticket.progress.total

  const moveBack = () =>
    back &&
    onAct(async () => {
      noteOwnWrite(await api.move(host.iii, { id: ticket.key, status: back.id, actor: 'user' }))
    })

  const closeAll = async () => {
    const yes = await confirm({
      title: `Close ${plural(count, 'subtask')}?`,
      description: `They move to ${doneLabel}. ${ticket.key} stays where it is.`,
      details: listed(open),
      confirmLabel: `Close ${plural(count, 'subtask')}`,
    })
    if (!yes) return
    await onAct(() =>
      eachChild(open, async (child) => {
        noteOwnWrite(await api.move(host.iii, { id: child.key, status: doneStatus, actor: 'user' }))
      }),
    )
  }

  const detachAll = async () => {
    const yes = await confirm({
      title: `Detach ${plural(count, 'subtask')}?`,
      description: `They stop being subtasks of ${ticket.key} and stay on the board as tickets of their own.`,
      details: listed(open),
      confirmLabel: `Detach ${plural(count, 'subtask')}`,
    })
    if (!yes) return
    await onAct(() =>
      eachChild(open, async (child) => {
        noteOwnWrite(await api.update(host.iii, { id: child.key, parent: null, actor: 'user' }))
      }),
    )
  }

  return (
    <>
      <StatusPanel
        className="kanban-record__banner"
        // The three actions sit under the sentence: in the action slot they would squeeze it.
        detail={
          <>
            {`${ticket.key} is in ${doneLabel} while ${count} of its ${total} subtasks are not. Progress shows ${ticket.progress.done} of ${total} because only the ${doneLabel} column counts.`}
            <div className="kanban-warn-actions">
              {back ? (
                <Button className="kanban-touch" disabled={busy} onClick={() => void moveBack()} variant="pill">
                  Move back to {back.label}
                </Button>
              ) : null}
              <Button className="kanban-touch" disabled={busy} onClick={() => void closeAll()} variant="pill">
                Close {plural(count, 'subtask')}…
              </Button>
              <Button className="kanban-touch" disabled={busy} onClick={() => void detachAll()} variant="pill">
                Detach {plural(count, 'subtask')}…
              </Button>
            </div>
          </>
        }
        headline={`${doneLabel} with ${count} open ${count === 1 ? 'subtask' : 'subtasks'}`}
        icon={<TriangleAlert size={16} />}
        role="status"
        variant="warn"
      />
      {dialog}
    </>
  )
}

/* ── the subtasks section ────────────────────────────────────────────── */

type SubtasksProps = {
  host: Host
  ticket: Ticket
  columns: Column[]
  priorities: string[]
  agents: AgentProfile[]
  doneStatus: string
  canceledStatus: string | null
  /** The board's live tickets and projects; null until the board is read. */
  tickets: TicketSummary[] | null
  projects: ProjectView[]
  releases: ReleaseView[]
  deleted: boolean
  busy: boolean
  canOpen: boolean
  onOpen: (key: string) => void
  /** The pickers are about to be read: refresh the board snapshot. */
  onPick: () => void
  onAct: Act
}

const LIMIT = 8

/** The projects and releases as the worker has them now: the board snapshot on screen can be older than a write made elsewhere. */
async function readPlan(host: Host, projects: ProjectView[], releases: ReleaseView[]) {
  try {
    const [read, ready] = await Promise.all([api.projects(host.iii), api.releases(host.iii)])
    return { projects: read.projects, releases: ready.releases }
  } catch {
    return { projects, releases }
  }
}

export function SubtasksSection({
  host,
  ticket,
  columns,
  priorities,
  agents,
  doneStatus,
  canceledStatus,
  tickets,
  projects,
  releases,
  deleted,
  busy,
  canOpen,
  onOpen,
  onPick,
  onAct,
}: SubtasksProps) {
  const titleId = useId()
  const [view, setView] = useState<'open' | 'all'>('open')
  const [expanded, setExpanded] = useState(false)
  const [title, setTitle] = useState('')
  // What the last create left out of the prefill, so it is not a surprise.
  const [notice, setNotice] = useState<string | null>(null)
  const children = ticket.children
  const open = openChildren(children, doneStatus, canceledStatus)
  const shown = view === 'open' ? open : children
  const visible = expanded ? shown : shown.slice(0, LIMIT)
  const { done, total } = ticket.progress
  const firstColumn = columns[0]
  // What a new subtask takes from this ticket; the hint under the field reads the same object.
  const prefill = subtaskDefaults(ticket, projects, releases, firstColumn?.id)

  const pickable = useMemo(() => {
    if (!tickets) return []
    const reasons = attachReasons(tickets, ticket.id)
    return attachOrder(tickets, reasons).map((candidate) => {
      const reason = reasons.get(candidate.id)
      return {
        value: candidate.key,
        label: candidate.title,
        description: reason
          ? `${candidate.key} · ${reason}`
          : `${candidate.key} · ${statusLabel(columns, candidate.status)} · ${priorityLabel(candidate.priority)}`,
        disabled: Boolean(reason),
      }
    })
  }, [tickets, ticket.id, columns])

  const detach = (child: Brief) =>
    onAct(async () => {
      noteOwnWrite(await api.update(host.iii, { id: child.key, parent: null, actor: 'user' }))
    })

  const attach = (key: string) =>
    onAct(async () => {
      noteOwnWrite(await api.update(host.iii, { id: key, parent: ticket.key, actor: 'user' }))
    })

  const create = async () => {
    const next = title.trim()
    if (!next || busy) return
    setNotice(null)
    const left: { project?: string } = {}
    const ok = await onAct(async () => {
      // The prefill is built from what the worker has now, not from the snapshot: the parent's project may
      // have been archived in another pane, and that refuses the create.
      const hasPlan = Boolean(ticket.project_id || ticket.release_id)
      const plan = hasPlan ? await readPlan(host, projects, releases) : { projects, releases }
      const fields = subtaskDefaults(ticket, plan.projects, plan.releases, firstColumn?.id)
      const send = async (body: Record<string, unknown>) =>
        noteOwnWrite(await api.create(host.iii, { title: next, ...body, actor: 'user' }))
      if ('project' in prefill && !('project' in fields)) {
        const archived = plan.projects.find((project) => project.id === ticket.project_id)
        left.project = archived?.name ?? ticket.project_id ?? ''
      }
      try {
        await send(fields)
      } catch (caught) {
        // Archived between the read and the write: file it without the project instead of failing.
        const archived = archivedProjectIn(errorMessage(caught))
        if (!archived || !('project' in fields)) throw caught
        const { project: _archived, ...without } = fields
        await send(without)
        left.project = archived
      }
    })
    // What was typed while the request ran stays.
    if (ok) {
      setTitle((now) => (now.trim() === next ? '' : now))
      if (left.project !== undefined) {
        setNotice(`Created without the archived project ${left.project}.`)
        // The snapshot is behind the worker: read it again so the hint and the next create agree.
        onPick()
      }
    }
  }

  const agentName = (id: string) => agents.find((agent) => agent.id === id)?.name ?? id

  return (
    <section aria-labelledby={titleId} className="kanban-subtasks">
      <div className="kanban-section__head">
        <h2 className="kanban-section__title" id={titleId}>
          Subtasks
        </h2>
        {/* Every subtask canceled: no progress (`total` leaves them out), but the toggle still reaches them. */}
        {children.length > 0 ? (
          <>
            {total > 0 ? (
              <>
                <span className="kanban-mono">
                  {done} of {total} done
                </span>
                <ProgressBar className="kanban-bar--head" done={done} total={total} />
              </>
            ) : null}
            <SegmentedControl
              aria-label="Show subtasks"
              className="kanban-activity-filter"
              itemClassName="kanban-touch"
              onChange={(next) => {
                setView(next)
                setExpanded(false)
              }}
              options={[
                { value: 'open', label: `Open ${open.length}` },
                { value: 'all', label: `All ${children.length}` },
              ]}
              value={view}
              variant="radio"
            />
          </>
        ) : null}
      </div>

      {children.length === 0 ? (
        <p className="kanban-muted">No subtasks yet.</p>
      ) : shown.length === 0 ? (
        <p className="kanban-muted">
          {canceledStatus ? 'Every subtask is done or canceled.' : 'Every subtask is done.'}
        </p>
      ) : (
        <ul className="kanban-subs">
          {visible.map((child) => (
            <li className="kanban-subrow" key={child.id}>
              <button
                className="kanban-sub"
                data-assignee={child.assignee ? 'true' : undefined}
                disabled={!canOpen}
                onClick={() => onOpen(child.key)}
                type="button"
              >
                <StatusGlyph
                  canceledStatus={canceledStatus}
                  columns={columns}
                  doneStatus={doneStatus}
                  status={child.status}
                />
                <span className="kanban-mono">{child.key}</span>
                <span className="kanban-sub__title">{child.title}</span>
                <PriorityChip priorities={priorities} priority={child.priority} />
                {child.assignee ? <span className="kanban-sub__who">{agentName(child.assignee)}</span> : null}
              </button>
              <IconButton
                className="kanban-touch kanban-sub__x"
                disabled={deleted || busy}
                label={`Detach ${child.key} from ${ticket.key}`}
                onClick={() => void detach(child)}
                tooltip={`Detach from ${ticket.key}`}
                variant="ghost"
              >
                <Unlink size={16} />
              </IconButton>
            </li>
          ))}
        </ul>
      )}
      {shown.length > LIMIT && !expanded ? (
        <Button className="kanban-touch kanban-subs__more" onClick={() => setExpanded(true)} size="sm" variant="ghost">
          Show {shown.length - LIMIT} more
        </Button>
      ) : null}

      {deleted ? null : (
        <div className="kanban-subadd">
          <div className="kanban-subadd__row">
            <Input
              aria-label="New subtask title"
              className="kanban-subadd__title"
              onChange={(value) => {
                setTitle(value)
                setNotice(null)
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                  event.preventDefault()
                  void create()
                }
              }}
              placeholder="Add a subtask — press Enter to create it"
              value={title}
            />
            <Selector
              aria-label="Add an existing ticket as a subtask"
              className="kanban-selector kanban-subadd__pick"
              contentClassName="kanban-pick-menu"
              disabled={busy}
              emptyMessage="No ticket matches"
              loading={!tickets}
              onChange={(key) => void attach(key)}
              onOpenChange={(isOpen) => {
                if (isOpen) onPick()
              }}
              options={pickable}
              placeholder="Add existing ticket…"
              searchPlaceholder="Find a ticket by key or title"
              triggerIcon={<Link2 size={16} />}
              value={undefined}
            />
          </div>
          <p className="kanban-muted kanban-subadd__hint">
            {prefillHint(firstColumn?.label ?? 'the first column', 'project' in prefill, 'release' in prefill)}{' '}
            One level only: a ticket with subtasks can't become a subtask.
          </p>
          {notice ? (
            <p className="kanban-muted kanban-subadd__hint" role="status">
              {notice}
            </p>
          ) : null}
        </div>
      )}
    </section>
  )
}
