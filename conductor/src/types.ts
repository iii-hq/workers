export type AgentKind = 'claude' | 'codex' | 'gemini' | 'aider' | 'cursor' | 'amp' | 'opencode' | 'qwen' | 'remote'

export interface AgentSpec {
  kind: AgentKind
  bin?: string
  args?: string[]
  function_id?: string
  prompt?: string
  worktree?: boolean
}

export interface GateSpec {
  function_id: string
  description?: string
}

export interface DispatchInput {
  task: string
  agents: AgentSpec[]
  gates?: GateSpec[]
  cwd: string
  timeoutMs?: number
}

export interface DispatchResult {
  ok: boolean
  run_id: string
  agents: number
  gates: number
}

export interface AgentRunState {
  agent: AgentSpec
  status: 'pending' | 'running' | 'failed' | 'finished'
  startedAt?: number
  finishedAt?: number
  exitCode?: number
  output?: string
  error?: string
  diff?: string
  worktreePath?: string
  branch?: string
  gateResults?: Record<string, { ok: boolean; reason?: string }>
}

export interface RunState {
  id: string
  task: string
  cwd: string
  startedAt: number
  finishedAt?: number
  agents: AgentRunState[]
  winnerIndex?: number
}

export interface MergeResult {
  ok: boolean
  reason?: string
  run_id: string
  winner?: { index: number; agent: AgentSpec; diff: string; branch?: string }
  losers: number[]
}
