import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

const THOUGHT = `straightforward dispatch: this is an echo through
the engine, so i'll trigger \`engine::echo\` with the user's text verbatim and
report what comes back.`

const BODY = `## echoed

i triggered \`engine::echo\` against your prompt and got the expected mirror back.
nothing else to do here — the worker is alive and responsive.

\`\`\`json
{ "ok": true }
\`\`\`

ready for the next instruction.`

export const happyAgent = makeBackend(
  'happy-agent',
  async function* (prompt, _mode, _model, opts) {
    const signal = opts?.signal
    const text = prompt.replace(/\s+/g, ' ').trim().slice(0, 200) || '(empty)'
    yield* streamThought(THOUGHT, { signal })
    yield* streamFcall({
      functionId: 'engine::echo',
      input: { text },
      output: { text },
      waitMs: 700,
      signal,
    })
    yield* streamAssistant(BODY, { signal })
  },
)
