import { makeBackend, streamAssistant } from './helpers'

const BODY = `slow drip. each token arrives roughly every 200ms so you can
watch the renderer settle. useful for catching layout-jitter or cursor
flicker that fast streams hide.

- bullet one
- bullet two
- bullet three

\`\`\`text
sl   o   w
\`\`\`
`

export const slowTokens = makeBackend(
  'slow-tokens',
  async function* (_prompt, _mode, _model, opts) {
    yield* streamAssistant(BODY, { signal: opts?.signal, meanDelayMs: 200 })
  },
)
