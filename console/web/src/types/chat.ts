export type Mode = 'plan' | 'ask' | 'agent'

export type ModelId =
  | 'gpt-5.5-medium'
  | 'claude-opus-4.7-thinking'
  | 'gemini-2.5-pro'
  | 'composer-2-fast'

export interface ModelOption {
  id: ModelId
  label: string
}

export const MODELS: ModelOption[] = [
  { id: 'gpt-5.5-medium', label: 'gpt-5.5 medium' },
  { id: 'claude-opus-4.7-thinking', label: 'claude opus 4.7 thinking' },
  { id: 'gemini-2.5-pro', label: 'gemini 2.5 pro' },
  { id: 'composer-2-fast', label: 'composer 2 fast' },
]

export const MODES: { id: Mode; label: string }[] = [
  { id: 'plan', label: 'plan' },
  { id: 'ask', label: 'ask' },
  { id: 'agent', label: 'agent' },
]

export const DEFAULT_MODEL: ModelId = 'gpt-5.5-medium'
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
