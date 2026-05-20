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
