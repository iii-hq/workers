/**
 * Turn-orchestrator pre-flight error types surfaced when context-compaction
 * cannot recover a session before a provider call.
 */

/** Thrown by a handler for a genuinely retryable failure. runTransition
 *  re-throws it so the turn-step queue applies backoff/retry/DLQ. Any other
 *  throw is treated as terminal and routes the session to `failed`. */
export class TransientError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TransientError';
  }
}

export class ContextOverflowError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ContextOverflowError';
  }
}

/** Another compaction holds the session's compaction lease (e.g. the async
 *  post-turn summarize of a large session outlives `busyTimeoutMs`). This is
 *  transient by construction — the lease TTL (300s) bounds it — so it extends
 *  {@link TransientError}: runTransition re-throws and the turn-step queue
 *  retries the step once the in-flight compaction releases, instead of
 *  killing the session with a terminal `failed`. */
export class CompactionBusyError extends TransientError {
  constructor(message: string) {
    super(message);
    this.name = 'CompactionBusyError';
  }
}

/** Persisted turn_state is missing fields required for the current FSM step. */
export class TurnStateInvariantError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TurnStateInvariantError';
  }
}
