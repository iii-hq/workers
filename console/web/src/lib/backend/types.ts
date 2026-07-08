import type { Mode, ModelId } from '@/types/chat'

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
    }

export interface ChatStreamOptions {
  signal?: AbortSignal
  /**
   * Reasoning/thinking level for the turn. Sent to `harness::send` as
   * `options.thinking_level`; omitted when 'off' or absent. The provider
   * degrades with a warning when the model can't honor it.
   */
  thinkingLevel?: import('@/types/chat').ThinkingLevel
  /**
   * Per-session filesystem scope root. The real backend forwards it as
   * `harness::send` `options.metadata.fs_scope.root`.
   */
  workingDir?: string | null
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
