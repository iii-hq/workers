import { makeBackend, streamAssistant } from './helpers'

const BODY = `fast firehose. tokens arrive about every 5ms. this stresses
the patch path — onPatchMessage runs once per token, so any react re-render
cost shows up as dropped frames here.

paragraphs are intentionally long so we hit the renderer's hot path with a
non-trivial amount of inline content. emphasis like *italic* and **bold**
plus inline code like \`useMemo\` is sprinkled in to keep the markdown
processor honest under high token rate.

| metric    | budget |
|-----------|--------|
| latency   | < 16ms |
| throughput| ~ 200 tok/s |
| dropped   | 0      |
`

export const fastTokens = makeBackend(
  'fast-tokens',
  async function* (_prompt, _mode, _model, opts) {
    yield* streamAssistant(BODY, { signal: opts?.signal, meanDelayMs: 5 })
  },
)
