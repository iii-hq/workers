import {
  coderReadWindowNumbered,
  coderSearchContext,
  coderUpdateSkillDiscovery,
} from '@/stories/fixtures/coder-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

export const coderUpdate = makeBackend(
  'coder-update',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought('locating the discovery section in SKILL.md…', {
      signal,
    })
    yield* streamFcall({
      functionId: 'coder::search',
      input: coderSearchContext.input,
      output: coderSearchContext.output,
      waitMs: 500,
      signal,
    })
    yield* streamFcall({
      functionId: 'coder::read-file',
      input: coderReadWindowNumbered.input,
      output: coderReadWindowNumbered.output,
      waitMs: 500,
      signal,
    })
    yield* streamFcall({
      functionId: 'coder::update-file',
      input: coderUpdateSkillDiscovery.input,
      output: coderUpdateSkillDiscovery.output,
      waitMs: 800,
      signal,
    })
    yield* streamAssistant('updated discovery guidance in the skill doc.', {
      signal,
    })
  },
)
