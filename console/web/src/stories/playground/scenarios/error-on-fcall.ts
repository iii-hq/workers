import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

const THOUGHT = `dispatching to the rate-limiter behind engine::echo. if
this comes back with an error i'll surface it in plain language without
retrying — the user can decide whether to back off.`

const BODY = `## blocked

the call returned a structured error rather than data, so i'm stopping
here. the contract is: errors are shaped \`{ "error": { "kind", "message" } }\`,
which gives the chat surface enough to render them without guessing.

retry once after a few seconds; if it persists, the upstream worker is
saturated and the rate-limit policy needs a tuning pass.`

/**
 * Function-trigger ends with an error payload (instead of a data payload).
 * Backends should never throw to signal semantic failures — they should
 * embed the error in the fcall-end output so the renderer can show it.
 */
export const errorOnFcall = makeBackend(
  'error-on-fcall',
  async function* (prompt, _mode, _model, opts) {
    const signal = opts?.signal
    const text = prompt.replace(/\s+/g, ' ').trim().slice(0, 200) || '(empty)'
    yield* streamThought(THOUGHT, { signal })
    yield* streamFcall({
      functionId: 'engine::echo',
      input: { text },
      output: {
        error: {
          kind: 'rate_limited',
          message: 'too many requests in window; retry after 5s',
          retryAfterMs: 5000,
        },
      },
      waitMs: 600,
      signal,
    })
    yield* streamAssistant(BODY, { signal })
  },
)
