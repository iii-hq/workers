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

/** Operating mode — the harness prepends a short paragraph before the identity prompt. */
export type HarnessSendMode = 'plan' | 'ask' | 'agent'

/** Per-send options frozen onto the turn record. */
export interface HarnessSendOptions {
  system_prompt?: string
  mode?: HarnessSendMode
  max_turns?: number
  thinking_level?: HarnessThinkingLevel
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

export interface HarnessSendRequest {
  /** Omit to create a new session. */
  session_id?: string
  /** A string is sugar for a user text message. */
  message: string
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

export interface HarnessStatusReport {
  session_id: string
  turn_id?: string
  status: HarnessTurnStatus
  step: number
  turn_count: number
  depth: number
  max_turns: number
  /** function_call_ids parked awaiting a deferred result / approval. */
  pending_function_calls: string[]
  children: HarnessChildRef[]
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
