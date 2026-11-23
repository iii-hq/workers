import type {
  AskClass,
  NeedsYouItem,
  ControlPlaneBoard,
  ControlPlaneProject,
  ControlPlanePullRequest as ModelPullRequest,
  ControlPlaneQuestion,
  ControlPlaneRelease,
  ControlPlaneRun as ModelRun,
  ControlPlaneTicket,
} from './model.js'
export * from './model.js'
export type ControlPlaneRun = ModelRun & { context_size: number }
export type ControlPlanePullRequest = ModelPullRequest & { as_of: string }
export type { ControlPlaneSummary } from './summary.js'

export interface ControlPlaneClient {
  call<T = unknown>(id: string, input?: unknown): Promise<T>
}

export interface SessionMeta {
  session_id: string
  title: string
  description: string
  status: 'idle' | 'working' | 'done' | 'error'
  status_reason?: string
  kind?: 'user' | 'automation' | 'e2e'
  metadata?: Record<string, unknown>
  created_at: number
  updated_at: number
  message_count: number
}

export interface TranscriptItem {
  entry_id: string
  message?: {
    role: string
    content: Array<{ type: string; text?: string; [key: string]: unknown }>
  }
  custom?: { custom_type: string; data: unknown }
}
export type AgentMessage = NonNullable<TranscriptItem['message']>

export interface ControlPlaneConfig {
  stall_after_min: number
  summary: { model?: string; interval_min: number; language: string }
  prs: { watched_interval_min: number; idle_interval_min: number }
  escalate: Record<string, AskClass[]>
  at_risk_days: number
}

export interface Direction {
  release: ControlPlaneRelease
  state: 'on-track' | 'at-risk'
  reasons: Array<'past-target' | 'open-cards' | 'escalated-question'>
}

export interface ControlPlaneData {
  as_of: string
  completed_root_turns?: string[]
  sourceHash: string
  projects: ControlPlaneProject[]
  board: ControlPlaneBoard | null
  tickets: ControlPlaneTicket[]
  releases: ControlPlaneRelease[]
  releaseCatalog: ControlPlaneRelease[]
  runs: ControlPlaneRun[]
  items: Array<NeedsYouItem & { urgency: 'now' | 'no-rush' }>
  questions: ControlPlaneQuestion[]
  prs: ControlPlanePullRequest[]
  repositories: string[]
  direction: Direction[]
  exceptions: {
    stalled: ControlPlaneRun[]
    failing_prs: ControlPlanePullRequest[]
    blocked_cards: ControlPlaneTicket[]
    old_questions: ControlPlaneQuestion[]
  }
  median_decision_wait_ms: number | null
  loading: {
    board: boolean
    sessions: boolean
    needsYou: boolean
    questions: boolean
    prs: boolean
  }
  errors: {
    board: string | null
    sessions: string | null
    needsYou: string | null
    questions: string | null
    prs: string | null
  }
}
