export type Mode = 'plan' | 'ask' | 'agent'

/** Composite `provider::<catalog_model_id>` (matches harness models-catalog). */
export const CATALOG_MODEL_KEY_SEP = '::' as const

export type ModelId = string

export interface ModelOption {
  id: ModelId
  label: string
  contextWindow?: number
}

/**
 * When the engine is down or mock backend runs, picker still needs options.
 * Ids mirror the seeded catalog (`harness/.../models.json`).
 */
export const STATIC_MODEL_OPTIONS: ModelOption[] = [
  {
    id: `openai${CATALOG_MODEL_KEY_SEP}gpt-5`,
    label: 'gpt-5',
    contextWindow: 400_000,
  },
  {
    id: `anthropic${CATALOG_MODEL_KEY_SEP}claude-opus-4-7`,
    label: 'claude opus 4.7',
    contextWindow: 1_000_000,
  },
  {
    id: `google${CATALOG_MODEL_KEY_SEP}gemini-2-5-pro`,
    label: 'gemini 2.5 pro',
    contextWindow: 1_048_576,
  },
  {
    id: `openai${CATALOG_MODEL_KEY_SEP}gpt-5-mini`,
    label: 'gpt-5 mini',
    contextWindow: 400_000,
  },
]

export const DEFAULT_MODEL: ModelId = STATIC_MODEL_OPTIONS[0].id

export const MODES: { id: Mode; label: string }[] = [
  { id: 'plan', label: 'plan' },
  { id: 'ask', label: 'ask' },
  { id: 'agent', label: 'agent' },
]

export const DEFAULT_MODE: Mode = 'agent'

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
 * `kind: 'compaction'` represents collapsed history; messages before it
 * must NOT be reshipped on subsequent `run::start` calls. The translator
 * (`translateUiHistoryForBackend`) honours this barrier.
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

export interface Conversation {
  id: string
  title: string
  /** flips to true after the user explicitly renames; otherwise auto-derived */
  titleManual?: boolean
  model: ModelId
  mode: Mode
  messages: Message[]
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
