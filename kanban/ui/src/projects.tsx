import {
  Button,
  Chip,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
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
  Select,
  Selector,
  Skeleton,
  StatusPanel,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import {
  Archive,
  ArrowUpRight,
  ChevronRight,
  Columns3,
  Folder,
  ListFilter,
  MoreHorizontal,
  Pencil,
  Plus,
  RotateCcw,
  Tag,
  TriangleAlert,
} from 'lucide-react'
import { type ReactNode, useEffect, useId, useMemo, useRef, useState } from 'react'
import {
  agentOptions,
  ColumnTiles,
  ConvertDialog,
  DrillBar,
  NewProjectDialog,
  openPane,
  ProgressBar,
  ProjectGlyph,
  StatusBadge,
  TONES,
  takePanelRequest,
  titleCase,
  useAgents,
  useInlineEdit,
  usePlanBoard,
} from './plan-dialogs'
import { groupProjects, notDone, plural, resolveEntity, topLabels } from './plan-rules'
import {
  type AgentProfile,
  announcePlanChange,
  api,
  BOARD_PAGE_ID,
  type BoardData,
  type GlyphTone,
  type ProjectView,
  RELEASES_PAGE_ID,
  type TicketSummary,
} from './shared'

export function ProjectsPage({
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
  const [stored, setStored] = usePaneState<string | null>(`kanban:project:${paneKey}`, null)
  /** Narrow panes show the list or one project, never both. */
  const [drilled, setDrilled] = useState(false)
  const [creating, setCreating] = useState(false)
  const [converting, setConverting] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const seq = useRef(0)
  const announce = (text: string) => setAnnouncement({ seq: ++seq.current, text, urgency: 'polite' })

  const projects = useMemo(() => data?.projects ?? [], [data])
  const groups = useMemo(() => groupProjects(projects), [projects])
  const tickets = useMemo(() => (data?.columns ?? []).flatMap((column) => column.tickets), [data])
  const chosen = resolveEntity(projects, stored)
  const fallback = narrow ? undefined : (groups.active[0] ?? groups.archived[0])
  const selected = chosen ?? fallback
  const detailShown = Boolean(selected) && (!narrow || drilled)
  const canOpenPane = Boolean(host.panels?.open)

  const pick = (project: ProjectView) => {
    setStored(project.id)
    setDrilled(true)
  }

  // Other screens open a project (or a dialog) through `host.panels.open({ pageId, context })`.
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

  // What is on screen is remembered by id: a project reached by name survives a rename, and the first one shown
  // stays put when archiving moves it to another group.
  // (A ref that is not loaded yet, a release just created, is never replaced by the first one.)
  const pinned = chosen?.id ?? (stored ? undefined : fallback?.id)
  useEffect(() => {
    if (pinned && pinned !== stored) setStored(pinned)
  }, [pinned, stored, setStored])

  useEffect(
    () => commands?.register([{ id: 'refresh', title: 'Refresh projects', run: () => void reload() }]),
    [commands, reload],
  )

  const withoutProject = tickets.filter((ticket) => !ticket.project_id).length

  const header = (
    <PageHeader
      actions={
        narrow ? (
          // One project open: the way back is the toolbar under the header, and nothing is created from here.
          detailShown ? null : (
            <IconButton
              className="kanban-plan-touch"
              label="New project"
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
              New project
            </Button>
          </>
        )
      }
      description={data ? `${plural(projects.length, 'project')} · ${data.id_prefix}` : undefined}
      icon={<Folder size={16} />}
      onClose={onRequestClose}
      title="Projects"
    />
  )

  const row = (project: ProjectView) => {
    const current = !narrow && selected?.id === project.id
    return (
      <ListItem
        aria-current={current ? 'true' : undefined}
        key={project.id}
        label={project.name}
        leading={<ProjectGlyph color={project.color} />}
        onClick={() => pick(project)}
        selected={current}
        trailing={
          <span className="kanban-plan-mini">
            <span className="kanban-plan-mono">
              {project.done}/{project.total}
            </span>
            {narrow ? <ChevronRight size={16} /> : null}
          </span>
        }
      />
    )
  }

  const rail = (
    <PageSidebar label="Projects" narrow={narrow} width={300}>
      <nav aria-label="Projects" className="kanban-plan-nav">
        <List>
          {groups.active.length > 0 ? (
            <ListGroup aria-label="Active">
              <Eyebrow as="div" className="kanban-plan-eyebrow">
                Active
              </Eyebrow>
              {groups.active.map(row)}
            </ListGroup>
          ) : null}
          {groups.archived.length > 0 ? (
            <ListGroup aria-label="Archived">
              <Eyebrow as="div" className="kanban-plan-eyebrow">
                Archived
              </Eyebrow>
              {groups.archived.map(row)}
            </ListGroup>
          ) : null}
          {canOpenPane ? (
            <ListGroup aria-label="Board filters">
              <Eyebrow as="div" className="kanban-plan-eyebrow">
                Board filters
              </Eyebrow>
              <ListItem
                label="No project"
                leading={<ListFilter size={16} />}
                onClick={() => openPane(host, BOARD_PAGE_ID, { group: 'project', project: '' })}
                trailing={
                  <span className="kanban-plan-mini">
                    <span className="kanban-plan-mono">{withoutProject}</span>
                    <ArrowUpRight size={16} />
                  </span>
                }
              />
            </ListGroup>
          ) : null}
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
            headline="The projects could not be read"
            icon={<TriangleAlert size={16} />}
            role="alert"
            variant="alert"
          />
        </div>
      </PageMain>
    ) : (
      <>
        <PageSidebar label="Projects" narrow={narrow} width={300}>
          <div aria-busy={loading} className="kanban-plan-nav">
            {[0, 1, 2].map((index) => (
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
  } else if (projects.length === 0) {
    body = (
      <PageMain>
        <div className="kanban-plan-empty">
          <EmptyState
            action={{ label: 'Create from a label…', onClick: () => setConverting(true) }}
            description="Group tickets by what they belong to. You can start from a label like iii-cloud."
            icon={Folder}
            title="No projects yet"
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
            {narrow ? <DrillBar name={selected.name} onBack={() => setDrilled(false)} root="Projects" /> : null}
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
              <ProjectDetail
                agents={agents}
                announce={announce}
                canOpenPane={canOpenPane}
                data={data}
                host={host}
                key={selected.id}
                onError={setActionError}
                project={selected}
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

      <NewProjectDialog
        agents={agents}
        host={host}
        onCreated={(project) => {
          setCreating(false)
          setStored(project.id)
          setDrilled(true)
          announce(`Created ${project.name}`)
          void reload(true)
        }}
        onOpenChange={setCreating}
        open={creating}
      />
      {data ? (
        <ConvertDialog
          data={data}
          host={host}
          into="project"
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
      <LiveRegion announcement={announcement} />
    </PageShell>
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
}) {
  const uid = useId()
  const { confirm, dialog } = useConfirm()
  const archived = project.status === 'archived'
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
    await api.updateProject(host.iii, { id: project.id, ...changes })
    announcePlanChange()
    await reload(true)
  }
  /** A selector change has no field to show an error in: it goes to the page. */
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

  const archive = () =>
    guarded(async () => {
      const open = notDone(project)
      const ok = await confirm({
        title: `Archive ${project.name}?`,
        description: `${project.name} moves to Archived. Nothing is deleted.`,
        details: [
          ...(open > 0
            ? [`${plural(open, 'open ticket')} keep${open === 1 ? 's' : ''} the project and stay on the board`]
            : []),
          'Archived projects take no new tickets',
          'You can restore it from Projects › Archived',
        ],
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

  const lead = project.lead ?? undefined

  return (
    <div className="kanban-plan-main">
      <div className="kanban-plan-stack">
        <header className="kanban-plan-head">
          <div className="kanban-plan-head__id">
            <div className="kanban-plan-head__line">
              {rename.editing ? (
                rename.editor
              ) : (
                <>
                  <h2 className="kanban-plan-title">{project.name}</h2>
                  <IconButton
                    className="kanban-plan-touch"
                    label={`Rename ${project.name}`}
                    onClick={rename.start}
                    variant="ghost"
                  >
                    <Pencil size={16} />
                  </IconButton>
                </>
              )}
              <StatusBadge
                title={archived ? 'Derived: archived' : 'Derived: not archived'}
                tone={archived ? 'ghost' : 'ink'}
              >
                {archived ? 'Archived' : 'Active'}
              </StatusBadge>
            </div>
            {project.source_label ? (
              <span className="kanban-plan-sub">
                Created from the label <span className="kanban-plan-mono">{project.source_label}</span>
              </span>
            ) : null}
          </div>
          <div className="kanban-plan-head__actions">
            {canOpenPane ? (
              <Button
                className="kanban-plan-touch"
                onClick={() => openPane(host, BOARD_PAGE_ID, { group: 'project', project: project.id })}
                variant="pill"
              >
                <Columns3 size={16} />
                Open on the board
              </Button>
            ) : null}
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <IconButton className="kanban-plan-touch" label={`More actions for ${project.name}`} variant="ghost">
                  <MoreHorizontal size={16} />
                </IconButton>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={rename.start}>
                  <Pencil size={16} />
                  Rename…
                </DropdownMenuItem>
                {archived ? (
                  <DropdownMenuItem disabled={busy} onSelect={() => void restore()}>
                    <RotateCcw size={16} />
                    Restore
                  </DropdownMenuItem>
                ) : (
                  <DropdownMenuItem disabled={busy} onSelect={() => void archive()}>
                    <Archive size={16} />
                    Archive…
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </header>

        {archived ? (
          <StatusPanel
            action={
              <Button className="kanban-plan-touch" disabled={busy} onClick={() => void restore()} variant="pill">
                Restore
              </Button>
            }
            detail="It takes no new tickets. The tickets it has keep it."
            headline={`${project.name} is archived`}
            icon={<Archive size={16} />}
            variant="info"
          />
        ) : null}

        <section aria-label="Progress" className="kanban-plan-progress">
          <div className="kanban-plan-progress__line kanban-plan-progress__line--lead">
            <span className="kanban-plan-mono">
              {project.done} of {project.total} done
            </span>
            <ProgressBar done={project.done} total={project.total} />
          </div>
          <ColumnTiles
            canceledStatus={data.canceled_status}
            columns={data.columns}
            counts={project.counts}
            doneStatus={data.done_status}
            extra={
              <div className="kanban-plan-tile kanban-plan-tile--gap">
                <span className="kanban-plan-tile__label">Unassigned</span>
                <span className="kanban-plan-tile__n">{unassigned}</span>
              </div>
            }
          />
        </section>

        <div className="kanban-plan-pair kanban-plan-pair--panels">
          <Panel aria-labelledby={`${uid}-details`} role="region">
            <PanelHeader>
              <h3 className="kanban-plan-panelh" id={`${uid}-details`}>
                Details
              </h3>
            </PanelHeader>
            <PanelBody className="kanban-plan-kvs">
              <div className="kanban-plan-srow">
                <span className="kanban-plan-k" id={`${uid}-lead`}>
                  Lead
                </span>
                <Selector
                  allowEmpty
                  aria-label="Lead"
                  className="kanban-plan-control"
                  emptyLabel="None"
                  onChange={(next) => void patchQuietly({ lead: next })}
                  onClear={() => void patchQuietly({ lead: null })}
                  options={agentOptions(agents, project.lead)}
                  placeholder="None — set a lead"
                  value={lead}
                />
              </div>
              <div className="kanban-plan-srow">
                <span className="kanban-plan-k" id={`${uid}-color`}>
                  Color
                </span>
                <span className="kanban-plan-colorrow">
                  <ProjectGlyph color={project.color} />
                  <Select<GlyphTone>
                    allowEmpty
                    aria-label="Color"
                    className="kanban-plan-control"
                    emptyLabel="No color"
                    onChange={(next) => void patchQuietly({ color: next })}
                    onClear={() => void patchQuietly({ color: null })}
                    options={TONES.map((tone) => ({ value: tone, label: titleCase(tone) }))}
                    placeholder="No color"
                    value={project.color ?? undefined}
                  />
                </span>
              </div>
              <div className="kanban-plan-kv kanban-plan-kv--desc">
                <span className="kanban-plan-k">Description</span>
                {project.description.trim() && !describe.editing ? (
                  <IconButton
                    className="kanban-plan-touch"
                    label="Edit the description"
                    onClick={describe.start}
                    variant="ghost"
                  >
                    <Pencil size={16} />
                  </IconButton>
                ) : (
                  <span />
                )}
              </div>
              {describe.editing ? (
                describe.editor
              ) : project.description.trim() ? (
                <p className="kanban-plan-desc">{project.description}</p>
              ) : (
                <button className="kanban-plan-link kanban-plan-touch" onClick={describe.start} type="button">
                  Add a description
                </button>
              )}
            </PanelBody>
          </Panel>

          <Panel aria-labelledby={`${uid}-releases`} role="region">
            <PanelHeader>
              <h3 className="kanban-plan-panelh" id={`${uid}-releases`}>
                Releases
              </h3>
            </PanelHeader>
            {inAnyRelease ? (
              <PanelBody className="kanban-plan-kvs">
                {releaseRows.map((row) => {
                  const body = (
                    <>
                      <span className="kanban-plan-split__name">{row.name}</span>
                      <span className="kanban-plan-mono">
                        {row.done}/{row.total}
                      </span>
                      <ProgressBar done={row.done} total={row.total} />
                    </>
                  )
                  return row.id && canOpenPane ? (
                    <button
                      className="kanban-plan-relrow kanban-plan-touch"
                      key={row.id}
                      onClick={() => openPane(host, RELEASES_PAGE_ID, { id: row.id as string })}
                      type="button"
                    >
                      {body}
                    </button>
                  ) : (
                    <div className="kanban-plan-relrow" key={row.id ?? 'none'}>
                      {body}
                    </div>
                  )
                })}
              </PanelBody>
            ) : (
              <EmptyState
                action={
                  canOpenPane
                    ? {
                        label: 'Open in List',
                        onClick: () => openPane(host, BOARD_PAGE_ID, { layout: 'list', project: project.id }),
                      }
                    : undefined
                }
                compact
                description="Add them to a release in bulk from the List layout."
                icon={Folder}
                title="No ticket of this project is in a release"
              />
            )}
          </Panel>
        </div>

        {labels.length > 0 ? (
          <Panel aria-labelledby={`${uid}-labels`} role="region">
            <PanelHeader>
              <h3 className="kanban-plan-panelh" id={`${uid}-labels`}>
                Most used labels
              </h3>
              {project.source_label ? (
                <span className="kanban-plan-hint kanban-plan-panelh__end">
                  Besides <span className="kanban-plan-mono">{project.source_label}</span>
                </span>
              ) : null}
            </PanelHeader>
            <PanelBody className="kanban-plan-chips">
              {labels.map((entry) => (
                <Chip key={entry.label}>
                  {entry.label} <span className="kanban-plan-mono kanban-plan-chip__n">{entry.count}</span>
                </Chip>
              ))}
            </PanelBody>
          </Panel>
        ) : null}
      </div>
      {dialog}
    </div>
  )
}
