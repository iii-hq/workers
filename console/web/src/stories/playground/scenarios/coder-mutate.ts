import {
  coderCreateSkillDoc,
  coderTreeSnapshot,
} from '@/stories/fixtures/coder-fixtures'
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
    yield* streamThought('scouting the workers tree, then scaffolding…', {
      signal,
    })
    yield* streamFcall({
      functionId: 'coder::tree',
      input: coderTreeSnapshot.input,
      output: coderTreeSnapshot.output,
      waitMs: 500,
      signal,
    })
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
