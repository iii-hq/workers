import { coderUpdateSkillDiscovery } from '@/stories/fixtures/coder-fixtures'
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
    yield* streamThought('expanding the discovery section in SKILL.md…', {
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
