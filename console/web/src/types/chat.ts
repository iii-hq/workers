export type Mode = 'ask' | 'agent'

/** Composite `provider::<catalog_model_id>` (matches harness models-catalog). */
export const CATALOG_MODEL_KEY_SEP = '::' as const

export type ModelId = string

export interface ModelOption {
  id: ModelId
  label: string
  contextWindow?: number
  supportsThinking?: boolean
  reasoningEfforts?: ReasoningEffortOption[]
}

export interface ReasoningEffortOption {
  effort: string
  description?: string
}

export const MODES: { id: Mode; label: string }[] = [
  { id: 'ask', label: 'ask' },
  { id: 'agent', label: 'agent' },
]

export const DEFAULT_MODE: Mode = 'agent'

/** Model-selected reasoning effort. `default` omits every effort override. */
export type ThinkingLevel = string

/** Compatibility choices for providers that only advertise a thinking flag. */
export const THINKING_LEVELS: ThinkingLevel[] = [
  'default',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
]

export const DEFAULT_THINKING_LEVEL: ThinkingLevel = 'default'

export type Role = 'user' | 'assistant' | 'thought' | 'function-trigger'

export interface Attachment {
  id: string
  name: string
  size: number
  type: string
  /** present only for previewable text/image attachments under ~1MB */
  dataUrl?: string
  /**
   * The picked file, for attachment kinds a worker reads at send time (PDFs go
   * through `pdf::to-markdown` — see `lib/pdf-attachments.ts`). Browser-only and
   * deliberately not persisted: a conversation reloaded from history keeps the
   * chip, not the bytes.
   */
  file?: File
}

interface BaseMessage {
  id: string
  createdAt: number
}

export interface UserMessage extends BaseMessage {
  role: 'user'
  content: string
  attachments?: Attachment[]
  notification?: boolean
  /** A react-fired task delivered into this session — machine-sent, not typed. */
  reaction?: boolean
  /** A direct `harness::spawn` task seeding this session — machine-sent, not typed. */
  spawn?: boolean
  /**
   * The firing event (or join inputs) `harness::react` appended to the task,
   * split off by the entry mapper: rendered as collapsible JSON, not prose.
   */
  reactionEvent?: { label: 'event' | 'inputs'; json: string }
}

export interface AssistantMessage extends BaseMessage {
  role: 'assistant'
  content: string
  model?: ModelId
  mode?: Mode
  streaming?: boolean
  /**
   * What the memory worker fed this turn (from the entry origin's hook
   * annotations): the bank, how many memories were injected, and their
   * ids so the chip can fetch details on demand.
   */
  memory?: {
    bank: string
    memories: number
    memoryIds: string[]
    rules?: number
    truncated?: boolean
    /** Recall ran with the semantic (embedding) signal fused in. */
    semantic?: boolean
  }
}

export interface ThoughtMessage extends BaseMessage {
  role: 'thought'
  content: string
  durationMs: number
  streaming?: boolean
}

export interface FunctionTriggerMessage extends BaseMessage {
  role: 'function-trigger'
  functionId: string
  input: unknown
  output?: unknown
  durationMs?: number
  running?: boolean
  /**
   * An `agent_trigger` wrapper whose target function is not known yet (its
   * arguments are still streaming). The UI renders a placeholder instead of
   * the literal `agent_trigger` while running.
   */
  unresolvedTarget?: boolean
  /**
   * The function id was resolved from an enclosing invocation rather than
   * this record itself — e.g. TracesV2 rendering an inner span of
   * `execute <fn>`. The card header stays verb-less: "triggering/triggered
   * ƒ <fn>" would claim an invocation this card's timing doesn't measure.
   */
  identityInherited?: boolean
  /** awaiting user approval before execution; lifecycle: pending → running → done */
  pendingApproval?: boolean
  /** iii function_call_id — set on pending entries so the approve/deny UI can resolve. */
  functionTriggerId?: string
  /** iii session_id owning this call — paired with functionTriggerId for approval::resolve. */
  sessionId?: string
  /**
   * Present when this pending call is a filesystem-access grant request rather
   * than a plain function-trigger approval — renders `FilesystemAccessPrompt`
   * instead of the standard approve/deny/always row.
   */
  filesystemAccess?: {
    requestedRoot: string
    attemptedPath?: string
    errorCode?: string
  }
}

/**
 * A subscription fire, mirrored from the harness `trigger_fired` custom entry
 * (`subscriptions/fired.rs`). Drives the turn-less "trigger fired" chat notice
 * and lets the panel keep a fired `once` trigger visible after it unregisters.
 */
export interface TriggerFiredData {
  subscription_id: string
  /** Engine trigger id — correlates to a live panel row's `id`. */
  trigger_id?: string
  target: 'notify' | 'spawn'
  label?: string
  model?: string
  once: boolean
  /** This fire unregistered the binding (once teardown / join predecessor GC). */
  retired: boolean
  scope?: string
  key?: string
  child_session_id?: string
  join?: {
    id: string
    key: string
    arrived: number
    expected: number
    completed: boolean
  }
  note?: string
  fired_at: number
}

/**
 * `kind: 'compaction'` renders the collapsed-history marker in the
 * transcript. The session-manager transcript is the single source of truth
 * for what the provider sees, so this marker is purely presentational.
 * `kind: 'trigger-fired'` renders a turn-less subscription-fire notice.
 */
export interface SystemMessage extends BaseMessage {
  role: 'system'
  content: string
  tone?: 'info' | 'warn' | 'error'
  kind?: 'notice' | 'compaction' | 'trigger-fired'
  summaryText?: string
  tokensBefore?: number
  /** Present on `kind: 'trigger-fired'`. */
  trigger?: TriggerFiredData
}

export type Message =
  | UserMessage
  | AssistantMessage
  | ThoughtMessage
  | FunctionTriggerMessage
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
  functionTriggerId?: string
  sessionId?: string
  filesystemAccess?: {
    requestedRoot: string
    attemptedPath?: string
    errorCode?: string
  }
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
   * Per-session filesystem scope root. Confines this chat's shell/coder
   * operations to one project directory. Chosen explicitly (no silent default),
   * shown as a full-path banner, and re-scopable mid-conversation.
   */
  workingDir?: string | null
  /**
   * Named memory bank for this chat (session metadata `memory_bank`). The
   * memory worker injects that bank's rules + recalled memories into every
   * turn and extracts new memories back into it. Null = the worker's
   * configured default bank.
   */
  memoryBank?: string | null
  messages: Message[]
  /**
   * Spawn-parent session id, from the child session's
   * `SessionMeta.metadata.parent_session_id` (set by the harness on spawn).
   * Absent for root/orchestrator chats. Drives the sidebar tree grouping.
   */
  parentId?: string
  /** Spawn depth: 0 = root orchestrator (from `metadata.depth`). */
  depth?: number
  /**
   * Who created this child session (from `metadata.spawned_by`, stamped by the
   * harness): a trigger reaction or an agent's direct `harness::spawn`.
   * Absent on root chats and pre-existing sessions. Drives the sidebar icon.
   */
  spawnedBy?: 'trigger' | 'agent'
  /** Driver-owned session status (spinner + sidebar indicator). */
  status?: ConversationStatus
  statusReason?: string
  /** Local-only until the first send creates the session server-side. */
  draft?: boolean
  /**
   * Unsent composer input restored from `SessionMeta.draft` (persisted via
   * `session::set-draft`, so a page refresh doesn't lose what was typed).
   * Distinct from `draft` above, which marks a not-yet-created session.
   */
  draftText?: string
  /** Transcript fetched from session-manager at least once. */
  hydrated?: boolean
  createdAt: number
  updatedAt: number
}

const KNOWN_ROLES: ReadonlySet<Role> = new Set<Role>([
  'user',
  'assistant',
  'thought',
  'function-trigger',
])

export function isKnownRole(role: unknown): role is Role {
  return typeof role === 'string' && KNOWN_ROLES.has(role as Role)
}
