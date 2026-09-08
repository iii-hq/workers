/**
 * session-manager wire types — the subset of the contract in
 * `session-manager/architecture/integration.md` the console consumes.
 */

import type { AttachmentMeta } from '@/lib/attachments/store'

export type SessionStatus = 'idle' | 'working' | 'done' | 'error'

export type ContentBlock =
  | { type: 'text'; text: string }
  /**
   * `data` is empty when the read asked for `include_image_data: false` and
   * the block names its stored original in `attachment_id`; the bytes are
   * then fetched on demand. A block without `attachment_id` always carries
   * its bytes inline — there is nowhere else to get them from.
   */
  | { type: 'image'; mime: string; data: string; attachment_id?: string }
  /**
   * A reference to bytes session-manager keeps (`session::put-attachment`),
   * never the bytes themselves. The harness strips it before the model sees
   * the message; the console draws a chip from it and fetches on demand.
   */
  | {
      type: 'file'
      attachment_id: string
      name: string
      mime: string
      size: number
    }
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

export type AgentMessage =
  | { role: 'user'; content: ContentBlock[]; timestamp: number }
  | {
      role: 'assistant'
      content: ContentBlock[]
      stop_reason: 'end' | 'length' | 'function_call' | 'aborted' | 'error'
      native_stop_reason?: string
      error_message?: string
      /** The agent worker that ran the turn, when it names itself. */
      agent?: string
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
  /** App-defined; console keys coexist with harness linkage/presentation. */
  metadata?: Record<string, unknown>
  forked_from?: string
  /**
   * Unsent composer input parked via `session::set-draft` (event-silent,
   * never bumps `updated_at`); absent when nothing is parked.
   */
  draft?: string
  /**
   * Attachments parked alongside `draft` (uploaded while composing, referenced
   * by `session::set-draft { attachment_ids }`); absent when none. A page
   * refresh rebuilds the composer chips from these.
   */
  draft_attachments?: AttachmentMeta[]
  created_at: number
  updated_at: number
  message_count: number
}

/**
 * One row of `session::messages` / `session::messages-tail` /
 * `session::messages-range` — exactly one of `message` / `custom`.
 *
 * `elided` marks a placeholder inside a collapsed activity run on a tail
 * page: the assistant keeps its text and every `function_call` id + function
 * id but `arguments: {}`, a `function_result` keeps its pairing fields with
 * `content: []`. It says "fetch me later through `session::messages-range`",
 * never "this message was that small".
 */
export type TranscriptItem = {
  entry_id: string
  message?: AgentMessage
  custom?: { custom_type: string; data: unknown }
  origin?: Record<string, unknown>
  elided?: boolean
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
