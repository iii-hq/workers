import type { SystemPromptState } from '@/components/chat/system-prompt-selection'

export type Mode = 'ask' | 'agent'

/** Composite `provider::<catalog_model_id>` (matches harness models-catalog). */
export const CATALOG_MODEL_KEY_SEP = '::' as const

export type ModelId = string

export interface ModelOption {
  id: ModelId
  label: string
  contextWindow?: number
  supportsThinking?: boolean
  /**
   * Whether the model reads images. `undefined` means the router did not say —
   * an older catalog, or a model it has no row for — and callers treat that as
   * "assume it can" rather than refusing to send a picture on missing metadata.
   */
  supportsVision?: boolean
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
  /**
   * Harness binding that produced a machine-authored notification. Live
   * entries carry it in `origin.binding`; persisted histories recover it
   * from the deterministic `e_fire_*` / `e_expire_*` entry id.
   */
  triggerBindingId?: string
  /** A trigger-fired task delivered into this session — machine-sent, not typed. */
  reaction?: boolean
  /** A direct `harness::spawn` task seeding this session — machine-sent, not typed. */
  spawn?: boolean
  /**
   * A validation nudge: the harness re-prompting the turn after the output
   * contract or a `post-turn` validator rejected its result — machine-sent,
   * never typed by anyone.
   */
  validation?: boolean
  /**
   * The firing event (or join inputs) `harness::spawn` appended to the task,
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
   * Why the provider ended this assistant entry. `function_call` marks an
   * intermediate update that continues into tools; `end` marks the turn's
   * final prose. Older/local fixtures may omit it.
   */
  stopReason?: 'end' | 'length' | 'function_call' | 'aborted' | 'error'
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
  /**
   * Short user-facing action supplied by the agent_trigger wrapper. Calls
   * recorded before this field existed omit it and keep the function-id
   * fallback.
   */
  description?: string
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
  /** Engine trigger id. */
  trigger_id?: string
  /**
   * The binding's target function id (`harness::send` for a wake, else the
   * called function). Records written before the delivery hop carry the
   * legacy words `'notify'` / `'spawn'`; historical spawn records may also
   * carry `model` / `child_session_id`.
   */
  target: string
  /** Trigger source and canonical registration config on newer records. */
  trigger_type?: string
  config?: unknown
  label?: string
  /** Human-readable event text declared as registration metadata.action. */
  action?: string
  model?: string
  once: boolean
  /** Durable binding fire counter after this activity; absent on older records. */
  fires?: number
  /** This activity retired the binding; inspect `retirement_reason` for why. */
  retired: boolean
  scope?: string
  key?: string
  note?: string
  /** Structured outcome on newer records; absent on historical transcripts. */
  outcome?:
    | 'delivered'
    | 'delivery_failed'
    | 'skipped'
    | 'expired'
    | 'unregistered'
    | 'invalidated'
  /** Why a binding was retired, when the outcome ended its lifecycle. */
  retirement_reason?:
    | 'once_consumed'
    | 'max_fires'
    | 'expired'
    | 'unregistered'
    | 'invalidated'
    | 'exhausted'
  /**
   * What the fire delivered: the payload sent to a ƒ-call target (post
   * conditions/projection/stamping; the attempted payload when dispatch
   * failed), or the post-conditions event of a wake. Absent on skip/expiry/gc
   * records and on records from before this field existed.
   */
  payload?: unknown
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
  /** User-facing remediation supplied by a structured lifecycle record. */
  nextActions?: string[]
  /** Diagnostic context kept behind a collapsed disclosure. */
  technicalDetails?: SystemNoticeTechnicalDetails
  /**
   * Live-only fallback for a durable transcript entry with the same id.
   * It may fill a delivery gap, but must never replace the transcript-backed
   * message when lifecycle and transcript events arrive out of order.
   */
  provisional?: boolean
  summaryText?: string
  tokensBefore?: number
  /** Present on `kind: 'trigger-fired'`. */
  trigger?: TriggerFiredData
}

export interface SystemNoticeTechnicalDetails {
  code?: string
  class?: string
  detail?: string
  provider?: string
  model?: string
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
  nextActions?: string[]
  technicalDetails?: SystemNoticeTechnicalDetails
  summaryText?: string
  tokensBefore?: number
}

/** Mirrors session-manager's SessionStatus. */
export type ConversationStatus = 'idle' | 'working' | 'done' | 'error'

export interface ConversationMetadataEdits {
  title?: string
  titleManual?: boolean
  model?: ModelId | null
  thinkingLevel?: ThinkingLevel
  mode?: Mode
  workingDir?: string | null
  memoryBank?: string | null
  systemPrompt?: SystemPromptState
  /** An own `undefined` value clears the selected agent profile. */
  agentProfile?: AgentProfileSnapshot | undefined
  /** An own `undefined` value records an explicit All-skills selection. */
  skills?: string[] | undefined
}

/** Harness-provided presentation hints for a spawned sub-agent session. */
export type SubagentIcon =
  | 'agent'
  | 'code'
  | 'search'
  | 'terminal'
  | 'database'
  | 'test'
  | 'review'
  | 'docs'
  | 'design'

export type SubagentColor =
  | 'neutral'
  | 'blue'
  | 'purple'
  | 'teal'
  | 'green'
  | 'amber'
  | 'rose'

/**
 * Mirrors `SessionMeta.metadata.subagent_display`. The enums deliberately
 * keep arbitrary wire values out of CSS classes and icon lookup tables.
 */
export interface SubagentAppearance {
  name: string
  icon?: SubagentIcon
  color?: SubagentColor
}

/** Directory agent profile frozen onto session metadata at selection/send. */
export interface AgentProfileSnapshot {
  id: string
  name: string
  model?: ModelId
  reasoningEffort?: ThinkingLevel
  icon?: SubagentIcon
  color?: SubagentColor
}

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
  /**
   * Per-session reasoning effort, persisted as session metadata. Drafts seed
   * it from the most recently chosen effort, just like `model`.
   */
  thinkingLevel?: ThinkingLevel
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
  /**
   * System-prompt choice for this chat (session metadata `system_prompt`).
   * Picked on the new-session screen before the first send, read-only after
   * it — which is why it lives on the record rather than in ChatView state:
   * ChatPanel keys ChatView by conversation id, so a local reset on a tab
   * switch would be invisible with no interactive control left to show it.
   * Omitted = `DEFAULT_SYSTEM_PROMPT_STATE`.
   */
  systemPrompt?: SystemPromptState
  /** Selected Directory agent profile, frozen for session identity/rendering. */
  agentProfile?: AgentProfileSnapshot
  /** Undefined/empty means all model-invocable skills; otherwise exact IDs. */
  skills?: string[]
  /**
   * One-time conversion of legacy skill addons after an authoritative empty
   * transcript read. `candidate` retains the legacy body while hydration is
   * pending; `ready` carries the complete body-free metadata replacement.
   * `empty` remembers a successful empty transcript read so metadata that
   * arrives later can be finalized immediately.
   */
  legacySkillMigration?:
    | {
        state: 'empty'
        metadata?: Record<string, unknown>
        edits?: ConversationMetadataEdits
      }
    | {
        state: 'candidate' | 'ready'
        metadata: Record<string, unknown>
        edits?: ConversationMetadataEdits
      }
  /** Whether durable transcript entries already exist for this session. */
  started?: boolean
  /** Raw SessionMeta.metadata, retained because session::set-meta replaces it. */
  sessionMetadata?: Record<string, unknown>
  /** Last authoritative SessionMeta/event timestamp; excludes local UI edits. */
  serverMetaUpdatedAt?: number
  /** Last authoritative title/metadata timestamp for unordered events. */
  serverMetadataUpdatedAt?: number
  /** Last authoritative lifecycle timestamp for unordered status events. */
  serverStatusUpdatedAt?: number
  messages: Message[]
  /**
   * Spawn-parent session id, from the child session's
   * `SessionMeta.metadata.parent_session_id` (set by the harness on spawn).
   * Absent for root/orchestrator chats. Drives the sidebar tree grouping.
   */
  parentId?: string
  /** Parent function call that created this sub-agent session. */
  parentFunctionCallId?: string
  /** Spawn depth: 0 = root orchestrator (from `metadata.depth`). */
  depth?: number
  /**
   * Who created this child session (from `metadata.spawned_by`, stamped by the
   * harness): a trigger reaction or an agent's direct `harness::spawn`.
   * Absent on root chats and pre-existing sessions. Drives the sidebar icon.
   */
  spawnedBy?: 'trigger' | 'agent'
  /** Optional chip identity from `metadata.subagent_display`. */
  subagentAppearance?: SubagentAppearance
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
