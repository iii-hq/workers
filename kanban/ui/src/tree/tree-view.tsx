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
  MessageSquare,
  Network,
  Palette,
  RefreshCw,
  Search,
  Terminal,
  TriangleAlert,
  X,
} from 'lucide-react'
import { type KeyboardEvent, memo, type ReactNode, useCallback, useMemo, useRef, useState } from 'react'
import type { AgentProfile, BoardData, GlyphTone, ProjectView, TicketSummary } from '../shared'
import { AssigneeAvatar, assigneeName } from '../ui/avatar'
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

/** A project heading over its tickets, when the tree shows every project. */
type ProjectGroup = {
  kind: 'project'
  id: string
  name: string
  color: GlyphTone | null
  roots: TicketNode[]
}
type Node = TreeNode | ProjectGroup

const kidsOf = (node: Node): Node[] => (node.kind === 'project' ? node.roots : childrenOfNode(node))

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

/* Tailwind (`tw:`, see tailwind.mjs): the board card's surface, so a node reads as the same object as a card. */
const CARD = [
  'kb-tree__card tw:relative tw:flex tw:w-full tw:max-w-[560px] tw:flex-col tw:gap-2 tw:rounded-xl tw:px-3.5 tw:py-3 tw:text-left',
  'tw:border tw:border-neutral-100 tw:bg-white tw:text-neutral-900 tw:transition-colors',
  'tw:hover:border-neutral-200 tw:dark:border-neutral-900 tw:dark:bg-neutral-950 tw:dark:text-neutral-100 tw:dark:hover:border-neutral-800',
].join(' ')
const MUTED = 'tw:text-neutral-500 tw:dark:text-neutral-400'
const TITLE =
  'tw:min-w-0 tw:flex-1 tw:truncate tw:text-sm tw:leading-5 tw:font-medium tw:text-neutral-900 tw:dark:text-neutral-100'
const META = `tw:flex tw:min-w-0 tw:items-center tw:gap-1.5 tw:text-xs tw:leading-4 ${MUTED}`
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
  /** Pick a project from the "every project" tree. */
  onOpenProject: (id: string) => void
}

/**
 * Tree: the project, its tickets and subtasks, and under each ticket the agent sessions that worked it, with the
 * sessions those spawned. Live: the board's events redraw the tickets, session-manager's the sessions.
 */
export function TreeView({ host, data, project, agents, paneKey, onOpenTicket, onOpenProject }: TreeViewProps) {
  const { sessions, loading, error, reload } = useAgentSessions(host.iii, true)
  const [hideDone, setHideDone] = usePaneState<boolean>(`kanban:tree-hide-done:${paneKey}`, false)
  // Nodes the user opened or closed; the rest follow `defaultOpen`.
  const [toggled, setToggled] = usePaneState<Record<string, boolean>>(`kanban:tree-open:${paneKey}`, {})
  const [activeId, setActiveId] = useState<string | null>(null)
  const treeRef = useRef<HTMLUListElement | null>(null)
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

  const roots: Node[] = useMemo(() => {
    const shown = hideDone ? withoutDone(tree.roots) : tree.roots
    if (project) return shown
    // Every project: one heading per project, in the board's project order, then the tickets in none.
    const groups = new Map<string | null, TicketNode[]>()
    for (const node of shown) {
      const id =
        node.ticket.project_id && data.projects.some((p) => p.id === node.ticket.project_id)
          ? node.ticket.project_id
          : null
      groups.set(id, [...(groups.get(id) ?? []), node])
    }
    const out: ProjectGroup[] = []
    for (const p of data.projects) {
      const nodes = groups.get(p.id)
      if (nodes?.length)
        out.push({ kind: 'project', id: `project:${p.id}`, name: p.name, color: p.color, roots: nodes })
    }
    const loose = groups.get(null)
    if (loose?.length) out.push({ kind: 'project', id: 'project:none', name: 'No project', color: null, roots: loose })
    return out
  }, [tree, hideDone, project, data.projects])

  /**
   * Open by default: projects, anything not finished, and anything with running or failed work under it (a finished
   * orchestrator whose sub-agent failed must not hide that). A finished subtree stays folded until asked for.
   */
  const needsEyes = useMemo(() => {
    const ids = new Set<string>()
    const visit = (node: Node): boolean => {
      let flagged = node.kind !== 'project' && (node.state === 'active' || node.state === 'failed')
      for (const kid of kidsOf(node)) if (visit(kid)) flagged = true
      if (flagged) ids.add(node.id)
      return flagged
    }
    for (const node of roots) visit(node)
    return ids
  }, [roots])
  const defaultOpen = useCallback(
    (node: Node) => {
      if (node.kind === 'project') return true
      return needsEyes.has(node.id) || (node.state !== 'done' && node.state !== 'canceled')
    },
    [needsEyes],
  )
  const isOpen = useCallback(
    (node: Node) => (toggled && typeof toggled === 'object' ? toggled[node.id] : undefined) ?? defaultOpen(node),
    [toggled, defaultOpen],
  )
  const setOpen = useCallback(
    (node: Node, open: boolean) => setToggled({ ...(toggled ?? {}), [node.id]: open }),
    [toggled, setToggled],
  )
  const setAll = (open: boolean) => {
    const next: Record<string, boolean> = {}
    const visit = (nodes: Node[]) => {
      for (const node of nodes) {
        if (kidsOf(node).length > 0) next[node.id] = open
        visit(kidsOf(node))
      }
    }
    visit(roots)
    setToggled(next)
  }

  /** The rows on screen, in order: what the arrow keys move through. */
  const visible = useMemo(() => {
    const rows: { node: Node; parent: Node | null }[] = []
    const visit = (nodes: Node[], parent: Node | null) => {
      for (const node of nodes) {
        rows.push({ node, parent })
        if (isOpen(node)) visit(kidsOf(node), node)
      }
    }
    visit(roots, null)
    return rows
  }, [roots, isOpen])
  const focusedId = visible.some((row) => row.node.id === activeId) ? activeId : (visible[0]?.node.id ?? null)

  const focusRow = (id: string) => {
    setActiveId(id)
    treeRef.current?.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(id)}"]`)?.focus()
  }

  const activate = (node: Node) => {
    if (node.kind === 'ticket') onOpenTicket(node.ticket.key)
    else if (node.kind === 'session') openSession(host, node.session.id)
    else if (node.id !== 'project:none') onOpenProject(node.id.slice('project:'.length))
  }

  const onKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
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
        if (kids.length > 0 && isOpen(node)) setOpen(node, false)
        else if (parent) focusRow(parent.id)
        else return
        break
      case 'Enter':
        activate(node)
        break
      case '*': {
        // Open every sibling of the focused row (WAI-ARIA tree).
        const siblings = parent ? kidsOf(parent) : roots
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
    `${stats.sessions} ${stats.sessions === 1 ? 'agent session' : 'agent sessions'}`,
  ]

  let body: ReactNode
  if (loading && sessions.length === 0 && !error) {
    body = <TreeSkeleton />
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
  } else if (roots.length === 0) {
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
    body = (
      <ul
        aria-label={project ? `${project.name}: tickets and agents` : 'Projects, tickets and agents'}
        className="kb-tree"
        onKeyDown={onKeyDown}
        ref={treeRef}
        // biome-ignore lint/a11y/noNoninteractiveElementToInteractiveRole: a WAI-ARIA tree is a list of treeitems
        role="tree"
      >
        {roots.map((node, index) => (
          <TreeRow
            agents={agents}
            data={data}
            focusedId={focusedId}
            isOpen={isOpen}
            key={node.id}
            level={1}
            node={node}
            now={now}
            onActivate={activate}
            onFocusRow={setActiveId}
            onToggle={setOpen}
            position={index + 1}
            setSize={roots.length}
          />
        ))}
      </ul>
    )
  }

  return (
    <div className="kb-tree-view tw:bg-neutral-100 tw:dark:bg-black">
      <div aria-label="Tree controls" className={SUBBAR} role="toolbar">
        <span className={`tw:flex tw:min-w-0 tw:items-center tw:gap-2 tw:text-[13px] ${MUTED}`}>
          <span className="tw:truncate">{summary.join(' · ')}</span>
          {stats.running > 0 ? (
            <span className="tw:flex tw:shrink-0 tw:items-center tw:gap-1.5 tw:font-medium tw:text-glyph-blue">
              <span aria-hidden="true" className="tw:size-1.5 tw:rounded-full tw:bg-glyph-blue" />
              {stats.running} running
            </span>
          ) : null}
          {stats.failed > 0 ? (
            <span className="tw:flex tw:shrink-0 tw:items-center tw:gap-1.5 tw:font-medium tw:text-alert">
              <span aria-hidden="true" className="tw:size-1.5 tw:rounded-full tw:bg-alert" />
              {stats.failed} failed
            </span>
          ) : null}
        </span>
        <span className="tw:flex-1" />
        {/* biome-ignore lint/a11y/noLabelWithoutControl: the switch inside is the control */}
        <label className={`tw:flex tw:shrink-0 tw:cursor-pointer tw:items-center tw:gap-2 tw:text-[13px] ${MUTED}`}>
          <ShadcnSwitch checked={Boolean(hideDone)} onCheckedChange={(checked: boolean) => setHideDone(checked)} />
          Hide done
        </label>
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
      <div className="kb-tree-scroll">{body}</div>
    </div>
  )
}

/** Open a session's chat beside the board (`chat:<id>` reuses its pane when it is already open). */
function openSession(host: Host, sessionId: string) {
  host.panels?.openScreen?.({ screen: `chat:${sessionId}` })
}

type RowProps = {
  node: Node
  level: number
  position: number
  setSize: number
  data: BoardData
  agents: AgentProfile[]
  now: number
  focusedId: string | null
  isOpen: (node: Node) => boolean
  onToggle: (node: Node, open: boolean) => void
  onActivate: (node: Node) => void
  onFocusRow: (id: string) => void
}

/** One tree item: its card, then (open) its children in a group hung off the card by the connector. */
const TreeRow = memo(function TreeRow(props: RowProps) {
  const { node, level, position, setSize, focusedId, isOpen, onToggle, onActivate, onFocusRow } = props
  const kids = kidsOf(node)
  const open = kids.length > 0 && isOpen(node)
  const label =
    node.kind === 'project'
      ? node.name
      : node.kind === 'ticket'
        ? `${node.ticket.key} ${node.ticket.title}`
        : node.session.name
  return (
    <li
      aria-expanded={kids.length > 0 ? open : undefined}
      aria-label={`${label}, ${
        node.kind === 'project'
          ? 'project'
          : node.kind === 'ticket'
            ? (props.data.columns.find((column) => column.id === node.ticket.status)?.label ?? node.ticket.status)
            : STATE_LABEL[node.state]
      }`}
      aria-level={level}
      aria-posinset={position}
      aria-setsize={setSize}
      aria-selected={false}
      className="kb-tree__item"
      data-kind={node.kind}
      data-node-id={node.id}
      data-state={node.kind === 'project' ? undefined : node.state}
      onFocus={(event) => {
        if (event.target === event.currentTarget) onFocusRow(node.id)
      }}
      role="treeitem"
      tabIndex={focusedId === node.id ? 0 : -1}
    >
      {/* biome-ignore lint/a11y/useKeyWithClickEvents lint/a11y/noStaticElementInteractions: the treeitem takes the keys (Enter) */}
      <div
        className="kb-tree__row"
        onClick={() => {
          onFocusRow(node.id)
          onActivate(node)
        }}
      >
        {node.kind === 'project' ? (
          <ProjectHead count={kids.length} node={node} onToggle={() => onToggle(node, !open)} open={open} />
        ) : node.kind === 'ticket' ? (
          <TicketCard {...props} node={node} onToggle={() => onToggle(node, !open)} open={open} />
        ) : (
          <SessionCard {...props} node={node} onToggle={() => onToggle(node, !open)} open={open} />
        )}
      </div>
      {open ? (
        // biome-ignore lint/a11y/useSemanticElements: a WAI-ARIA tree nests its items in a group
        <ul className="kb-tree__group" role="group">
          {kids.map((kid, index) => (
            <TreeRow {...props} key={kid.id} level={level + 1} node={kid} position={index + 1} setSize={kids.length} />
          ))}
        </ul>
      ) : null}
    </li>
  )
})

/** The count of what is folded under a node and the chevron that folds it ("3 ⌄"). */
function Toggle({ count, open, onToggle, noun }: { count: number; open: boolean; onToggle: () => void; noun: string }) {
  if (count === 0) return null
  return (
    <ShadcnButton
      aria-hidden="true"
      className={`kb-tree__toggle tw:h-6 tw:gap-1 tw:px-1.5 tw:text-xs tw:tabular-nums ${MUTED}`}
      data-open={open ? 'true' : undefined}
      onClick={(event) => {
        event.stopPropagation()
        onToggle()
      }}
      size="xs"
      tabIndex={-1}
      title={`${open ? 'Hide' : 'Show'} ${count} ${noun}`}
      type="button"
      variant="ghost"
    >
      {count}
      <ChevronDown className="kb-tree__chevron" size={16} />
    </ShadcnButton>
  )
}

/** A session's live state ("Running" turns), or a ticket's column (`label`, no spinner: a column is not a process). */
function StateBadge({ state, label }: { state: WorkState; label?: string }) {
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
    <span className="kb-tree__badge" data-kind={label ? 'column' : 'session'} data-state={state}>
      {label ? null : state === 'active' ? (
        <span aria-hidden="true" className="kb-tree__spinner iii-ui-spin" />
      ) : Icon && !label ? (
        <Icon aria-hidden="true" size={16} />
      ) : null}
      {label ?? STATE_LABEL[state]}
    </span>
  )
}

function ProjectHead({
  node,
  count,
  open,
  onToggle,
}: {
  node: ProjectGroup
  count: number
  open: boolean
  onToggle: () => void
}) {
  return (
    <div className="kb-tree__project">
      <span aria-hidden="true" className="kb-plan-glyph">
        <span className="kb-label__dot" data-tone={node.color ?? undefined} />
      </span>
      <span className="tw:truncate tw:text-sm tw:font-medium tw:text-neutral-900 tw:dark:text-neutral-100">
        {node.name}
      </span>
      <Toggle count={count} noun={count === 1 ? 'ticket' : 'tickets'} onToggle={onToggle} open={open} />
    </div>
  )
}

function TicketCard({
  node,
  data,
  agents,
  now,
  open,
  onToggle,
}: RowProps & { node: TicketNode; open: boolean; onToggle: () => void }) {
  const { ticket } = node
  const column = data.columns.find((c) => c.id === ticket.status)
  const subtasks = node.subtasks.length
  const doneSubs = node.subtasks.filter((sub) => sub.state === 'done').length
  const agentsHere = countSessions(node.sessions)
  const kids = node.sessions.length + subtasks
  return (
    <div className={CARD} data-kind="ticket">
      <div className="tw:flex tw:min-w-0 tw:items-center tw:gap-2">
        <ColumnStatusIcon
          canceledStatus={data.canceled_status}
          columns={data.columns}
          doneStatus={data.done_status}
          status={ticket.status}
        />
        <span className={`tw:shrink-0 tw:text-[13px] tw:font-medium tw:tabular-nums ${MUTED}`}>{ticket.key}</span>
        <span className={TITLE}>{ticket.title}</span>
        <StateBadge label={column?.label ?? ticket.status} state={node.state} />
      </div>
      <div className="tw:flex tw:min-w-0 tw:items-center tw:gap-2">
        <span className={META}>
          <AssigneeAvatar agents={agents} id={ticket.assignee} />
          <span className="tw:truncate">{assigneeName(ticket.assignee, agents)}</span>
          {subtasks > 0 ? (
            <>
              <span aria-hidden="true">·</span>
              <span className="tw:shrink-0 tw:tabular-nums">
                {doneSubs}/{subtasks} subtasks
              </span>
            </>
          ) : null}
          {agentsHere > 0 ? (
            <>
              <span aria-hidden="true">·</span>
              <span className="tw:shrink-0 tw:tabular-nums">
                {agentsHere} {agentsHere === 1 ? 'agent' : 'agents'}
              </span>
            </>
          ) : null}
          <span aria-hidden="true">·</span>
          <span className="tw:shrink-0">{formatRelative(ticket.updated_at, now)}</span>
        </span>
        <span className="tw:flex-1" />
        <Toggle count={kids} noun="items" onToggle={onToggle} open={open} />
      </div>
    </div>
  )
}

function SessionCard({
  node,
  open,
  onToggle,
  now,
}: RowProps & { node: SessionNode; open: boolean; onToggle: () => void }) {
  const { session } = node
  const Icon = SESSION_ICONS[session.icon ?? ''] ?? Bot
  const model = modelName(session.model)
  const who = [node.inferred ? 'Orchestrator' : null, session.agent?.name ?? null, model].filter(Boolean).join(' · ')
  const kids = node.children.length
  return (
    <div className={CARD} data-kind="session">
      <div className="tw:flex tw:min-w-0 tw:items-start tw:gap-2.5">
        <span aria-hidden="true" className="kb-tree__icon" data-tone={session.color ?? undefined}>
          <Icon size={16} />
        </span>
        <span className="tw:flex tw:min-w-0 tw:flex-1 tw:flex-col">
          <span className={TITLE}>{session.name}</span>
          {who ? <span className={`tw:truncate tw:text-xs tw:leading-4 ${MUTED}`}>{who}</span> : null}
        </span>
        <StateBadge state={node.state} />
      </div>
      {session.statusReason && node.state === 'failed' ? (
        <p className={`tw:m-0 tw:line-clamp-2 tw:text-[13px] tw:leading-5 ${MUTED}`}>{session.statusReason}</p>
      ) : null}
      <div className="tw:flex tw:min-w-0 tw:items-center tw:gap-2">
        <span className={META}>
          <MessageSquare className="tw:size-[14px] tw:shrink-0" size={16} />
          <span className="tw:tabular-nums">{session.messageCount}</span>
          <span aria-hidden="true">·</span>
          <span className="tw:tabular-nums" title="From its first to its latest message">
            {formatSpan(session.updatedAt - session.createdAt)}
          </span>
          <span aria-hidden="true">·</span>
          <span className="tw:truncate">{formatRelative(session.updatedAt, now)}</span>
          {node.elsewhere > 0 ? (
            <>
              <span aria-hidden="true">·</span>
              <span className="tw:shrink-0" title="Sessions it spawned for other tickets show under those tickets">
                {node.elsewhere} on other tickets
              </span>
            </>
          ) : null}
        </span>
        <span className="tw:flex-1" />
        <Toggle count={kids} noun={kids === 1 ? 'session' : 'sessions'} onToggle={onToggle} open={open} />
      </div>
    </div>
  )
}

function countSessions(nodes: readonly SessionNode[]): number {
  let total = 0
  walk(nodes, () => {
    total += 1
  })
  return total
}

function TreeSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading the tree" className="kb-tree tw:flex tw:flex-col tw:gap-2" role="status">
      {[0, 1, 2, 1, 1].map((depth, index) => (
        <ShadcnSkeleton
          className={`tw:h-[74px] tw:w-full tw:max-w-[560px] tw:rounded-xl ${depth ? 'tw:ml-7' : ''}`}
          key={index}
        />
      ))}
    </div>
  )
}
