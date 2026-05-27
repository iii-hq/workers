/**
 * Turn-orchestrator pre-flight error types surfaced when context-compaction
 * cannot recover a session before a provider call.
 */

export class ContextOverflowError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ContextOverflowError';
  }
}

export class CompactionBusyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CompactionBusyError';
  }
}

/** Thrown by a handler for a genuinely retryable failure. runTransition
 *  re-throws it so the turn-step queue applies backoff/retry/DLQ. Any other
 *  throw is treated as terminal and routes the session to `failed`. */
export class TransientError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TransientError';
  }
}

/** Persisted turn_state is missing fields required for the current FSM step. */
export class TurnStateInvariantError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TurnStateInvariantError';
  }
}
