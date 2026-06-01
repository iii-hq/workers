import { makeBackend, streamThought } from './helpers'

const THOUGHT = `let me reason about this carefully — i'll lay out the
constraints first, then sketch the data flow, then ask whether the simplest
implementation actually meets the requirements before we add complexity.`

/**
 * Streams half the thought tokens, then throws an AbortError. Verifies that
 * ChatView's catch+finally clears the streaming flag on the dangling thought
 * message and the surface returns to "ready" without an unhandled rejection.
 */
export const abortMidThought = makeBackend(
  'abort-mid-thought',
  async function* (_prompt, _mode, _model, opts) {
    yield* streamThought(THOUGHT, {
      signal: opts?.signal,
      meanDelayMs: 18,
      truncateAt: 0.5,
    })
    throw new DOMException('aborted by scenario', 'AbortError')
  },
)
