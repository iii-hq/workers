import type { Mode, ModelId } from '@/types/chat'
import type { AgentMessage } from '@/types/iii-agent-event'

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

export interface ChatStreamOptions {
  signal?: AbortSignal
  /** mean delay between assistant tokens, in ms */
  meanDelayMs?: number
  /**
   * Stable session_id for the chat conversation. All `stream()` calls
   * for the same conversation must pass the same value so the engine
   * groups every turn under one session in the traces UI.
   *
   * The real backend uses it as `session_id` in `ui::subscribe` and
   * `run::start`; the mock backend ignores it. When omitted, the real
   * backend falls back to a fresh `console-<uuid>` so callers that
   * haven't been updated yet still work (with the pre-fix behavior of
   * one session per send).
   *
   * Mirrors the `harness/web` strategy: see
   * `workers/harness/web/src/App.tsx` `newSessionId()` and the
   * `active ?? draftId ?? newSessionId()` plumbing.
   */
  sessionId?: string
  /**
   * Prior conversation turns to ship along with the new user prompt.
   * Without this, `run::start` overwrites the orchestrator's flat
   * message state with only the latest user message and the assistant
   * loses all context from earlier user submissions. ChatView builds
   * this from `conversation.messages` minus the just-appended user
   * turn. Real backend prepends it to the payload's `messages` array;
   * mock backend ignores.
   */
  history?: AgentMessage[]
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
   * Powers `/compact`. `history` is reconciled into session-tree first so a
   * stale mirror doesn't yield a spurious 'empty'. `contextWindow` skips
   * the server's `models::get` lookup when known.
   */
  compactSession?(
    sessionId: string,
    model: ModelId,
    history?: AgentMessage[],
    contextWindow?: number,
  ): Promise<CompactResult>
}
