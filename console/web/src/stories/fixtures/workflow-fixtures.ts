import type { FunctionTriggerMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

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
    output,
    durationMs: 1432,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- a realistic join-heavy DAG ----------------
 * Mirrors the live adversarial-blog run: discover → select → 4 named critics
 * → synthesize (JOIN over all 4) → draft → 2 reviewers → final_rewrite (JOIN
 * over draft + both reviews). The two array-`from` joins are what the new
 * multi-input wiring exists for, so the DagSummary shows the `join` badge. */
const blogDef = {
  version: 1,
  nodes: {
    discover: {
      agent: { model: 'claude-opus-4-8' },
      input: {
        from: 'run_input',
        template: 'Find currently-viral SE articles.',
      },
    },
    select: {
      agent: { model: 'claude-sonnet-4-6' },
      input: {
        from: 'node:discover',
        template: 'Pick the single best article.',
      },
      depends_on: ['discover'],
    },
    critic_skeptic: {
      agent: { model: 'claude-sonnet-4-6' },
      input: {
        from: 'node:select',
        template: 'Attack the argument as a skeptic.',
      },
      depends_on: ['select'],
    },
    critic_contract: {
      agent: { model: 'claude-sonnet-4-6' },
      input: {
        from: 'node:select',
        template: 'Attack from the contract angle.',
      },
      depends_on: ['select'],
    },
    critic_observability: {
      agent: { model: 'claude-sonnet-4-6' },
      input: { from: 'node:select', template: 'Attack on observability.' },
      depends_on: ['select'],
    },
    critic_orchestration: {
      agent: { model: 'claude-sonnet-4-6' },
      input: { from: 'node:select', template: 'Attack on orchestration.' },
      depends_on: ['select'],
    },
    synthesize: {
      agent: { model: 'claude-opus-4-8' },
      input: {
        from: [
          'node:critic_skeptic',
          'node:critic_contract',
          'node:critic_observability',
          'node:critic_orchestration',
        ],
        template: 'Distill the strongest hardened argument.',
      },
      depends_on: [
        'critic_skeptic',
        'critic_contract',
        'critic_observability',
        'critic_orchestration',
      ],
    },
    draft: {
      agent: { model: 'claude-opus-4-8' },
      input: { from: 'node:synthesize', template: 'Write the technical post.' },
      depends_on: ['synthesize'],
    },
    review_slop: {
      agent: { model: 'claude-sonnet-4-6' },
      input: {
        from: 'node:draft',
        template: 'Hunt AI slop / marketing tells.',
      },
      depends_on: ['draft'],
    },
    review_technical: {
      agent: { model: 'claude-sonnet-4-6' },
      input: { from: 'node:draft', template: 'Check technical accuracy.' },
      depends_on: ['draft'],
    },
    final_rewrite: {
      agent: { model: 'claude-opus-4-8' },
      input: {
        from: ['node:draft', 'node:review_slop', 'node:review_technical'],
        template: 'Apply every fix; make it read like an expert human.',
      },
      depends_on: ['draft', 'review_slop', 'review_technical'],
    },
  },
  output: { from: 'node:final_rewrite' },
}

const blogInput = {
  task: 'viral SE article -> iii thesis -> adversarial harden -> blog -> de-slop',
}

const FINAL_POST = `# The loop can be legible even when the code isn't

Armin Ronacher's "The Coming Loop" names something a lot of us have felt and
mostly tried not to think about. You start an agent loop, it runs, code appears,
tests pass — and somewhere in the middle of that you stop being the author.

I work on iii, a worker mesh, and I want to make a specific, scoped claim: iii
is a partial answer to Ronacher's problem. A real one, but narrow.

## Enumerable is not comprehensible

You can list every function and every call in a system and still not understand
it. A DAG with two hundred nodes is fully enumerable and completely
incomprehensible…`

/* ---------------- workflow::start ---------------- */

export const workflowStartLaunched = base(
  'wf-start',
  'workflow::start',
  {
    definition: blogDef,
    input: blogInput,
    notify: { function_id: 'console::wf-done' },
  },
  wrapHarness({ run_id: 'r_f58318bb102841a99134d7a6e7c164df' }),
)

export const workflowStartRunning = base(
  'wf-start-running',
  'workflow::start',
  { definition: blogDef, input: blogInput },
  undefined,
  { running: true },
)

/* ---------------- workflow::status ---------------- */

export const workflowStatusInFlight = base(
  'wf-status-inflight',
  'workflow::status',
  { run_id: 'r_f58318bb102841a99134d7a6e7c164df' },
  wrapHarness({
    run_id: 'r_f58318bb102841a99134d7a6e7c164df',
    status: 'awaiting_nodes',
    nodes: {
      discover: 'done',
      select: 'done',
      critic_skeptic: 'done',
      critic_contract: 'done',
      critic_observability: 'running',
      critic_orchestration: 'running',
      synthesize: 'pending',
      draft: 'pending',
      review_slop: 'pending',
      review_technical: 'pending',
      final_rewrite: 'pending',
    },
    node_results: {
      discover: 'r_f58318bb/discover',
      select: 'r_f58318bb/select',
      critic_skeptic: 'r_f58318bb/critic_skeptic',
      critic_contract: 'r_f58318bb/critic_contract',
    },
  }),
)

export const workflowStatusCompleted = base(
  'wf-status-ok',
  'workflow::status',
  { run_id: 'r_f58318bb102841a99134d7a6e7c164df' },
  wrapHarness({
    run_id: 'r_f58318bb102841a99134d7a6e7c164df',
    status: 'completed',
    nodes: {
      discover: 'done',
      select: 'done',
      critic_skeptic: 'done',
      critic_contract: 'done',
      critic_observability: 'done',
      critic_orchestration: 'done',
      synthesize: 'done',
      draft: 'done',
      review_slop: 'done',
      review_technical: 'done',
      final_rewrite: 'done',
    },
    result: FINAL_POST,
  }),
)

export const workflowStatusFailed = base(
  'wf-status-fail',
  'workflow::status',
  { run_id: 'r_9a01' },
  wrapHarness({
    run_id: 'r_9a01',
    status: 'failed',
    nodes: {
      discover: 'done',
      select: 'done',
      draft: 'failed',
    },
    node_errors: {
      draft:
        'tools.1.custom.input_schema.type: Field required (provider rejected empty schema)',
    },
    result_error: "node 'draft' failed after 2 retries",
  }),
)

/** A fanned-out run: numeric `#i` ordering (read#2 must precede read#10). */
export const workflowStatusFanout = base(
  'wf-status-fanout',
  'workflow::status',
  { run_id: 'r_fan' },
  wrapHarness({
    run_id: 'r_fan',
    status: 'awaiting_nodes',
    nodes: {
      plan: 'done',
      'read#0': 'done',
      'read#1': 'done',
      'read#2': 'running',
      'read#10': 'pending',
      synthesize: 'pending',
    },
  }),
)

/* ---------------- workflow::node-result ---------------- */

export const workflowNodeResultText = base(
  'wf-node-text',
  'workflow::node-result',
  { run_id: 'r_f58318bb102841a99134d7a6e7c164df', node_uid: 'final_rewrite' },
  wrapHarness({ result: FINAL_POST }),
)

export const workflowNodeResultJson = base(
  'wf-node-json',
  'workflow::node-result',
  { run_id: 'r_f58318bb102841a99134d7a6e7c164df', node_uid: 'select' },
  wrapHarness({
    result: {
      url: 'https://lucumr.pocoo.org/the-coming-loop',
      title: 'The Coming Loop',
      thesis:
        'Agentic loops outrun human comprehension; iii makes them legible.',
      virality: { source: 'hn', points: 412 },
    },
  }),
)

/* ---------------- workflow::stop ---------------- */

export const workflowStop = base(
  'wf-stop',
  'workflow::stop',
  { run_id: 'r_f58318bb102841a99134d7a6e7c164df' },
  wrapHarness({
    run_id: 'r_f58318bb102841a99134d7a6e7c164df',
    status: 'cancelled',
  }),
)

/* ---------------- error: Rule 5 rejection at start ---------------- */

export const workflowStartRejected = base(
  'wf-start-rejected',
  'workflow::start',
  {
    definition: {
      version: 1,
      nodes: {
        draft: {
          agent: { model: 'claude-opus-4-8' },
          input: { from: 'run_input' },
        },
        reviews: {
          agent: { model: 'claude-sonnet-4-6' },
          input: { from: 'node:draft' },
          depends_on: ['draft'],
        },
        collate: {
          agent: { model: 'claude-sonnet-4-6' },
          input: { from: 'node:reviews' },
          depends_on: ['draft', 'reviews'],
        },
      },
      output: { from: 'node:collate' },
    },
    input: {},
  },
  {
    error: {
      kind: 'function_error',
      message:
        "invalid workflow definition: node 'collate' depends_on 'draft' but never consumes it (input.from = \"node:reviews\").",
      details: { function_id: 'workflow::start', code: 'invalid_def' },
      content: [
        {
          type: 'text',
          text: "node 'collate' depends_on 'draft' but never consumes it; read it via input.from (use an array like [\"node:draft\", …]) or remove it from depends_on.",
        },
      ],
    },
  },
)

export const workflowFixtures = [
  workflowStartLaunched,
  workflowStartRunning,
  workflowStatusInFlight,
  workflowStatusCompleted,
  workflowStatusFailed,
  workflowStatusFanout,
  workflowNodeResultText,
  workflowNodeResultJson,
  workflowStop,
  workflowStartRejected,
] as const
