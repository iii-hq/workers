import type { Mode, ModelId } from '@/types/chat'
import type { ChatBackend, ChatStreamOptions, StreamEvent } from './types'

const PLAN_BODY = `## plan

i'd start by laying out the work as a sequence of small, reversible steps —
this keeps the surface area auditable and lets you bail out if the shape of
the problem changes.

1. read the request carefully and restate it in one sentence.
2. enumerate the constraints (latency, footprint, dependencies).
3. sketch the data flow on paper before writing any code.
4. pick the smallest slice that proves the design, then iterate.

\`\`\`text
problem -> constraints -> sketch -> slice -> ship
\`\`\`

a one-liner you can keep in your back pocket: *plan the work, then work the
plan*. nothing fancy, but it survives contact with reality.`

const ASK_BODY = `## what i can tell you

short answer: it depends on whether you're optimizing for **read** speed or
**write** speed. a few things worth keeping in mind:

- a flat \`Map\` gives you \`O(1)\` lookups but is awkward for ordered iteration.
- a sorted list trades insert cost for cheap range scans.
- if you need both, a btree-backed index is usually the right function.

| structure | lookup | ordered range |
|-----------|--------|---------------|
| map       | o(1)   | no            |
| sorted    | o(log) | yes           |
| btree     | o(log) | yes           |

pick the one whose worst case matches your hottest path. you can always swap
later — the interface is the thing that matters.`

const AGENT_BODY = `## echoed

i ran \`engine::echo\` against your prompt and got the expected mirror back.
nothing else to do here — the worker is alive and responsive.

\`\`\`json
{ "ok": true }
\`\`\`

ready for the next instruction.`

const PLAN_THOUGHT = `restating the request in one line, then enumerating
constraints. the user mentioned shape and direction but not scale, so i'll
plan for the smaller end of the range and flag the bigger case as a follow-up.`

const AGENT_THOUGHT = `straightforward dispatch: this is an echo through
the engine, so i'll call \`engine::echo\` with the user's text verbatim and
report what comes back.`

function bodyFor(mode: Mode): string {
  if (mode === 'plan') return PLAN_BODY
  if (mode === 'ask') return ASK_BODY
  return AGENT_BODY
}

async function* mockStream(
  prompt: string,
  mode: Mode,
  _model: ModelId,
  options: ChatStreamOptions = {},
): AsyncGenerator<StreamEvent> {
  const { signal, meanDelayMs = 25 } = options
  const trimmedPrompt = prompt.replace(/\s+/g, ' ').trim().slice(0, 200)

  /* phase 1: thought (plan + agent) */
  if (mode === 'plan' || mode === 'agent') {
    const thoughtBody = mode === 'plan' ? PLAN_THOUGHT : AGENT_THOUGHT
    const targetDuration = mode === 'plan' ? 1500 : 1000
    const startedAt = Date.now()
    yield { kind: 'thought-start' }
    const thoughtTokens = thoughtBody.split(/(\s+)/)
    const perToken = Math.max(
      8,
      targetDuration / Math.max(1, thoughtTokens.length),
    )
    for (const token of thoughtTokens) {
      if (signal?.aborted) return
      if (token) yield { kind: 'thought-token', token }
      await sleep(perToken * (0.6 + Math.random() * 0.8), signal)
    }
    yield { kind: 'thought-end', durationMs: Date.now() - startedAt }
  }

  /* phase 2: function call (agent only) */
  if (mode === 'agent') {
    if (signal?.aborted) return
    const fcallStart = Date.now()
    const input = { text: trimmedPrompt || '(empty)' }
    yield { kind: 'fcall-start', functionId: 'engine::echo', input }
    await sleep(500 + Math.random() * 400, signal)
    if (signal?.aborted) return
    const output = { text: trimmedPrompt || '(empty)' }
    yield { kind: 'fcall-end', output, durationMs: Date.now() - fcallStart }
  }

  /* phase 3: assistant body */
  const body = bodyFor(mode)
  const tokens = body.split(/(\s+)/)
  for (const token of tokens) {
    if (signal?.aborted) return
    if (token) yield { kind: 'assistant-token', token }
    await sleep(meanDelayMs * (0.5 + Math.random()), signal)
  }
  yield { kind: 'assistant-end' }
}

export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) return resolve()
    const t = setTimeout(resolve, ms)
    signal?.addEventListener(
      'abort',
      () => {
        clearTimeout(t)
        resolve()
      },
      { once: true },
    )
  })
}

export const mockBackend: ChatBackend = {
  id: 'mock',
  stream: mockStream,
  async compactSession(_sessionId, _model, _history, _contextWindow) {
    return { status: 'empty' }
  },
}
