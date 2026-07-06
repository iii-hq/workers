import {
  spawnDepthError,
  spawnTextDone,
} from '@/stories/fixtures/harness-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

/**
 * A gated `harness::spawn` (approve → child running → markdown result),
 * then a second spawn that trips the depth guard and renders the error view.
 */
export const harnessSpawn = makeBackend(
  'harness-spawn',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought('delegating the CI triage to a child agent…', {
      signal,
    })
    yield* streamFcall({
      functionId: 'harness::spawn',
      input: spawnTextDone.input,
      output: spawnTextDone.output,
      pendingApproval: true,
      approvalWaitMs: 1800,
      waitMs: 2200,
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
      'child finished; the second spawn hit the depth guard as expected.',
      { signal },
    )
  },
)
