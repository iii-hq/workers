import {
  type Host,
  ShadcnButton,
  ShadcnInput,
  ShadcnProgress,
  ShadcnTabs,
  ShadcnTabsList,
  ShadcnTabsTrigger,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ChevronLeft, ChevronRight, CornerLeftUp, Link2, Plus, TriangleAlert, Unlink } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { statusStep } from './board-state'
import {
  type AgentProfile,
  api,
  type Brief,
  type Column,
  noteOwnWrite,
  priorityLabel,
  type ProjectView,
  type ReleaseView,
  statusLabel,
  type Ticket,
  type TicketSummary,
} from './shared'
import {
  archivedProjectIn,
  attachOrder,
  attachReasons,
  doneWarning,
  openChildren,
  plural,
  prefillHint,
  progressPercent,
  siblings,
  subtaskDefaults,
} from './subtask-rules'
import { AssigneeAvatar } from './ui/avatar'
import { PriorityIcon, StatusIcon } from './ui/glyphs'
import { IconAction } from './ui/icon-action'
import { SearchPicker } from './ui/search-picker'

/** A ticket key that opens the ticket, in activity sentences. */
export function TicketRef({ ticketKey, onOpen }: { ticketKey: string; onOpen?: (key: string) => void }) {
  return onOpen ? (
    <button className="kb-ref" onClick={() => onOpen(ticketKey)} type="button">
      {ticketKey}
    </button>
  ) : (
    <span className="kb-ref">{ticketKey}</span>
  )
}

/** Subtask progress as a thin shadcn bar. */
function Progress({ done, total, className }: { done: number; total: number; className?: string }) {
  return (
    <ShadcnProgress
      aria-label={`${done} of ${total} subtasks done`}
      className={className ? `kb-progress ${className}` : 'kb-progress'}
      value={progressPercent({ done, total })}
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

/** "Subtask of KAN-1 · title · status · 2 of 5 done", then the previous and next sibling. */
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
    <div aria-label="Parent ticket" className="kb-parentbar" role="group">
      <button
        aria-label={`Subtask of ${parent.key}${deleted}, ${parent.title}. ${status}${
          progress ? `, ${progress.done} of ${progress.total} subtasks done` : ''
        }. Open the parent`}
        className="kb-parentbar__link"
        disabled={!canOpen}
        onClick={() => onOpen(parent.key)}
        type="button"
      >
        <CornerLeftUp size={16} />
        <span className="kb-parentbar__of">Subtask of</span>
        <span className="kb-mono">{parent.key}</span>
        {deleted ? <span className="kb-parentbar__of">{deleted}</span> : null}
        <span className="kb-parentbar__title" title={parent.title}>
          {parent.title}
        </span>
        <span className="kb-parentbar__status">
          <StatusIcon step={statusStep(columns, parent.status, doneStatus, canceledStatus)} />
          {status}
        </span>
        {progress && progress.total > 0 ? (
          <span className="kb-parentbar__progress">
            <span className="kb-parentbar__count">
              {progress.done}/{progress.total}
            </span>
            <Progress done={progress.done} total={progress.total} />
          </span>
        ) : null}
      </button>
      {previous || next ? (
        <div className="kb-parentbar__nav">
          <IconAction
            disabled={!canOpen || !previous}
            label={previous ? `Previous subtask ${previous.key}` : 'No previous subtask'}
            onClick={() => previous && onOpen(previous.key)}
            size="icon-xs"
          >
            <ChevronLeft size={16} />
          </IconAction>
          <IconAction
            disabled={!canOpen || !next}
            label={next ? `Next subtask ${next.key}` : 'No next subtask'}
            onClick={() => next && onOpen(next.key)}
            size="icon-xs"
          >
            <ChevronRight size={16} />
          </IconAction>
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

/** A warn banner while a parent sits in done with subtasks open: the sentence, then the three ways out. */
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
      description: `They move to ${doneLabel}. This ticket stays where it is.`,
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
      description: 'They stop being subtasks of this ticket and stay on the board as tickets of their own.',
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
      <div className="kb-banner" data-tone="warn" role="status">
        <TriangleAlert size={16} />
        <div className="kb-banner__text">
          <p className="kb-banner__title">
            {doneLabel} with {count} open {count === 1 ? 'subtask' : 'subtasks'}
          </p>
          <p className="kb-banner__detail">
            {`${ticket.key} is in ${doneLabel} while ${count} of its ${total} subtasks are not. Progress shows ${ticket.progress.done} of ${total} because only the ${doneLabel} column counts.`}
          </p>
          <div className="kb-banner__actions">
            {back ? (
              <ShadcnButton disabled={busy} onClick={() => void moveBack()} size="sm" type="button" variant="outline">
                Move back to {back.label}
              </ShadcnButton>
            ) : null}
            <ShadcnButton disabled={busy} onClick={() => void closeAll()} size="sm" type="button" variant="outline">
              Close {plural(count, 'subtask')}…
            </ShadcnButton>
            <ShadcnButton disabled={busy} onClick={() => void detachAll()} size="sm" type="button" variant="outline">
              Detach {plural(count, 'subtask')}…
            </ShadcnButton>
          </div>
        </div>
      </div>
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
        label: `${candidate.key}  ${candidate.title}`,
        description: reason ?? `${statusLabel(columns, candidate.status)} · ${priorityLabel(candidate.priority)}`,
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

  return (
    <section aria-labelledby={titleId} className="kb-section">
      <header className="kb-section__head">
        <h2 className="kb-section__title" id={titleId}>
          Subtasks
        </h2>
        {/* Every subtask canceled: no progress (`total` leaves them out), but the toggle still reaches them. */}
        {children.length > 0 && total > 0 ? (
          <span className="kb-section__progress">
            <span className="kb-section__count">
              {done}/{total}
            </span>
            <Progress done={done} total={total} />
          </span>
        ) : null}
        <span className="kb-section__spacer" />
        {children.length > 0 ? (
          <ShadcnTabs
            className="kb-section__tabs"
            onValueChange={(next) => {
              setView(next === 'all' ? 'all' : 'open')
              setExpanded(false)
            }}
            value={view}
          >
            <ShadcnTabsList aria-label="Show subtasks">
              <ShadcnTabsTrigger value="open">
                Open <span className="kb-section__tabcount">{open.length}</span>
              </ShadcnTabsTrigger>
              <ShadcnTabsTrigger value="all">
                All <span className="kb-section__tabcount">{children.length}</span>
              </ShadcnTabsTrigger>
            </ShadcnTabsList>
          </ShadcnTabs>
        ) : null}
      </header>

      {children.length === 0 ? null : shown.length === 0 ? (
        <p className="kb-muted">{canceledStatus ? 'Every subtask is done or canceled.' : 'Every subtask is done.'}</p>
      ) : (
        <ul className="kb-rows">
          {visible.map((child) => (
            <li className="kb-row" key={child.id}>
              <button
                className="kb-row__main"
                disabled={!canOpen}
                onClick={() => onOpen(child.key)}
                type="button"
              >
                <StatusIcon step={statusStep(columns, child.status, doneStatus, canceledStatus)} />
                <span className="kb-mono kb-row__key">{child.key}</span>
                <span className="kb-row__title">{child.title}</span>
                <span className="kb-row__meta">
                  <PriorityIcon priorities={priorities} priority={child.priority} />
                  <AssigneeAvatar agents={agents} id={child.assignee} />
                </span>
              </button>
              <span className="kb-row__action">
                <IconAction
                  disabled={deleted || busy}
                  label={`Detach ${child.key} from ${ticket.key}`}
                  onClick={() => void detach(child)}
                  size="icon-xs"
                >
                  <Unlink size={16} />
                </IconAction>
              </span>
            </li>
          ))}
        </ul>
      )}
      {shown.length > LIMIT && !expanded ? (
        <ShadcnButton className="kb-rows__more" onClick={() => setExpanded(true)} size="sm" type="button" variant="ghost">
          Show {shown.length - LIMIT} more
        </ShadcnButton>
      ) : null}

      {deleted ? null : (
        <div className="kb-subadd">
          <div className="kb-subadd__row">
            <div className="kb-subadd__field">
              <Plus aria-hidden="true" size={16} />
              <ShadcnInput
                aria-label="New subtask title"
                onChange={(event) => {
                  setTitle(event.target.value)
                  setNotice(null)
                }}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                    event.preventDefault()
                    void create()
                  }
                }}
                placeholder="Add a subtask — press Enter"
                value={title}
              />
            </div>
            <SearchPicker
              align="end"
              disabled={busy}
              empty="No ticket can be attached."
              label="Add an existing ticket"
              onChange={(key) => void attach(key)}
              onOpenChange={(isOpen) => {
                if (isOpen) onPick()
              }}
              options={pickable}
              searchPlaceholder="Find a ticket by key or title…"
              trigger={
                <ShadcnButton size="sm" type="button" variant="outline">
                  <Link2 size={16} />
                  {tickets ? 'Add existing' : 'Loading…'}
                </ShadcnButton>
              }
              value={null}
            />
          </div>
          <p className="kb-muted kb-subadd__hint">
            {prefillHint(firstColumn?.label ?? 'the first column', 'project' in prefill, 'release' in prefill)}{' '}
            One level only: a ticket with subtasks can't become a subtask.
          </p>
          {notice ? (
            <p className="kb-muted kb-subadd__hint" role="status">
              {notice}
            </p>
          ) : null}
        </div>
      )}
    </section>
  )
}
