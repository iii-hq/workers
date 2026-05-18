export type Mode = 'plan' | 'ask' | 'agent'

/** Composite `provider::<catalog_model_id>` (matches harness-node models-catalog). */
export const CATALOG_MODEL_KEY_SEP = '::' as const

export type ModelId = string

export interface ModelOption {
  id: ModelId
  label: string
}

/**
 * When the engine is down or mock backend runs, picker still needs options.
 * Ids mirror the seeded catalog (`harness-node/.../models.json`).
 */
export const STATIC_MODEL_OPTIONS: ModelOption[] = [
  { id: `openai${CATALOG_MODEL_KEY_SEP}gpt-5`, label: 'gpt-5' },
  {
    id: `anthropic${CATALOG_MODEL_KEY_SEP}claude-opus-4-7`,
    label: 'claude opus 4.7',
  },
  {
    id: `google${CATALOG_MODEL_KEY_SEP}gemini-2-5-pro`,
    label: 'gemini 2.5 pro',
  },
  { id: `openai${CATALOG_MODEL_KEY_SEP}gpt-5-mini`, label: 'gpt-5 mini' },
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
}

export type Message =
  | UserMessage
  | AssistantMessage
  | ThoughtMessage
  | FunctionCallMessage

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
