import {
  type Host,
  type LiveAnnouncement,
  LiveRegion,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  ShadcnBadge,
  ShadcnButton,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuItem,
  ShadcnDropdownMenuTrigger,
  ShadcnSkeleton,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import {
  Archive,
  ArrowUpRight,
  ChevronRight,
  Columns3,
  FolderOpen,
  ListFilter,
  Milestone,
  MoreHorizontal,
  Pencil,
  Plus,
  RotateCcw,
  Tag,
} from 'lucide-react'
import { type ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { Callout, Crumbs, FolderGlyph, LeadPicker, Prop, Section, TonePicker } from './plan/parts'
import { TicketRows } from './plan/ticket-rows'
import {
  ColumnTiles,
  ConvertDialog,
  NewProjectDialog,
  openPane,
  ProgressBar,
  StatusBadge,
  takePanelRequest,
  useAgents,
  useInlineEdit,
  usePlanBoard,
} from './plan-dialogs'
import { byProgress, groupProjects, plural, resolveEntity, topLabels } from './plan-rules'
import {
  type AgentProfile,
  announcePlanChange,
  api,
  BOARD_PAGE_ID,
  type BoardData,
  type ProjectView,
  RELEASES_PAGE_ID,
  type TicketSummary,
} from './shared'
import { AssigneeAvatar, assigneeName } from './ui/avatar'
import { Empty, Notice } from './ui/feedback'
import { IconAction } from './ui/icon-action'
import { LabelBadge } from './ui/label-badge'

/**
 * The hub's Overview (src/board.tsx): every project as a list, or — once one is picked in the header — that
 * project's progress, tickets, releases and details. The board data and the dialogs belong to the hub, so the
 * Board and Tree views show the same reads and nothing is fetched twice.
 */
export function ProjectsOverview({
  host,
  data,
  agents,
  error,
  loading,
  reload,
  project,
  onSelect,
  onOpenBoard,
  onOpenTicket,
  onNewProject,
  onConvert,
  announce,
}: {
  host: Host
  data: BoardData | null
  agents: AgentProfile[]
  error: string | null
  loading: boolean
  reload: (quiet?: boolean) => Promise<void>
  /** The project picked in the header; null = the list of every project. */
  project: ProjectView | null
  onSelect: (id: string | null) => void
  /** The board of a project (`''` = the tickets in no project), optionally as the list. */
  onOpenBoard: (project: string, layout?: 'list') => void
  onOpenTicket: (key: string) => void
  onNewProject: () => void
  onConvert: () => void
  announce: (text: string) => void
}) {
  // A failed project action, with the project it failed on: it is not shown over another project.
  const [actionError, setActionError] = useState<{ project: string; message: string } | null>(null)
  const projects = useMemo(() => data?.projects ?? [], [data])
  const groups = useMemo(() => groupProjects(projects), [projects])
  const tickets = useMemo(() => (data?.columns ?? []).flatMap((column) => column.tickets), [data])
  const canOpenPane = Boolean(host.panels?.open)
  const withoutProject = tickets.filter((ticket) => !ticket.project_id).length

  let body: ReactNode
  if (!data) {
    body = error ? (
      <Notice actionLabel="Retry" detail={error} onAction={() => void reload()} title="The projects could not be read" />
    ) : (
      <div aria-busy={loading} aria-label="Loading projects" className="kb-plan-list">
        {[0, 1, 2].map((index) => (
          <ShadcnSkeleton className="kb-plan-skeleton kb-plan-skeleton--row" key={index} />
        ))}
      </div>
    )
  } else if (project) {
    body = (
      <>
        <Crumbs name={project.name} onBack={() => onSelect(null)} root="Projects" />
        {actionError?.project === project.id ? (
          <Notice
            actionLabel="Dismiss"
            detail={actionError.message}
            onAction={() => setActionError(null)}
            title="That change was not saved"
          />
        ) : null}
        <ProjectDetail
          agents={agents}
          announce={announce}
          canOpenPane={canOpenPane}
          data={data}
          host={host}
          key={project.id}
          onError={(message) =>
            setActionError((prev) =>
              message !== null ? { project: project.id, message } : prev?.project === project.id ? null : prev,
            )
          }
          onOpenBoard={onOpenBoard}
          onOpenTicket={onOpenTicket}
          project={project}
          reload={reload}
          tickets={tickets}
        />
      </>
    )
  } else if (projects.length === 0) {
    body = (
      <Empty
        action={
          <div className="kb-plan-empty-actions">
            <ShadcnButton onClick={onConvert} size="sm" type="button" variant="outline">
              <Tag size={16} />
              Create from a label
            </ShadcnButton>
            <ShadcnButton onClick={onNewProject} size="sm" type="button">
              <Plus size={16} />
              New project
            </ShadcnButton>
          </div>
        }
        description="Group tickets by what they belong to. You can start from a label like iii-cloud."
        icon={FolderOpen}
        title="No projects yet"
      />
    )
  } else {
    body = (
      <div className="kb-plan-list">
        {(
          [
            ['Active', groups.active],
            ['Archived', groups.archived],
          ] as const
        ).map(([label, items]) =>
          items.length === 0 ? null : (
            <section aria-label={label} className="kb-plan-group" key={label}>
              <header className="kb-plan-group__head">
                <span className="kb-plan-group__title">{label}</span>
                <span className="kb-plan-group__count">{items.length}</span>
              </header>
              <ul className="kb-plan-rows">
                {items.map((item) => (
                  <li key={item.id}>
                    <ProjectRow agents={agents} data={data} onOpen={() => onSelect(item.id)} project={item} tickets={tickets} />
                  </li>
                ))}
              </ul>
            </section>
          ),
        )}
        <section aria-label="Board filters" className="kb-plan-group">
          <header className="kb-plan-group__head">
            <span className="kb-plan-group__title">Board filters</span>
          </header>
          <ul className="kb-plan-rows">
            <li>
              <button className="kb-plan-row kb-plan-row--filter" onClick={() => onOpenBoard('')} type="button">
                <span className="kb-plan-row__icon">
                  <ListFilter size={16} />
                </span>
                <span className="kb-plan-row__name">
                  <span className="kb-plan-row__title">No project</span>
                  <span className="kb-plan-row__desc">Tickets that belong to no project, on the board</span>
                </span>
                <span className="kb-plan-row__pct">{plural(withoutProject, 'ticket')}</span>
                <ChevronRight aria-hidden="true" className="kb-plan-row__chevron" size={16} />
              </button>
            </li>
          </ul>
        </section>
      </div>
    )
  }

  return (
    <div className="kb-plan-scroll" data-autofocus="" tabIndex={-1}>
      {body}
    </div>
  )
}

/** The Projects page's palette rows and panel requests, now served by the hub (see board.tsx). */
export { ConvertDialog, NewProjectDialog }

/* ── one row of the list ──────────────────────────────────────────────── */

/** The releases a project's tickets sit in, by name, most tickets first. */
function releasesOf(project: ProjectView, tickets: TicketSummary[], data: BoardData): string[] {
  const counts = new Map<string, number>()
  for (const ticket of tickets) {
    if (ticket.project_id !== project.id || !ticket.release_id) continue
    counts.set(ticket.release_id, (counts.get(ticket.release_id) ?? 0) + 1)
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .map(([id]) => data.releases.find((release) => release.id === id)?.name ?? 'Unknown release')
}

function ProjectRow({
  project,
  data,
  tickets,
  agents,
  onOpen,
}: {
  project: ProjectView
  data: BoardData
  tickets: TicketSummary[]
  agents: AgentProfile[]
  onOpen: () => void
}) {
  const pct = project.total > 0 ? Math.round((project.done / project.total) * 100) : 0
  const mine = tickets.filter((ticket) => ticket.project_id === project.id)
  const labels = topLabels(mine, project.source_label, 2)
  const releases = releasesOf(project, tickets, data)
  return (
    <button className="kb-plan-row kb-plan-row--project" onClick={onOpen} type="button">
      <span className="kb-plan-row__icon">
        <FolderGlyph color={project.color} />
      </span>
      <span className="kb-plan-row__name">
        <span className="kb-plan-row__title">{project.name}</span>
        {project.description ? <span className="kb-plan-row__desc">{project.description}</span> : null}
      </span>
      <span className="kb-plan-row__lead">
        <AssigneeAvatar agents={agents} id={project.lead} />
        <span className="kb-plan-row__lead-name">{project.lead ? assigneeName(project.lead, agents) : 'No lead'}</span>
      </span>
      <span className="kb-plan-row__badges">
        {releases.slice(0, 2).map((name) => (
          <ShadcnBadge className="kb-plan-release-badge" key={name} variant="outline">
            <Milestone size={16} />
            {name}
          </ShadcnBadge>
        ))}
        {labels.map((entry) => (
          <LabelBadge key={entry.label} label={entry.label} />
        ))}
      </span>
      <span className="kb-plan-row__progress">
        <ProgressBar done={project.done} total={project.total} />
        <span className="kb-plan-row__pct">{project.total > 0 ? `${pct}%` : '—'}</span>
      </span>
      <ChevronRight aria-hidden="true" className="kb-plan-row__chevron" size={16} />
    </button>
  )
}

/* ── one project ──────────────────────────────────────────────────────── */

function ProjectDetail({
  host,
  project,
  data,
  tickets,
  agents,
  canOpenPane,
  reload,
  announce,
  onError,
  onOpenBoard,
  onOpenTicket,
}: {
  host: Host
  project: ProjectView
  data: BoardData
  tickets: TicketSummary[]
  agents: AgentProfile[]
  canOpenPane: boolean
  reload: (quiet?: boolean) => Promise<void>
  announce: (text: string) => void
  onError: (message: string | null) => void
  onOpenBoard: (project: string, layout?: 'list') => void
  onOpenTicket: (key: string) => void
}) {
  const { confirm, dialog } = useConfirm()
  const archived = project.status === 'archived'
  const [busy, setBusy] = useState(false)
  const [showDone, setShowDone] = useState(false)
  const busyRef = useRef(false)

  /** One action at a time: a second click while one runs, or while its confirmation is open, does nothing. */
  const guarded = async (work: () => Promise<void>) => {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(true)
    onError(null)
    try {
      await work()
    } catch (caught) {
      onError(errorMessage(caught))
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }

  const patch = async (changes: Record<string, unknown>) => {
    await api.updateProject(host.iii, { id: project.id, ...changes })
    announcePlanChange()
    await reload(true)
  }
  /** A picker change has no field to show an error in: it goes to the page. */
  const patchQuietly = async (changes: Record<string, unknown>) => {
    onError(null)
    try {
      await patch(changes)
    } catch (caught) {
      onError(errorMessage(caught))
    }
  }
  const rename = useInlineEdit({
    emptyAllowed: false,
    label: 'name',
    onSave: (name) => patch({ name }),
    value: project.name,
  })
  const describe = useInlineEdit({
    label: 'description',
    multiline: true,
    onSave: (description) => patch({ description }),
    placeholder: 'What is this project for?',
    value: project.description,
  })

  const mine = useMemo(() => tickets.filter((ticket) => ticket.project_id === project.id), [tickets, project.id])
  const unassigned = mine.filter((ticket) => !ticket.assignee).length
  const labels = useMemo(() => topLabels(mine, project.source_label), [mine, project.source_label])
  const [open, done] = useMemo(() => {
    const sorted = [...mine].sort(byProgress(data.columns))
    const closed = (ticket: TicketSummary) =>
      ticket.status === data.done_status || ticket.status === data.canceled_status
    return [sorted.filter((ticket) => !closed(ticket)), sorted.filter(closed)]
  }, [mine, data.columns, data.done_status, data.canceled_status])
  const doneHeading = data.canceled_status ? 'Done or canceled' : 'Done'
  const releaseRows = useMemo(() => {
    const rows = new Map<string | null, { total: number; done: number }>()
    for (const ticket of mine) {
      const key = ticket.release_id ?? null
      const row = rows.get(key) ?? { total: 0, done: 0 }
      // A canceled ticket keeps its release row but leaves the total, as the worker counts it.
      if (ticket.status !== data.canceled_status) row.total += 1
      if (ticket.status === data.done_status) row.done += 1
      rows.set(key, row)
    }
    return [...rows.entries()]
      .map(([id, row]) => ({
        id,
        name: id ? (data.releases.find((release) => release.id === id)?.name ?? 'Unknown release') : 'No release',
        ...row,
      }))
      .sort(
        (a, b) =>
          Number(a.id === null) - Number(b.id === null) || a.name.localeCompare(b.name, 'en', { numeric: true }),
      )
  }, [mine, data.releases, data.done_status, data.canceled_status])
  const inAnyRelease = releaseRows.some((row) => row.id !== null)
  const pct = project.total > 0 ? Math.round((project.done / project.total) * 100) : 0

  const archive = () =>
    guarded(async () => {
      const ok = await confirm({
        title: 'Archive this project?',
        description: 'It moves to Archived and takes no new tickets. Nothing is deleted; you can restore it.',
        confirmLabel: 'Archive',
      })
      if (!ok) return
      await api.archiveProject(host.iii, project.id)
      announcePlanChange()
      announce(`Archived ${project.name}`)
      await reload(true)
    })

  const restore = () =>
    guarded(async () => {
      await api.restoreProject(host.iii, project.id)
      announcePlanChange()
      announce(`Restored ${project.name}`)
      await reload(true)
    })

  const hasDescription = project.description.trim() !== ''

  return (
    <div className="kb-plan-detail">
      <header className="kb-plan-head">
        <div className="kb-plan-head__top">
          <span className="kb-plan-head__icon">
            <FolderGlyph color={project.color} />
          </span>
          <StatusBadge title={archived ? 'Derived: archived' : 'Derived: not archived'} tone={archived ? 'ghost' : 'ink'}>
            {archived ? 'Archived' : 'Active'}
          </StatusBadge>
          {project.source_label ? (
            <span className="kb-plan-head__source">
              From the label <span className="kb-plan-mono">{project.source_label}</span>
            </span>
          ) : null}
          <span className="kb-plan-spacer" />
          <div className="kb-plan-head__actions">
            <ShadcnButton onClick={() => onOpenBoard(project.id)} size="sm" type="button" variant="outline">
              <Columns3 size={16} />
              Open the board
            </ShadcnButton>
            <ShadcnDropdownMenu>
              <ShadcnDropdownMenuTrigger
                render={<ShadcnButton aria-label="More actions for this project" size="icon-sm" type="button" variant="ghost" />}
              >
                <MoreHorizontal size={16} />
              </ShadcnDropdownMenuTrigger>
              <ShadcnDropdownMenuContent align="end" className="kb-menu">
                <ShadcnDropdownMenuGroup>
                  <ShadcnDropdownMenuItem onClick={rename.start}>
                    <Pencil size={16} />
                    Rename…
                  </ShadcnDropdownMenuItem>
                  {archived ? (
                    <ShadcnDropdownMenuItem disabled={busy} onClick={() => void restore()}>
                      <RotateCcw size={16} />
                      Restore
                    </ShadcnDropdownMenuItem>
                  ) : (
                    <ShadcnDropdownMenuItem disabled={busy} onClick={() => void archive()}>
                      <Archive size={16} />
                      Archive…
                    </ShadcnDropdownMenuItem>
                  )}
                </ShadcnDropdownMenuGroup>
              </ShadcnDropdownMenuContent>
            </ShadcnDropdownMenu>
          </div>
        </div>
        {rename.editing ? (
          rename.editor
        ) : (
          <h2 className="kb-plan-title">
            <button className="kb-plan-title__edit" onClick={rename.start} title="Rename" type="button">
              {project.name}
            </button>
          </h2>
        )}
        {describe.editing ? (
          describe.editor
        ) : hasDescription ? (
          <button className="kb-plan-desc" onClick={describe.start} title="Edit the description" type="button">
            {project.description}
          </button>
        ) : (
          <button className="kb-plan-desc" data-empty="true" onClick={describe.start} type="button">
            Add a description…
          </button>
        )}
      </header>

      <div className="kb-plan-detail__grid">
        <div className="kb-plan-detail__main">
          {archived ? (
            <Callout
              action={
                <ShadcnButton disabled={busy} onClick={() => void restore()} size="sm" type="button" variant="outline">
                  Restore
                </ShadcnButton>
              }
              detail="It takes no new tickets. The tickets it has keep it."
              icon={Archive}
              title="This project is archived"
            />
          ) : null}

          <section aria-label="Progress" className="kb-plan-progress">
            <div className="kb-plan-progress__line">
              <span className="kb-plan-progress__pct">{project.total > 0 ? `${pct}%` : '—'}</span>
              <span className="kb-plan-progress__of">
                {project.done} of {plural(project.total, 'ticket')} done
              </span>
            </div>
            <ProgressBar done={project.done} total={project.total} />
            <ColumnTiles
              canceledStatus={data.canceled_status}
              columns={data.columns}
              counts={project.counts}
              doneStatus={data.done_status}
              extra={
                <div className="kb-plan-tile" data-zero={unassigned === 0 ? 'true' : undefined}>
                  <span className="kb-plan-tile__label">
                    <AssigneeAvatar agents={agents} id={null} />
                    Unassigned
                  </span>
                  <span className="kb-plan-tile__n">{unassigned}</span>
                </div>
              }
            />
          </section>

          {open.length > 0 ? (
            <Section count={open.length} flush title="Not done">
              <TicketRows agents={agents} canOpenPane={canOpenPane} data={data} host={host} label="Not done" onOpen={onOpenTicket} tickets={open} />
            </Section>
          ) : null}
          {done.length > 0 ? (
            <Section
              action={
                <ShadcnButton
                  aria-expanded={showDone}
                  onClick={() => setShowDone((now) => !now)}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  {showDone ? 'Hide' : 'Show'}
                </ShadcnButton>
              }
              count={done.length}
              flush
              title={doneHeading}
            >
              {showDone ? (
                <TicketRows agents={agents} canOpenPane={canOpenPane} data={data} host={host} label={doneHeading} onOpen={onOpenTicket} tickets={done} />
              ) : null}
            </Section>
          ) : null}
          {mine.length === 0 ? <p className="kb-plan-quiet">No ticket belongs to this project yet.</p> : null}
        </div>

        <aside aria-label="Project details" className="kb-plan-detail__aside">
          <Section title="Details">
            <Prop label="Lead">
              <LeadPicker agents={agents} onChange={(next) => void patchQuietly({ lead: next })} quiet value={project.lead} />
            </Prop>
            <Prop label="Color">
              <TonePicker allowNone onChange={(next) => void patchQuietly({ color: next })} quiet value={project.color} />
            </Prop>
            <Prop label="Tickets">
              <span className="kb-plan-value">{plural(mine.length, 'ticket')}</span>
            </Prop>
          </Section>

          <Section title="Releases">
            {inAnyRelease ? (
              releaseRows.map((row) => {
                const inner = (
                  <>
                    <div className="kb-plan-split__top">
                      <span className="kb-plan-split__name">
                        <Milestone size={16} />
                        {row.name}
                      </span>
                      <span className="kb-plan-split__n">
                        {row.done}/{row.total}
                      </span>
                    </div>
                    <ProgressBar done={row.done} total={row.total} />
                  </>
                )
                return row.id && canOpenPane ? (
                  <button
                    className="kb-plan-split kb-plan-split--link"
                    key={row.id}
                    onClick={() => openPane(host, RELEASES_PAGE_ID, { id: row.id as string })}
                    type="button"
                  >
                    {inner}
                  </button>
                ) : (
                  <div className="kb-plan-split" key={row.id ?? 'none'}>
                    {inner}
                  </div>
                )
              })
            ) : (
              <div className="kb-plan-section-empty">
                <p className="kb-plan-faint">No ticket of this project is in a release.</p>
                <ShadcnButton onClick={() => onOpenBoard(project.id, 'list')} size="sm" type="button" variant="outline">
                  Open in List
                </ShadcnButton>
              </div>
            )}
          </Section>

          {labels.length > 0 ? (
            <Section title="Most used labels">
              {project.source_label ? (
                <p className="kb-plan-faint">
                  Besides <span className="kb-plan-mono">{project.source_label}</span>
                </p>
              ) : null}
              <div className="kb-plan-labels">
                {labels.map((entry) => (
                  <span className="kb-plan-labels__item" key={entry.label}>
                    <LabelBadge label={entry.label} />
                    <span className="kb-plan-labels__n">{entry.count}</span>
                  </span>
                ))}
              </div>
            </Section>
          ) : null}
        </aside>
      </div>
      {dialog}
    </div>
  )
}
