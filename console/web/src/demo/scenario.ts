/**
 * The canned turn the landing-page demo replays.
 *
 * The chat half of the stream is the real `ChatBackend.stream()` contract
 * (`@/lib/backend/types`) — the same `StreamEvent`s the live harness backend
 * emits, in the same order, consumed by the same reducer the console's
 * `ChatView` uses. Three demo-only markers ride alongside it so the trace
 * pane and the callout layer stay in lockstep with the transcript without a
 * second timeline to keep in sync:
 *
 *   demo-span-open / demo-span-close   the waterfall's spans
 *   demo-callout                       the annotation chip
 *
 * Span shapes mirror what a real turn produces: one `harness::turn step`
 * root per durable loop step (`workers/harness/src/functions/turn.rs`),
 * `execute router::chat` → `execute provider::anthropic::stream` for the
 * generation, and `execute <fn>` for each dispatched call.
 */

import type { StreamEvent } from '@/lib/backend'
import { sleep, tokenize } from '@/stories/playground/scenarios/helpers'

export const PROMPT = 'build a payments ledger service with a durable db'

export const MODEL_ID = 'anthropic::claude-opus-4-7'
export const SESSION_ID = 'console-payments-ledger-demo'
export const TRACE_ID = 'trace-payments-ledger-0000000001'

/**
 * Global playback rate. `prefers-reduced-motion` sets it near zero so the
 * turn fills in at once and holds, instead of animating for a minute.
 */
let SPEED = 1
export function setScenarioSpeed(multiplier: number) {
  SPEED = multiplier
}
const nap = (ms: number, signal?: AbortSignal) => sleep(ms * SPEED, signal)

export type CalloutAnchor = 'transcript' | 'waterfall' | 'composer'

export interface Callout {
  title: string
  text: string
  anchor: CalloutAnchor
}

export interface DemoSpanInit {
  id: string
  parent?: string
  name: string
  service: string
  kind?: string
  attributes?: Array<[string, unknown]>
}

/** A child session `harness::spawn` created, as the sidebar shows it. */
export interface DemoSession {
  id: string
  title: string
  /** The task the parent handed down: the child's seeding user message. */
  task: string
}

export type DemoEvent =
  | StreamEvent
  | { kind: 'demo-span-open'; span: DemoSpanInit }
  | {
      kind: 'demo-span-close'
      id: string
      status?: 'OK' | 'ERROR'
      /** Close the span this long after it opened, rather than now. */
      durationMs?: number
    }
  | { kind: 'demo-callout'; callout: Callout }
  | { kind: 'demo-session-open'; session: DemoSession }
  | { kind: 'demo-session-done'; id: string; result: string }

export interface ScenarioOptions {
  signal?: AbortSignal
  /**
   * Resolves when the gated call is released — by the viewer clicking
   * approve/deny on the real card, or by the demo's own timeout. The demo
   * never denies; a deny resolves the same way so the card can't hang.
   */
  gate: (functionTriggerId: string) => Promise<void>
}

/* ── span bookkeeping ─────────────────────────────────────────────────── */

let spanSeq = 0
function spanId(prefix: string): string {
  spanSeq += 1
  return `${prefix}-${String(spanSeq).padStart(3, '0')}`
}

/** Turn-identity baggage the harness stamps on every span of a step. */
const TURN_TAGS: Array<[string, unknown]> = [
  ['iii.session.id', SESSION_ID],
  ['iii.message.id', 'turn-01'],
  ['iii.tag.kind', 'harness.turn'],
  ['iii.tag.message', PROMPT],
]

/* ── stream fragments ─────────────────────────────────────────────────── */

/**
 * Time to leave a finished block of prose on screen before the next thing
 * lands on top of it. Scaled by length and capped: the point is that the
 * reader gets to finish the sentence, not that the demo waits out a full
 * read of the closing answer.
 */
function readingDwellMs(body: string): number {
  return Math.min(2800, Math.max(600, tokenize(body).length * 10))
}

async function* thought(
  body: string,
  signal?: AbortSignal,
  meanDelayMs = 38,
): AsyncGenerator<DemoEvent> {
  const startedAt = Date.now()
  yield { kind: 'thought-start' }
  for (const token of tokenize(body)) {
    if (signal?.aborted) return
    if (token) yield { kind: 'thought-token', token }
    await nap(meanDelayMs * (0.6 + Math.random() * 0.8), signal)
  }
  yield { kind: 'thought-end', durationMs: Date.now() - startedAt }
  await nap(readingDwellMs(body), signal)
}

async function* assistant(
  body: string,
  signal?: AbortSignal,
  meanDelayMs = 38,
): AsyncGenerator<DemoEvent> {
  for (const token of tokenize(body)) {
    if (signal?.aborted) return
    if (token) yield { kind: 'assistant-token', token }
    await nap(meanDelayMs * (0.5 + Math.random()), signal)
  }
  yield { kind: 'assistant-end' }
  await nap(readingDwellMs(body), signal)
}

/**
 * A held beat after a result worth reading: the worker catalogue, a worker's
 * contract, a test run's output. Rendered as a thought so the pause is a
 * visible line in the transcript rather than the demo appearing to stall,
 * and placed BETWEEN durable steps so it never inflates a step's span.
 */
async function* readPause(
  signal: AbortSignal | undefined,
  holdMs = 2800,
): AsyncGenerator<DemoEvent> {
  const startedAt = Date.now()
  yield { kind: 'thought-start' }
  for (const token of tokenize('letting the user read…')) {
    if (signal?.aborted) return
    if (token) yield { kind: 'thought-token', token }
    await nap(55, signal)
  }
  await nap(holdMs, signal)
  if (signal?.aborted) return
  yield { kind: 'thought-end', durationMs: Date.now() - startedAt }
}

interface CallOptions {
  fn: string
  /** Worker that runs it — the `execute` span's service name. */
  worker: string
  input: unknown
  output: unknown
  /** How long the demo lingers on the call, so a viewer can see it happen. */
  runMs: number
  /**
   * What the card and the span report, when that differs from how long the
   * demo dwells. An engine-local call really does finish in microseconds;
   * animating it that fast would make it invisible, and reporting the dwell
   * instead would claim a trigger registration costs a third of a second.
   */
  reportedMs?: number
  parentSpan: string
  signal?: AbortSignal
  /** Nested spans opened inside the call, as [name, service, share-of-runMs]. */
  inner?: Array<[string, string, number]>
  /** Hold the call in the approval gate before it executes. */
  gate?: (functionTriggerId: string) => Promise<void>
  functionTriggerId?: string
  callout?: Callout
}

async function* call(opts: CallOptions): AsyncGenerator<DemoEvent> {
  const {
    fn,
    worker,
    input,
    output,
    runMs,
    reportedMs,
    parentSpan,
    signal,
    inner,
    gate,
    functionTriggerId,
    callout,
  } = opts
  const startedAt = Date.now()

  yield {
    kind: 'fcall-start',
    functionId: fn,
    input,
    ...(gate
      ? {
          pendingApproval: true,
          functionTriggerId,
          sessionId: SESSION_ID,
        }
      : {}),
  }
  if (callout) yield { kind: 'demo-callout', callout }

  if (gate && functionTriggerId) {
    await gate(functionTriggerId)
    if (signal?.aborted) return
    yield { kind: 'fcall-approval-cleared', functionTriggerId, running: true }
  }

  const execSpan = spanId('exec')
  yield {
    kind: 'demo-span-open',
    span: {
      id: execSpan,
      parent: parentSpan,
      name: `execute ${fn}`,
      service: worker,
      kind: 'internal',
      attributes: [...TURN_TAGS, ['iii.function.id', fn]],
    },
  }

  if (inner?.length) {
    let elapsed = 0
    for (const [name, service, share] of inner) {
      const childSpan = spanId('inner')
      yield {
        kind: 'demo-span-open',
        span: {
          id: childSpan,
          parent: execSpan,
          name,
          service,
          kind: 'internal',
          attributes: TURN_TAGS,
        },
      }
      const slice = runMs * share
      await nap(slice, signal)
      elapsed += slice
      if (signal?.aborted) return
      yield {
        kind: 'demo-span-close',
        id: childSpan,
        /* Inner work takes its share of the REPORTED time, so a child can
           never outlast the call it happened inside. */
        ...(reportedMs === undefined ? {} : { durationMs: reportedMs * share }),
      }
    }
    await nap(Math.max(0, runMs - elapsed), signal)
  } else {
    await nap(runMs, signal)
  }
  if (signal?.aborted) return

  yield {
    kind: 'demo-span-close',
    id: execSpan,
    ...(reportedMs === undefined ? {} : { durationMs: reportedMs }),
  }
  yield {
    kind: 'fcall-end',
    output,
    durationMs: reportedMs ?? Date.now() - startedAt,
    ...(functionTriggerId ? { functionTriggerId } : {}),
  }
}

/**
 * One durable loop step: the harness dequeues, asks the router for a
 * completion, then dispatches whatever the model asked for. `body` streams
 * the model's visible output; the router/provider spans close when it ends.
 */
async function* step(
  index: number,
  body: AsyncGenerator<DemoEvent>,
  signal: AbortSignal | undefined,
  after: (stepSpan: string) => AsyncGenerator<DemoEvent>,
): AsyncGenerator<DemoEvent> {
  const stepSpan = spanId('step')
  const routerSpan = spanId('router')
  const providerSpan = spanId('provider')

  yield {
    kind: 'demo-span-open',
    span: {
      id: stepSpan,
      name: 'harness::turn step',
      service: 'harness',
      kind: 'internal',
      attributes: [...TURN_TAGS, ['iii.turn.step', index]],
    },
  }
  yield {
    kind: 'demo-span-open',
    span: {
      id: routerSpan,
      parent: stepSpan,
      name: 'execute router::chat',
      service: 'llm-router',
      kind: 'internal',
      attributes: [...TURN_TAGS, ['gen_ai.request.model', 'claude-opus-4-7']],
    },
  }
  yield {
    kind: 'demo-span-open',
    span: {
      id: providerSpan,
      parent: routerSpan,
      name: 'execute provider::anthropic::stream',
      service: 'llm-provider-anthropic',
      kind: 'client',
      attributes: TURN_TAGS,
    },
  }

  yield* body
  if (signal?.aborted) return

  yield { kind: 'demo-span-close', id: providerSpan }
  yield { kind: 'demo-span-close', id: routerSpan }

  yield* after(stepSpan)
  if (signal?.aborted) return

  yield { kind: 'demo-span-close', id: stepSpan }
}

/**
 * One step that dispatches every child in `CHILDREN` as a parallel batch.
 *
 * All three cards land together; the gated one holds while the other two are
 * already running, which is what the batch actually looks like when a
 * deny-by-default policy only objects to one call in it. Each child opens its
 * own `execute harness::spawn` span with the child's own `harness::turn step`
 * nested inside, so the fan-out shows up as three branches of one trace, and
 * its own session, so it shows up as three rows in the sidebar.
 */
async function* spawnFanOut(
  stepSpan: string,
  gate: ScenarioOptions['gate'],
  signal: AbortSignal | undefined,
): AsyncGenerator<DemoEvent> {
  const startedAt = Date.now()
  const spans = new Map<string, string>()

  /** Open the child's span pair and its session — it is running now. */
  function* launch(child: ChildSpec): Generator<DemoEvent> {
    const execSpan = spanId('exec')
    const childStep = spanId('child')
    spans.set(child.callId, execSpan)
    yield {
      kind: 'demo-span-open',
      span: {
        id: execSpan,
        parent: stepSpan,
        name: 'execute harness::spawn',
        service: 'harness',
        kind: 'internal',
        attributes: [...TURN_TAGS, ['iii.child.session_id', child.sessionId]],
      },
    }
    yield {
      kind: 'demo-span-open',
      span: {
        id: childStep,
        parent: execSpan,
        name: 'harness::turn step',
        service: 'harness',
        kind: 'internal',
        attributes: [
          ['iii.session.id', child.sessionId],
          ['iii.tag.kind', 'harness.subagent'],
          ['iii.tag.display_name', `Sub-agent · ${child.title}`],
        ],
      },
    }
    spans.set(`${child.callId}:step`, childStep)
    yield {
      kind: 'demo-session-open',
      session: { id: child.sessionId, title: child.title, task: child.task },
    }
  }

  /* Every card at once — a parallel tool-call batch is one assistant turn. */
  for (const child of CHILDREN) {
    yield {
      kind: 'fcall-start',
      functionId: 'harness::spawn',
      input: spawnInput(child),
      functionTriggerId: child.callId,
      sessionId: SESSION_ID,
      ...(child.gated ? { pendingApproval: true } : {}),
    }
  }
  for (const child of CHILDREN) {
    if (!child.gated) yield* launch(child)
  }

  yield {
    kind: 'demo-callout',
    callout: {
      anchor: 'transcript',
      title: 'three sub-agents, one held at the gate',
      text: 'Each child is its own session with its own budget and its own function policy, listed under this chat as it starts and readable on its own. Only `ledger core` asked for `database::*`, a write scope this session does not hold, so only that one waits for a human. Click approve to release it.',
    },
  }

  for (const child of CHILDREN) {
    if (!child.gated) continue
    await gate(child.callId)
    if (signal?.aborted) return
    yield {
      kind: 'fcall-approval-cleared',
      functionTriggerId: child.callId,
      running: true,
    }
    yield* launch(child)
  }

  /* Children report as they finish, not in dispatch order. */
  const finishing = [...CHILDREN].sort(
    (a, b) => a.finishAfterMs - b.finishAfterMs,
  )
  let waited = 0
  for (const child of finishing) {
    await nap(child.finishAfterMs - waited, signal)
    if (signal?.aborted) return
    waited = child.finishAfterMs
    const childStep = spans.get(`${child.callId}:step`)
    const execSpan = spans.get(child.callId)
    if (childStep) yield { kind: 'demo-span-close', id: childStep }
    if (execSpan) yield { kind: 'demo-span-close', id: execSpan }
    yield {
      kind: 'demo-session-done',
      id: child.sessionId,
      result: child.resultText,
    }
    yield {
      kind: 'fcall-end',
      output: spawnResult(child),
      durationMs: Date.now() - startedAt,
      functionTriggerId: child.callId,
    }
  }
}

/* ── outputs ──────────────────────────────────────────────────────────── */

const CONNECTED_AT = () => Date.now() - 4 * 60 * 60 * 1000

const WORKERS_LIST = () => ({
  workers: [
    {
      id: 'wrk_01hq4m8database',
      name: 'database',
      description: 'Durable postgres: tables, queries, migrations.',
      version: '0.21.0',
      runtime: 'rust',
      os: 'linux',
      status: 'connected',
      function_count: 9,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 0,
      isolation: 'container',
      tag: 'core',
    },
    {
      id: 'wrk_01hq4m8shell00',
      name: 'shell',
      description: 'Scoped command execution and filesystem access.',
      version: '0.21.0',
      runtime: 'rust',
      os: 'linux',
      status: 'connected',
      function_count: 16,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 0,
      isolation: 'container',
    },
    {
      id: 'wrk_01hq4m8coder000',
      name: 'coder',
      description: 'Reads, writes and patches files in a scoped workspace.',
      version: '0.21.0',
      runtime: 'node',
      os: 'linux',
      status: 'connected',
      function_count: 12,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 0,
      isolation: 'container',
    },
    {
      id: 'wrk_01hq4m8harness0',
      name: 'harness',
      description: 'The durable agent turn loop.',
      version: '0.21.0',
      runtime: 'rust',
      os: 'linux',
      status: 'connected',
      function_count: 14,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 1,
      isolation: 'container',
    },
    {
      id: 'wrk_01hq4m8observ00',
      name: 'observability',
      description: 'OTel collector: traces, logs and metrics for the engine.',
      version: '0.21.0',
      runtime: 'rust',
      os: 'linux',
      status: 'connected',
      function_count: 6,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 0,
      isolation: 'container',
    },
  ],
})

const DATABASE_INFO = () => ({
  worker: {
    id: 'wrk_01hq4m8database',
    name: 'database',
    description: 'Durable postgres: tables, queries, migrations.',
    version: '0.21.0',
    runtime: 'rust',
    os: 'linux',
    status: 'connected',
    function_count: 9,
    connected_at_ms: CONNECTED_AT(),
    active_invocations: 0,
    isolation: 'container',
    internal: false,
    pid: 1421,
    latest_metrics: null,
  },
  functions: [
    {
      function_id: 'database::create_table',
      worker_name: 'database',
      description: 'Create a table from a column spec.',
    },
    {
      function_id: 'database::query',
      worker_name: 'database',
      description: 'Run a parameterised read query.',
    },
    {
      function_id: 'database::execute',
      worker_name: 'database',
      description: 'Run a parameterised write statement.',
    },
    {
      function_id: 'database::transaction',
      worker_name: 'database',
      description: 'Run several statements atomically.',
    },
    {
      function_id: 'database::migrate',
      worker_name: 'database',
      description: 'Apply pending migrations.',
    },
  ],
  trigger_types: [
    {
      id: 'database.row_changed',
      worker_name: 'database',
      description: 'Fires when a watched table changes.',
    },
  ],
  registered_triggers: [],
})

/**
 * The fan-out. Three children, dispatched in one step, each its own session
 * with its own budget and its own function policy. Only the first asks for
 * `database::*` — the write scope this session does not hold — so only the
 * first stops at the approval gate; the other two dispatch immediately.
 */
interface ChildSpec {
  /** iii function_call_id, and the key the approval resolves against. */
  callId: string
  sessionId: string
  title: string
  task: string
  allow: string[]
  /** Held at the gate before it may run. */
  gated?: boolean
  resultText: string
  resultDetails: Record<string, unknown>
  /** Delay after the gate clears before this child reports, in ms. */
  finishAfterMs: number
}

const CHILDREN: ChildSpec[] = [
  {
    callId: 'fc_spawn_ledger_core',
    sessionId: 'console-sub-ledger-core',
    title: 'ledger core',
    task: `Write the payments-ledger worker's core against the existing \`database\` worker.

\`payments::charge::record\` and \`payments::charge::refund\`, double-entry rows,
\`database::transaction\` for anything that writes two rows. Create the
\`ledger_entries\` and \`ledger_accounts\` tables.`,
    allow: ['coder::*', 'database::*'],
    gated: true,
    resultText:
      'ledger core done: charge::record + charge::refund over a double-entry schema, 2 tables created.',
    resultDetails: {
      functions: ['payments::charge::record', 'payments::charge::refund'],
      tables: ['ledger_entries', 'ledger_accounts'],
      turns_used: 7,
    },
    finishAfterMs: 3400,
  },
  {
    callId: 'fc_spawn_ledger_webhook',
    sessionId: 'console-sub-ledger-webhook',
    title: 'stripe webhook',
    task: `Write \`payments::webhook::stripe\` and \`payments::ledger::reconcile\` for the
payments-ledger worker. Idempotent on the provider event id: a replayed event
must post nothing and return the original entry.`,
    allow: ['coder::*', 'web::search'],
    resultText:
      'webhook + reconcile done: dedupe keyed on provider_event_id, replays return the original entry.',
    resultDetails: {
      functions: ['payments::webhook::stripe', 'payments::ledger::reconcile'],
      idempotency_key: 'provider_event_id',
      turns_used: 5,
    },
    finishAfterMs: 1100,
  },
  {
    callId: 'fc_spawn_ledger_tests',
    sessionId: 'console-sub-ledger-tests',
    title: 'test suite',
    task: `Write the payments-ledger test suite. Cover double-entry balance, refunds
over the original amount, webhook replay, concurrent charges on one account,
and that the schema matches the migration.`,
    allow: ['coder::*', 'shell::exec'],
    resultText:
      'test suite done: 11 tests over the ledger, webhook and schema.',
    resultDetails: { tests: 11, files: ['tests/ledger.rs'], turns_used: 4 },
    finishAfterMs: 400,
  },
]

function spawnInput(child: ChildSpec) {
  return {
    task: child.task,
    model: 'claude-opus-4-7',
    session_id: child.sessionId,
    parent_session_id: SESSION_ID,
    options: {
      mode: 'agent',
      max_turns: 12,
      output: { type: 'text' },
      functions: { allow: child.allow, deny: ['compose::*', 'worker::remove'] },
    },
  }
}

function spawnResult(child: ChildSpec) {
  return {
    content: [{ type: 'text' as const, text: child.resultText }],
    details: { session_id: child.sessionId, ...child.resultDetails },
  }
}

const TEST_STDOUT = `   Compiling payments-ledger v0.1.0
    Finished test profile in 4.21s
     Running tests/ledger.rs

test charge_record_writes_double_entry ... ok
test charge_record_is_idempotent ......... ok
test refund_reverses_original_entry ...... ok
test refund_rejects_over_refund .......... ok
test stripe_webhook_dedupes_by_event_id .. ok
test stripe_webhook_ignores_unknown_type . ok
test reconcile_balances_to_zero .......... ok
test reconcile_flags_orphan_entries ...... ok
test balance_reads_committed_only ........ ok
test concurrent_charges_serialize ........ ok
test schema_matches_migration ............ ok

test result: ok. 11 passed; 0 failed; finished in 1.83s`

const HTTP_TRIGGERS = [
  ['payments::charge::record', 'POST', '/payments/charges'],
  ['payments::charge::refund', 'POST', '/payments/refunds'],
  ['payments::webhook::stripe', 'POST', '/payments/webhooks/stripe'],
  ['payments::ledger::reconcile', 'POST', '/payments/reconcile'],
] as const

const FINAL_ANSWER = `\`payments-ledger\` is live on the engine and answering.

| function | trigger | backed by |
| --- | --- | --- |
| \`payments::charge::record\` | \`POST /payments/charges\` | \`database::transaction\` |
| \`payments::charge::refund\` | \`POST /payments/refunds\` | \`database::transaction\` |
| \`payments::webhook::stripe\` | \`POST /payments/webhooks/stripe\` | \`database::execute\` |
| \`payments::ledger::reconcile\` | \`POST /payments/reconcile\` | \`database::query\` |

Nothing here was scaffolded from a template. The \`database\` worker was already
connected, so the ledger tables went straight onto it and the new worker joined
the same engine the rest of your workers are on.

**About those three children.** \`ledger core\`, \`stripe webhook\` and
\`test suite\` are still listed under this chat. They are real sessions, not log
lines: each one had its own transcript, its own turn budget and its own
function policy, and you can open any of them to read what it did. Only
\`ledger core\` needed a scope this session lacks, so only that one waited on
you.

**About the pane on the right.** That is not a replay of this chat, it is the
trace the engine recorded while it happened. Every row above opened a span:
each \`harness::turn step\` is one durable step of my loop off the queue, the
three \`harness::spawn\` branches are the children running at the same time, and
\`execute payments::charge::record\` near the bottom is the function they built
answering a live request. In the console you would click any bar for its
arguments, its result, and its logs. If this turn had crashed halfway, the loop
would have resumed from the last completed step, into the same trace.`

/* ── the script ───────────────────────────────────────────────────────── */

export async function* runScenario(
  opts: ScenarioOptions,
): AsyncGenerator<DemoEvent> {
  const { signal, gate } = opts
  spanSeq = 0

  yield { kind: 'turn-status', phase: 'accepted' }
  await nap(420, signal)
  yield { kind: 'turn-status', phase: 'started' }
  await nap(260, signal)

  /* step 1 — look at what is already running */
  yield* step(
    1,
    thought(
      'The user wants a payments ledger with durable storage. Before I write a line of it I should look at what is already connected to this engine. iii keeps a live catalog of every running worker, so a durable database may already be here. If it is, there is nothing to scaffold and nothing to deploy alongside.',
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: 'engine::workers::list',
        worker: 'iii',
        input: { status: 'connected' },
        output: WORKERS_LIST(),
        runMs: 900,
        reportedMs: 2.1,
        parentSpan: stepSpan,
        signal,
        callout: {
          anchor: 'transcript',
          title: 'discovery, not scaffolding',
          text: 'The agent reads the engine’s live worker catalog. Whatever is already running is already callable: no boilerplate, no glue service.',
        },
      }),
  )
  if (signal?.aborted) return
  yield* readPause(signal)

  /* step 2 — read the database worker's contract */
  yield* step(
    2,
    thought(
      '`database` is right there: durable postgres, nine functions. Let me read its contract so the ledger is written against the real signatures instead of guesses.',
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: 'engine::workers::info',
        worker: 'iii',
        input: { name: 'database' },
        output: DATABASE_INFO(),
        runMs: 700,
        reportedMs: 1.4,
        parentSpan: stepSpan,
        signal,
        callout: {
          anchor: 'waterfall',
          title: 'the trace is building itself',
          text: 'Every call opens a span. This is the console’s own trace view, filling in live. The observability comes with the runtime; nothing here was instrumented by hand.',
        },
      }),
  )
  if (signal?.aborted) return
  yield* readPause(signal)

  /* step 3 — fan out to three sub-agents; one of them stops at the gate */
  yield* step(
    3,
    assistant(
      '`database` gives me durable postgres with transactions, so the ledger can be double-entry without a second datastore. Three pieces here are independent, so I’ll run them as three sub-agents and stay on the wiring myself.',
      signal,
    ),
    signal,
    (stepSpan) => spawnFanOut(stepSpan, gate, signal),
  )
  if (signal?.aborted) return
  yield* readPause(signal)

  /* step 4 — install the worker the children wrote */
  yield* step(
    4,
    thought(
      'All three children are back: core, webhook and tests. Install the worker so the engine can route to it.',
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: 'worker::add',
        worker: 'worker-manager',
        input: {
          source: { kind: 'local', path: './payments-ledger' },
          wait: true,
        },
        output: {
          name: 'payments-ledger',
          version: '0.1.0',
          status: 'installed',
          awaited_ready: true,
          config_path: './config.yaml',
        },
        runMs: 1600,
        parentSpan: stepSpan,
        signal,
      }),
  )
  if (signal?.aborted) return

  /* step 5 — one HTTP trigger per function, dispatched in a single step */
  yield* step(
    5,
    thought(
      'Now the entry points. Each function gets an HTTP trigger; the engine owns the routing, so there is no gateway to write.',
      signal,
    ),
    signal,
    async function* (stepSpan) {
      for (const [fn, method, path] of HTTP_TRIGGERS) {
        yield* call({
          fn: 'engine::register_trigger',
          worker: 'iii',
          input: {
            trigger_type: 'http',
            function_id: fn,
            config: { method, path },
          },
          output: {
            id: `trg_${path.replace(/\W+/g, '_')}`,
            trigger_type: 'http',
            function_id: fn,
            registered: true,
          },
          runMs: 320,
          reportedMs: 0.31,
          parentSpan: stepSpan,
          signal,
        })
        if (signal?.aborted) return
      }
    },
  )
  if (signal?.aborted) return

  /* step 6 — run the child's tests against the live worker */
  yield* step(
    6,
    thought(
      'Run the tests the child wrote against the installed worker.',
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: 'shell::exec',
        worker: 'shell',
        input: {
          command: 'iii test payments-ledger',
          cwd: '/workspace/payments-ledger',
          timeout_ms: 120000,
        },
        output: {
          exit_code: 0,
          stdout: TEST_STDOUT,
          stderr: '',
          duration_ms: 6041,
          timed_out: false,
          stdout_truncated: false,
          stderr_truncated: false,
        },
        runMs: 2600,
        reportedMs: 6041,
        parentSpan: stepSpan,
        signal,
      }),
  )
  if (signal?.aborted) return
  yield* readPause(signal)

  /* step 7 — call the thing it just built */
  yield* step(
    7,
    thought(
      'Tests pass. Last check: call the function for real, through the engine, the same way the HTTP trigger will.',
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: 'payments::charge::record',
        worker: 'payments-ledger',
        input: {
          amount: 4200,
          currency: 'usd',
          customer_id: 'cus_8Kd2Qw',
          provider_event_id: 'evt_3PfL2m',
        },
        output: {
          entry_id: 'led_01hq5r7t9m',
          account: 'cus_8Kd2Qw',
          amount: 4200,
          currency: 'usd',
          balance: 4200,
          posted_at: new Date().toISOString(),
          idempotent_replay: false,
        },
        runMs: 900,
        reportedMs: 12.4,
        parentSpan: stepSpan,
        signal,
        inner: [['execute database::transaction', 'database', 0.6]],
        callout: {
          anchor: 'waterfall',
          title: 'a worker that did not exist a minute ago',
          text: 'The new worker’s span lands under the same trace, next to the `database` call it makes. Nothing was re-instrumented to get it there.',
        },
      }),
  )
  if (signal?.aborted) return
  yield* readPause(signal)

  /* step 8 — the answer, and a tour of the pane on the right */
  yield* step(
    8,
    assistant(FINAL_ANSWER, signal, 22),
    signal,
    async function* () {
      yield {
        kind: 'demo-callout',
        callout: {
          anchor: 'waterfall',
          title: 'one trace, eight durable steps',
          text: 'Each `harness::turn step` row is one queue item. A crash mid-turn resumes from the last completed step instead of starting the conversation over.',
        },
      }
    },
  )
}
