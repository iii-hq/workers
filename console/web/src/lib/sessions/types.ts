/**
 * session-manager wire types — the subset of the contract in
 * `session-manager/architecture/integration.md` the console consumes.
 */

export type SessionStatus = 'idle' | 'working' | 'done' | 'error'

export type ContentBlock =
  | { type: 'text'; text: string }
  | { type: 'image'; mime: string; data: string }
  | { type: 'thinking'; text: string; signature?: string }
  | {
      type: 'function_call'
      id: string
      function_id: string
      arguments: unknown
    }
  | {
      type: 'function_result'
      function_call_id: string
      content: ContentBlock[]
      is_error?: boolean
    }

/**
 * Token / cost accounting reported by the provider (session-manager `Usage`).
 *
 * Every field is optional and providers report disjoint subsets: codex never
 * reports `cache_write`, anthropic never reports `reasoning`. Absent means
 * "not reported", which is NOT the same as zero — see `lib/session-usage.ts`.
 *
 * The fields are also not portably additive. Anthropic's `input` EXCLUDES
 * cached tokens (they arrive as separate `cache_read` / `cache_write`), while
 * OpenAI's `input` INCLUDES `cache_read` and its `output` includes
 * `reasoning`. So the only cross-provider total is `input + output`; cache and
 * reasoning are descriptive only. (provider-anthropic/src/sse.rs:180,
 * provider-openai/src/sse.rs:156; matches eval/src/report.rs:59-63.)
 */
export type Usage = {
  input?: number
  output?: number
  cache_read?: number
  cache_write?: number
  reasoning?: number
  cost_usd?: number
}

export type AgentMessage =
  | { role: 'user'; content: ContentBlock[]; timestamp: number }
  | {
      role: 'assistant'
      content: ContentBlock[]
      stop_reason: 'end' | 'length' | 'function_call' | 'aborted' | 'error'
      native_stop_reason?: string
      error_message?: string
      error_kind?: string
      warnings?: string[]
      usage?: Usage
      model: string
      provider: string
      timestamp: number
    }
  | {
      role: 'function_result'
      function_call_id: string
      function_id: string
      content: ContentBlock[]
      details: unknown
      is_error: boolean
      timestamp: number
    }
  | {
      role: 'custom'
      custom_type: string
      content: ContentBlock[]
      display?: string
      details?: unknown
      timestamp: number
    }

export type SessionMeta = {
  session_id: string
  title: string
  description: string
  status: SessionStatus
  status_reason?: string
  /** App-defined; the console stores { surface, model, mode, title_manual }. */
  metadata?: Record<string, unknown>
  forked_from?: string
  /**
   * Unsent composer input parked via `session::set-draft` (event-silent,
   * never bumps `updated_at`); absent when nothing is parked.
   */
  draft?: string
  created_at: number
  updated_at: number
  message_count: number
}

/** One row of `session::messages` — exactly one of `message` / `custom`. */
export type TranscriptItem = {
  entry_id: string
  message?: AgentMessage
  custom?: { custom_type: string; data: unknown }
  origin?: Record<string, unknown>
}

export const SESSION_TRIGGER_TYPES = [
  'session::created',
  'session::message-added',
  'session::message-updated',
  'session::status-changed',
  'session::meta-updated',
  'session::deleted',
] as const

export type SessionTriggerType = (typeof SESSION_TRIGGER_TYPES)[number]

// Event payloads (per the trigger table in integration.md §5).

export type SessionCreatedEvent = {
  session_id: string
  title: string
  description: string
  status: SessionStatus
  forked_from?: string
  created_at: number
}

export type MessageAddedEvent = {
  session_id: string
  entry_id: string
  parent_id: string | null
  message?: AgentMessage
  custom?: { custom_type: string; data: unknown }
  origin?: Record<string, unknown>
  timestamp: number
}

export type MessageUpdatedEvent = {
  session_id: string
  entry_id: string
  message: AgentMessage
  revision: number
  origin?: Record<string, unknown>
  timestamp: number
}

export type StatusChangedEvent = {
  session_id: string
  status: SessionStatus
  previous_status: SessionStatus
  status_reason?: string
  timestamp: number
}

export type MetaUpdatedEvent = {
  session_id: string
  title: string
  description: string
  metadata?: Record<string, unknown>
  timestamp: number
}

export type SessionDeletedEvent = {
  session_id: string
  timestamp: number
}
