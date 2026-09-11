/**
 * Wire payloads for the reactive surface the console binds around a harness
 * turn. The Rust harness no longer publishes an `agent::events` firehose;
 * instead it emits async turn-boundary triggers, and approval-gate owns the
 * human-in-the-loop inbox triggers. These hand-written shapes mirror the
 * golden schemas so the translator (`lib/backend/translate.ts`) can consume
 * the trigger JSON without a generated client:
 *
 *   - `harness::turn-started` / `harness::turn-completed`
 *     (`harness/src/events.rs`)
 *   - `approval::pending-created` / `approval::pending-resolved`
 *     (`approval-gate/tests/golden/schemas/approval.pending-*.json`)
 *
 * Transcript CONTENT (assistant text, thinking, function-trigger blocks, function
 * results) is NOT carried here — it renders from `session::message-added` /
 * `session::message-updated` reconciled by the conversations layer
 * (`lib/sessions/*`). These trigger payloads carry only turn lifecycle and
 * approval state.
 */

/** Parent linkage on a sub-agent turn (set only for spawned children). */
export interface TurnParentLink {
  session_id: string
  turn_id: string
  function_call_id: string
}

/** `harness::turn-started` — a turn began executing (first loop step). */
export interface TurnStartedEvent {
  session_id: string
  turn_id: string
  timestamp: number
  parent?: TurnParentLink
}

/**
 * `harness::turn-completed` — a turn reached a terminal status, carrying its
 * result (or the failure reason). The session's coarse status flips in
 * lock-step (`done` for completed/cancelled, `error` for failed).
 */
export interface TurnCompletedEvent {
  session_id: string
  turn_id: string
  status: 'completed' | 'cancelled' | 'failed'
  /** Present on a turn with an output contract. */
  result?: unknown
  /** Present when the turn failed; accepted on legacy completed events. */
  result_error?: string
  /** Short human-readable reason (failed / cancelled). */
  reason?: string
  /**
   * `false` while the session still owns an armed wake (a one-shot notify
   * subscription): the run continues and a later turn in this session carries
   * the real outcome. Absent on legacy events — treat as terminal.
   */
  terminal?: boolean
  timestamp: number
  parent?: TurnParentLink
}

/**
 * `harness::message-queued` — a message parked in the session's server-side
 * queue while a turn step streams. A refresh signal, not the message itself:
 * read the queue via `harness::status` → `queued`.
 */
export interface MessageQueuedEvent {
  session_id: string
  /** Transcript entry id the row lands under when the queue drains. */
  entry_id: string
  queued_at: number
  timestamp: number
}

/**
 * `harness::triggers-changed` — the session's trigger-binding set or a fire
 * count changed. A doorbell, not the data: refetch `harness::triggers::list`.
 */
export interface TriggersChangedEvent {
  session_id: string
  timestamp: number
}

/** Outcome of a resolved approval (approval-gate `ResolvedOutcome`). */
export type ResolvedOutcome = 'allow' | 'deny' | 'timeout' | 'aborted'

/**
 * `approval::pending-created` payload (the `PendingApprovalRecord`). A model
 * function trigger was held for a human. `arguments_excerpt` is redacted and
 * clipped — safe to render, but the un-redacted input also arrives via the
 * session transcript's `function_call` block.
 */
export interface PendingApprovalRecord {
  function_call_id: string
  function_id: string
  session_id: string
  turn_id: string
  arguments_excerpt?: unknown
  assistant_excerpt?: string | null
  depth?: number
  pending_at: number
  expires_at: number
  session_title?: string | null
  session_description?: string | null
  session_metadata?: Record<string, unknown> | null
  /** Always `"pending"` on the create event; absent on `list-pending` rows. */
  status?: 'pending'
  /** Discriminates a plain function-trigger approval from a filesystem-access request. */
  kind?: 'function' | 'filesystem_access'
  /** Present when `kind === 'filesystem_access'` — the jail-scope rejection that
   * triggered the ask. */
  access_request?: {
    requested_root: string
    attempted_path: string
    error_code: string
  }
}

/** `approval::pending-resolved` payload — a held call left the inbox. */
export interface PendingResolvedEvent {
  function_call_id: string
  function_id: string
  session_id: string
  turn_id: string
  outcome: ResolvedOutcome
  /** Operator-supplied on deny. */
  reason?: string | null
  session_metadata?: Record<string, unknown> | null
  resolved_at: number
}
