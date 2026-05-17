import { makeBackend, streamAssistant } from './helpers'

const BODY = `## what i can tell you

short answer: it depends on whether you're optimizing for **read** speed or
**write** speed. a few things worth keeping in mind:

- a flat \`Map\` gives you \`O(1)\` lookups but is awkward for ordered iteration.
- a sorted list trades insert cost for cheap range scans.
- if you need both, a btree-backed index is usually the right tool.

| structure | lookup | ordered range |
|-----------|--------|---------------|
| map       | o(1)   | no            |
| sorted    | o(log) | yes           |
| btree     | o(log) | yes           |

pick the one whose worst case matches your hottest path. you can always swap
later — the interface is the thing that matters.`

export const happyAsk = makeBackend(
  'happy-ask',
  async function* (_prompt, _mode, _model, opts) {
    yield* streamAssistant(BODY, { signal: opts?.signal })
  },
)
