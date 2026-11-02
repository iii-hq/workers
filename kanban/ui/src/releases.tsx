import {
  type Host,
  type LiveAnnouncement,
  LiveRegion,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  ShadcnButton,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuItem,
  ShadcnDropdownMenuLabel,
  ShadcnDropdownMenuTrigger,
  ShadcnSkeleton,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import {
  ChevronRight,
  CircleCheck,
  Columns3,
  Milestone,
  MoreHorizontal,
  Pencil,
  Plus,
  RotateCcw,
  Tag,
  Trash2,
} from 'lucide-react'
import { type ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { Callout, Crumbs, Prop, Section } from './plan/parts'
import { TicketRows } from './plan/ticket-rows'
import {
  ColumnTiles,
  ConvertDialog,
  NewReleaseDialog,
  openPane,
  ProgressBar,
  ShipDialog,
  StatusBadge,
  takePanelRequest,
  useAgents,
  useInlineEdit,
  usePlanBoard,
} from './plan-dialogs'
import {
  byProgress,
  driftTickets,
  formatDay,
  formatStamp,
  formatTarget,
  groupReleases,
  isIsoDay,
  liveCount,
  nameList,
  plural,
  relativeDay,
  releaseStatusText,
  resolveEntity,
  ticketsWithLabel,
} from './plan-rules'
import {
  type AgentProfile,
  announcePlanChange,
  api,
  BOARD_PAGE_ID,
  type BoardData,
  type ReleaseView,
  type TicketSummary,
} from './shared'
import { Empty, Notice } from './ui/feedback'
import { ColumnStatusIcon } from './ui/glyphs'
import { IconAction } from './ui/icon-action'

const FULL_DAY = { month: 'short', day: 'numeric', year: 'numeric' } as const
const STATUS_TONE = { active: 'ink', planned: 'ghost', released: 'ok' } as const
const STATUS_TITLE = {
  active: 'Derived: some tickets are past the first column',
  planned: 'Derived: every ticket is still in the first column',
  released: 'Derived: marked as released',
} as const

export function ReleasesPage({
  host,
  onRequestClose,
  panelContext,
  panelSide,
  paneId,
  tabId,
  commands,
}: PageRenderProps & { host: Host }) {
  const paneKey = paneId || tabId || 'default'
  const { data, loading, error, reload } = usePlanBoard(host)
  const agents = useAgents(host)
  const { ref, narrow } = useContainerNarrow({ below: 720 })
  const [stored, setStored] = usePaneState<string | null>(`kanban:release:${paneKey}`, null)
  /** The list is the landing; a release opens over it and the way back is the breadcrumb. Kept per pane. */
  const [drilled, setDrilled] = usePaneState<boolean>(`kanban:release-open:${paneKey}`, false)
  const [creating, setCreating] = useState(false)
  const [converting, setConverting] = useState(false)
  const [shipping, setShipping] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const seq = useRef(0)
  const announce = (text: string) => setAnnouncement({ seq: ++seq.current, text, urgency: 'polite' })

  const releases = useMemo(() => data?.releases ?? [], [data])
  const groups = useMemo(() => groupReleases(releases), [releases])
  const tickets = useMemo(() => (data?.columns ?? []).flatMap((column) => column.tickets), [data])
  const selected = resolveEntity(releases, stored)
  const detailShown = Boolean(drilled && selected)

  const pick = (release: ReleaseView) => {
    setStored(release.id)
    setDrilled(true)
    setActionError(null)
  }
  const back = () => {
    setDrilled(false)
    setActionError(null)
  }

  // Other screens open a release (or a dialog) through `host.panels.open({ pageId, context })`.
  // The host hands the last context to every mount of the page: each request is acted on once.
  useEffect(() => {
    const request = takePanelRequest(panelContext)
    if (request.id) {
      setStored(request.id)
      setDrilled(true)
    }
    if (request.action === 'new') setCreating(true)
    if (request.action === 'convert') setConverting(true)
  }, [panelContext, setStored, setDrilled])

  // What is on screen is remembered by id: a release reached by name survives a rename.
  // (A ref that is not loaded yet, a release just created, is never dropped.)
  useEffect(() => {
    if (selected && selected.id !== stored) setStored(selected.id)
  }, [selected, stored, setStored])

  useEffect(
    () => commands?.register([{ id: 'refresh', title: 'Refresh releases', run: () => void reload() }]),
    [commands, reload],
  )

  const canOpenPane = Boolean(host.panels?.open)

  const header = (
    <PageHeader
      actions={
        narrow ? (
          detailShown ? null : (
            <IconAction label="New release" onClick={() => setCreating(true)} variant="default">
              <Plus size={16} />
            </IconAction>
          )
        ) : (
          <>
            <ShadcnButton disabled={!data} onClick={() => setConverting(true)} size="sm" type="button" variant="ghost">
              <Tag size={16} />
              Create from a label…
            </ShadcnButton>
            <ShadcnButton onClick={() => setCreating(true)} size="sm" type="button">
              <Plus size={16} />
              New release
            </ShadcnButton>
          </>
        )
      }
      description={data ? `${plural(releases.length, 'release')} · ${data.id_prefix}` : undefined}
      icon={<Milestone size={16} />}
      onClose={onRequestClose}
      title="Releases"
    />
  )

  let body: ReactNode
  if (!data) {
    body = error ? (
      <Notice actionLabel="Retry" detail={error} onAction={() => void reload()} title="The releases could not be read" />
    ) : (
      <div aria-busy={loading} aria-label="Loading releases" className="kb-plan-list">
        {[0, 1, 2, 3].map((index) => (
          <ShadcnSkeleton className="kb-plan-skeleton kb-plan-skeleton--row" key={index} />
        ))}
      </div>
    )
  } else if (releases.length === 0) {
    body = (
      <Empty
        action={
          <div className="kb-plan-empty-actions">
            <ShadcnButton onClick={() => setConverting(true)} size="sm" type="button" variant="outline">
              <Tag size={16} />
              Create from a label…
            </ShadcnButton>
            <ShadcnButton onClick={() => setCreating(true)} size="sm" type="button">
              <Plus size={16} />
              New release
            </ShadcnButton>
          </div>
        }
        description="Group tickets into a delivery with a target date. You can start from a label like phase-2."
        icon={Milestone}
        title="No releases yet"
      />
    )
  } else if (detailShown && selected) {
    body = (
      <>
        <Crumbs name={selected.name} onBack={back} root="Releases" />
        {actionError ? (
          <Notice
            actionLabel="Dismiss"
            detail={actionError}
            onAction={() => setActionError(null)}
            title="That change was not saved"
          />
        ) : null}
        <ReleaseDetail
          agents={agents}
          announce={announce}
          canOpenPane={canOpenPane}
          data={data}
          host={host}
          key={selected.id}
          onDeleted={back}
          onError={setActionError}
          onShip={() => setShipping(true)}
          release={selected}
          reload={reload}
          tickets={tickets}
        />
      </>
    )
  } else {
    body = (
      <div className="kb-plan-list">
        {(
          [
            ['Active', groups.active],
            ['Planned', groups.planned],
            ['Released', groups.released],
          ] as const
        ).map(([label, items]) =>
          items.length === 0 ? null : (
            <section aria-label={label} className="kb-plan-group" key={label}>
              <header className="kb-plan-group__head">
                <span className="kb-plan-group__title">{label}</span>
                <span className="kb-plan-group__count">{items.length}</span>
              </header>
              <ul className="kb-plan-rows">
                {items.map((release) => (
                  <li key={release.id}>
                    <ReleaseRow data={data} onOpen={() => pick(release)} release={release} />
                  </li>
                ))}
              </ul>
            </section>
          ),
        )}
      </div>
    )
  }

  return (
    <PageShell className="kb kb-shell kb-plan" data-narrow={narrow ? 'true' : undefined} ref={ref}>
      {header}
      <PageBody side={panelSide}>
        <PageMain>
          <div className="kb-plan-scroll" data-autofocus="" tabIndex={-1}>
            {body}
          </div>
        </PageMain>
      </PageBody>

      <NewReleaseDialog
        host={host}
        onCreated={(release) => {
          setCreating(false)
          setStored(release.id)
          setDrilled(true)
          announce(`Created ${release.name}`)
          void reload(true)
        }}
        onOpenChange={setCreating}
        open={creating}
      />
      {data ? (
        <ConvertDialog
          data={data}
          host={host}
          into="release"
          onDone={(outcome) => {
            const first = outcome.entities[0]
            if (first) {
              setStored(first.id)
              setDrilled(true)
            }
            announce(outcome.text)
            void reload(true)
          }}
          onOpenChange={setConverting}
          open={converting}
        />
      ) : null}
      {selected ? (
        <ShipDialog
          host={host}
          onOpenChange={setShipping}
          onShipped={({ release, moved, to }) => {
            setShipping(false)
            const where =
              moved.length === 0
                ? ''
                : to
                  ? ` · ${plural(moved.length, 'ticket')} moved to ${to.name}`
                  : ` · ${plural(moved.length, 'ticket')} taken out of any release`
            announce(`${release.name} marked as released${where}`)
            void reload(true)
          }}
          open={shipping}
          release={selected}
          releases={releases}
        />
      ) : null}
      <LiveRegion announcement={announcement} />
    </PageShell>
  )
}

/* ── one row of the list ──────────────────────────────────────────────── */

/** A release in the list: name and description, status, target, progress, and where its tickets stand. */
function ReleaseRow({ release, data, onOpen }: { release: ReleaseView; data: BoardData; onOpen: () => void }) {
  const pct = release.total > 0 ? Math.round((release.done / release.total) * 100) : 0
  const when =
    release.status === 'released'
      ? `Released ${formatStamp(release.released_at ?? '')}`
      : release.target_at
        ? formatTarget(release.target_at)
        : 'No target'
  const counts = data.columns.filter((column) => (release.counts[column.id] ?? 0) > 0)
  return (
    <button className="kb-plan-row" onClick={onOpen} type="button">
      <span className="kb-plan-row__icon" data-status={release.status}>
        <Milestone size={16} />
      </span>
      <span className="kb-plan-row__name">
        <span className="kb-plan-row__title">{release.name}</span>
        {release.description ? <span className="kb-plan-row__desc">{release.description}</span> : null}
      </span>
      <span className="kb-plan-row__status">
        <StatusBadge title={STATUS_TITLE[release.status]} tone={STATUS_TONE[release.status]}>
          {releaseStatusText(release)}
        </StatusBadge>
      </span>
      <span className="kb-plan-row__when" data-none={!release.target_at && release.status !== 'released' ? 'true' : undefined}>
        {when}
      </span>
      <span className="kb-plan-row__counts" aria-label="Tickets by status">
        {counts.map((column) => (
          <span className="kb-plan-row__count" key={column.id} title={column.label}>
            <ColumnStatusIcon
              canceledStatus={data.canceled_status}
              columns={data.columns}
              doneStatus={data.done_status}
              status={column.id}
            />
            {release.counts[column.id]}
          </span>
        ))}
      </span>
      <span className="kb-plan-row__progress">
        <ProgressBar done={release.done} total={release.total} />
        <span className="kb-plan-row__pct">{release.total > 0 ? `${pct}%` : '—'}</span>
      </span>
      <ChevronRight aria-hidden="true" className="kb-plan-row__chevron" size={16} />
    </button>
  )
}

/* ── one release ──────────────────────────────────────────────────────── */

function ReleaseDetail({
  host,
  release,
  data,
  tickets,
  agents,
  canOpenPane,
  reload,
  announce,
  onError,
  onShip,
  onDeleted,
}: {
  host: Host
  release: ReleaseView
  data: BoardData
  tickets: TicketSummary[]
  agents: AgentProfile[]
  canOpenPane: boolean
  reload: (quiet?: boolean) => Promise<void>
  announce: (text: string) => void
  onError: (message: string | null) => void
  onShip: () => void
  onDeleted: () => void
}) {
  const { confirm, dialog } = useConfirm()
  const columns = data.columns
  const released = release.status === 'released'
  const [showDone, setShowDone] = useState(false)
  const [busy, setBusy] = useState(false)
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
    await api.updateRelease(host.iii, { id: release.id, ...changes })
    announcePlanChange()
    await reload(true)
  }
  const rename = useInlineEdit({
    emptyAllowed: false,
    label: 'name',
    onSave: (name) => patch({ name }),
    value: release.name,
  })
  const retarget = useInlineEdit({
    label: 'target date',
    mono: true,
    onSave: (next) => patch({ target_at: next === '' ? null : next }),
    placeholder: 'YYYY-MM-DD',
    validate: (next) => (isIsoDay(next) ? null : 'Use the form YYYY-MM-DD'),
    value: release.target_at ?? '',
  })
  const describe = useInlineEdit({
    label: 'description',
    multiline: true,
    onSave: (description) => patch({ description }),
    placeholder: 'What this release delivers',
    value: release.description,
  })

  const mine = useMemo(() => tickets.filter((ticket) => ticket.release_id === release.id), [tickets, release.id])
  // Canceled tickets are closed: they sit with the done ones, never with the open.
  const [open, done] = useMemo(() => {
    const sorted = [...mine].sort(byProgress(columns))
    const closed = (ticket: TicketSummary) =>
      ticket.status === data.done_status || ticket.status === data.canceled_status
    return [sorted.filter((ticket) => !closed(ticket)), sorted.filter(closed)]
  }, [mine, data.done_status, data.canceled_status, columns])
  const doneHeading = data.canceled_status ? 'Done or canceled' : 'Done'
  // Canceled tickets are out of `total` but still in the release: the worker refuses the delete while any is.
  const live = liveCount(release)
  const pct = release.total > 0 ? Math.round((release.done / release.total) * 100) : 0

  const withLabel = release.source_label ? ticketsWithLabel(tickets, release.source_label) : []
  const drift = released ? [] : driftTickets(release, tickets)
  const elsewhere = withLabel.filter((ticket) => ticket.release_id && ticket.release_id !== release.id)

  const addDrift = () =>
    guarded(async () => {
      if (!release.source_label) return
      const result = await api.convertLabel(host.iii, { label: release.source_label, into: 'release', id: release.id })
      announcePlanChange()
      announce(`Added ${plural(result.tickets.length, 'ticket')} to ${release.name}`)
      await reload(true)
    })

  const reopen = () =>
    guarded(async () => {
      const ok = await confirm({
        title: 'Reopen this release?',
        description:
          'It is Active or Planned again, by where its tickets are. Tickets that were moved out do not come back.',
        confirmLabel: 'Reopen',
      })
      if (!ok) return
      await api.reopenRelease(host.iii, release.id)
      announcePlanChange()
      announce(`Reopened ${release.name}`)
      await reload(true)
    })

  const remove = () =>
    guarded(async () => {
      const ok = await confirm({
        title: 'Delete this release?',
        description:
          'No live ticket is in it. Deleted tickets may still reference it; the worker refuses the delete if any do. This cannot be undone.',
        confirmLabel: 'Delete',
        tone: 'danger',
      })
      if (!ok) return
      try {
        await api.deleteRelease(host.iii, release.id)
      } catch (caught) {
        // Deleted tickets are not on this page: the worker's "move them" has nothing to point at.
        const message = errorMessage(caught)
        throw message.includes('(deleted ones included)')
          ? new Error(`${message.split(';')[0]}. Restore or move those tickets first.`)
          : caught
      }
      announcePlanChange()
      announce(`Deleted ${release.name}`)
      onDeleted()
      await reload(true)
    })

  const projectName = (id: string | null) =>
    id ? (data.projects.find((project) => project.id === id)?.name ?? 'Unknown project') : 'No project'
  const absent = data.projects.filter(
    (project) => project.status !== 'archived' && !release.projects.some((row) => row.project_id === project.id),
  )

  const actions = (
    <div className="kb-plan-head__actions">
      {canOpenPane ? (
        <ShadcnButton
          onClick={() => openPane(host, BOARD_PAGE_ID, { group: 'release', release: release.id })}
          size="sm"
          type="button"
          variant="outline"
        >
          <Columns3 size={16} />
          Open on the board
        </ShadcnButton>
      ) : null}
      {released ? (
        <ShadcnButton disabled={busy} onClick={() => void reopen()} size="sm" type="button" variant="outline">
          <RotateCcw size={16} />
          Reopen…
        </ShadcnButton>
      ) : (
        <ShadcnButton onClick={onShip} size="sm" type="button">
          <CircleCheck size={16} />
          Mark as released…
        </ShadcnButton>
      )}
      <ShadcnDropdownMenu>
        <ShadcnDropdownMenuTrigger
          render={<ShadcnButton aria-label="More actions for this release" size="icon-sm" type="button" variant="ghost" />}
        >
          <MoreHorizontal size={16} />
        </ShadcnDropdownMenuTrigger>
        <ShadcnDropdownMenuContent align="end" className="kb-menu">
          <ShadcnDropdownMenuGroup>
            <ShadcnDropdownMenuItem onClick={rename.start}>
              <Pencil size={16} />
              Rename…
            </ShadcnDropdownMenuItem>
            <ShadcnDropdownMenuItem disabled={live > 0 || busy} onClick={() => void remove()} variant="destructive">
              <Trash2 size={16} />
              Delete…
            </ShadcnDropdownMenuItem>
            {live > 0 ? (
              <ShadcnDropdownMenuLabel className="kb-plan-menu-note">
                It has {plural(live, 'ticket')}. Move them to another release first.
              </ShadcnDropdownMenuLabel>
            ) : null}
          </ShadcnDropdownMenuGroup>
        </ShadcnDropdownMenuContent>
      </ShadcnDropdownMenu>
    </div>
  )

  const hasDescription = release.description.trim() !== ''

  return (
    <div className="kb-plan-detail">
      <header className="kb-plan-head">
        <div className="kb-plan-head__top">
          <span className="kb-plan-head__icon" data-status={release.status}>
            <Milestone size={16} />
          </span>
          <StatusBadge title={STATUS_TITLE[release.status]} tone={STATUS_TONE[release.status]}>
            {releaseStatusText(release)}
          </StatusBadge>
          {release.source_label ? (
            <span className="kb-plan-head__source">
              From the label <span className="kb-plan-mono">{release.source_label}</span>
            </span>
          ) : null}
          <span className="kb-plan-spacer" />
          {actions}
        </div>
        {rename.editing ? (
          rename.editor
        ) : (
          <h2 className="kb-plan-title">
            <button className="kb-plan-title__edit" onClick={rename.start} title="Rename" type="button">
              {release.name}
            </button>
          </h2>
        )}
        {describe.editing ? (
          describe.editor
        ) : hasDescription ? (
          <button className="kb-plan-desc" onClick={describe.start} title="Edit the description" type="button">
            {release.description}
          </button>
        ) : (
          <button className="kb-plan-desc" data-empty="true" onClick={describe.start} type="button">
            Add a description…
          </button>
        )}
      </header>

      <div className="kb-plan-detail__grid">
        <div className="kb-plan-detail__main">
          {release.source_label ? (
            drift.length > 0 ? (
              <Callout
                action={
                  <ShadcnButton disabled={busy} onClick={() => void addDrift()} size="sm" type="button" variant="outline">
                    Add {plural(drift.length, 'ticket')}
                  </ShadcnButton>
                }
                detail="They carry the label but are in no release yet."
                icon={Tag}
                title={
                  <>
                    {plural(drift.length, 'ticket')} with <span className="kb-plan-mono">{release.source_label}</span>{' '}
                    {drift.length === 1 ? 'is' : 'are'} outside this release
                  </>
                }
              />
            ) : (
              <p className="kb-plan-quiet">
                <CircleCheck size={16} />
                <span>
                  {elsewhere.length === 0 ? 'All ' : ''}
                  {plural(withLabel.length - elsewhere.length, 'ticket')} with{' '}
                  <span className="kb-plan-mono">{release.source_label}</span>{' '}
                  {withLabel.length - elsewhere.length === 1 ? 'is' : 'are'} here
                  {elsewhere.length > 0 ? ` · ${elsewhere.length} in another release` : ''}
                </span>
              </p>
            )
          ) : null}

          <section aria-label="Progress" className="kb-plan-progress">
            <div className="kb-plan-progress__line">
              <span className="kb-plan-progress__pct">{release.total > 0 ? `${pct}%` : '—'}</span>
              <span className="kb-plan-progress__of">
                {release.done} of {plural(release.total, 'ticket')} done
              </span>
            </div>
            <ProgressBar done={release.done} total={release.total} />
            <ColumnTiles
              canceledStatus={data.canceled_status}
              columns={columns}
              counts={release.counts}
              doneStatus={data.done_status}
            />
          </section>

          {open.length > 0 ? (
            <Section count={open.length} flush title="Not done">
              <TicketRows agents={agents} canOpenPane={canOpenPane} data={data} host={host} label="Not done" tickets={open} />
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
                <TicketRows agents={agents} canOpenPane={canOpenPane} data={data} host={host} label={doneHeading} tickets={done} />
              ) : null}
            </Section>
          ) : null}
          {mine.length === 0 ? (
            <p className="kb-plan-quiet">No ticket is in this release yet. Add them from the board or the List layout.</p>
          ) : null}
        </div>

        <aside aria-label="Release details" className="kb-plan-detail__aside">
          <Section title="Details">
            <Prop label="Target">
              {retarget.editing ? (
                retarget.editor
              ) : (
                <button className="kb-plan-value" onClick={retarget.start} title="Edit the target date" type="button">
                  {release.target_at ? (
                    <>
                      <span>{formatDay(release.target_at)}</span>
                      {released ? null : <span className="kb-plan-faint">{relativeDay(release.target_at)}</span>}
                    </>
                  ) : (
                    <span className="kb-plan-faint">Set a date</span>
                  )}
                  <Pencil className="kb-plan-value__pen" size={16} />
                </button>
              )}
            </Prop>
            {released && release.released_at ? (
              <Prop label="Released">
                <span className="kb-plan-value" title={new Date(release.released_at).toLocaleString()}>
                  {new Date(release.released_at).toLocaleDateString('en-US', FULL_DAY)}
                </span>
              </Prop>
            ) : null}
            <Prop label="Tickets">
              <span className="kb-plan-value">{plural(live, 'ticket')}</span>
            </Prop>
          </Section>

          <Section title="By project">
            {release.projects.length === 0 ? <p className="kb-plan-faint">No tickets in this release yet.</p> : null}
            {release.projects.map((row) => (
              <div className="kb-plan-split" key={row.project_id ?? 'none'}>
                <div className="kb-plan-split__top">
                  <span className="kb-plan-split__name">{projectName(row.project_id)}</span>
                  <span className="kb-plan-split__n">
                    {row.done}/{row.total}
                  </span>
                </div>
                <ProgressBar done={row.done} total={row.total} />
              </div>
            ))}
            {release.projects.length > 0 && absent.length > 0 ? (
              <p className="kb-plan-faint">
                {nameList(absent.map((project) => project.name))} {absent.length === 1 ? 'has' : 'have'} no tickets in
                this release.
              </p>
            ) : null}
          </Section>
        </aside>
      </div>
      {dialog}
    </div>
  )
}
