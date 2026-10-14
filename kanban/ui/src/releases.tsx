import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  EmptyState,
  Eyebrow,
  type Host,
  IconButton,
  List,
  ListGroup,
  ListItem,
  type LiveAnnouncement,
  LiveRegion,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Panel,
  PanelBody,
  PanelHeader,
  Skeleton,
  StatusPanel,
  uiClasses,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import {
  Bot,
  ChevronRight,
  CircleCheck,
  Columns3,
  Flag,
  MoreHorizontal,
  Pencil,
  Plus,
  Tag,
  Trash2,
  TriangleAlert,
} from 'lucide-react'
import { type ReactNode, useEffect, useId, useMemo, useRef, useState } from 'react'
import {
  ColumnTiles,
  ConvertDialog,
  DrillBar,
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
  StatusGlyph,
  TICKET_PAGE_ID,
  type TicketSummary,
} from './shared'

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
  /** Narrow panes show the list or one release, never both. */
  const [drilled, setDrilled] = useState(false)
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
  const chosen = resolveEntity(releases, stored)
  const fallback = narrow ? undefined : (groups.active[0] ?? groups.planned[0] ?? groups.released[0])
  const selected = chosen ?? fallback
  const detailShown = Boolean(selected) && (!narrow || drilled)

  const pick = (release: ReleaseView) => {
    setStored(release.id)
    setDrilled(true)
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
  }, [panelContext, setStored])

  // What is on screen is remembered by id: a release reached by name survives a rename, and the first one shown
  // stays put when its status moves it to another group.
  // (A ref that is not loaded yet, a release just created, is never replaced by the first one.)
  const pinned = chosen?.id ?? (stored ? undefined : fallback?.id)
  useEffect(() => {
    if (pinned && pinned !== stored) setStored(pinned)
  }, [pinned, stored, setStored])

  useEffect(
    () => commands?.register([{ id: 'refresh', title: 'Refresh releases', run: () => void reload() }]),
    [commands, reload],
  )

  const count = plural(releases.length, 'release')
  const canOpenPane = Boolean(host.panels?.open)

  const header = (
    <PageHeader
      actions={
        narrow ? (
          // One release open: the way back is the toolbar under the header, and nothing is created from here.
          detailShown ? null : (
            <IconButton
              className="kanban-plan-touch"
              label="New release"
              onClick={() => setCreating(true)}
              variant="primary"
            >
              <Plus size={16} />
            </IconButton>
          )
        ) : (
          <>
            <Button disabled={!data} onClick={() => setConverting(true)} variant="ghost">
              <Tag size={16} />
              Create from a label…
            </Button>
            <Button onClick={() => setCreating(true)} variant="primary">
              <Plus size={16} />
              New release
            </Button>
          </>
        )
      }
      description={data ? `${count} · ${data.id_prefix}` : undefined}
      icon={<Flag size={16} />}
      onClose={onRequestClose}
      title="Releases"
    />
  )

  const rail = (
    <PageSidebar label="Releases" narrow={narrow} width={300}>
      <nav aria-label="Releases" className="kanban-plan-nav">
        <List>
          {(
            [
              ['Active', groups.active],
              ['Planned', groups.planned],
              ['Released', groups.released],
            ] as const
          ).map(([label, items]) =>
            items.length === 0 ? null : (
              <ListGroup aria-label={label} key={label}>
                <Eyebrow as="div" className="kanban-plan-eyebrow">
                  {label}
                </Eyebrow>
                {items.map((release) => {
                  const current = !narrow && selected?.id === release.id
                  return (
                    <ListItem
                      aria-current={current ? 'true' : undefined}
                      description={
                        release.status === 'released'
                          ? `Released ${formatStamp(release.released_at ?? '')}`
                          : release.target_at
                            ? `Target ${formatTarget(release.target_at)}`
                            : 'No target date'
                      }
                      key={release.id}
                      label={release.name}
                      onClick={() => pick(release)}
                      selected={current}
                      trailing={
                        <span className="kanban-plan-mini">
                          <span className="kanban-plan-mini__count">
                            <span className="kanban-plan-mono">
                              {release.done}/{release.total}
                            </span>
                            <ProgressBar done={release.done} total={release.total} />
                          </span>
                          {narrow ? <ChevronRight size={16} /> : null}
                        </span>
                      }
                    />
                  )
                })}
              </ListGroup>
            ),
          )}
        </List>
        {narrow ? (
          <Button
            className="kanban-plan-touch kanban-plan-nav__more"
            onClick={() => setConverting(true)}
            variant="ghost"
          >
            <Tag size={16} />
            Create from a label…
          </Button>
        ) : null}
      </nav>
    </PageSidebar>
  )

  let body: ReactNode
  if (!data) {
    body = error ? (
      <PageMain>
        <div className="kanban-pad">
          <StatusPanel
            action={
              <Button onClick={() => void reload()} variant="pill">
                Retry
              </Button>
            }
            detail={error}
            headline="The releases could not be read"
            icon={<TriangleAlert size={16} />}
            role="alert"
            variant="alert"
          />
        </div>
      </PageMain>
    ) : (
      <>
        <PageSidebar label="Releases" narrow={narrow} width={300}>
          <div className="kanban-plan-nav" aria-busy={loading}>
            {[0, 1, 2, 3].map((index) => (
              <Skeleton className="kanban-plan-skeleton kanban-plan-skeleton--row" key={index} />
            ))}
          </div>
        </PageSidebar>
        {narrow ? null : (
          <PageMain>
            <div className="kanban-plan-scroll">
              <div className="kanban-plan-main">
                <Skeleton className="kanban-plan-skeleton kanban-plan-skeleton--title" />
                <Skeleton className="kanban-plan-skeleton kanban-plan-skeleton--bar" />
                <Skeleton className="kanban-plan-skeleton kanban-plan-skeleton--block" />
              </div>
            </div>
          </PageMain>
        )}
      </>
    )
  } else if (releases.length === 0) {
    body = (
      <PageMain>
        <div className="kanban-plan-empty">
          <EmptyState
            action={{ label: 'Create from a label…', onClick: () => setConverting(true) }}
            description="Group tickets into a delivery with a target date. You can start from a label like phase-2."
            icon={Flag}
            title="No releases yet"
          />
        </div>
      </PageMain>
    )
  } else {
    body = (
      <>
        {narrow && detailShown ? null : rail}
        {detailShown && selected ? (
          <PageMain>
            {narrow ? <DrillBar name={selected.name} onBack={() => setDrilled(false)} root="Releases" /> : null}
            <div className="kanban-plan-scroll" data-autofocus="" tabIndex={-1}>
              {actionError ? (
                <div className="kanban-pad">
                  <StatusPanel
                    action={
                      <Button onClick={() => setActionError(null)} variant="pill">
                        Dismiss
                      </Button>
                    }
                    detail={actionError}
                    headline="That change was not saved"
                    icon={<TriangleAlert size={16} />}
                    role="alert"
                    variant="alert"
                  />
                </div>
              ) : null}
              <ReleaseDetail
                agents={agents}
                announce={announce}
                canOpenPane={canOpenPane}
                data={data}
                host={host}
                key={selected.id}
                narrow={narrow}
                onError={setActionError}
                onShip={() => setShipping(true)}
                release={selected}
                reload={reload}
                tickets={tickets}
              />
            </div>
          </PageMain>
        ) : null}
      </>
    )
  }

  return (
    <PageShell className="kanban-shell kanban-plan" data-narrow={narrow ? 'true' : undefined} ref={ref}>
      {header}
      <PageBody side={panelSide}>{body}</PageBody>

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

/* ── one release ──────────────────────────────────────────────────────── */

function ReleaseDetail({
  host,
  release,
  data,
  tickets,
  agents,
  narrow,
  canOpenPane,
  reload,
  announce,
  onError,
  onShip,
}: {
  host: Host
  release: ReleaseView
  data: BoardData
  tickets: TicketSummary[]
  agents: AgentProfile[]
  narrow: boolean
  canOpenPane: boolean
  reload: (quiet?: boolean) => Promise<void>
  announce: (text: string) => void
  onError: (message: string | null) => void
  onShip: () => void
}) {
  const uid = useId()
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
  const open = useMemo(
    () => mine.filter((ticket) => ticket.status !== data.done_status).sort(byProgress(columns)),
    [mine, data.done_status, columns],
  )
  const done = useMemo(
    () => mine.filter((ticket) => ticket.status === data.done_status).sort(byProgress(columns)),
    [mine, data.done_status, columns],
  )

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
        title: `Reopen ${release.name}?`,
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
        title: `Delete ${release.name}?`,
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
      await reload(true)
    })

  const projectName = (id: string | null) =>
    id ? (data.projects.find((project) => project.id === id)?.name ?? 'Unknown project') : 'No project'
  const absent = data.projects.filter(
    (project) => project.status !== 'archived' && !release.projects.some((row) => row.project_id === project.id),
  )

  /* The pieces; a wide pane puts them in two columns, a narrow one in a single stack. */

  const actions = (
    <>
      {canOpenPane ? (
        <Button
          className="kanban-plan-touch kanban-plan-actions__wide"
          onClick={() => openPane(host, BOARD_PAGE_ID, { group: 'release', release: release.id })}
          variant="pill"
        >
          <Columns3 size={16} />
          Open on the board
        </Button>
      ) : null}
      {released ? (
        <Button className="kanban-plan-touch" disabled={busy} onClick={() => void reopen()} variant="pill">
          Reopen…
        </Button>
      ) : (
        <Button className="kanban-plan-touch" onClick={onShip} variant="pill">
          Mark as released…
        </Button>
      )}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <IconButton className="kanban-plan-touch" label={`More actions for ${release.name}`} variant="ghost">
            <MoreHorizontal size={16} />
          </IconButton>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem disabled={release.total > 0 || busy} onSelect={() => void remove()}>
            <Trash2 size={16} />
            Delete…
          </DropdownMenuItem>
          {release.total > 0 ? (
            <DropdownMenuLabel className="kanban-plan-menu-note">
              It has {plural(release.total, 'ticket')}. Move them to another release first.
            </DropdownMenuLabel>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  )

  const head = (
    <header className="kanban-plan-head">
      <div className="kanban-plan-head__id">
        <div className="kanban-plan-head__line">
          {rename.editing ? (
            rename.editor
          ) : (
            <>
              <h2 className="kanban-plan-title">{release.name}</h2>
              <IconButton
                className="kanban-plan-touch"
                label={`Rename ${release.name}`}
                onClick={rename.start}
                variant="ghost"
              >
                <Pencil size={16} />
              </IconButton>
            </>
          )}
          <StatusBadge title={STATUS_TITLE[release.status]} tone={STATUS_TONE[release.status]}>
            {releaseStatusText(release)}
          </StatusBadge>
        </div>
        {release.source_label ? (
          <span className="kanban-plan-sub">
            Created from the label <span className="kanban-plan-mono">{release.source_label}</span>
          </span>
        ) : null}
      </div>
      {narrow ? null : <div className="kanban-plan-head__actions">{actions}</div>}
    </header>
  )

  const target = (
    <div className="kanban-plan-kv">
      <span className="kanban-plan-k">Target</span>
      <span className="kanban-plan-v">
        {retarget.editing ? (
          retarget.editor
        ) : release.target_at ? (
          <>
            <span className="kanban-plan-mono">{formatDay(release.target_at)}</span>
            {released ? null : <span className="kanban-plan-faint">{relativeDay(release.target_at)}</span>}
          </>
        ) : (
          <span className="kanban-plan-faint">No target date</span>
        )}
      </span>
      {retarget.editing ? (
        <span />
      ) : (
        <IconButton className="kanban-plan-touch" label="Edit the target date" onClick={retarget.start} variant="ghost">
          <Pencil size={16} />
        </IconButton>
      )}
    </div>
  )

  const shippedOn =
    released && release.released_at ? (
      <div className="kanban-plan-kv">
        <span className="kanban-plan-k">Released</span>
        <span className="kanban-plan-v">
          <span className="kanban-plan-mono" title={new Date(release.released_at).toLocaleString()}>
            {new Date(release.released_at).toLocaleDateString('en-US', FULL_DAY)}
          </span>
        </span>
        <span />
      </div>
    ) : null

  const hasDescription = release.description.trim() !== ''
  const editDescription =
    hasDescription && !describe.editing ? (
      <IconButton
        className="kanban-plan-touch kanban-plan-panelh__end"
        label="Edit the description"
        onClick={describe.start}
        variant="ghost"
      >
        <Pencil size={16} />
      </IconButton>
    ) : null
  const description = describe.editing ? (
    describe.editor
  ) : hasDescription ? (
    <p className="kanban-plan-desc">{release.description}</p>
  ) : (
    <button className="kanban-plan-link kanban-plan-touch" onClick={describe.start} type="button">
      Add a description
    </button>
  )

  const nudge = release.source_label ? (
    drift.length > 0 ? (
      <StatusPanel
        action={
          <Button className="kanban-plan-touch" disabled={busy} onClick={() => void addDrift()} variant="pill">
            Add {plural(drift.length, 'ticket')}
          </Button>
        }
        detail="They carry the label but are in no release yet."
        headline={
          <>
            {plural(drift.length, 'ticket')} with <span className="kanban-plan-mono">{release.source_label}</span>{' '}
            {drift.length === 1 ? 'is' : 'are'} outside this release
          </>
        }
        icon={<Tag size={16} />}
        role="status"
        variant="info"
      />
    ) : (
      <p className="kanban-plan-quiet">
        <CircleCheck size={16} />
        <span>
          {elsewhere.length === 0 ? 'All ' : ''}
          {plural(withLabel.length - elsewhere.length, 'ticket')} with{' '}
          <span className="kanban-plan-mono">{release.source_label}</span>{' '}
          {withLabel.length - elsewhere.length === 1 ? 'is' : 'are'} here
          {elsewhere.length > 0 ? ` · ${elsewhere.length} in another release` : ''}
        </span>
      </p>
    )
  ) : null

  const progress = (
    <section aria-label="Progress" className="kanban-plan-progress">
      <div className="kanban-plan-progress__line">
        <ProgressBar done={release.done} total={release.total} />
        <span className="kanban-plan-mono">
          {release.done} of {release.total} done
        </span>
      </div>
      <ColumnTiles columns={columns} counts={release.counts} />
    </section>
  )

  const byProject = (
    <Panel aria-labelledby={`${uid}-projects`} role="region">
      <PanelHeader>
        <h3 className="kanban-plan-panelh" id={`${uid}-projects`}>
          By project
        </h3>
      </PanelHeader>
      <PanelBody className="kanban-plan-kvs">
        {release.projects.length === 0 ? <p className="kanban-plan-faint">No tickets in this release yet.</p> : null}
        {release.projects.map((row) => (
          <div className="kanban-plan-split" key={row.project_id ?? 'none'}>
            <div className="kanban-plan-split__top">
              <span className="kanban-plan-split__name">{projectName(row.project_id)}</span>
              <span className="kanban-plan-mono">{row.total}</span>
            </div>
            <span className="kanban-plan-faint">
              {row.done} done · {row.total - row.done} not done
            </span>
          </div>
        ))}
        {release.projects.length > 0 && absent.length > 0 ? (
          <p className="kanban-plan-faint">
            {nameList(absent.map((project) => project.name))} {absent.length === 1 ? 'has' : 'have'} no tickets in this
            release.
          </p>
        ) : null}
      </PanelBody>
    </Panel>
  )

  // Both lists are panels: the same surface, the Done one folded until asked for.
  const lists = (
    <>
      {open.length > 0 ? (
        <Panel aria-labelledby={`${uid}-open`} role="region">
          <PanelHeader>
            <h3 className="kanban-plan-panelh" id={`${uid}-open`}>
              Not done
            </h3>
            <span className="kanban-plan-count">{open.length}</span>
          </PanelHeader>
          <TicketRows
            agents={agents}
            canOpenPane={canOpenPane}
            columns={columns}
            host={host}
            label="Not done"
            tickets={open}
          />
        </Panel>
      ) : null}
      {done.length > 0 ? (
        <Panel aria-label="Done" role="region">
          <button
            aria-expanded={showDone}
            aria-label={`${showDone ? 'Hide' : 'Show'} the ${done.length} done tickets`}
            className={`${uiClasses.panelHeader} kanban-plan-collapse__trigger kanban-plan-touch`}
            onClick={() => setShowDone((now) => !now)}
            type="button"
          >
            <ChevronRight className="kanban-plan-collapse__chevron" size={16} />
            <span>Done</span>
            <span className="kanban-plan-count">{done.length}</span>
          </button>
          {showDone ? (
            <TicketRows
              agents={agents}
              canOpenPane={canOpenPane}
              columns={columns}
              host={host}
              label="Done"
              tickets={done}
            />
          ) : null}
        </Panel>
      ) : null}
    </>
  )

  if (narrow) {
    // The target right under the title, the numbers, who it is for, what can be done; the tickets come last.
    return (
      <div className="kanban-plan-main">
        <div className="kanban-plan-stack">
          {head}
          <div className="kanban-plan-kvs kanban-plan-kvs--flat">
            {target}
            {shippedOn}
          </div>
          {nudge}
          {progress}
          {byProject}
          <div className="kanban-plan-actions">{actions}</div>
          <Panel aria-labelledby={`${uid}-description`} role="region">
            <PanelHeader>
              <h3 className="kanban-plan-panelh" id={`${uid}-description`}>
                Description
              </h3>
              {editDescription}
            </PanelHeader>
            <PanelBody className="kanban-plan-kvs">{description}</PanelBody>
          </Panel>
          {lists}
        </div>
        {dialog}
      </div>
    )
  }

  return (
    <div className="kanban-plan-main">
      <div className="kanban-plan-detail">
        <div className="kanban-plan-detail__main">
          {head}
          {nudge}
          {progress}
          {lists}
        </div>

        <aside aria-label="Release details" className="kanban-plan-detail__aside">
          <Panel aria-labelledby={`${uid}-details`} role="region">
            <PanelHeader>
              <h3 className="kanban-plan-panelh" id={`${uid}-details`}>
                Details
              </h3>
            </PanelHeader>
            <PanelBody className="kanban-plan-kvs">
              {target}
              {shippedOn}
              <div className="kanban-plan-kv kanban-plan-kv--desc">
                <span className="kanban-plan-k">Description</span>
                {editDescription ?? <span />}
              </div>
              {description}
            </PanelBody>
          </Panel>
          {byProject}
        </aside>
      </div>
      {dialog}
    </div>
  )
}

/** Tickets of one list: the first few, then the rest on request. Each opens its ticket in a pane. */
function TicketRows({
  host,
  tickets,
  columns,
  agents,
  canOpenPane,
  label,
}: {
  host: Host
  tickets: TicketSummary[]
  columns: BoardData['columns']
  agents: AgentProfile[]
  canOpenPane: boolean
  label: string
}) {
  const [all, setAll] = useState(false)
  const shown = all ? tickets : tickets.slice(0, 6)
  return (
    <div className="kanban-plan-rows">
      <ul aria-label={label} className="kanban-plan-rows__list">
        {shown.map((ticket) => {
          const agent = agents.find((candidate) => candidate.id === ticket.assignee)
          const assignee = agent?.name ?? ticket.assignee
          return (
            <li key={ticket.id}>
              <button
                className="kanban-plan-trow kanban-plan-touch"
                disabled={!canOpenPane}
                onClick={() => openPane(host, TICKET_PAGE_ID, { id: ticket.key })}
                type="button"
              >
                <StatusGlyph columns={columns} status={ticket.status} />
                <span className="kanban-plan-mono kanban-plan-trow__key">{ticket.key}</span>
                <span className="kanban-plan-trow__title">{ticket.title}</span>
                {assignee ? (
                  <span className="kanban-plan-trow__who">
                    <span className={uiClasses.treeItemIcon} data-color={agent?.color ?? undefined}>
                      <Bot size={16} />
                    </span>
                    <span className="kanban-plan-trow__name">{assignee}</span>
                  </span>
                ) : null}
              </button>
            </li>
          )
        })}
      </ul>
      {tickets.length > shown.length ? (
        <Button
          className="kanban-plan-touch kanban-plan-rows__more"
          onClick={() => setAll(true)}
          size="sm"
          variant="ghost"
        >
          Show {tickets.length - shown.length} more
        </Button>
      ) : null}
    </div>
  )
}
