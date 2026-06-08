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
    }
  | { kind: 'fcall-end'; output: unknown; durationMs: number }
  | { kind: 'assistant-token'; token: string }
  | { kind: 'assistant-end' }
  | {
      /**
       * Server-side context-compaction finished rewriting the session's
       * flat-state. ChatView consumes this to append a compaction marker
       * into the conversation so the CTX bar drops to the post-compaction
       * count without wiping the user's transcript.
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
  /** mean delay between assistant tokens, in ms */
  meanDelayMs?: number
  /**
   * Stable session_id for the chat conversation. All `stream()` calls
   * for the same conversation must pass the same value so the engine
   * groups every turn under one session in the traces UI.
   *
   * The real backend uses it as the `group_id` of its scoped `agent::events`
   * stream trigger and as `session_id` in `harness::trigger`; the mock backend
   * ignores it. When omitted, the real
   * backend falls back to a fresh `console-<uuid>` so callers that
   * haven't been updated yet still work (with the pre-fix behavior of
   * one session per send).
   *
   * Mirrors the `harness/web` strategy: see
   * `workers/harness/web/src/App.tsx` `newSessionId()` and the
   * `active ?? draftId ?? newSessionId()` plumbing.
   */
  sessionId?: string
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
  ): Promise<void>
  /**
   * Powers `/compact`. Compacts the session-tree (the single source of
   * truth) directly. `contextWindow` skips the server's `models::get`
   * lookup when known.
   */
  compactSession?(
    sessionId: string,
    model: ModelId,
    contextWindow?: number,
  ): Promise<CompactResult>
}
