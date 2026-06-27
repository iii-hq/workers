export type Mode = 'plan' | 'ask' | 'agent'

/** Composite `provider::<catalog_model_id>` (matches harness models-catalog). */
export const CATALOG_MODEL_KEY_SEP = '::' as const

export type ModelId = string

export interface ModelOption {
  id: ModelId
  label: string
  contextWindow?: number
}

export const MODES: { id: Mode; label: string }[] = [
  { id: 'plan', label: 'plan' },
  { id: 'ask', label: 'ask' },
  { id: 'agent', label: 'agent' },
]

export const DEFAULT_MODE: Mode = 'agent'

/** Reasoning effort sent to harness::send as `thinking_level`; 'off' is omitted. */
export type ThinkingLevel =
  | 'off'
  | 'minimal'
  | 'low'
  | 'medium'
  | 'high'
  | 'xhigh'

export const THINKING_LEVELS: ThinkingLevel[] = [
  'off',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
]

export const DEFAULT_THINKING_LEVEL: ThinkingLevel = 'off'

export type Role = 'user' | 'assistant' | 'thought' | 'function-call'

export interface Attachment {
  id: string
  name: string
  size: number
  type: string
  /** present only for previewable text/image attachments under ~1MB */
  dataUrl?: string
}

interface BaseMessage {
  id: string
  createdAt: number
}

export interface UserMessage extends BaseMessage {
  role: 'user'
  content: string
  attachments?: Attachment[]
  notification?: { label?: string }
}

export interface AssistantMessage extends BaseMessage {
  role: 'assistant'
  content: string
  model?: ModelId
  mode?: Mode
  streaming?: boolean
}

export interface ThoughtMessage extends BaseMessage {
  role: 'thought'
  content: string
  durationMs: number
  streaming?: boolean
}

export interface FunctionCallMessage extends BaseMessage {
  role: 'function-call'
  functionId: string
  input: unknown
  output?: unknown
  durationMs?: number
  running?: boolean
  /** awaiting user approval before execution; lifecycle: pending → running → done */
  pendingApproval?: boolean
  /** iii function_call_id — set on pending entries so the approve/deny UI can resolve. */
  functionCallId?: string
  /** iii session_id owning this call — paired with functionCallId for approval::resolve. */
  sessionId?: string
}

/**
 * `kind: 'compaction'` renders the collapsed-history marker in the
 * transcript. The session-manager transcript is the single source of truth
 * for what the provider sees, so this marker is purely presentational.
 */
export interface SystemMessage extends BaseMessage {
  role: 'system'
  content: string
  tone?: 'info' | 'warn' | 'error'
  kind?: 'notice' | 'compaction'
  summaryText?: string
  tokensBefore?: number
}

export type Message =
  | UserMessage
  | AssistantMessage
  | ThoughtMessage
  | FunctionCallMessage
  | SystemMessage

/**
 * Loose patch shape passed to updateMessage(). Lists every patchable field
 * across every Message variant; consumers pass only what they need. `id`,
 * `role`, and `createdAt` are never patchable.
 */
export interface MessagePatch {
  content?: string
  attachments?: Attachment[]
  model?: ModelId
  mode?: Mode
  streaming?: boolean
  durationMs?: number
  running?: boolean
  output?: unknown
  pendingApproval?: boolean
  /** Set during fcall-start dedupe so resolve handlers know which iii call to resolve. */
  functionCallId?: string
  sessionId?: string
  /** SystemMessage variant. */
  tone?: 'info' | 'warn' | 'error'
  kind?: 'notice' | 'compaction'
  summaryText?: string
  tokensBefore?: number
}

/** Mirrors session-manager's SessionStatus. */
export type ConversationStatus = 'idle' | 'working' | 'done' | 'error'

export interface Conversation {
  /**
   * The engine session_id (`console-<uuid>` for console-created chats).
   * Conversations are backed by the session-manager worker; this id is used
   * verbatim for `session::*` triggers and `harness::send`.
   */
  id: string
  title: string
  /** flips to true after the user explicitly renames; otherwise auto-derived */
  titleManual?: boolean
  model: ModelId | null
  mode: Mode
  /**
   * Per-session working directory. Confines this chat's shell/coder operations
   * to one project dir (the harness forwards it as `base_dir`). Chosen
   * explicitly (no silent default), shown as a full-path banner, and re-scopable
   * mid-conversation.
   */
  workingDir?: string | null
  messages: Message[]
  /**
   * Spawn-parent session id, from the child session's
   * `SessionMeta.metadata.parent_session_id` (set by the harness on spawn).
   * Absent for root/orchestrator chats. Drives the sidebar tree grouping.
   */
  parentId?: string
  /** Spawn depth: 0 = root orchestrator (from `metadata.depth`). */
  depth?: number
  /** Driver-owned session status (spinner + sidebar indicator). */
  status?: ConversationStatus
  statusReason?: string
  /** Local-only until the first send creates the session server-side. */
  draft?: boolean
  /** Transcript fetched from session-manager at least once. */
  hydrated?: boolean
  createdAt: number
  updatedAt: number
}

const KNOWN_ROLES: ReadonlySet<Role> = new Set<Role>([
  'user',
  'assistant',
  'thought',
  'function-call',
])

export function isKnownRole(role: unknown): role is Role {
  return typeof role === 'string' && KNOWN_ROLES.has(role as Role)
}
