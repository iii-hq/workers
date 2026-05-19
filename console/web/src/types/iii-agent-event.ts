/**
 * Hand-written TypeScript shapes for the iii agent event stream.
 *
 * These mirror the Rust types in
 * `harness/crates/harness-types/src/{agent_event,agent_message,content,function}.rs`
 * so console/web's translator (`lib/backend/translate.ts`) can consume the
 * wire JSON without depending on a generated client. Keep this file in
 * lock-step with the Rust enums when new variants land — the translator's
 * exhaustive switch will catch most drifts, but new optional fields slip
 * through silently.
 *
 * Wire conventions:
 *   - `AgentEvent`     is tagged with `type` (snake_case).
 *   - `AgentMessage`   is tagged with `role` (snake_case).
 *   - `ContentBlock`   is tagged with `type` (camelCase, e.g. `functionCall`).
 *   - Legacy aliases (`tool_call_id`, `tool_name`, `tool_execution_*`,
 *     `toolCall`, `tool_result`) are tolerated by the Rust enums via
 *     serde aliases; we type the canonical fields only.
 */

/** Free-form text block. */
export interface TextContent {
  text: string
}

/** Base64 image block. */
export interface ImageContent {
  /** MIME type like `image/png`. */
  mime: string
  /** Base64-encoded bytes. */
  data: string
}

/** Extended-thinking content block (Anthropic, OpenAI reasoning, etc.). */
export interface ThinkingContent {
  text: string
  signature?: unknown
}

/** A content block inside `AgentMessage.content`. */
export type ContentBlock =
  | ({ type: 'text' } & TextContent)
  | ({ type: 'image' } & ImageContent)
  | {
      type: 'functionCall'
      id: string
      function_id: string
      arguments: unknown
    }
  | {
      type: 'functionResult'
      function_call_id: string
      content: ContentBlock[]
      is_error: boolean
    }
  | ({ type: 'thinking' } & ThinkingContent)

export interface UserMessage {
  role: 'user'
  content: ContentBlock[]
  timestamp: number
}

export interface AssistantMessage {
  role: 'assistant'
  content: ContentBlock[]
  stop_reason: string
  error_message?: string
  error_kind?: string
  usage?: unknown
  model: string
  provider: string
  timestamp: number
}

export interface FunctionResultMessage {
  role: 'function_result'
  function_call_id: string
  function_id: string
  content: ContentBlock[]
  details: unknown
  is_error: boolean
  timestamp: number
}

export interface CustomMessage {
  role: 'custom'
  custom_type: string
  content: unknown
  display?: string
  details?: unknown
  timestamp: number
}

export type AgentMessage =
  | UserMessage
  | AssistantMessage
  | FunctionResultMessage
  | CustomMessage

/** Function call request emitted by an assistant message. */
export interface FunctionCall {
  id: string
  function_id: string
  arguments: unknown
}

/** Result of executing a function. */
export interface FunctionResult {
  content: ContentBlock[]
  details: unknown
  terminate?: boolean
}

/**
 * Provider-side streaming event. Mirrors the Rust enum
 * `harness/crates/harness-types/src/stream_event.rs::AssistantMessageEvent`.
 *
 * The orchestrator forwards each event verbatim inside an
 * `AgentEvent::MessageUpdate` so the frontend can render token-by-token
 * (Phase 2.A). `Done`/`Error` are terminal — the orchestrator emits a
 * separate `MessageStart`/`MessageEnd` pair after them.
 */
export type AssistantMessageEvent =
  | { type: 'start'; partial: AssistantMessage }
  | { type: 'text_start'; partial: AssistantMessage }
  | { type: 'text_delta'; partial: AssistantMessage; delta: string }
  | { type: 'text_end'; partial: AssistantMessage }
  | { type: 'thinking_start'; partial: AssistantMessage }
  | { type: 'thinking_delta'; partial: AssistantMessage; delta: string }
  | { type: 'thinking_end'; partial: AssistantMessage }
  | { type: 'functioncall_start'; partial: AssistantMessage }
  | { type: 'functioncall_delta'; partial: AssistantMessage; delta: string }
  | { type: 'functioncall_end'; partial: AssistantMessage }
  | {
      type: 'usage'
      input: number
      output: number
      cache_read?: number
      cache_write?: number
      cost_usd?: number | null
    }
  | {
      type: 'stop'
      stop_reason: 'end' | 'length' | 'function_call' | 'aborted' | 'error'
      error_message?: string
      error_kind?:
        | 'auth_expired'
        | 'rate_limited'
        | 'context_overflow'
        | 'transient'
        | 'permanent'
    }
  | { type: 'done'; message: AssistantMessage }
  | { type: 'error'; error: AssistantMessage }

/**
 * Discriminated `AgentEvent` matching the wire shape on `agent::events`.
 * `MessageUpdate` and `FunctionExecutionUpdate` are present in the enum but
 * not emitted by today's turn-orchestrator — the translator stubs them out
 * but accepts them so a Phase 2 backend lands without a frontend change.
 */
export type AgentEvent =
  | { type: 'agent_start' }
  | { type: 'agent_end'; messages: AgentMessage[] }
  | { type: 'turn_start' }
  | {
      type: 'turn_end'
      message: AgentMessage
      function_results: FunctionResultMessage[]
    }
  | { type: 'message_start'; message: AgentMessage }
  | {
      type: 'message_update'
      message: AgentMessage
      llm_event: AssistantMessageEvent
    }
  | { type: 'message_end'; message: AgentMessage }
  | {
      type: 'function_execution_start'
      function_call_id: string
      function_id: string
      args: unknown
    }
  | {
      type: 'function_execution_update'
      function_call_id: string
      function_id: string
      args: unknown
      partial_result: unknown
    }
  | {
      type: 'function_execution_end'
      function_call_id: string
      function_id: string
      result: FunctionResult
      is_error: boolean
    }
  | {
      type: 'turn_state_changed'
      event_type: 'state:created' | 'state:updated'
      new_value: Record<string, unknown>
      old_value?: Record<string, unknown>
    }

export type TurnStateChangedEvent = Extract<
  AgentEvent,
  { type: 'turn_state_changed' }
>

/**
 * Envelope the harness fanout pushes to `ui::session::event::<browser_id>`.
 * See `harness/src/fanout.rs` `subscribers_for(session_id)`.
 */
export interface SessionEventEnvelope {
  session_id: string
  event: AgentEvent
}
