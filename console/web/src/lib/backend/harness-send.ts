/**
 * Typed wrappers over the Rust harness's consumer surface — `harness::send`,
 * `harness::stop`, `harness::status`. Shapes mirror the golden schemas in
 * `harness/tests/golden/schemas/harness.{send,stop,status}.json`.
 *
 * In the iii ecosystem every bus invocation is a *trigger*; these helpers go
 * through `IiiClient.trigger`. `harness::send` returns immediately after
 * persisting the user message and seeding (or merging into) the turn — the
 * transcript then streams via session-manager events, not from this response.
 */

import type { IiiClient } from '@/lib/iii-client'

/** Reasoning effort the harness forwards to the router. `off` is never sent. */
export type HarnessThinkingLevel =
  | 'minimal'
  | 'low'
  | 'medium'
  | 'high'
  | 'xhigh'

/** How allowed functions reach the model (harness.md § Exposure modes). */
export type ExposeMode = 'agent_trigger' | 'native'

/**
 * Fail-closed dispatch policy. Absent (or with an empty `allow`) the harness
 * denies every model-requested call — a plain chat loop.
 */
export interface HarnessFunctionPolicy {
  allow?: string[]
  deny?: string[]
  expose?: ExposeMode
}

/** The turn's deliverable; defaults to free text. */
export type HarnessOutputContract =
  | { type: 'text' }
  | { type: 'json'; schema?: unknown }

/** Operating mode — the harness prepends a short paragraph before the
 * identity prompt. In `ask` mode the harness also caps `functions` at its
 * read-only baseline server-side, whatever policy we send. */
export type HarnessSendMode = 'ask' | 'agent'

/** Per-send options frozen onto the turn record. */
export interface HarnessSendOptions {
  system_prompt?: string
  /** How `system_prompt` combines with the built-in identity prompt:
   * `enrich` (harness default) appends it; `override` replaces it
   * verbatim. Snake_case values travel on the wire as-is. */
  system_prompt_strategy?: 'enrich' | 'override'
  mode?: HarnessSendMode
  max_turns?: number
  thinking_level?: HarnessThinkingLevel
  /** Provider-native options, namespaced by provider id. */
  provider_options?: Record<string, unknown>
  output?: HarnessOutputContract
  functions?: HarnessFunctionPolicy
  /** Tracing passthrough (session_id / message_id propagate as baggage). */
  metadata?: Record<string, unknown>
}

/** Session create/ensure options applied when the send materialises it. */
export interface HarnessSessionInit {
  title?: string
  /** Lands on `SessionMeta.metadata` — the tenancy hook. */
  metadata?: Record<string, unknown>
}

/** A text content block on a structured user message. */
export interface HarnessTextBlock {
  type: 'text'
  text: string
}

/**
 * The structured form of `harness::send`'s `message` (MessageInput::Message
 * with `role: user`). The console uses it when a send carries `#file(...)`
 * attachment blocks; plain sends keep the string-sugar form.
 */
export interface HarnessUserMessage {
  role: 'user'
  content: HarnessTextBlock[]
  timestamp: number
}

export interface HarnessSendRequest {
  /** Omit to create a new session. */
  session_id?: string
  /** A string is sugar for a user text message. */
  message: string | HarnessUserMessage
  model: string
  provider?: string
  /** Webhook dedupe: a repeated key returns the original `{session_id, turn_id}`. */
  idempotency_key?: string
  session?: HarnessSessionInit
  options?: HarnessSendOptions
}

export interface HarnessSendResponse {
  session_id: string
  turn_id: string
  accepted: boolean
  /** True when folded into an in-flight turn (steering). */
  merged?: boolean
  /**
   * True when the message was queued while a step was streaming; it lands in
   * the transcript when the stream ends.
   */
  queued?: boolean
  /** True when `idempotency_key` matched an earlier send. */
  deduplicated?: boolean
}

/** The coarse, harness-internal turn lifecycle (harness.md § API Reference). */
export type HarnessTurnStatus =
  | 'running'
  | 'awaiting_functions'
  | 'completed'
  | 'cancelled'
  | 'failed'

export interface HarnessChildRef {
  function_call_id: string
  session_id: string
  turn_id: string
}

/**
 * One message parked in the harness's per-session queue while a step streams
 * (mirrors `harness_queue` rows surfaced by `harness::status`). `message` is
 * the wire AgentMessage; only text blocks matter for preview purposes.
 */
export interface HarnessQueuedMessage {
  id: string
  session_id: string
  /** Deterministic transcript entry id the drain will append under. */
  entry_id: string
  message: {
    role: string
    content?: Array<{ type: string; text?: string }>
  }
  queued_at: number
}

export interface HarnessStatusReport {
  session_id: string
  turn_id?: string
  status: HarnessTurnStatus
  step: number
  turn_count: number
  depth: number
  max_turns: number
  validation_retries: number
  max_validation_retries: number
  transient_resumes: number
  max_transient_resumes: number
  partial_result_available: boolean
  /** function_call_ids parked awaiting a deferred result / approval. */
  pending_function_calls: string[]
  children: HarnessChildRef[]
  /** Messages queued while a step streams, in arrival order. */
  queued?: HarnessQueuedMessage[]
  result?: unknown
  result_error?: string | null
}

export interface HarnessStopResponse {
  stopping: boolean
}

/** Whether a status report represents a turn that is still in flight. */
export function isTurnActive(status: HarnessTurnStatus): boolean {
  return status === 'running' || status === 'awaiting_functions'
}

/**
 * The deterministic session-manager entry id the harness assigns to the user
 * message when `harness::send` carries an `idempotency_key` — `e_idem_<key>`
 * with unsafe characters replaced (mirrors `harness/src/ids.rs`). The console
 * predicts it so an optimistically-appended user row reconciles in place when
 * the `session::message-added` snapshot arrives.
 */
export function predictedUserEntryId(idempotencyKey: string): string {
  const safe = idempotencyKey.replace(/[^A-Za-z0-9_-]/g, '_')
  return `e_idem_${safe}`
}

/** Chat-side system-prompt selection: a resolved body + combine strategy. */
export interface SystemPromptSelection {
  body: string
  strategy: 'enrich' | 'override'
}

/**
 * Map a selection to `harness::send` option fields. Default (null) or a
 * blank body yields NO fields — the send looks exactly like today's.
 */
export function toSystemPromptOptions(
  sel: SystemPromptSelection | null | undefined,
): Pick<HarnessSendOptions, 'system_prompt' | 'system_prompt_strategy'> {
  if (!sel || !sel.body.trim()) return {}
  return { system_prompt: sel.body, system_prompt_strategy: sel.strategy }
}

/**
 * Trigger `harness::send`: persist the user message and seed (or merge into)
 * the turn. Returns before the turn runs.
 */
export function sendTurn(
  client: Pick<IiiClient, 'trigger'>,
  req: HarnessSendRequest,
): Promise<HarnessSendResponse> {
  return client.trigger<HarnessSendResponse>(
    'harness::send',
    req as unknown as Record<string, unknown>,
  )
}

/**
 * Trigger `harness::stop`: set the abort flag and `router::abort` any live
 * stream. Omit `turnId` to stop the session's current turn.
 */
export async function stopTurn(
  client: Pick<IiiClient, 'trigger'>,
  sessionId: string,
  turnId?: string,
): Promise<boolean> {
  const res = await client.trigger<HarnessStopResponse>('harness::stop', {
    session_id: sessionId,
    ...(turnId ? { turn_id: turnId } : {}),
  })
  return Boolean(res?.stopping)
}

/**
 * Trigger `harness::status`: a point-in-time read of the session's turn
 * record. Returns `null` when no turn has ever run for the session.
 */
export function getTurnStatus(
  client: Pick<IiiClient, 'trigger'>,
  sessionId: string,
): Promise<HarnessStatusReport | null> {
  return client.trigger<HarnessStatusReport | null>('harness::status', {
    session_id: sessionId,
  })
}
