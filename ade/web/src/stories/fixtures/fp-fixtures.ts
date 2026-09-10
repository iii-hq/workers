import type { FunctionTriggerMessage } from '@/types/chat'

const now = Date.now()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionTriggerMessage>,
): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId,
    input,
    ...(output !== undefined ? { output } : {}),
    durationMs: 88,
    createdAt: now,
    ...extra,
  }
}

/** entry-mapper success shape: { content, details }. */
function resultEnvelope(text: string, details: unknown) {
  return { content: [{ type: 'text' as const, text }], details }
}

/* ---------------- fp::pipe ---------------- */

const pipePreview =
  '## Circuit breakers\n\nA circuit breaker wraps a remote call and trips open after N consecutive failures, short-circuiting further calls while the downstream recovers. Half-open probes decide when to close it again'

const pipeThrough = [
  {
    function: 'scrapling::fetch',
    payload: {
      url: 'https://example.com/circuit-breakers',
      format: 'markdown',
      main_content_only: true,
    },
  },
  { function: 'fp::get', payload: { path: '/content' } },
  { function: 'fp::take', payload: { n: 20_000 } },
  { function: 'state::set', payload: { scope: 'research', key: 'article' } },
]

const pipeDoneDetails = {
  steps: [
    { function: 'scrapling::fetch', chars: 84_213 },
    { function: 'fp::get', chars: 81_902 },
    { function: 'fp::take', chars: 20_000 },
    { function: 'state::set', chars: 46 },
  ],
  value_preview: `${pipePreview}…`,
}

/** The canonical fetch → get → take → set pipeline, with receipts. */
export const pipeDone = base(
  'pipe-done',
  'fp::pipe',
  { through: pipeThrough },
  resultEnvelope(JSON.stringify(pipeDoneDetails), pipeDoneDetails),
  { durationMs: 2_431 },
)

export const pipeRunning = base(
  'pipe-running',
  'fp::pipe',
  { through: pipeThrough },
  undefined,
  { running: true, durationMs: undefined },
)

/** The step list IS the approval surface — pipe steps run with the fp
    worker's authority, so the pipe is approval-gated by default. */
export const pipePending = base(
  'pipe-pending',
  'fp::pipe',
  { through: pipeThrough },
  undefined,
  { pendingApproval: true },
)

/** Step failure — the handler error message names the step and carries the
    completed-step trail. */
export const pipeStepError = base(
  'pipe-step-error',
  'fp::pipe',
  {
    through: [
      pipeThrough[0],
      { function: 'fp::get', payload: { path: '/body' } },
    ],
  },
  {
    error: {
      kind: 'function_error',
      message:
        'pipe failed at step 2 (fp::get): path "/body" matched nothing; available top-level keys: content, status · completed: scrapling::fetch→84213ch',
      details: {
        message:
          'pipe failed at step 2 (fp::get): path "/body" matched nothing; available top-level keys: content, status · completed: scrapling::fetch→84213ch',
      },
      content: [
        {
          type: 'text' as const,
          text: 'pipe failed at step 2 (fp::get): path "/body" matched nothing; available top-level keys: content, status · completed: scrapling::fetch→84213ch',
        },
      ],
    },
  },
)

/* ---------------- fp transforms ---------------- */

/** Direct transform call — success details is the `UtilResponse { value }`
    wrapper. */
export const utilGetDone = base(
  'util-get-done',
  'fp::get',
  {
    value: { content: `${pipePreview}…`, status: 200 },
    path: '/content',
  },
  resultEnvelope(`${pipePreview}…`, { value: `${pipePreview}…` }),
)

export const utilFilterDone = base(
  'util-filter-done',
  'fp::filter',
  {
    value: [
      { status: 'active', id: 1 },
      { status: 'gone', id: 2 },
      { status: 'active', id: 3 },
    ],
    matches: { status: 'active' },
  },
  resultEnvelope('2 elements', {
    value: [
      { status: 'active', id: 1 },
      { status: 'active', id: 3 },
    ],
  }),
)

export const utilTakeRunning = base(
  'util-take-running',
  'fp::take',
  { value: `${pipePreview}…`, n: 200 },
  undefined,
  { running: true, durationMs: undefined },
)

/** The live haiku-4.5 trap: fp::pipe {value} as a seed step — the refusal
    redirects to the seed rule. */
export const pipeSeedError = base(
  'pipe-seed-error',
  'fp::pipe',
  {
    through: [
      { function: 'fp::pipe', payload: { value: [1, 2, 3, 4, 5] } },
      { function: 'fp::take', payload: { n: 3 } },
    ],
  },
  {
    error: {
      kind: 'function_error',
      message:
        'step 1 (fp::pipe): pipes do not nest — to start from a literal value, seed the FIRST transform step\'s payload ("value": …) instead of wrapping the value in a pipe step',
      details: {
        message:
          'step 1 (fp::pipe): pipes do not nest — to start from a literal value, seed the FIRST transform step\'s payload ("value": …) instead of wrapping the value in a pipe step',
      },
      content: [
        {
          type: 'text' as const,
          text: 'step 1 (fp::pipe): pipes do not nest — to start from a literal value, seed the FIRST transform step\'s payload ("value": …) instead of wrapping the value in a pipe step',
        },
      ],
    },
  },
)

/* ---------------- the seven fp-compat ops ---------------- */

/** Number result renders as JSON, not a string pane. */
export const utilSizeDone = base(
  'util-size-done',
  'fp::size',
  { value: { content: `${pipePreview}…`, status: 200 } },
  resultEnvelope('2', { value: 2 }),
)

/** null-only removal — 0, false and "" survive (deviation from lodash). */
export const utilCompactDone = base(
  'util-compact-done',
  'fp::compact',
  { value: [0, null, false, '', 1, null, 'x'] },
  resultEnvelope('[0,false,"",1,"x"]', { value: [0, false, '', 1, 'x'] }),
)

/** Negative index chip — the last element JSON Pointer cannot reach. */
export const utilNthDone = base(
  'util-nth-done',
  'fp::nth',
  {
    value: [
      { name: 'Alice', score: 92 },
      { name: 'Bob', score: 78 },
      { name: 'Cara', score: 85 },
    ],
    n: -1,
  },
  resultEnvelope('{"name":"Cara","score":85}', {
    value: { name: 'Cara', score: 85 },
  }),
)

/** Miss → default; the `default` chip renders alongside `path`. */
export const utilGetOrDone = base(
  'util-getor-done',
  'fp::getOr',
  {
    value: { content: 'body', status: 200 },
    path: '/etag',
    default: 'no-etag',
  },
  resultEnvelope('no-etag', { value: 'no-etag' }),
)

export const utilFlattenDone = base(
  'util-flatten-done',
  'fp::flatten',
  { value: [1, [2, 3], [[4]]] },
  resultEnvelope('[1,2,3,[4]]', { value: [1, 2, 3, [4]] }),
)

export const utilSortByDone = base(
  'util-sortby-done',
  'fp::sortBy',
  {
    value: [
      { name: 'Alice', score: 92 },
      { name: 'Bob', score: 78 },
      { name: 'Cara', score: 85 },
    ],
    path: '/score',
  },
  resultEnvelope('3 elements', {
    value: [
      { name: 'Bob', score: 78 },
      { name: 'Cara', score: 85 },
      { name: 'Alice', score: 92 },
    ],
  }),
)

export const utilReverseDone = base(
  'util-reverse-done',
  'fp::reverse',
  { value: ['first', 'second', 'third'] },
  resultEnvelope('3 elements', { value: ['third', 'second', 'first'] }),
)

export const fpFixtures = [
  pipeDone,
  pipeRunning,
  pipePending,
  pipeStepError,
  pipeSeedError,
  utilGetDone,
  utilFilterDone,
  utilTakeRunning,
  utilSizeDone,
  utilCompactDone,
  utilNthDone,
  utilGetOrDone,
  utilFlattenDone,
  utilSortByDone,
  utilReverseDone,
] as const
