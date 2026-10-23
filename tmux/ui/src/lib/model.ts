/**
 * Pure view-model over the worker's snapshots: what the session tree, the
 * window badges and the ports view show. No React and no bus calls, so the
 * tests drive it directly.
 */

import type {
  AgentState,
  ComposeStack,
  Listener,
  PaneRef,
  PortsSnapshot,
  Session,
  SessionsSnapshot,
  SubagentStatus,
  SubagentTranscript,
  TranscriptItem,
  Window,
} from './types'

export interface AgentBadge {
  paneId: string
  id: string
  label: string
  state: AgentState
  sinceMs: number
  subagents: SubagentStatus[]
}

/** A sub-agent at work, as a row under its window. */
export interface SubagentRow {
  key: string
  /** The sub-agent's id, as `tmux::subagents::transcript` takes it. */
  id: string
  paneId: string
  /** The agent that runs it, e.g. `Claude Code`. */
  agentLabel: string
  description: string
  kind: string | null
  depth: number
  sinceMs: number
}

export interface ComposeBadge {
  namespace: string
  pid: number
  /** The engine WebSocket port of that stack, when the port scan found it. */
  wsPort: number | null
}

export interface WindowRow {
  key: string
  sessionId: string
  sessionName: string
  window: Window
  label: string
  /** The session's current window (tmux state, synced with every client). */
  active: boolean
  agents: AgentBadge[]
  /** Every agent's sub-agents at work in the window, oldest first. */
  subagents: SubagentRow[]
  compose: ComposeBadge | null
  /** Distinct listening ports of every process under the window's panes. */
  ports: number[]
  /** The foreground command of the active pane, for windows without an agent. */
  command: string
}

export interface AgentCounts {
  working: number
  waiting: number
  idle: number
  /** Sub-agents at work under those agents. */
  subagents: number
}

export interface SessionRow {
  key: string
  session: Session
  /** Clients other than console terminals (someone's real terminal). */
  externalClients: number
  consoleClients: number
  windows: WindowRow[]
  agents: AgentCounts
  /** The session the page's terminal is attached to. */
  current: boolean
}

const STATE_ORDER: Record<AgentState, number> = { waiting: 0, working: 1, idle: 2 }

export function agentStateLabel(state: AgentState): string {
  switch (state) {
    case 'working':
      return 'working'
    case 'waiting':
      return 'waiting for you'
    default:
      return 'idle'
  }
}

/** `StatusDot` tone: accent marks live work, warn asks for the operator. */
export function agentStateTone(state: AgentState): 'accent' | 'warn' | 'ink' {
  if (state === 'working') return 'accent'
  if (state === 'waiting') return 'warn'
  return 'ink'
}

/** `1 sub-agent`, `3 sub-agents`. */
export function subagentCountLabel(count: number): string {
  return `${count} sub-agent${count === 1 ? '' : 's'}`
}

/** The heading of one transcript item. */
export function transcriptItemLabel(item: Pick<TranscriptItem, 'kind' | 'tool' | 'error'>, agentLabel: string): string {
  switch (item.kind) {
    case 'user':
      return `Message from ${agentLabel}`
    case 'text':
      return 'Sub-agent'
    case 'thinking':
      return 'Thinking'
    case 'tool_call':
      return item.tool ?? 'Tool call'
    case 'tool_result':
      return `${item.tool ?? 'Tool'} ${item.error ? 'failed' : 'result'}`
  }
}

/** How long the transcript view waits before asking again. */
export function transcriptPollMs(transcript: Pick<SubagentTranscript, 'working'> | null, failed: boolean): number {
  if (failed) return 5_000
  return transcript && !transcript.working ? 10_000 : 2_000
}

export function windowLabel(window: Pick<Window, 'index' | 'name'>): string {
  return `${window.index}:${window.name}`
}

export function composeWsPort(ports: PortsSnapshot | null, namespace: string, pid: number): number | null {
  const stacks = ports?.composes ?? []
  const stack = stacks.find((candidate) => candidate.pid === pid) ?? stacks.find((c) => c.namespace === namespace)
  return stack?.engine?.ws_port ?? null
}

function windowRow(session: Session, window: Window, ports: PortsSnapshot | null): WindowRow {
  const agents: AgentBadge[] = []
  const subagents: SubagentRow[] = []
  const portSet = new Set<number>()
  let compose: ComposeBadge | null = null
  for (const pane of window.panes) {
    if (pane.agent) {
      const running = pane.agent.subagents ?? []
      agents.push({
        paneId: pane.id,
        id: pane.agent.id,
        label: pane.agent.label,
        state: pane.agent.state,
        sinceMs: pane.agent.since_ms,
        subagents: running,
      })
      for (const sub of running) {
        subagents.push({
          key: `${pane.id}/${sub.id}`,
          id: sub.id,
          paneId: pane.id,
          agentLabel: pane.agent.label,
          description: sub.description,
          kind: sub.kind,
          depth: Math.max(1, sub.depth),
          sinceMs: sub.since_ms,
        })
      }
    }
    if (pane.compose && !compose) {
      compose = {
        namespace: pane.compose.namespace,
        pid: pane.compose.pid,
        wsPort: composeWsPort(ports, pane.compose.namespace, pane.compose.pid),
      }
    }
    for (const listener of pane.ports) portSet.add(listener.port)
  }
  agents.sort((a, b) => STATE_ORDER[a.state] - STATE_ORDER[b.state] || a.label.localeCompare(b.label))
  subagents.sort((a, b) => a.sinceMs - b.sinceMs || a.key.localeCompare(b.key))
  const activePane = window.panes.find((pane) => pane.active) ?? window.panes[0]
  return {
    key: window.id,
    sessionId: session.id,
    sessionName: session.name,
    window,
    label: windowLabel(window),
    active: window.active,
    agents,
    subagents,
    compose,
    ports: [...portSet].sort((a, b) => a - b),
    command: activePane?.command ?? '',
  }
}

/** The session tree: sessions by name, windows by index, badges per window. */
export function buildSessionTree(
  sessions: SessionsSnapshot | null,
  ports: PortsSnapshot | null,
  currentSessionId: string | null,
): SessionRow[] {
  if (!sessions) return []
  return [...sessions.sessions]
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((session) => {
      const windows = [...session.windows].sort((a, b) => a.index - b.index).map((w) => windowRow(session, w, ports))
      const agents: AgentCounts = { working: 0, waiting: 0, idle: 0, subagents: 0 }
      for (const row of windows) {
        for (const agent of row.agents) agents[agent.state] += 1
        agents.subagents += row.subagents.length
      }
      return {
        key: session.id,
        session,
        externalClients: Math.max(0, session.attached - session.console_attached),
        consoleClients: session.console_attached,
        windows,
        agents,
        current: session.id === currentSessionId,
      }
    })
}

/** A session by id (`$5`) or exact name. */
export function findSession(snapshot: SessionsSnapshot | null, ref: string | null): Session | null {
  if (!snapshot || !ref) return null
  return snapshot.sessions.find((s) => s.id === ref) ?? snapshot.sessions.find((s) => s.name === ref) ?? null
}

export function activeWindow(session: Session | null): Window | null {
  if (!session) return null
  return session.windows.find((w) => w.active) ?? session.windows[0] ?? null
}

/**
 * Where the terminal attaches when the page opens: the remembered session
 * if it still exists, else the one someone is looking at in a real terminal,
 * else the first by name.
 */
export function pickInitialSession(snapshot: SessionsSnapshot | null, remembered: string | null): Session | null {
  if (!snapshot || snapshot.sessions.length === 0) return null
  const known = findSession(snapshot, remembered)
  if (known) return known
  const sorted = [...snapshot.sessions].sort((a, b) => a.name.localeCompare(b.name))
  return sorted.find((s) => s.attached - s.console_attached > 0) ?? sorted[0]
}

export interface PortGroups {
  composes: ComposeStack[]
  /** Listeners outside every compose stack, by port. */
  others: Listener[]
}

export function groupPorts(ports: PortsSnapshot | null): PortGroups {
  if (!ports) return { composes: [], others: [] }
  const composes = [...ports.composes].sort((a, b) => a.namespace.localeCompare(b.namespace) || a.pid - b.pid)
  const others = ports.listeners
    .filter((listener) => listener.compose === null)
    .sort((a, b) => a.port - b.port || a.address.localeCompare(b.address))
  return { composes, others }
}

/** Every listener of one compose stack, for its worker rows. */
export function stackListeners(ports: PortsSnapshot | null, stack: ComposeStack): Listener[] {
  if (!ports) return []
  return ports.listeners.filter((listener) => listener.compose === stack.namespace).sort((a, b) => a.port - b.port)
}

export function locationLabel(location: PaneRef | null): string {
  if (!location) return ''
  return `${location.session}:${location.window_index}`
}

/** A browser link for a port, except the engine's WebSocket. */
export function portUrl(listener: Pick<Listener, 'port' | 'role'>): string | null {
  if (listener.role === 'engine') return null
  return `http://localhost:${listener.port}`
}

/** Keep the distinguishing tail of a long path. */
export function shortPath(path: string | null, home?: string): string {
  if (!path) return ''
  if (home && (path === home || path.startsWith(`${home}/`))) return `~${path.slice(home.length)}`
  return path
}

/** Names the worker accepts for sessions and windows. */
export function validateName(name: string): string | null {
  const trimmed = name.trim()
  if (trimmed.length === 0) return null
  if (trimmed.length > 64) return 'Use at most 64 characters.'
  if (trimmed.startsWith('-')) return 'A name cannot start with "-".'
  if (/[.:]/.test(trimmed)) return 'tmux names cannot contain "." or ":".'
  // biome-ignore lint/suspicious/noControlCharactersInRegex: rejecting control characters is the point.
  if (/[\u0000-\u001f\u007f]/.test(trimmed)) return 'Control characters are not allowed.'
  return null
}

export function validateDirectory(dir: string): string | null {
  const trimmed = dir.trim()
  if (trimmed.length === 0) return null
  if (!(trimmed.startsWith('/') || trimmed === '~' || trimmed.startsWith('~/'))) {
    return 'Use an absolute path or one under ~/.'
  }
  return null
}

export function validateCommand(command: string): string | null {
  const trimmed = command.trim()
  if (trimmed.length === 0) return null
  if (trimmed.startsWith('-')) return 'A command cannot start with "-".'
  return null
}

/** Request fields with empty optionals left out, so the worker applies its defaults. */
export function compactRequest(fields: Record<string, string | boolean | undefined>): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(fields)) {
    if (value === undefined) continue
    if (typeof value === 'string') {
      const trimmed = value.trim()
      if (trimmed) out[key] = trimmed
    } else out[key] = value
  }
  return out
}

/**
 * A fetch and a push race; the snapshot generated later wins, so a slow
 * `list` response can never paint over a newer `tmux::changed` event.
 */
export function newerSnapshot<T extends { generated_at_ms: number }>(current: T | null, next: T): T {
  if (current && next.generated_at_ms < current.generated_at_ms) return current
  return next
}

/** Errors meaning the terminal is gone and a fresh one has to open. */
export function terminalGone(error: unknown): boolean {
  const text = `${errorText(error)} ${String(error)}`.toLowerCase()
  return text.includes('not found') || text.includes('is closed') || text.includes('credentials are invalid')
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  try {
    return JSON.stringify(error) ?? ''
  } catch {
    return ''
  }
}
