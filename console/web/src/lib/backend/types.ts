import type { Mode, ModelId } from '@/types/chat'
import type { SessionTriggerInfo } from './triggers'

/**
 * The streaming contract every ChatBackend honors. The order is:
 *   (thought-start ... thought-token* ... thought-end)?
 *   (fcall-start ... fcall-end)*
 *   (assistant-token+ ... assistant-end)?
 *
 * Aborts may interrupt at any token boundary; the consumer's `finally` is
 * responsible for closing out streaming flags. Errors that aren't aborts
 * surface either as a thrown exception or as an `fcall-end` payload whose
 * `output` carries the error shape (caller-defined).
 *
 * See PLAYGROUND.md for the full contract.
 */
export type StreamEvent =
  | { kind: 'thought-start' }
  | { kind: 'thought-token'; token: string }
  | { kind: 'thought-end'; durationMs: number }
  | {
      kind: 'fcall-start'
      functionId: string
      input: unknown
      pendingApproval?: boolean
      /** iii function_call_id — needed to resolve approval. */
      functionCallId?: string
      /** iii session_id owning this call — needed to resolve approval. */
      sessionId?: string
      /**
       * Present when the pending approval is a filesystem-access request
       * (`PendingApprovalRecord.kind === 'filesystem_access'`) rather than a
       * plain function-call approval. Drives `FilesystemAccessPrompt` instead of
       * the standard approve/deny/always row.
       */
      filesystemAccess?: {
        requestedRoot: string
        attemptedPath?: string
        errorCode?: string
      }
    }
  | {
      kind: 'fcall-end'
      output: unknown
      durationMs: number
      /**
       * iii function_call_id of the call that ended. Parallel tool calls (and
       * approval-resolved calls, which emit no fcall-start) finish out of
       * order, so the consumer MUST match the end to its card by this id, not
       * by "the most recently started call".
       */
      functionCallId?: string
    }
  | {
      /**
       * A call left the pending-approval set without executing (aborted, or
       * resolved out-of-band): clear its card's approval prompt so it doesn't
       * hang on "approving…". A resolved-and-executed call also emits
       * `fcall-end`; both patching to non-pending is idempotent.
       */
      kind: 'fcall-approval-cleared'
      functionCallId: string
      /**
       * True when the approval was allowed: the released call is now
       * executing, so the card flips to the running state until its result
       * pairs in from the transcript.
       */
      running?: boolean
    }
  | { kind: 'assistant-token'; token: string }
  | { kind: 'assistant-end' }
  | {
      /**
       * A compaction finished. ChatView consumes this to append a compaction
       * marker into the conversation so the CTX bar drops to the
       * post-compaction count without wiping the user's transcript. On the
       * real backend the marker renders from the session's `compaction` custom
       * entry instead; this event is emitted by mock/playground backends.
       */
      kind: 'compaction'
      /** 'async' = post-TurnEnd background; 'sync' = pre-flight in-turn. */
      mode: 'async' | 'sync'
      summaryText: string
      tokensBefore: number
      compactionEntryId: string
      tailStartId: string | null
    }
  | {
      /**
       * Surfaces the assistant turn's terminal `stop_reason` when it's
       * NOT a clean `end`. Without this the UI used to swallow every
       * non-clean termination — `length` (max_tokens hit), `error`
       * (provider failure mid-stream), `aborted` (user/timeout abort)
       * — and the user saw a truncated reply with no indication of why.
       * ChatView renders this as a system notice with appropriate tone.
       */
      kind: 'stop-reason'
      reason: 'length' | 'error' | 'aborted' | 'function_call'
      /** Optional explanatory text, e.g. the provider's error_message. */
      message?: string
      /** Stable transcript id when this notice also has a persisted entry. */
      entryId?: string
      /** The assistant output above is preserved evidence, not a successful result. */
      partialResultAvailable?: boolean
    }
  | {
      /**
       * Pre-content turn lifecycle, for the thinking shimmer's detail line.
       * `accepted` = harness::send seeded a turn (step-0 sits on the durable
       * queue); `merged` = folded into an in-flight turn; `queued` = parked
       * in the server-side queue (the queued strip owns that surface);
       * `started` = harness::turn-started fired, the loop is running.
       */
      kind: 'turn-status'
      phase: 'accepted' | 'merged' | 'queued' | 'started'
    }

export interface ChatStreamOptions {
  signal?: AbortSignal
  /**
   * Reasoning effort for the turn. `default` omits the override; exact native
   * values also travel through namespaced provider options.
   */
  thinkingLevel?: import('@/types/chat').ThinkingLevel
  /**
   * Per-session filesystem scope root. The real backend forwards it as
   * `harness::send` `options.metadata.fs_scope.root`.
   */
  workingDir?: string | null
  /**
   * Named directory prompt. The real backend resolves it via
   * `directory::prompts::get` on every send and forwards body + strategy as
   * `harness::send` `options.system_prompt` / `system_prompt_strategy`.
   * A failed fetch FAILS the send (no silent fallback).
   */
  systemPromptName?: string | null
  /**
   * Extra text content blocks appended after the prompt on the outgoing
   * user message — `#file(...)` mention expansions (`<attached-file …>`
   * blocks). When present the real backend sends a structured
   * `harness::send` message instead of the string-sugar form. Mock
   * backends ignore this.
   */
  attachedBlocks?: string[]
  /** mean delay between assistant tokens, in ms */
  meanDelayMs?: number
  /**
   * Per-send message id (`msg-<uuid>`). The harness derives the user
   * message's session-manager entry id from it (`<message_id>-user-0`), so
   * the console's optimistic user message reconciles in place when the
   * `session::message-added` snapshot arrives. The real backend mints one
   * when omitted.
   */
  messageId?: string
  /**
   * Stable session_id for the chat conversation. All `stream()` calls
   * for the same conversation must pass the same value so the engine
   * groups every turn under one session in the traces UI.
   *
   * The real backend uses it as the `session_id` in `harness::send` and as
   * the config filter on its scoped `harness::turn-completed` / `approval::*`
   * trigger subscriptions; the mock backend ignores it. When omitted, the
   * real backend falls back to a fresh `console-<uuid>` (one session per
   * send) so callers that haven't been updated yet still work.
   */
  sessionId?: string
  /**
   * Whether the optional standalone `approval-gate` worker is connected. When
   * `false`, the real backend skips the `approval::pending-*` trigger
   * subscriptions and the `approval::list-pending` catch-up read — those
   * functions don't exist without the gate. Defaults to `true` so callers that
   * haven't been updated keep the approval surface.
   */
  approvalGateAvailable?: boolean
  /**
   * Whether a pending approval belongs on this chat surface. The real backend
   * uses an unscoped approval subscription when provided, then applies this
   * live predicate so approvals held by newly-created subagent sessions can be
   * mirrored into their parent chat while retaining the child session id for
   * resolution.
   */
  approvalSessionMatcher?: (approval: {
    session_id: string
    session_metadata?: Record<string, unknown> | null
  }) => boolean
  /**
   * ChatView keeps approval events subscribed for the lifetime of the selected
   * conversation. Set this on a turn stream to avoid a second subscription
   * delivering the same pending card while that watcher is active.
   */
  approvalEventsExternallyManaged?: boolean
}

/** Events that an approval watcher can deliver outside a running turn. */
export type ApprovalStreamEvent = Extract<
  StreamEvent,
  { kind: 'fcall-start' | 'fcall-approval-cleared' }
>

export interface ApprovalWatchOptions {
  /** Root session displayed by the chat surface. */
  sessionId: string
  /** Skip all approval RPCs when the optional gate worker is absent. */
  approvalGateAvailable?: boolean
  /** Filters global approval events to this conversation and its descendants. */
  approvalSessionMatcher?: ChatStreamOptions['approvalSessionMatcher']
  /** Changes when the visible child-session tree changes, forcing a catch-up. */
  refreshKey?: string
}

export type CompactResult =
  | {
      status: 'ok'
      tokensBefore: number
      autoContinued: boolean
      summaryText: string
    }
  | { status: 'busy' }
  | { status: 'overflow'; message: string }
  | { status: 'empty' }
  | { status: 'error'; message: string }

/**
 * Preview of a message parked in the harness's server-side queue while a step
 * streams. `id` is the deterministic transcript entry id the drain will
 * append under — the same id the drained row arrives with, so consumers can
 * dedupe against local drafts and the transcript.
 */
export interface QueuedMessagePreview {
  id: string
  text: string
  queuedAt: number
}

export interface ChatBackend {
  /** stable identifier used by the playground for telemetry / labels */
  readonly id: string
  stream(
    prompt: string,
    mode: Mode,
    model: ModelId,
    opts?: ChatStreamOptions,
  ): AsyncGenerator<StreamEvent>
  /**
   * Watch approvals independently of a parent turn. This keeps subagent
   * approvals actionable after `harness::spawn` has returned to the parent.
   */
  watchApprovals?(
    opts: ApprovalWatchOptions,
    onEvent: (event: ApprovalStreamEvent) => void,
  ): () => void
  /**
   * Resolve a pending approval. Returns when the iii bus accepts the
   * decision; the actual session resume happens asynchronously via
   * approval-gate's `resume_session` poll.
   */
  resolveApproval?(
    sessionId: string,
    functionCallId: string,
    decision: 'allow' | 'deny',
    /**
     * Filesystem access duration. Sent only for filesystem_access resolutions.
     */
    opts?: { accessDuration?: 'once' | 'session' | 'always' },
  ): Promise<void>
  /**
   * Send a message into a session whose turn is already streaming, WITHOUT
   * opening a second stream loop. The harness queues it (`queued: true`) and
   * delivers it after the stream ends; the row then renders via session
   * events. Backends that don't support mid-stream queueing omit this and the
   * composer stays locked while streaming.
   */
  queueMessage?(
    prompt: string,
    mode: Mode,
    model: ModelId,
    opts?: ChatStreamOptions,
  ): Promise<void>
  /**
   * The session's server-side message queue (`harness::status` → `queued`):
   * everything parked while the current step streams — including messages
   * from other tabs and subagent/subscription notifications, not just this
   * tab's sends. Empty when idle.
   */
  listQueued?(sessionId: string): Promise<QueuedMessagePreview[]>
  /**
   * Remove a still-parked message from the server-side queue by its entry id
   * (`harness::unqueue`). Lets the composer pull a queued message back for
   * editing without the old version also draining into the transcript. A row
   * that already drained is a harmless no-op.
   */
  removeQueued?(sessionId: string, entryId: string): Promise<void>
  /**
   * Edit a still-parked queued message in place (`harness::edit_queued`),
   * preserving its delivery position — unlike remove + re-queue, which moves
   * it to the tail. `prompt` + `attachedBlocks` rebuild the content exactly
   * as a send would.
   */
  editQueued?(
    sessionId: string,
    entryId: string,
    prompt: string,
    opts?: { attachedBlocks?: string[] },
  ): Promise<void>
  /**
   * Subscribe to `harness::message-queued` for a session: fires when any
   * client's message parks in the server-side queue mid-stream — the signal
   * to refetch `listQueued`. Returns an unsubscribe.
   */
  onQueuedMessage?(sessionId: string, onEvent: () => void): () => void
  /**
   * The session's registered trigger subscriptions (notify + react bindings
   * the agent registered via the harness's `engine::register_trigger`
   * intercept).
   */
  listTriggers?(sessionId: string): Promise<SessionTriggerInfo[]>
  /** Unregister one of the session's triggers by engine trigger id. */
  unregisterTrigger?(triggerId: string): Promise<void>
  /**
   * Whether a state key currently exists (`state::get` non-null). Lets the
   * triggers strip mark a `state` binding whose watched key was never
   * written — the "armed on something nothing produces" stall. `null` =
   * unknown (call failed).
   */
  stateKeyExists?(
    scope: string | undefined,
    key: string,
  ): Promise<boolean | null>
  /**
   * Server-side cancel of the session's in-flight turn (`harness::stop`).
   * The client-side AbortSignal only stops rendering; without this the
   * harness keeps running the turn to completion.
   */
  abortRun?(sessionId: string): Promise<void>
  /**
   * Powers `/compact`. Compacts the session-manager transcript (the single
   * source of truth) directly. `contextWindow` skips the server's
   * `models::get` lookup when known.
   */
  compactSession?(
    sessionId: string,
    model: ModelId,
    contextWindow?: number,
  ): Promise<CompactResult>
}
