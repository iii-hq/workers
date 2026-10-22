/**
 * The tmux worker's wire shapes, as its functions and its `tmux::changed`
 * trigger type deliver them. Keys are snake_case; times are milliseconds
 * since the Unix epoch.
 */

export interface TmuxInfo {
  /** A usable tmux binary was found. */
  available: boolean
  path: string | null
  version: string | null
  /** Null while no server runs. */
  socket_path: string | null
  server_running: boolean
  /** Why tmux is unavailable, or the last list failure. */
  error: string | null
}

export type AgentState = 'working' | 'waiting' | 'idle'

export interface AgentStatus {
  /** Matcher id from the worker configuration: `claude`, `codex`, … */
  id: string
  label: string
  pid: number
  state: AgentState
  /** When the agent entered `state`. */
  since_ms: number
  /** The agent's session title (the pane title without its status glyph); null when it set none. */
  title?: string | null
  /**
   * Sub-agents at work, oldest first; they keep `state` at `working` after
   * the agent's own turn ended. Absent from workers that predate them.
   */
  subagents?: SubagentStatus[]
}

export interface SubagentStatus {
  /** Stable within the agent's session: Claude Code's agent id, Codex's thread id. */
  id: string
  /** What it was asked to do. */
  description: string
  /** Claude Code's agent type (`general-purpose`, `Explore`) or Codex's role; null when none. */
  kind: string | null
  /** 1 under the main agent, 2 under one of its sub-agents, … */
  depth: number
  /** When it started its current task. */
  since_ms: number
}

export interface ComposeRef {
  namespace: string
  pid: number
}

export interface PaneRef {
  session: string
  session_id: string
  window_index: number
  window_id: string
  pane_id: string
}

export type ListenerRole = 'engine' | 'worker' | 'compose' | 'other'

export interface Listener {
  port: number
  address: string
  family: 'ipv4' | 'ipv6'
  pid: number
  process: string
  worker: string | null
  compose: string | null
  role: ListenerRole
  location: PaneRef | null
}

export interface Pane {
  id: string
  index: number
  active: boolean
  pid: number
  command: string
  cwd: string
  title: string
  width: number
  height: number
  dead: boolean
  agent: AgentStatus | null
  compose: ComposeRef | null
  ports: Listener[]
}

export interface Window {
  id: string
  index: number
  name: string
  /** The current window of its session. */
  active: boolean
  activity_at_ms: number
  bell: boolean
  zoomed: boolean
  panes: Pane[]
}

export interface Session {
  id: string
  name: string
  created_at_ms: number
  activity_at_ms: number
  /** Every attached tmux client, console terminals included. */
  attached: number
  /** How many of those are console terminals. */
  console_attached: number
  windows: Window[]
}

export interface SessionsSnapshot {
  tmux: TmuxInfo
  sessions: Session[]
  generated_at_ms: number
}

export interface ComposeWorker {
  name: string
  pid: number
  ports: number[]
}

export interface ComposeEngine {
  pid: number
  ports: number[]
  ws_port: number | null
}

export interface ComposeStack {
  namespace: string
  pid: number
  project_dir: string | null
  compose_file: string | null
  engine: ComposeEngine | null
  workers: ComposeWorker[]
  location: PaneRef | null
}

export interface PortsSnapshot {
  /** False on platforms without a port scanner. */
  supported: boolean
  error: string | null
  composes: ComposeStack[]
  listeners: Listener[]
  generated_at_ms: number
}

/** The `tmux::changed` payload. */
export interface Overview {
  sessions: SessionsSnapshot
  ports: PortsSnapshot
}

/** One push to `iii::tmux-ui::pty-output::<browserId>`. */
export interface TerminalOutputEvent {
  terminal_id: string
  /** 1, 2, 3… per terminal; null only on the final eof event. */
  sequence: number | null
  /** Base64 bytes. */
  data: string | null
  /** The tmux client exited: detached, session killed, server gone, closed. */
  eof: boolean
  exit_code: number | null
  error: string | null
}

export interface TerminalOpenResponse {
  terminal_id: string
  access_key: string
  session_id: string
  client_tty: string
}

export interface TerminalFrameWire {
  sequence: number
  data: string
}

export interface TerminalReplayResponse {
  frames: TerminalFrameWire[]
  truncated: boolean
  next_sequence: number
}

export interface CreateSessionResponse {
  session_id: string
  session: string
  window_id: string
  window_index: number
  pane_id: string
}

export interface CreateWindowResponse {
  session_id: string
  window_id: string
  window_index: number
  pane_id: string
}
