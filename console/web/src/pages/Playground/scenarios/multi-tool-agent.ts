import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

const THOUGHT = `multi-step agent: i'll list workers, then inspect the
healthiest one, then echo a probe through it. each tool call is sequential
so the chat surface needs to reset its fcall pointer between calls.`

const BODY = `## probe complete

ran three tools end-to-end: listed workers, inspected \`worker-7\` (least
loaded), and echoed a probe through it. all three calls returned cleanly,
so the dispatch path is healthy.

\`\`\`text
list -> info(worker-7) -> echo(worker-7, "ping")
\`\`\`

ready for the next instruction.`

/**
 * Three sequential function calls back-to-back. Surfaces any pointer-reset
 * bug in ChatView (the consumer must clear its fcallId after each fcall-end
 * so the next fcall-start gets a fresh slot).
 */
export const multiToolAgent = makeBackend(
  'multi-tool-agent',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought(THOUGHT, { signal })
    yield* streamFcall({
      functionId: 'engine::list',
      input: {},
      output: {
        workers: ['worker-1', 'worker-3', 'worker-7'],
      },
      waitMs: 450,
      signal,
    })
    yield* streamFcall({
      functionId: 'engine::info',
      input: { id: 'worker-7' },
      output: {
        id: 'worker-7',
        load: 0.12,
        version: '0.4.1',
        skills: ['echo', 'tokenize', 'embed'],
      },
      waitMs: 500,
      signal,
    })
    yield* streamFcall({
      functionId: 'engine::echo',
      input: { workerId: 'worker-7', text: 'ping' },
      output: { text: 'ping' },
      waitMs: 350,
      signal,
    })
    yield* streamAssistant(BODY, { signal })
  },
)
