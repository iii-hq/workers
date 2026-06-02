import { coderCreateSkillDoc } from '@/stories/fixtures/coder-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

export const coderMutate = makeBackend(
  'coder-mutate',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought('scaffolding the iii skill doc…', { signal })
    yield* streamFcall({
      functionId: 'coder::create-file',
      input: coderCreateSkillDoc.input,
      output: coderCreateSkillDoc.output,
      waitMs: 700,
      signal,
    })
    yield* streamAssistant('created `workers/iii/skills/SKILL.md`.', { signal })
  },
)
