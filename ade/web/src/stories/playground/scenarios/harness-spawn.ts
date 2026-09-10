import {
  spawnDepthError,
  spawnDirectDone,
} from '@/stories/fixtures/harness-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

/**
 * A gated `harness::spawn` (approve → child session created → the live
 * sub-agent card), then a second spawn that trips the depth guard and
 * renders the error view.
 */
export const harnessSpawn = makeBackend(
  'harness-spawn',
  async function* (_prompt, _model, opts) {
    const signal = opts?.signal
    yield* streamThought('delegating the CI triage to a child agent…', {
      signal,
    })
    yield* streamFcall({
      functionId: 'harness::spawn',
      input: spawnDirectDone.input,
      output: spawnDirectDone.output,
      pendingApproval: true,
      approvalWaitMs: 1800,
      waitMs: 900,
      signal,
    })
    yield* streamFcall({
      functionId: 'harness::spawn',
      input: spawnDepthError.input,
      output: spawnDepthError.output,
      waitMs: 500,
      signal,
    })
    yield* streamAssistant(
      'the child session is running the triage; the second spawn hit the depth guard as expected.',
      { signal },
    )
  },
)
