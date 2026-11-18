import { type Host, ShadcnButton, ShadcnSkeleton, ShadcnSwitch } from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import {
  Bot,
  Check,
  ChevronDown,
  ChevronsDownUp,
  ChevronsUpDown,
  CircleDashed,
  ClipboardCheck,
  Code2,
  Database,
  FileText,
  FlaskConical,
  type LucideIcon,
  Maximize2,
  Minus,
  Network,
  Palette,
  Plus,
  RefreshCw,
  Search,
  SquareKanban,
  Terminal,
  TriangleAlert,
  X,
} from 'lucide-react'
import {
  type CSSProperties,
  type KeyboardEvent,
  memo,
  type ReactNode,
  useCallback,
  useMemo,
  useRef,
  useState,
} from 'react'
import { type StatusStep, statusStep } from '../board-state'
import type { AgentProfile, BoardData, GlyphTone, ProjectView, TicketSummary } from '../shared'
import { assigneeName } from '../ui/avatar'
import { Empty, Notice } from '../ui/feedback'
import { ColumnStatusIcon } from '../ui/glyphs'
import { IconAction } from '../ui/icon-action'
import {
  buildTree,
  childrenOfNode,
  formatSpan,
  modelName,
  type ProjectTree,
  type SessionNode,
  type TicketNode,
  type TreeNode,
  type WorkState,
  walk,
  withoutDone,
} from './agent-tree'
import { useAgentSessions } from './use-agent-sessions'
import { MAX_SCALE, MIN_SCALE, useCamera } from './use-camera'
import { useChartMotion } from './use-chart-motion'

/** The chart's root (the project, or every project) and, under "every project", one per project. */
type GroupNode = {
  kind: 'group'
  id: string
  name: string
  color: GlyphTone | null
  /** The project it stands for; null for "All projects" and "No project". */
  project: ProjectView | null
  children: Node[]
}
type Node = TreeNode | GroupNode

const kidsOf = (node: Node): Node[] => (node.kind === 'group' ? node.children : childrenOfNode(node))

/**
 * How a node lays out its children, the reference chart's way: down the middle while the chart has a single line
 * (project › feature ticket), across in a row at the first fan-out (the orchestrator's agents, the feature's
 * subtasks), and stacked under each column below that (an agent's phases).
 */
type Layout = 'single' | 'row' | 'stack'
const layoutOf = (count: number, spine: boolean): Layout => (!spine ? 'stack' : count === 1 ? 'single' : 'row')

const SESSION_ICONS: Record<string, LucideIcon> = {
  agent: Bot,
  code: Code2,
  search: Search,
  terminal: Terminal,
  database: Database,
  test: FlaskConical,
  review: ClipboardCheck,
  docs: FileText,
  design: Palette,
}

const STATE_LABEL: Record<WorkState, string> = {
  active: 'Running',
  pending: 'Pending',
  done: 'Done',
  failed: 'Failed',
  canceled: 'Canceled',
}

const MUTED = 'tw:text-neutral-500 tw:dark:text-neutral-400'
const SUBBAR =
  'tw:flex tw:min-h-11 tw:shrink-0 tw:items-center tw:gap-2 tw:bg-neutral-50 tw:px-6 tw:py-1.5 tw:dark:bg-black'

export type TreeViewProps = {
  host: Host
  data: BoardData
  /** The project the hub is on; null = every project. */
  project: ProjectView | null
  agents: AgentProfile[]
  paneKey: string
  /** A ticket's details, in the hub's ticket dialog. */
  onOpenTicket: (key: string) => void
  /** Pick a project from the "every project" chart. */
  onOpenProject: (id: string) => void
}

/**
 * Tree: the project as a chart of its work — its tickets and subtasks, and under each ticket the agent sessions
 * that worked it, with the sessions those spawned. Live: the board's events redraw the tickets, session-manager's
 * the sessions.
 */
export function TreeView({ host, data, project, agents, paneKey, onOpenTicket, onOpenProject }: TreeViewProps) {
  const { sessions, loading, error, reload } = useAgentSessions(host.iii, true)
  const [hideDone, setHideDone] = usePaneState<boolean>(`kanban:tree-hide-done:${paneKey}`, false)
  // Nodes the user opened or closed; the rest follow `defaultOpen`.
  const [toggled, setToggled] = usePaneState<Record<string, boolean>>(`kanban:tree-open:${paneKey}`, {})
  const [activeId, setActiveId] = useState<string | null>(null)
  const chartRef = useRef<HTMLDivElement | null>(null)
  const now = Date.now()

  const tickets = useMemo<TicketSummary[]>(() => data.columns.flatMap((column) => column.tickets), [data])
  const tree: ProjectTree = useMemo(
    () =>
      buildTree({
        tickets,
        sessions,
        prefix: data.id_prefix,
        columns: data.columns,
        doneStatus: data.done_status,
        canceledStatus: data.canceled_status,
        inScope: project ? (ticket) => ticket.project_id === project.id : undefined,
      }),
    [tickets, sessions, data, project],
  )

  /** One root: the project on screen, or "All projects" over one node per project. */
  const root: GroupNode | null = useMemo(() => {
    const shown = hideDone ? withoutDone(tree.roots) : tree.roots
    if (shown.length === 0) return null
    if (project) {
      return {
        kind: 'group',
        id: `project:${project.id}`,
        name: project.name,
        color: project.color,
        project,
        children: shown,
      }
    }
    const groups = new Map<string | null, TicketNode[]>()
    for (const node of shown) {
      const id =
        node.ticket.project_id && data.projects.some((p) => p.id === node.ticket.project_id)
          ? node.ticket.project_id
          : null
      groups.set(id, [...(groups.get(id) ?? []), node])
    }
    const children: GroupNode[] = []
    for (const p of data.projects) {
      const nodes = groups.get(p.id)
      if (nodes?.length)
        children.push({
          kind: 'group',
          id: `project:${p.id}`,
          name: p.name,
          color: p.color,
          project: p,
          children: nodes,
        })
    }
    const loose = groups.get(null)
    if (loose?.length)
      children.push({
        kind: 'group',
        id: 'project:none',
        name: 'No project',
        color: null,
        project: null,
        children: loose,
      })
    return { kind: 'group', id: 'all', name: 'All projects', color: null, project: null, children }
  }, [tree, hideDone, project, data.projects])

  /**
   * Open by default: the groups, anything not finished, and anything with running or failed work under it (a
   * finished orchestrator whose sub-agent failed must not hide that). A finished subtree stays folded until asked.
   */
  const needsEyes = useMemo(() => {
    const ids = new Set<string>()
    const visit = (node: Node): boolean => {
      let flagged = node.kind !== 'group' && (node.state === 'active' || node.state === 'failed')
      for (const kid of kidsOf(node)) if (visit(kid)) flagged = true
      if (flagged) ids.add(node.id)
      return flagged
    }
    if (root) visit(root)
    return ids
  }, [root])
  // Decided once per node, when it first appears: work that finishes while you watch stays open (it never folds
  // itself away under you); a subtree that was already finished when the chart opened starts folded.
  const firstSeen = useRef(new Map<string, boolean>())
  const sessionsPending = loading && sessions.length === 0 && !error
  const scopeKey = project?.id ?? 'all'
  const seenFor = useRef(scopeKey)
  if (seenFor.current !== scopeKey) {
    seenFor.current = scopeKey
    firstSeen.current = new Map()
  }
  const defaultOpen = useCallback(
    (node: Node) => {
      const known = firstSeen.current.get(node.id)
      if (known !== undefined) return known
      const open =
        node.kind === 'group' || needsEyes.has(node.id) || (node.state !== 'done' && node.state !== 'canceled')
      // Not before the sessions are read: a failed agent under a finished ticket must still open it.
      if (!sessionsPending) firstSeen.current.set(node.id, open)
      return open
    },
    [needsEyes, sessionsPending],
  )
  const isOpen = useCallback(
    (node: Node) => (toggled && typeof toggled === 'object' ? toggled[node.id] : undefined) ?? defaultOpen(node),
    [toggled, defaultOpen],
  )
  const camera = useCamera(paneKey)
  /** The card a fold was asked on: it stays where it is while the layout moves (use-chart-motion). */
  const anchor = useRef<string | null>(null)
  const holdStill = (id: string) => {
    anchor.current = id
  }
  useChartMotion({ camera, chart: chartRef, anchor, chartKey: root?.id ?? null })
  const setOpen = useCallback(
    (node: Node, open: boolean) => {
      holdStill(node.id)
      setToggled({ ...(toggled ?? {}), [node.id]: open })
    },
    [toggled, setToggled],
  )
  const setAll = (open: boolean) => {
    if (!root) return
    const next: Record<string, boolean> = {}
    const visit = (node: Node) => {
      // The root stays open: collapsing it would leave one card.
      if (kidsOf(node).length > 0 && node !== root) next[node.id] = open
      kidsOf(node).forEach(visit)
    }
    visit(root)
    holdStill(root.id)
    setToggled(next)
  }

  /** The cards on screen, depth first: what the arrow keys move through. */
  const visible = useMemo(() => {
    const rows: { node: Node; parent: Node | null }[] = []
    const visit = (node: Node, parent: Node | null) => {
      rows.push({ node, parent })
      if (isOpen(node)) for (const kid of kidsOf(node)) visit(kid, node)
    }
    if (root) visit(root, null)
    return rows
  }, [root, isOpen])
  const focusedId = visible.some((row) => row.node.id === activeId) ? activeId : (visible[0]?.node.id ?? null)

  const focusRow = (id: string) => {
    setActiveId(id)
    const el = chartRef.current?.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(id)}"]`)
    el?.focus({ preventScroll: true })
    camera.reveal(el?.querySelector('.kb-node') ?? null)
  }

  const activate = (node: Node) => {
    if (node.kind === 'ticket') onOpenTicket(node.ticket.key)
    else if (node.kind === 'session') openSession(host, node.session.id)
    else if (node.project && node.project.id !== project?.id) onOpenProject(node.project.id)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = visible.findIndex((row) => row.node.id === focusedId)
    if (index < 0) return
    const { node, parent } = visible[index]
    const kids = kidsOf(node)
    const go = (next: number) => {
      const row = visible[Math.max(0, Math.min(visible.length - 1, next))]
      if (row) focusRow(row.node.id)
    }
    switch (event.key) {
      case 'ArrowDown':
        go(index + 1)
        break
      case 'ArrowUp':
        go(index - 1)
        break
      case 'Home':
        go(0)
        break
      case 'End':
        go(visible.length - 1)
        break
      case 'ArrowRight':
        if (kids.length === 0) return
        if (!isOpen(node)) setOpen(node, true)
        else focusRow(kids[0].id)
        break
      case 'ArrowLeft':
        if (kids.length > 0 && isOpen(node) && node !== root) setOpen(node, false)
        else if (parent) focusRow(parent.id)
        else return
        break
      case 'Enter':
        activate(node)
        break
      case '*': {
        // Open every sibling of the focused card (WAI-ARIA tree).
        const siblings = parent ? kidsOf(parent) : []
        setToggled({ ...(toggled ?? {}), ...Object.fromEntries(siblings.map((sibling) => [sibling.id, true])) })
        break
      }
      default:
        return
    }
    event.preventDefault()
  }

  const { stats } = tree
  const summary = [
    `${stats.tickets} ${stats.tickets === 1 ? 'ticket' : 'tickets'}`,
    `${stats.sessions} ${stats.sessions === 1 ? 'agent' : 'agents'}`,
  ]

  let body: ReactNode
  /** The chart itself (not a skeleton or an empty state): it goes on the camera's stage. */
  let charted = false
  if (loading && sessions.length === 0 && !error) {
    body = <ChartSkeleton />
  } else if (tree.roots.length === 0) {
    body = (
      <Empty
        description={
          project
            ? 'Tickets filed under this project, and the agents working them, show here.'
            : 'Tickets and the agents working them show here.'
        }
        icon={Network}
        title={project ? 'No tickets in this project yet' : 'No tickets yet'}
      />
    )
  } else if (!root) {
    body = (
      <Empty
        action={
          <ShadcnButton onClick={() => setHideDone(false)} size="sm" type="button" variant="outline">
            Show done
          </ShadcnButton>
        }
        description="Every ticket here is done or canceled."
        icon={Check}
        title="Nothing in progress"
      />
    )
  } else {
    charted = true
    body = (
      <div
        aria-label={project ? `${project.name}: tickets and agents` : 'Projects, tickets and agents'}
        className="kb-chart"
        onKeyDown={onKeyDown}
        ref={chartRef}
        role="tree"
      >
        <ChartItem
          agents={agents}
          data={data}
          focusedId={focusedId}
          isOpen={isOpen}
          level={1}
          node={root}
          now={now}
          onActivate={activate}
          onFocusRow={setActiveId}
          onToggle={setOpen}
          position={1}
          setSize={1}
          spine
          stats={tree.stats}
        />
      </div>
    )
  }

  return (
    <div className="kb-tree-view tw:bg-neutral-100 tw:dark:bg-black">
      <div aria-label="Tree controls" className={SUBBAR} role="toolbar">
        <span className={`tw:flex tw:min-w-0 tw:items-center tw:gap-3 tw:text-[13px] ${MUTED}`}>
          <span className="tw:truncate">{summary.join(' · ')}</span>
          {stats.running > 0 ? (
            <span className="tw:shrink-0 tw:font-medium tw:text-glyph-blue">{stats.running} running</span>
          ) : null}
          {stats.failed > 0 ? (
            <span className="tw:shrink-0 tw:font-medium tw:text-alert">{stats.failed} failed</span>
          ) : null}
        </span>
        <span className="tw:flex-1" />
        {/* biome-ignore lint/a11y/noLabelWithoutControl: the switch inside is the control */}
        <label className={`tw:flex tw:shrink-0 tw:cursor-pointer tw:items-center tw:gap-2 tw:text-[13px] ${MUTED}`}>
          <ShadcnSwitch checked={Boolean(hideDone)} onCheckedChange={(checked: boolean) => setHideDone(checked)} />
          Hide done
        </label>
        {/* biome-ignore lint/a11y/useSemanticElements: a fieldset would bring its own border and legend into a toolbar */}
        <span aria-label="Zoom" className="kb-zoom" role="group">
          <IconAction disabled={camera.scale <= MIN_SCALE + 0.001} label="Zoom out" onClick={camera.zoomOut}>
            <Minus size={16} />
          </IconAction>
          <button className="kb-zoom__value" onClick={camera.actualSize} title="Back to 100%" type="button">
            {Math.round(camera.scale * 100)}%
          </button>
          <IconAction disabled={camera.scale >= MAX_SCALE - 0.001} label="Zoom in" onClick={camera.zoomIn}>
            <Plus size={16} />
          </IconAction>
          <IconAction
            className={camera.follow ? 'kb-zoom__fit' : undefined}
            label={camera.follow ? 'Fitting the chart as it grows' : 'Fit the chart'}
            onClick={camera.fit}
          >
            <Maximize2 size={16} />
          </IconAction>
        </span>
        <IconAction label="Expand all" onClick={() => setAll(true)}>
          <ChevronsUpDown size={16} />
        </IconAction>
        <IconAction label="Collapse all" onClick={() => setAll(false)}>
          <ChevronsDownUp size={16} />
        </IconAction>
        <IconAction label="Refresh agents" onClick={() => void reload()}>
          <RefreshCw size={16} />
        </IconAction>
      </div>
      {error ? (
        <Notice
          actionLabel="Retry"
          detail={`The tickets still show; their agent sessions do not. ${error}`}
          onAction={() => void reload()}
          title="Agent sessions could not be read"
        />
      ) : null}
      {charted && root ? (
        <div
          className="kb-chart-scroll"
          data-panning={camera.panning ? 'true' : undefined}
          ref={camera.viewportRef}
          {...camera.handlers}
        >
          <div className="kb-chart-stage" key={root.id} ref={camera.stageRef}>
            {body}
          </div>
        </div>
      ) : (
        <div className="kb-chart-empty">{body}</div>
      )}
    </div>
  )
}

/** Open a session's chat beside the board (`chat:<id>` reuses its pane when it is already open). */
function openSession(host: Host, sessionId: string) {
  host.panels?.openScreen?.({ screen: `chat:${sessionId}` })
}

type ItemProps = {
  node: Node
  level: number
  position: number
  setSize: number
  /** On the chart's single line from the root: its children may run down the middle or across. */
  spine: boolean
  data: BoardData
  agents: AgentProfile[]
  stats: ProjectTree['stats']
  now: number
  focusedId: string | null
  isOpen: (node: Node) => boolean
  onToggle: (node: Node, open: boolean) => void
  onActivate: (node: Node) => void
  onFocusRow: (id: string) => void
  /** Its place among its siblings: the stagger of its entrance. */
  enterIndex?: number
}

/** One node: its card, then (open) its children laid out by `layoutOf`, hung off the card by the connectors. */
const ChartItem = memo(function ChartItem(props: ItemProps) {
  const { node, level, position, setSize, spine, data, focusedId, isOpen, onToggle, onActivate, onFocusRow } = props
  const kids = kidsOf(node)
  const open = kids.length > 0 && isOpen(node)
  const layout = layoutOf(kids.length, spine)
  const toggle = () => onToggle(node, !open)
  const name =
    node.kind === 'group'
      ? node.name
      : node.kind === 'ticket'
        ? `${node.ticket.key} ${node.ticket.title}`
        : node.inferred
          ? `Orchestrator: ${node.session.name}`
          : node.session.name
  const state =
    node.kind === 'group'
      ? 'project'
      : node.kind === 'ticket'
        ? (data.columns.find((column) => column.id === node.ticket.status)?.label ?? node.ticket.status)
        : STATE_LABEL[node.state]
  return (
    <div
      aria-expanded={kids.length > 0 ? open : undefined}
      aria-label={`${name}, ${state}`}
      aria-level={level}
      aria-posinset={position}
      aria-selected={false}
      aria-setsize={setSize}
      className="kb-chart__item"
      style={props.enterIndex ? ({ '--i': props.enterIndex } as CSSProperties) : undefined}
      data-kids={open ? layout : undefined}
      data-node-id={node.id}
      data-spine={spine ? 'true' : undefined}
      data-state={node.kind === 'group' ? undefined : node.state}
      onFocus={(event) => {
        if (event.target === event.currentTarget) onFocusRow(node.id)
      }}
      role="treeitem"
      tabIndex={focusedId === node.id ? 0 : -1}
    >
      {/* biome-ignore lint/a11y/useKeyWithClickEvents lint/a11y/noStaticElementInteractions: the treeitem takes the keys (Enter) */}
      <div
        className="kb-chart__card"
        onClick={() => {
          onFocusRow(node.id)
          onActivate(node)
        }}
      >
        {node.kind === 'group' ? (
          <GroupCard {...props} node={node} onToggle={toggle} open={open} />
        ) : node.kind === 'ticket' ? (
          <TicketCard {...props} node={node} onToggle={toggle} open={open} />
        ) : (
          <SessionCard {...props} node={node} onToggle={toggle} open={open} />
        )}
      </div>
      {open ? (
        // biome-ignore lint/a11y/useSemanticElements: a WAI-ARIA tree nests its items in a group
        <div className="kb-chart__group" data-layout={layout} role="group">
          {kids.map((kid, index) => (
            <ChartItem
              {...props}
              enterIndex={index}
              key={kid.id}
              level={level + 1}
              node={kid}
              position={index + 1}
              setSize={kids.length}
              spine={layout === 'single'}
            />
          ))}
        </div>
      ) : null}
    </div>
  )
})

/** The reference card: a tile, a name over a quiet subtitle, the state; a line of what it is; then the facts. */
function NodeCard({
  tile,
  tone,
  name,
  sub,
  badge,
  description,
  note,
  facts,
  count,
  open,
  onToggle,
  kind,
}: {
  tile: ReactNode
  tone?: string | null
  name: string
  sub?: string | null
  badge: ReactNode
  description?: string | null
  /** A second line that matters more than the description (why a session failed). */
  note?: string | null
  facts: ReactNode
  count: number
  open: boolean
  onToggle: () => void
  kind: 'group' | 'ticket' | 'session'
}) {
  return (
    <div className="kb-node" data-kind={kind}>
      <div className="kb-node__head">
        <span aria-hidden="true" className="kb-node__tile" data-tone={tone ?? undefined}>
          {tile}
        </span>
        <span className="kb-node__names">
          <span className="kb-node__name">{name}</span>
          {sub ? <span className="kb-node__sub">{sub}</span> : null}
        </span>
        {badge}
      </div>
      {description ? <p className="kb-node__desc">{description}</p> : null}
      {note ? <p className="kb-node__note">{note}</p> : null}
      <div className="kb-node__foot">
        <span className="kb-node__facts">{facts}</span>
        {count > 0 ? (
          <ShadcnButton
            aria-hidden="true"
            className="kb-node__toggle"
            data-open={open ? 'true' : undefined}
            onClick={(event) => {
              event.stopPropagation()
              onToggle()
            }}
            size="xs"
            tabIndex={-1}
            title={`${open ? 'Hide' : 'Show'} ${count}`}
            type="button"
            variant="ghost"
          >
            {count}
            <ChevronDown size={16} />
          </ShadcnButton>
        ) : null}
      </div>
    </div>
  )
}

/** A session's live state ("Running" turns), or a ticket's column (`label`, no spinner: a column is not a process). */
function StateBadge({ state, label, step }: { state: WorkState; label?: string; step?: StatusStep }) {
  const Icon =
    state === 'done'
      ? Check
      : state === 'failed'
        ? TriangleAlert
        : state === 'canceled'
          ? X
          : state === 'pending'
            ? CircleDashed
            : null
  return (
    <span className="kb-node__badge" data-kind={label ? 'column' : 'session'} data-state={state} data-step={step}>
      {label ? null : state === 'active' ? (
        <span aria-hidden="true" className="kb-node__spinner iii-ui-spin" />
      ) : Icon ? (
        <Icon aria-hidden="true" size={16} />
      ) : null}
      {label ?? STATE_LABEL[state]}
    </span>
  )
}

const dot = <span aria-hidden="true">·</span>

/** The tickets and agent sessions under a node, as drawn (so "Hide done" changes them too). */
function tally(node: Node): { tickets: number; sessions: number } {
  const out = { tickets: 0, sessions: 0 }
  const visit = (n: Node) => {
    if (n.kind === 'ticket') out.tickets += 1
    if (n.kind === 'session') out.sessions += 1
    kidsOf(n).forEach(visit)
  }
  kidsOf(node).forEach(visit)
  return out
}

function GroupCard({
  node,
  data,
  agents,
  open,
  onToggle,
}: ItemProps & { node: GroupNode; open: boolean; onToggle: () => void }) {
  const p = node.project
  const counts = tally(node)
  const total = p ? p.total : node.children.length
  const pct = p && p.total > 0 ? Math.round((p.done / p.total) * 100) : null
  const isAll = node.id === 'all'
  return (
    <NodeCard
      badge={pct !== null ? <span className="kb-node__badge" data-kind="meta">{`${pct}%`}</span> : null}
      count={node.children.length}
      description={
        p?.description?.trim() ||
        (isAll
          ? `${data.projects.length} ${data.projects.length === 1 ? 'project' : 'projects'} on this board`
          : p
            ? `${p.done} of ${total} tickets done`
            : 'Tickets that belong to no project')
      }
      facts={
        <>
          <span>
            {counts.tickets} {counts.tickets === 1 ? 'ticket' : 'tickets'}
          </span>
          {dot}
          <span>
            {counts.sessions} {counts.sessions === 1 ? 'agent' : 'agents'}
          </span>
        </>
      }
      kind="group"
      name={node.name}
      onToggle={onToggle}
      open={open}
      sub={isAll ? 'Board' : p?.lead ? `Lead · ${assigneeName(p.lead, agents)}` : 'Project'}
      tile={isAll ? <SquareKanban size={16} /> : <span className="kb-label__dot" data-tone={node.color ?? undefined} />}
    />
  )
}

function TicketCard({
  node,
  data,
  agents,
  now,
  open,
  onToggle,
}: ItemProps & { node: TicketNode; open: boolean; onToggle: () => void }) {
  const { ticket } = node
  const column = data.columns.find((c) => c.id === ticket.status)
  const subtasks = node.subtasks.length
  const doneSubs = node.subtasks.filter((sub) => sub.state === 'done').length
  const agentsHere = countSessions(node.sessions)
  return (
    <NodeCard
      badge={
        <StateBadge
          label={column?.label ?? ticket.status}
          state={node.state}
          step={statusStep(data.columns, ticket.status, data.done_status, data.canceled_status)}
        />
      }
      count={node.sessions.length + subtasks}
      description={ticket.title}
      facts={
        <>
          {subtasks > 0 ? (
            <>
              <span>
                {doneSubs}/{subtasks} subtasks
              </span>
              {dot}
            </>
          ) : null}
          {agentsHere > 0 ? (
            <>
              <span>
                {agentsHere} {agentsHere === 1 ? 'agent' : 'agents'}
              </span>
              {dot}
            </>
          ) : null}
          <span>{formatRelative(ticket.updated_at, now)}</span>
        </>
      }
      kind="ticket"
      name={ticket.key}
      onToggle={onToggle}
      open={open}
      sub={assigneeName(ticket.assignee, agents)}
      tile={
        <ColumnStatusIcon
          canceledStatus={data.canceled_status}
          columns={data.columns}
          doneStatus={data.done_status}
          status={ticket.status}
        />
      }
    />
  )
}

function SessionCard({
  node,
  now,
  open,
  onToggle,
}: ItemProps & { node: SessionNode; open: boolean; onToggle: () => void }) {
  const { session } = node
  const Icon = SESSION_ICONS[session.icon ?? ''] ?? Bot
  const model = modelName(session.model)
  // The reference card: the role on top, the model under it, the task as the line below.
  const role = node.inferred ? 'Orchestrator' : (session.agent?.name ?? 'Agent')
  const sub = [node.inferred ? session.agent?.name : null, model].filter(Boolean).join(' · ') || null
  return (
    <NodeCard
      badge={<StateBadge state={node.state} />}
      count={node.children.length}
      description={session.name}
      facts={
        <>
          <span>
            {session.messageCount} {session.messageCount === 1 ? 'message' : 'messages'}
          </span>
          {dot}
          <span title="From its first to its latest message">{formatSpan(session.updatedAt - session.createdAt)}</span>
          {node.elsewhere > 0 ? (
            <>
              {dot}
              <span title="Sessions it spawned for other tickets show under those tickets">
                +{node.elsewhere} elsewhere
              </span>
            </>
          ) : (
            <>
              {dot}
              <span>{formatRelative(session.updatedAt, now)}</span>
            </>
          )}
        </>
      }
      kind="session"
      name={role}
      note={node.state === 'failed' ? session.statusReason : null}
      onToggle={onToggle}
      open={open}
      sub={sub}
      tile={<Icon size={16} />}
      tone={session.color}
    />
  )
}

function countSessions(nodes: readonly SessionNode[]): number {
  let total = 0
  walk(nodes, () => {
    total += 1
  })
  return total
}

function ChartSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading the tree" className="kb-chart kb-chart--skeleton" role="status">
      <ShadcnSkeleton className="kb-chart__sk" />
      <div className="kb-chart__sk-row">
        <ShadcnSkeleton className="kb-chart__sk" />
        <ShadcnSkeleton className="kb-chart__sk" />
        <ShadcnSkeleton className="kb-chart__sk" />
      </div>
    </div>
  )
}
