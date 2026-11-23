import {
  type Host,
  ShadcnButton,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuLabel,
  ShadcnDropdownMenuRadioGroup,
  ShadcnDropdownMenuRadioItem,
  ShadcnDropdownMenuTrigger,
  ShadcnSkeleton,
  ShadcnSwitch,
  ShadcnTooltip,
  ShadcnTooltipContent,
  ShadcnTooltipTrigger,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import {
  Bot,
  Check,
  ChevronDown,
  ChevronsDownUp,
  ChevronsUpDown,
  CircleDashed,
  CircleHelp,
  ClipboardCheck,
  Code2,
  Database,
  Expand,
  FileText,
  FlaskConical,
  FolderOpen,
  FolderPlus,
  Folders,
  ListTree,
  type LucideIcon,
  Maximize2,
  MessageCircleMore,
  Minus,
  Network,
  Palette,
  Pencil,
  Plus,
  Search,
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
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { type StatusStep, statusStep } from '../board-state'
import { NewProjectDialog } from '../plan-dialogs'
import {
  type AgentProfile,
  api,
  type BoardData,
  type GlyphTone,
  noteOwnWrite,
  type ProjectView,
  type TicketSummary,
} from '../shared'
import { assigneeName } from '../ui/avatar'
import { Empty, Notice } from '../ui/feedback'
import { ColumnStatusIcon } from '../ui/glyphs'
import { IconAction } from '../ui/icon-action'
import {
  type AgentSession,
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
import { draftChatBeside, openChatBeside } from './side-chat'
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
  waiting: 'Needs answer',
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
  /** This page's own workspace screen (`ext:kanban-board`): the chat opens in a column to its right. */
  screen: string
}

/**
 * Tree: the project as a chart of its work — its tickets and subtasks, and under each ticket the agent sessions
 * that worked it, with the sessions those spawned. Live: the board's events redraw the tickets, session-manager's
 * the sessions.
 */
export function TreeView({ host, data, project, agents, paneKey, onOpenTicket, onOpenProject, screen }: TreeViewProps) {
  const { sessions, links, prompted, loading, error, reload } = useAgentSessions(host.iii, true, project?.id ?? null)
  // The card whose chat is open beside the tree: it stays marked (and in view) while you read.
  const [selectedId, setSelectedId] = usePaneState<string | null>(`kanban:tree-selected:${paneKey}`, null)
  const [hideDone, setHideDone] = usePaneState<boolean>(`kanban:tree-hide-done:${paneKey}`, false)
  // Cards opened or closed by hand; the rest follow `depth` (below). A new key: folds saved when the tree started
  // folded would otherwise keep it folded.
  const [toggled, setToggled] = usePaneState<Record<string, boolean>>(`kanban:tree-folds:${paneKey}`, {})
  const [activeId, setActiveId] = useState<string | null>(null)
  const chartRef = useRef<HTMLDivElement | null>(null)
  /** The project being edited from its card (the project dialog). */
  const [editingProject, setEditingProject] = useState<ProjectView | null>(null)
  /** "No project"'s Create a project: the dialog, then its tickets move into the new project. */
  const [creatingProject, setCreatingProject] = useState(false)
  const [moveError, setMoveError] = useState<string | null>(null)
  /** Where the last press on the canvas began: a click that did not move is not a pan. */
  const pressedAt = useRef<{ x: number; y: number } | null>(null)
  // Beside a chat the tree's pane is narrow: the toolbar keeps its controls and drops their words.
  const { ref: narrowRef, narrow } = useContainerNarrow({ below: 900 })
  // Live lines ("Thinking · 12s") tick while agents run; nothing ticks when nothing runs.
  const [, setTick] = useState(0)
  const runningCount = useMemo(() => sessions.filter((s) => s.status === 'working').length, [sessions])
  useEffect(() => {
    if (runningCount === 0 && prompted.size === 0) return
    const timer = window.setInterval(() => setTick((n) => n + 1), 2000)
    return () => window.clearInterval(timer)
  }, [runningCount, prompted])
  const now = Date.now()
  // When each running session entered its current phase ("Thinking · 14s" counts from there). Read off the
  // sessions as they arrive: a new phase restarts its clock; a session first seen mid-phase starts from its
  // latest write.
  const phaseClock = useRef(new Map<string, { reason: string; since: number }>())
  const phaseSince = useMemo(() => {
    const clocks = phaseClock.current
    const seen = new Set<string>()
    for (const s of sessions) {
      if (s.status !== 'working') continue
      seen.add(s.id)
      const reason = s.statusReason ?? ''
      const known = clocks.get(s.id)
      if (!known || known.reason !== reason) clocks.set(s.id, { reason, since: known ? Date.now() : s.updatedAt })
    }
    for (const id of [...clocks.keys()]) if (!seen.has(id)) clocks.delete(id)
    return new Map([...clocks].map(([id, clock]) => [id, clock.since]))
  }, [sessions])

  const tickets = useMemo<TicketSummary[]>(() => data.columns.flatMap((column) => column.tickets), [data])
  /** The tickets in no project (or in one this board no longer has): what "No project" gathers. */
  const loose = useMemo(
    () => tickets.filter((t) => !t.project_id || !data.projects.some((p) => p.id === t.project_id)),
    [tickets, data.projects],
  )
  /** Move every loose ticket into `target`, one write each; the board's change events redraw the chart. */
  const looseRef = useRef(loose)
  looseRef.current = loose
  const moveInto = async (target: ProjectView) => {
    setMoveError(null)
    const failed: string[] = []
    // Decided when the project exists, not when the dialog opened: a ticket given a project meanwhile stays there.
    for (const ticket of [...looseRef.current]) {
      if (!looseRef.current.some((t) => t.id === ticket.id)) continue
      try {
        noteOwnWrite(await api.update(host.iii, { id: ticket.key, project: target.id, actor: 'user' }))
      } catch (caught) {
        failed.push(`${ticket.key}: ${errorMessage(caught)}`)
      }
    }
    if (failed.length) setMoveError(failed.join(' · '))
  }
  const tree: ProjectTree = useMemo(
    () =>
      buildTree({
        tickets,
        sessions,
        prefix: data.id_prefix,
        columns: data.columns,
        doneStatus: data.done_status,
        canceledStatus: data.canceled_status,
        links: links ?? undefined,
        inScope: project ? (ticket) => ticket.project_id === project.id : undefined,
      }),
    [tickets, sessions, links, data, project],
  )

  /** Every ticket node of the unfiltered tree, by ticket id (what a click resolves a conversation from). */
  const ticketNodes = useMemo(() => {
    const byId = new Map<string, TicketNode>()
    const visit = (node: TicketNode) => {
      byId.set(node.ticket.id, node)
      node.subtasks.forEach(visit)
    }
    tree.roots.forEach(visit)
    return byId
  }, [tree])
  const focusHold = useRef<(() => void) | null>(null)
  useEffect(() => () => focusHold.current?.(), [])

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
      const bucket = groups.get(id)
      if (bucket) bucket.push(node)
      else groups.set(id, [node])
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
   * Every level opens by default, so the parent → child lines read at a glance. `depth` folds the chart at one level
   * (Collapse all = the top level; the level menu = any level), and `toggled` holds the cards opened or closed by
   * hand on top of it. Both are pane state: they survive a reload and stay per pane.
   */
  const [depth, setDepth] = usePaneState<number | null>(`kanban:tree-depth:${paneKey}`, null)
  const isOpen = useCallback(
    (node: Node, level: number) => {
      // The chart's root is always open: a fold saved for it as a child (under All projects) doesn't fold the chart.
      if (node.id === root?.id) return true
      const own = toggled && typeof toggled === 'object' ? toggled[node.id] : undefined
      return own ?? (typeof depth !== 'number' || level < depth)
    },
    [toggled, depth, root],
  )
  const camera = useCamera(paneKey)
  /** The card a fold was asked on: it stays where it is while the layout moves (use-chart-motion). */
  const anchor = useRef<string | null>(null)
  const holdStill = (id: string) => {
    anchor.current = id
  }
  useChartMotion({ camera, chart: chartRef, anchor, chartKey: root?.id ?? null })
  /** Fold or unfold one card; `branch` (Option-click, Option-arrow) takes everything under it along. */
  const setOpen = useCallback(
    (node: Node, open: boolean, branch = false) => {
      holdStill(node.id)
      const next = { ...(toggled ?? {}), [node.id]: open }
      if (branch) {
        const visit = (n: Node) => {
          for (const kid of kidsOf(n)) {
            if (kidsOf(kid).length > 0) next[kid.id] = open
            visit(kid)
          }
        }
        visit(node)
      }
      setToggled(next)
    },
    [toggled, setToggled],
  )
  /** Fold the whole chart at `level` (null = unfold everything); hand-made folds are cleared. */
  const foldAt = (level: number | null) => {
    if (root) holdStill(root.id)
    setDepth(level)
    setToggled({})
  }
  /** What each level of the chart holds ("Tickets", "Subtasks, agents"), for the level menu. */
  const levels = useMemo(() => levelsOf(root), [root])

  /** The cards on screen, depth first: what the arrow keys move through. */
  const visible = useMemo(() => {
    const rows: { node: Node; parent: Node | null; level: number }[] = []
    const visit = (node: Node, parent: Node | null, level: number) => {
      rows.push({ node, parent, level })
      if (isOpen(node, level)) for (const kid of kidsOf(node)) visit(kid, node, level + 1)
    }
    if (root) visit(root, null, 1)
    return rows
  }, [root, isOpen])
  const focusedId = visible.some((row) => row.node.id === activeId) ? activeId : (visible[0]?.node.id ?? null)

  const focusRow = (id: string) => {
    setActiveId(id)
    const el = chartRef.current?.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(id)}"]`)
    el?.focus({ preventScroll: true })
    camera.reveal(el?.querySelector('.kb-node') ?? null)
  }

  // The selected card stays on screen when the pane narrows for the chat beside it.
  camera.keepInView(
    selectedId
      ? () => chartRef.current?.querySelector(`[data-node-id="${CSS.escape(selectedId)}"] > .kb-chart__card`) ?? null
      : null,
  )

  /** Once the chat column has opened (the tree's pane narrows), bring the selected card back into view. */
  const revealSoon = (id: string) => {
    window.setTimeout(() => {
      const el = chartRef.current?.querySelector(`[data-node-id="${CSS.escape(id)}"] > .kb-chart__card`)
      camera.reveal(el ?? null)
    }, 420)
  }

  /**
   * The chat that opens beside the tree focuses its composer as it mounts. The card that opened it keeps the keys
   * instead (D for details, arrows, Enter), so tickets can be browsed one after another; clicking into the chat
   * (or a moment passing) ends the hold, and typing there works as always.
   */
  const keepFocusOnCard = (id: string) => {
    // One hold at a time: the latest activation's.
    focusHold.current?.()
    const onFocusIn = (event: FocusEvent) => {
      const chart = chartRef.current
      if (!chart) return done()
      if (event.composedPath().includes(chart)) return
      // Only the composer's autofocus is turned back; any other move (Tab out, a dialog) ends the hold.
      const target = event.composedPath()[0]
      if (!(target instanceof HTMLElement) || !target.closest('[contenteditable="true"], textarea')) return done()
      chart.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(id)}"]`)?.focus({ preventScroll: true })
    }
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Tab') done()
    }
    const done = () => {
      document.removeEventListener('focusin', onFocusIn, true)
      document.removeEventListener('pointerdown', done, true)
      document.removeEventListener('keydown', onKey, true)
      window.clearTimeout(timer)
      if (focusHold.current === done) focusHold.current = null
    }
    document.addEventListener('focusin', onFocusIn, true)
    document.addEventListener('pointerdown', done, true)
    document.addEventListener('keydown', onKey, true)
    const timer = window.setTimeout(done, FOCUS_HOLD_MS)
    focusHold.current = done
  }

  /**
   * A card's chat, beside the tree. A ticket's: the session working it now, else the one that worked it last; a
   * ticket no agent has touched starts a chat about it (its mention in the composer, not sent). On a console without
   * a workspace chat, the ticket's details open instead. The chart's root: the project's (or board's) own chat. A
   * project under "All projects" opens that project.
   */
  const openChat = (node: Node) => {
    if (node.kind === 'ticket') {
      setSelectedId(node.id)
      // From the whole tree, not the filtered one: Hide done must not hide the ticket's finished conversation.
      const session = conversationOf(ticketNodes.get(node.ticket.id) ?? node)
      const opened = session
        ? openChatBeside(host, screen, session.id)
        : draftChatBeside(host, screen, {
            text: `@kanban(id="${node.ticket.key}") `,
            title: `${node.ticket.key} · ${node.ticket.title}`,
          })
      if (opened) {
        keepFocusOnCard(node.id)
        revealSoon(node.id)
      } else onOpenTicket(node.ticket.key)
    } else if (node.kind === 'session') {
      setSelectedId(node.id)
      if (openChatBeside(host, screen, node.session.id)) {
        keepFocusOnCard(node.id)
        revealSoon(node.id)
      }
    } else if (node.id === root?.id) askAboutProject()
    else if (node.project && node.project.id !== project?.id) onOpenProject(node.project.id)
  }

  /**
   * A click on a card (or Enter on it): a card with children folds or unfolds them, as a native tree does; a card
   * with none opens its chat. The chart's root is always open, so it has nothing to fold. Option-click takes the
   * whole branch along.
   */
  const activate = (node: Node, level: number, branch = false) => {
    if (kidsOf(node).length > 0 && node.id !== root?.id) setOpen(node, !isOpen(node, level), branch)
    else if (node.kind !== 'group') openChat(node)
  }

  /**
   * The project's own chat: the one this button opened for this project before (remembered by project id, and only
   * once it is a real session), else a new one about the project. A session merely titled like it is never taken.
   */
  const askAboutProject = () => {
    const title = project ? `Project · ${project.name}` : 'Board · all projects'
    const chatKey = `kanban:project-chat:${project?.id ?? 'all'}`
    const remembered = readLocal(chatKey)
    if (remembered && sessions.some((s) => s.id === remembered)) {
      openChatBeside(host, screen, remembered)
      return
    }
    const tops = tree.roots.slice(0, 3).map((n) => `@kanban(id="${n.ticket.key}")`)
    const scope = project ? `the kanban project “${project.name}”` : 'the kanban board'
    const count = `${tree.stats.tickets} ${tree.stats.tickets === 1 ? 'ticket' : 'tickets'}`
    draftChatBeside(
      host,
      screen,
      {
        title,
        text: `About ${scope} (${count}${tops.length ? `, starting from ${tops.join(' ')}` : ''}): `,
      },
      (id) => writeLocal(chatKey, id),
    )
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // The tree's keys are a card's keys: a button inside a card (Edit project, Ticket details) keeps its own Enter.
    if ((event.target as HTMLElement).getAttribute('role') !== 'treeitem') return
    const index = visible.findIndex((row) => row.node.id === focusedId)
    if (index < 0) return
    const { node, parent, level } = visible[index]
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
        if (!isOpen(node, level) || event.altKey) setOpen(node, true, event.altKey)
        else focusRow(kids[0].id)
        break
      case 'ArrowLeft':
        if (kids.length > 0 && isOpen(node, level) && node !== root) setOpen(node, false, event.altKey)
        else if (parent) focusRow(parent.id)
        else return
        break
      case 'Enter':
        activate(node, level, event.altKey)
        break
      case 'c':
      case 'C':
        // The card's chat, beside the tree (what a click on a card without children does).
        if ((node.kind === 'group' && node !== root) || event.metaKey || event.ctrlKey || event.altKey) return
        openChat(node)
        break
      case 'd':
      case 'D':
        // The ticket's details (description, properties, activity) in the dialog.
        if (node.kind !== 'ticket' || event.metaKey || event.ctrlKey || event.altKey) return
        onOpenTicket(node.ticket.key)
        break
      case 'Escape':
        if (!selectedId) return
        setSelectedId(null)
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
  const canChat = Boolean(host.chat?.openDraft || host.chat?.selectConversation)
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
          onChat={openChat}
          canChat={canChat}
          onFocusRow={setActiveId}
          onDetails={onOpenTicket}
          onEditProject={setEditingProject}
          onCreateProject={() => setCreatingProject(true)}
          position={1}
          phaseSince={phaseSince}
          prompted={prompted}
          selectedId={selectedId}
          setSize={1}
          spine
          stats={tree.stats}
        />
      </div>
    )
  }

  return (
    <div
      className="kb-tree-view tw:bg-neutral-100 tw:dark:bg-black"
      data-narrow={narrow ? 'true' : undefined}
      ref={narrowRef}
    >
      <div aria-label="Tree controls" className={SUBBAR} role="toolbar">
        <span className={`tw:flex tw:min-w-0 tw:items-center tw:gap-3 tw:text-[13px] ${MUTED}`}>
          {narrow ? null : <span className="tw:truncate">{summary.join(' · ')}</span>}
          {stats.running > 0 ? (
            <span className="tw:shrink-0 tw:font-medium tw:text-glyph-blue">{stats.running} running</span>
          ) : null}
          {stats.waiting > 0 ? <span className="kb-tree-waiting">{stats.waiting} waiting</span> : null}
          {stats.failed > 0 ? (
            <span className="tw:shrink-0 tw:font-medium tw:text-alert">{stats.failed} failed</span>
          ) : null}
        </span>
        <span className="tw:flex-1" />
        {canChat ? (
          narrow ? (
            <IconAction label={project ? 'Ask about this project' : 'Ask about the board'} onClick={askAboutProject}>
              <MessageCircleMore size={16} />
            </IconAction>
          ) : (
            <ShadcnButton className="tw:shrink-0" onClick={askAboutProject} size="sm" type="button" variant="outline">
              <MessageCircleMore size={16} />
              {project ? 'Ask about this project' : 'Ask about the board'}
            </ShadcnButton>
          )
        ) : null}
        {/* biome-ignore lint/a11y/noLabelWithoutControl: the switch inside is the control */}
        <label className={`tw:flex tw:shrink-0 tw:cursor-pointer tw:items-center tw:gap-2 tw:text-[13px] ${MUTED}`}>
          <ShadcnSwitch
            aria-label="Hide done"
            checked={Boolean(hideDone)}
            onCheckedChange={(checked: boolean) => setHideDone(checked)}
          />
          {narrow ? null : 'Hide done'}
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
        {/* biome-ignore lint/a11y/useSemanticElements: a fieldset would bring its own border and legend into a toolbar */}
        <span aria-label="Fold" className="tw:inline-flex tw:items-center tw:gap-0.5" role="group">
          <IconAction disabled={!root} label="Expand all" onClick={() => foldAt(null)}>
            <ChevronsUpDown size={16} />
          </IconAction>
          <IconAction disabled={!root} label="Collapse all" onClick={() => foldAt(2)}>
            <ChevronsDownUp size={16} />
          </IconAction>
          <LevelMenu depth={typeof depth === 'number' ? depth : null} levels={levels} onPick={foldAt} />
        </span>
      </div>
      {moveError ? (
        <Notice
          actionLabel="Dismiss"
          detail={moveError}
          onAction={() => setMoveError(null)}
          title="Some tickets could not be moved into the new project"
        />
      ) : null}
      {error ? (
        <Notice
          actionLabel="Retry"
          detail={`The tickets still show; their agent sessions do not. ${error}`}
          onAction={() => void reload()}
          title="Agent sessions could not be read"
        />
      ) : null}
      {charted && root ? (
        // biome-ignore lint/a11y/useKeyWithClickEvents lint/a11y/noStaticElementInteractions: Escape in the tree deselects too
        <div
          className="kb-chart-scroll"
          data-panning={camera.panning ? 'true' : undefined}
          onClick={(event) => {
            // A click on the empty canvas (not a pan) clears the selection.
            const down = pressedAt.current
            const still = down && Math.hypot(event.clientX - down.x, event.clientY - down.y) < 4
            if (still && !(event.target as HTMLElement).closest('.kb-chart__card')) setSelectedId(null)
          }}
          onPointerDownCapture={(event) => {
            pressedAt.current = { x: event.clientX, y: event.clientY }
          }}
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
      <NewProjectDialog
        agents={agents}
        host={host}
        moveCount={loose.length}
        onCreated={(created, options) => {
          setCreatingProject(false)
          if (options?.move) void moveInto(created)
        }}
        onOpenChange={(open) => {
          if (!open) setCreatingProject(false)
        }}
        open={creatingProject}
      />
      <NewProjectDialog
        agents={agents}
        host={host}
        // A save that finishes after another project's edit was opened leaves that newer edit alone.
        onCreated={(saved) => setEditingProject((current) => (current && current.id !== saved.id ? current : null))}
        onOpenChange={(open) => {
          if (!open) setEditingProject(null)
        }}
        open={editingProject !== null}
        project={editingProject}
      />
    </div>
  )
}

/**
 * The conversation a ticket's card opens: among the sessions that worked the ticket, the one at work now (or
 * whose sub-agents are), else the one active last. Its sub-agents' own chats open from their cards.
 */
function conversationOf(node: TicketNode): AgentSession | null {
  const live = (n: SessionNode): boolean => n.state === 'active' || n.children.some(live)
  const ranked = [...node.sessions].sort(
    (a, b) => Number(live(b)) - Number(live(a)) || b.session.updatedAt - a.session.updatedAt,
  )
  return ranked[0]?.session ?? null
}

/** The live phase the harness reports for a running session, in a person's words. */
export function activityLabel(reason: string | null): string {
  const phase = (reason ?? '').trim().toLowerCase()
  if (!phase) return 'Working'
  if (phase === 'preparing context') return 'Reading context'
  if (phase === 'running tools') return 'Calling functions'
  if (phase === 'waiting for pending calls') return 'Waiting on calls'
  if (phase === 'queueing next step') return 'Next step'
  if (phase === 'stopping') return 'Stopping'
  if (phase.startsWith('waiting for ')) return 'Thinking'
  return phase.charAt(0).toUpperCase() + phase.slice(1)
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
  isOpen: (node: Node, level: number) => boolean
  /** A click on the card: fold or unfold it, or (no children) open its chat. */
  onActivate: (node: Node, level: number, branch: boolean) => void
  /** The card's chat beside the tree (the root's: the project's own chat; a project under All: that project). */
  onChat: (node: Node) => void
  /** This console can show a chat (beside the page, or as the main chat). */
  canChat: boolean
  onFocusRow: (id: string) => void
  /** The card whose chat is open beside the tree. */
  selectedId: string | null
  /** When a prompt last reached each session (session id → ms). */
  prompted: ReadonlyMap<string, number>
  /** When each running session entered its current phase (session id → ms). */
  phaseSince: ReadonlyMap<string, number>
  /** A ticket's details, in the dialog. */
  onDetails: (key: string) => void
  /** Rename the project (or change its description, lead, color) in the project dialog. */
  onEditProject: (project: ProjectView) => void
  /** Make a project for the tickets that have none (the "No project" card). */
  onCreateProject: () => void
  /** Its place among its siblings: the stagger of its entrance. */
  enterIndex?: number
}

/** One node: its card, then (open) its children laid out by `layoutOf`, hung off the card by the connectors. */
const ChartItem = memo(function ChartItem(props: ItemProps) {
  const { node, level, position, setSize, spine, data, focusedId, isOpen, onActivate, onFocusRow } = props
  const kids = kidsOf(node)
  // The chart's root is always open: it has children but nothing to fold.
  const foldable = kids.length > 0 && level > 1
  const open = kids.length > 0 && isOpen(node, level)
  const layout = layoutOf(kids.length, spine)
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
      aria-selected={node.id === props.selectedId}
      aria-setsize={setSize}
      className="kb-chart__item"
      style={props.enterIndex ? ({ '--i': props.enterIndex } as CSSProperties) : undefined}
      data-kids={open ? layout : undefined}
      data-node-id={node.id}
      data-spine={spine ? 'true' : undefined}
      data-live={agentsAtWork(node) ? 'true' : undefined}
      data-waiting={!agentsAtWork(node) && waitsOnYou(node) ? 'true' : undefined}
      data-selected={node.id === props.selectedId ? 'true' : undefined}
      data-state={node.kind === 'group' ? undefined : node.state}
      data-fold={foldable ? (open ? 'open' : 'closed') : undefined}
      onFocus={(event) => {
        if (event.target === event.currentTarget) onFocusRow(node.id)
      }}
      role="treeitem"
      tabIndex={focusedId === node.id ? 0 : -1}
    >
      {/* biome-ignore lint/a11y/useKeyWithClickEvents lint/a11y/noStaticElementInteractions: the treeitem takes the keys (Enter) */}
      <div
        className="kb-chart__card"
        onClick={(event) => {
          onFocusRow(node.id)
          onActivate(node, level, event.altKey)
        }}
      >
        {node.kind === 'group' ? (
          <GroupCard {...props} count={foldable ? kids.length : 0} node={node} open={open} />
        ) : node.kind === 'ticket' ? (
          <TicketCard {...props} count={kids.length} node={node} open={open} />
        ) : (
          <SessionCard {...props} count={kids.length} node={node} open={open} />
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
              // Each project starts its own chart: under "All projects" it lays out as it does on its own.
              spine={layout === 'single' || node.kind === 'group'}
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
  live,
  actions,
  count,
  open,
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
  /** What an agent is doing right now (a live line under the description). */
  live?: ReactNode
  /** The card's own actions at the end of the footer (details, chat): icon buttons that never fold the card. */
  actions?: ReactNode
  /** Its children (0: nothing to fold, so no disclosure). */
  count: number
  open: boolean
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
      {live ? <p className="kb-node__live">{live}</p> : null}
      <div className="kb-node__foot">
        {count > 0 ? (
          // The disclosure, as a native tree draws it: the whole card is the control (aria-expanded on the item).
          <span aria-hidden="true" className="kb-node__fold" data-open={open ? 'true' : undefined}>
            <ChevronDown size={16} />
            {count}
          </span>
        ) : null}
        <span className="kb-node__facts">{facts}</span>
        {actions ? <span className="kb-node__actions">{actions}</span> : null}
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
            : state === 'waiting'
              ? CircleHelp
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

/** A card's icon action: its own click only — the card around it never folds or opens with it. */
function CardAction({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <IconAction
      className="kb-node__action"
      label={label}
      onClick={(event) => {
        event.stopPropagation()
        onClick()
      }}
      size="icon-xs"
    >
      {children}
    </IconAction>
  )
}

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
  count,
  level,
  canChat,
  onChat,
  onEditProject,
  onCreateProject,
}: ItemProps & { node: GroupNode; open: boolean; count: number }) {
  const p = node.project
  const counts = tally(node)
  const total = p ? p.total : node.children.length
  const pct = p && p.total > 0 ? Math.round((p.done / p.total) * 100) : null
  const isAll = node.id === 'all'
  return (
    <NodeCard
      badge={pct !== null ? <span className="kb-node__badge" data-kind="meta">{`${pct}%`}</span> : null}
      count={count}
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
      actions={
        <>
          {p ? (
            <CardAction label="Edit project" onClick={() => onEditProject(p)}>
              <Pencil size={16} />
            </CardAction>
          ) : node.id === 'project:none' ? (
            <CardAction label="Create a project for these tickets" onClick={onCreateProject}>
              <FolderPlus size={16} />
            </CardAction>
          ) : null}
          {level > 1 && p ? (
            // Under "All projects": the project's own tree.
            <CardAction label="Open project" onClick={() => onChat(node)}>
              <FolderOpen size={16} />
            </CardAction>
          ) : level === 1 && canChat ? (
            <CardAction
              label={p ? 'Ask about this project (C)' : 'Ask about the board (C)'}
              onClick={() => onChat(node)}
            >
              <MessageCircleMore size={16} />
            </CardAction>
          ) : null}
        </>
      }
      kind="group"
      name={node.name}
      open={open}
      sub={isAll ? 'Board' : p?.lead ? `Lead · ${assigneeName(p.lead, agents)}` : 'Project'}
      tile={isAll ? <Folders size={16} /> : <span className="kb-label__dot" data-tone={node.color ?? undefined} />}
    />
  )
}

function TicketCard({
  node,
  data,
  agents,
  now,
  open,
  count,
  canChat,
  onChat,
  onDetails,
}: ItemProps & { node: TicketNode; open: boolean; count: number }) {
  const { ticket } = node
  const column = data.columns.find((c) => c.id === ticket.status)
  const subtasks = node.subtasks.length
  const doneSubs = node.subtasks.filter((sub) => sub.state === 'done').length
  const agentsHere = countSessions(node.sessions)
  const working = workingUnder(node.sessions)
  const waiting = waitingUnder(node.sessions)
  const live =
    working.length > 0 ? (
      <>
        <span aria-hidden="true" className="kb-node__pulse iii-ui-pulse" />
        <span>
          {working.length === 1 ? '1 agent working' : `${working.length} agents working`}
          {' · '}
          {activityLabel(working[0].statusReason)}
        </span>
      </>
    ) : waiting.length > 0 ? (
      <>
        <span aria-hidden="true" className="kb-node__pulse" data-tone="waiting" />
        <span>
          {waiting.length === 1
            ? 'An agent is waiting for your answer'
            : `${waiting.length} agents are waiting for your answer`}
        </span>
      </>
    ) : null
  return (
    <NodeCard
      badge={
        <StateBadge
          label={column?.label ?? ticket.status}
          state={node.state}
          step={statusStep(data.columns, ticket.status, data.done_status, data.canceled_status)}
        />
      }
      count={count}
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
      actions={
        <>
          <CardAction label="Open details (D)" onClick={() => onDetails(ticket.key)}>
            <Expand size={16} />
          </CardAction>
          {canChat ? (
            <CardAction label="Open chat (C)" onClick={() => onChat(node)}>
              <MessageCircleMore size={16} />
            </CardAction>
          ) : null}
        </>
      }
      kind="ticket"
      live={live}
      name={ticket.key}
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
  count,
  canChat,
  onChat,
  prompted,
  phaseSince,
}: ItemProps & { node: SessionNode; open: boolean; count: number }) {
  const { session } = node
  // What it is doing now: a prompt that just reached it, its live phase, or a question waiting on you.
  const promptedAt = prompted.get(session.id)
  // Only while it has not finished: a quick agent that is already done says so instead.
  const finished = session.status === 'done' || session.status === 'error'
  const justPrompted = !finished && promptedAt !== undefined && now - promptedAt < PROMPT_FRESH_MS
  const awaiting = node.state === 'waiting'
  const live = justPrompted ? (
    <>
      <span aria-hidden="true" className="kb-node__pulse iii-ui-pulse" data-tone="prompt" />
      <span>Prompted {formatSpan(now - (promptedAt ?? now))} ago</span>
    </>
  ) : session.status === 'working' ? (
    <>
      <span aria-hidden="true" className="kb-node__pulse iii-ui-pulse" />
      <span>
        {activityLabel(session.statusReason)} · {formatSpan(now - (phaseSince.get(session.id) ?? session.updatedAt))}
      </span>
    </>
  ) : awaiting ? (
    <>
      <span aria-hidden="true" className="kb-node__pulse" data-tone="waiting" />
      <span>Waiting for your answer</span>
    </>
  ) : null
  const Icon = SESSION_ICONS[session.icon ?? ''] ?? Bot
  const model = modelName(session.model)
  // The reference card: the role on top, the model under it, the task as the line below.
  const role = node.inferred ? 'Orchestrator' : (session.agent?.name ?? 'Agent')
  const sub = [node.inferred ? session.agent?.name : null, model].filter(Boolean).join(' · ') || null
  return (
    <NodeCard
      actions={
        canChat ? (
          <CardAction label="Open chat (C)" onClick={() => onChat(node)}>
            <MessageCircleMore size={16} />
          </CardAction>
        ) : null
      }
      badge={<StateBadge state={node.state} />}
      count={count}
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
      live={live}
      name={role}
      note={node.state === 'failed' ? session.statusReason : null}
      open={open}
      sub={sub}
      tile={<Icon size={16} />}
      tone={session.color}
    />
  )
}

/**
 * Whether an agent is at work at or under a node: a running session, or a ticket (or project) with one somewhere
 * beneath it. Not the ticket's column — "In progress" with nobody running is not live.
 */
function agentsAtWork(node: Node): boolean {
  if (node.kind === 'session') return node.session.status === 'working' || node.children.some(agentsAtWork)
  if (node.kind === 'ticket') return node.sessions.some(agentsAtWork) || node.subtasks.some(agentsAtWork)
  return node.children.some(agentsAtWork)
}

/** How long a clicked card holds the keys against the chat it opened (its composer focuses as it mounts). */
const FOCUS_HOLD_MS = 1500
/** How long a prompt counts as "just" reaching a session. */
const PROMPT_FRESH_MS = 8000

/** The sessions at work under a ticket (one each, however often drawn), the latest active first. */
function workingUnder(nodes: readonly SessionNode[]): AgentSession[] {
  const found = new Map<string, AgentSession>()
  walk(nodes, (n) => {
    if (n.kind === 'session' && n.session.status === 'working') found.set(n.session.id, n.session)
  })
  return [...found.values()].sort((a, b) => b.updatedAt - a.updatedAt)
}

/** The sessions under a ticket blocked on the person's answer (one each). */
function waitingUnder(nodes: readonly SessionNode[]): AgentSession[] {
  const found = new Map<string, AgentSession>()
  walk(nodes, (n) => {
    if (n.kind === 'session' && n.state === 'waiting') found.set(n.session.id, n.session)
  })
  return [...found.values()]
}

/** A question waits on the person at or under this card (and no agent there is at work). */
function waitsOnYou(node: Node): boolean {
  if (node.kind === 'session') return node.state === 'waiting' || node.children.some(waitsOnYou)
  if (node.kind === 'ticket') return node.sessions.some(waitsOnYou) || node.subtasks.some(waitsOnYou)
  return node.children.some(waitsOnYou)
}

function countSessions(nodes: readonly SessionNode[]): number {
  let total = 0
  walk(nodes, () => {
    total += 1
  })
  return total
}

type Level = { level: number; holds: string }

/**
 * The levels the chart can fold at, from the top: a level any card of which has children, with what its cards are
 * ("Tickets", "Subtasks, agents"). The root's level is left out: it never folds.
 */
function levelsOf(root: GroupNode | null): Level[] {
  if (!root) return []
  const kinds = new Map<number, Set<string>>()
  const folds = new Set<number>()
  const visit = (node: Node, parent: Node | null, level: number) => {
    const kind =
      node.kind === 'group'
        ? 'Projects'
        : node.kind === 'session'
          ? 'Agents'
          : parent?.kind === 'ticket'
            ? 'Subtasks'
            : 'Tickets'
    const bucket = kinds.get(level) ?? new Set<string>()
    bucket.add(kind)
    kinds.set(level, bucket)
    const kids = kidsOf(node)
    if (kids.length > 0) folds.add(level)
    for (const kid of kids) visit(kid, node, level + 1)
  }
  visit(root, null, 1)
  return [...folds]
    .filter((level) => level > 1)
    .sort((a, b) => a - b)
    .map((level) => {
      const [first, ...rest] = [...(kinds.get(level) ?? [])]
      const holds = [first, ...rest.map((k) => k.toLowerCase())].join(', ')
      return { level, holds }
    })
}

/**
 * Fold the chart at a level: every card at it closes, and everything above stays open — "show me down to the
 * tickets". Expand all and Collapse all are its two ends.
 */
function LevelMenu({
  levels,
  depth,
  onPick,
}: {
  levels: Level[]
  depth: number | null
  onPick: (level: number | null) => void
}) {
  const value = depth === null || !levels.some((l) => l.level === depth) ? 'all' : String(depth)
  // Controlled: a radio row does not close a Base UI menu by itself.
  const [open, setOpen] = useState(false)
  return (
    <ShadcnDropdownMenu onOpenChange={setOpen} open={open}>
      <ShadcnTooltip>
        <ShadcnTooltipTrigger
          render={
            <ShadcnDropdownMenuTrigger
              disabled={levels.length === 0}
              render={<ShadcnButton aria-label="Fold at a level" size="icon-sm" type="button" variant="ghost" />}
            />
          }
        >
          <ListTree size={16} />
        </ShadcnTooltipTrigger>
        <ShadcnTooltipContent side="bottom">Fold at a level</ShadcnTooltipContent>
      </ShadcnTooltip>
      <ShadcnDropdownMenuContent align="end" className="kb-menu">
        <ShadcnDropdownMenuGroup>
          <ShadcnDropdownMenuLabel>Show down to</ShadcnDropdownMenuLabel>
          <ShadcnDropdownMenuRadioGroup
            onValueChange={(next: string) => {
              setOpen(false)
              onPick(next === 'all' ? null : Number(next))
            }}
            value={value}
          >
            {levels.map(({ level, holds }) => (
              <ShadcnDropdownMenuRadioItem className="kb-menu__row" key={level} value={String(level)}>
                <span className="kb-menu__label">{`Level ${level}`}</span>
                <span className="kb-menu__hint">{holds}</span>
              </ShadcnDropdownMenuRadioItem>
            ))}
            <ShadcnDropdownMenuRadioItem className="kb-menu__row" value="all">
              <span className="kb-menu__label">All levels</span>
            </ShadcnDropdownMenuRadioItem>
          </ShadcnDropdownMenuRadioGroup>
        </ShadcnDropdownMenuGroup>
      </ShadcnDropdownMenuContent>
    </ShadcnDropdownMenu>
  )
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

function readLocal(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeLocal(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // Private mode: the next Ask starts a new chat; nothing worse.
  }
}
