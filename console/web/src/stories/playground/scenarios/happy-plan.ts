import { makeBackend, streamAssistant, streamThought } from './helpers'

const THOUGHT = `restating the request in one line, then enumerating
constraints. the user mentioned shape and direction but not scale, so i'll
plan for the smaller end of the range and flag the bigger case as a follow-up.`

const BODY = `## plan

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

export const happyPlan = makeBackend(
  'happy-plan',
  async function* (_prompt, _mode, _model, opts) {
    yield* streamThought(THOUGHT, { signal: opts?.signal })
    yield* streamAssistant(BODY, { signal: opts?.signal })
  },
)
