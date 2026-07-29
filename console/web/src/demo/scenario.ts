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

import type { StreamEvent } from "@/lib/backend";
import { sleep, tokenize } from "@/stories/playground/scenarios/helpers";

export const PROMPT = "build a payments ledger service with a durable db";

export const MODEL_ID = "anthropic::claude-opus-4-7";
export const SESSION_ID = "console-payments-ledger-demo";
export const TRACE_ID = "trace-payments-ledger-0000000001";

/**
 * Global playback rate. `prefers-reduced-motion` sets it near zero so the
 * turn fills in at once and holds, instead of animating for a minute.
 */
let SPEED = 1;
export function setScenarioSpeed(multiplier: number) {
  SPEED = multiplier;
}
const nap = (ms: number, signal?: AbortSignal) => sleep(ms * SPEED, signal);

export type CalloutAnchor = "transcript" | "waterfall" | "composer";

export interface Callout {
  title: string;
  text: string;
  anchor: CalloutAnchor;
}

export interface DemoSpanInit {
  id: string;
  parent?: string;
  name: string;
  service: string;
  kind?: string;
  attributes?: Array<[string, unknown]>;
}

/** A child session `harness::spawn` created, as the sidebar shows it. */
export interface DemoSession {
  id: string;
  title: string;
  /** The task the parent handed down: the child's seeding user message. */
  task: string;
}

/**
 * A line a child wrote in its own transcript. Children are replayed a whole
 * entry at a time rather than token by token: only one session is on screen,
 * and the three off-screen ones would be animating at nobody.
 */
export type ChildEntry =
  | { role: "thought"; content: string; durationMs: number }
  | { role: "assistant"; content: string }
  | {
      role: "function-trigger";
      functionId: string;
      /** Worker that runs it — the child's `execute` span's service name. */
      worker: string;
      input: unknown;
      output: unknown;
      durationMs: number;
    };

export type DemoEvent =
  | StreamEvent
  | { kind: "demo-span-open"; span: DemoSpanInit }
  | {
      kind: "demo-span-close";
      id: string;
      status?: "OK" | "ERROR";
      /** Close the span this long after it opened, rather than now. */
      durationMs?: number;
    }
  | { kind: "demo-callout"; callout: Callout }
  | { kind: "demo-session-open"; session: DemoSession }
  | { kind: "demo-session-msg"; id: string; entry: ChildEntry }
  | { kind: "demo-session-done"; id: string; result: string };

export interface ScenarioOptions {
  signal?: AbortSignal;
  /**
   * Resolves when the gated call is released — by the viewer clicking
   * approve/deny on the real card, or by the demo's own timeout. The demo
   * never denies; a deny resolves the same way so the card can't hang.
   */
  gate: (functionTriggerId: string) => Promise<void>;
}

/* ── span bookkeeping ─────────────────────────────────────────────────── */

let spanSeq = 0;
function spanId(prefix: string): string {
  spanSeq += 1;
  return `${prefix}-${String(spanSeq).padStart(3, "0")}`;
}

/** Turn-identity baggage the harness stamps on every span of a step. */
const TURN_TAGS: Array<[string, unknown]> = [
  ["iii.session.id", SESSION_ID],
  ["iii.message.id", "turn-01"],
  ["iii.tag.kind", "harness.turn"],
  ["iii.tag.message", PROMPT],
];

/* ── stream fragments ─────────────────────────────────────────────────── */

/**
 * Time to leave a finished block of prose on screen before the next thing
 * lands on top of it. Scaled by length and capped: the point is that the
 * reader gets to finish the sentence, not that the demo waits out a full
 * read of the closing answer.
 */
function readingDwellMs(body: string): number {
  return Math.min(2800, Math.max(600, tokenize(body).length * 10));
}

async function* thought(
  body: string,
  signal?: AbortSignal,
  meanDelayMs = 38,
): AsyncGenerator<DemoEvent> {
  const startedAt = Date.now();
  yield { kind: "thought-start" };
  for (const token of tokenize(body)) {
    if (signal?.aborted) return;
    if (token) yield { kind: "thought-token", token };
    await nap(meanDelayMs * (0.6 + Math.random() * 0.8), signal);
  }
  yield { kind: "thought-end", durationMs: Date.now() - startedAt };
  await nap(readingDwellMs(body), signal);
}

async function* assistant(
  body: string,
  signal?: AbortSignal,
  meanDelayMs = 38,
): AsyncGenerator<DemoEvent> {
  for (const token of tokenize(body)) {
    if (signal?.aborted) return;
    if (token) yield { kind: "assistant-token", token };
    await nap(meanDelayMs * (0.5 + Math.random()), signal);
  }
  yield { kind: "assistant-end" };
  await nap(readingDwellMs(body), signal);
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
  const startedAt = Date.now();
  yield { kind: "thought-start" };
  for (const token of tokenize("letting the user read…")) {
    if (signal?.aborted) return;
    if (token) yield { kind: "thought-token", token };
    await nap(55, signal);
  }
  await nap(holdMs, signal);
  if (signal?.aborted) return;
  yield { kind: "thought-end", durationMs: Date.now() - startedAt };
}

interface CallOptions {
  fn: string;
  /** Worker that runs it — the `execute` span's service name. */
  worker: string;
  input: unknown;
  output: unknown;
  /** How long the demo lingers on the call, so a viewer can see it happen. */
  runMs: number;
  /**
   * What the card and the span report, when that differs from how long the
   * demo dwells. An engine-local call really does finish in microseconds;
   * animating it that fast would make it invisible, and reporting the dwell
   * instead would claim a trigger registration costs a third of a second.
   */
  reportedMs?: number;
  parentSpan: string;
  signal?: AbortSignal;
  /** Nested spans opened inside the call, as [name, service, share-of-runMs]. */
  inner?: Array<[string, string, number]>;
  /** Hold the call in the approval gate before it executes. */
  gate?: (functionTriggerId: string) => Promise<void>;
  functionTriggerId?: string;
  callout?: Callout;
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
  } = opts;
  const startedAt = Date.now();

  yield {
    kind: "fcall-start",
    functionId: fn,
    input,
    ...(gate
      ? {
          pendingApproval: true,
          functionTriggerId,
          sessionId: SESSION_ID,
        }
      : {}),
  };
  if (callout) yield { kind: "demo-callout", callout };

  if (gate && functionTriggerId) {
    await gate(functionTriggerId);
    if (signal?.aborted) return;
    yield { kind: "fcall-approval-cleared", functionTriggerId, running: true };
  }

  const execSpan = spanId("exec");
  yield {
    kind: "demo-span-open",
    span: {
      id: execSpan,
      parent: parentSpan,
      name: `execute ${fn}`,
      service: worker,
      kind: "internal",
      attributes: [...TURN_TAGS, ["iii.function.id", fn]],
    },
  };

  if (inner?.length) {
    let elapsed = 0;
    for (const [name, service, share] of inner) {
      const childSpan = spanId("inner");
      yield {
        kind: "demo-span-open",
        span: {
          id: childSpan,
          parent: execSpan,
          name,
          service,
          kind: "internal",
          attributes: TURN_TAGS,
        },
      };
      const slice = runMs * share;
      await nap(slice, signal);
      elapsed += slice;
      if (signal?.aborted) return;
      yield {
        kind: "demo-span-close",
        id: childSpan,
        /* Inner work takes its share of the REPORTED time, so a child can
           never outlast the call it happened inside. */
        ...(reportedMs === undefined ? {} : { durationMs: reportedMs * share }),
      };
    }
    await nap(Math.max(0, runMs - elapsed), signal);
  } else {
    await nap(runMs, signal);
  }
  if (signal?.aborted) return;

  yield {
    kind: "demo-span-close",
    id: execSpan,
    ...(reportedMs === undefined ? {} : { durationMs: reportedMs }),
  };
  yield {
    kind: "fcall-end",
    output,
    durationMs: reportedMs ?? Date.now() - startedAt,
    ...(functionTriggerId ? { functionTriggerId } : {}),
  };
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
  const stepSpan = spanId("step");
  const routerSpan = spanId("router");
  const providerSpan = spanId("provider");

  yield {
    kind: "demo-span-open",
    span: {
      id: stepSpan,
      name: "harness::turn step",
      service: "harness",
      kind: "internal",
      attributes: [...TURN_TAGS, ["iii.turn.step", index]],
    },
  };
  yield {
    kind: "demo-span-open",
    span: {
      id: routerSpan,
      parent: stepSpan,
      name: "execute router::chat",
      service: "llm-router",
      kind: "internal",
      attributes: [...TURN_TAGS, ["gen_ai.request.model", "claude-opus-4-7"]],
    },
  };
  yield {
    kind: "demo-span-open",
    span: {
      id: providerSpan,
      parent: routerSpan,
      name: "execute provider::anthropic::stream",
      service: "llm-provider-anthropic",
      kind: "client",
      attributes: TURN_TAGS,
    },
  };

  yield* body;
  if (signal?.aborted) return;

  yield { kind: "demo-span-close", id: providerSpan };
  yield { kind: "demo-span-close", id: routerSpan };

  yield* after(stepSpan);
  if (signal?.aborted) return;

  yield { kind: "demo-span-close", id: stepSpan };
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
 *
 * Each child then works out loud: its `work` beats append to its own
 * transcript and open their own `execute` spans under its turn step, so a
 * viewer who clicks a sidebar row watches that child reason and call, and
 * sees those calls in the same trace as the parent's.
 */
async function* spawnFanOut(
  stepSpan: string,
  gate: ScenarioOptions["gate"],
  signal: AbortSignal | undefined,
): AsyncGenerator<DemoEvent> {
  const startedAt = Date.now();
  const spans = new Map<string, string>();

  /** Open the child's span pair and its session — it is running now. */
  function* launch(child: ChildSpec): Generator<DemoEvent> {
    const execSpan = spanId("exec");
    const childStep = spanId("child");
    spans.set(child.callId, execSpan);
    yield {
      kind: "demo-span-open",
      span: {
        id: execSpan,
        parent: stepSpan,
        name: "execute harness::spawn",
        service: "harness",
        kind: "internal",
        attributes: [...TURN_TAGS, ["iii.child.session_id", child.sessionId]],
      },
    };
    yield {
      kind: "demo-span-open",
      span: {
        id: childStep,
        parent: execSpan,
        name: "harness::turn step",
        service: "harness",
        kind: "internal",
        attributes: [
          ["iii.session.id", child.sessionId],
          ["iii.tag.kind", "harness.subagent"],
          ["iii.tag.display_name", `Subagent · ${child.title}`],
        ],
      },
    };
    spans.set(`${child.callId}:step`, childStep);
    yield {
      kind: "demo-session-open",
      session: { id: child.sessionId, title: child.title, task: child.task },
    };
    /* Whatever the child has to say the moment it wakes up. */
    for (const beat of child.work) {
      if (beat.at === 0) yield* childBeat(child, beat.entry);
    }
  }

  /** One line of a child's own work: its transcript entry, and its span. */
  function* childBeat(
    child: ChildSpec,
    entry: ChildEntry,
  ): Generator<DemoEvent> {
    yield { kind: "demo-session-msg", id: child.sessionId, entry };
    if (entry.role !== "function-trigger") return;
    const callSpan = spanId("childcall");
    yield {
      kind: "demo-span-open",
      span: {
        id: callSpan,
        parent: spans.get(`${child.callId}:step`),
        name: `execute ${entry.functionId}`,
        service: entry.worker,
        kind: "internal",
        attributes: [
          ["iii.session.id", child.sessionId],
          ["iii.function.id", entry.functionId],
        ],
      },
    };
    /* Closed at what the call really costs, like every other call here: the
       child's turn span is the wide one, its dispatches are hairlines. */
    yield {
      kind: "demo-span-close",
      id: callSpan,
      durationMs: entry.durationMs,
    };
  }

  /* Every card at once — a parallel tool-call batch is one assistant turn. */
  for (const child of CHILDREN) {
    yield {
      kind: "fcall-start",
      functionId: "harness::spawn",
      input: spawnInput(child),
      functionTriggerId: child.callId,
      sessionId: SESSION_ID,
      ...(child.gated ? { pendingApproval: true } : {}),
    };
  }
  for (const child of CHILDREN) {
    if (!child.gated) yield* launch(child);
  }

  yield {
    kind: "demo-callout",
    callout: {
      anchor: "transcript",
      title: "three subagents, one held at the gate",
      text: "Each subagent is a real session started in iii's native way, the sandbox worker, with its own transcript, its own turn budget and its own function policy, listed under this chat as it starts. Only `ledger core` asked for `database::*`, a write scope this session does not hold, so only that one waits for a human. Click approve to release it.",
    },
  };

  for (const child of CHILDREN) {
    if (!child.gated) continue;
    await gate(child.callId);
    if (signal?.aborted) return;
    yield {
      kind: "fcall-approval-cleared",
      functionTriggerId: child.callId,
      running: true,
    };
    yield* launch(child);
  }

  yield {
    kind: "demo-callout",
    callout: {
      anchor: "transcript",
      title: "three sessions, running at once",
      text: "Every subagent is a session of its own, listed in the sidebar. Click one to watch it think and call while the others keep going; whatever it dispatches lands in this same trace, under its own `harness::turn step`.",
    },
  };

  /* One clock over all three: the children interleave, and each reports when
     it is done rather than in dispatch order. */
  const beats = [
    ...CHILDREN.flatMap((child) =>
      child.work
        .filter((beat) => beat.at > 0)
        .map((beat) => ({ at: beat.at, child, entry: beat.entry })),
    ),
    ...CHILDREN.map((child) => ({
      at: child.finishAfterMs,
      child,
      entry: undefined,
    })),
  ].sort((a, b) => a.at - b.at);

  let waited = 0;
  for (const beat of beats) {
    await nap(beat.at - waited, signal);
    if (signal?.aborted) return;
    waited = beat.at;

    if (beat.entry) {
      yield* childBeat(beat.child, beat.entry);
      continue;
    }

    const child = beat.child;
    const childStep = spans.get(`${child.callId}:step`);
    const execSpan = spans.get(child.callId);
    if (childStep) yield { kind: "demo-span-close", id: childStep };
    if (execSpan) yield { kind: "demo-span-close", id: execSpan };
    yield {
      kind: "demo-session-done",
      id: child.sessionId,
      result: child.resultText,
    };
    yield {
      kind: "fcall-end",
      output: spawnResult(child),
      durationMs: Date.now() - startedAt,
      functionTriggerId: child.callId,
    };
  }
}

/* ── outputs ──────────────────────────────────────────────────────────── */

const CONNECTED_AT = () => Date.now() - 4 * 60 * 60 * 1000;

const WORKERS_LIST = () => ({
  workers: [
    {
      id: "wrk_01hq4m8database",
      name: "database",
      description: "Durable postgres: tables, queries, migrations.",
      version: "0.21.0",
      runtime: "rust",
      os: "linux",
      status: "connected",
      function_count: 9,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 0,
      isolation: "container",
      tag: "core",
    },
    {
      id: "wrk_01hq4m8shell00",
      name: "shell",
      description: "Scoped command execution and filesystem access.",
      version: "0.21.0",
      runtime: "rust",
      os: "linux",
      status: "connected",
      function_count: 16,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 0,
      isolation: "container",
    },
    {
      id: "wrk_01hq4m8coder000",
      name: "coder",
      description: "Reads, writes and patches files in a scoped workspace.",
      version: "0.21.0",
      runtime: "node",
      os: "linux",
      status: "connected",
      function_count: 12,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 0,
      isolation: "container",
    },
    {
      id: "wrk_01hq4m8harness0",
      name: "harness",
      description: "The durable agent turn loop.",
      version: "0.21.0",
      runtime: "rust",
      os: "linux",
      status: "connected",
      function_count: 14,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 1,
      isolation: "container",
    },
    {
      id: "wrk_01hq4m8observ00",
      name: "observability",
      description: "OTel collector: traces, logs and metrics for the engine.",
      version: "0.21.0",
      runtime: "rust",
      os: "linux",
      status: "connected",
      function_count: 6,
      connected_at_ms: CONNECTED_AT(),
      active_invocations: 0,
      isolation: "container",
    },
  ],
});

const DATABASE_INFO = () => ({
  worker: {
    id: "wrk_01hq4m8database",
    name: "database",
    description: "Durable postgres: tables, queries, migrations.",
    version: "0.21.0",
    runtime: "rust",
    os: "linux",
    status: "connected",
    function_count: 9,
    connected_at_ms: CONNECTED_AT(),
    active_invocations: 0,
    isolation: "container",
    internal: false,
    pid: 1421,
    latest_metrics: null,
  },
  functions: [
    {
      function_id: "database::create_table",
      worker_name: "database",
      description: "Create a table from a column spec.",
    },
    {
      function_id: "database::query",
      worker_name: "database",
      description: "Run a parameterised read query.",
    },
    {
      function_id: "database::execute",
      worker_name: "database",
      description: "Run a parameterised write statement.",
    },
    {
      function_id: "database::transaction",
      worker_name: "database",
      description: "Run several statements atomically.",
    },
    {
      function_id: "database::migrate",
      worker_name: "database",
      description: "Apply pending migrations.",
    },
  ],
  trigger_types: [
    {
      id: "database.row_changed",
      worker_name: "database",
      description: "Fires when a watched table changes.",
    },
  ],
  registered_triggers: [],
});

/**
 * The fan-out. Three children, dispatched in one step, each its own session
 * with its own budget and its own function policy. Only the first asks for
 * `database::*` — the write scope this session does not hold — so only the
 * first stops at the approval gate; the other two dispatch immediately.
 */
interface ChildSpec {
  /** iii function_call_id, and the key the approval resolves against. */
  callId: string;
  sessionId: string;
  title: string;
  task: string;
  allow: string[];
  /** Held at the gate before it may run. */
  gated?: boolean;
  /** The child's own transcript, played out while the parent waits. */
  work: ChildBeat[];
  resultText: string;
  resultDetails: Record<string, unknown>;
  /** Delay after the gate clears before this child reports, in ms. */
  finishAfterMs: number;
}

interface ChildBeat {
  /** ms after the gate clears; 0 lands the moment the child starts. */
  at: number;
  entry: ChildEntry;
}

/* The source the children write, shown by the coder card as they write it. */

const CHARGE_RECORD_RS = `use crate::types::{Charge, Entry};
use iii_sdk::{Error, IIIClient, RegisterFunction, TriggerRequest};
use serde_json::json;

/// A charge is two rows: debit the customer, credit the house account.
/// Both land in one \`database::transaction\`, or neither does.
pub fn register(iii: &IIIClient) {
    let db = iii.clone();
    iii.register_function(
        "payments::charge::record",
        RegisterFunction::new_async(move |charge: Charge| {
            let db = db.clone();
            async move {
                if let Some(posted) = replay_of(&db, &charge.provider_event_id).await? {
                    return Ok::<Entry, Error>(posted);
                }
                let rows = db
                    .trigger(TriggerRequest {
                        function_id: "database::transaction".into(),
                        payload: json!({ "statements": double_entry(&charge) }),
                        action: None,
                        timeout_ms: Some(5_000),
                    })
                    .await?;
                Ok(Entry::from_rows(&charge, rows)?)
            }
        })
        .description("Post a charge to the ledger as a balanced double entry."),
    );
}`;

const CHARGE_REFUND_RS = `use crate::types::{Entry, Refund};
use iii_sdk::{Error, IIIClient, RegisterFunction, TriggerRequest};
use serde_json::json;

/// A refund reverses a posted entry and can never exceed what is left of it.
pub fn register(iii: &IIIClient) {
    let db = iii.clone();
    iii.register_function(
        "payments::charge::refund",
        RegisterFunction::new_async(move |refund: Refund| {
            let db = db.clone();
            async move {
                let original = fetch_entry(&db, &refund.entry_id).await?;
                guard_refundable(&original, &refund)?;
                let rows = db
                    .trigger(TriggerRequest {
                        function_id: "database::transaction".into(),
                        payload: json!({ "statements": reversal(&original, &refund) }),
                        action: None,
                        timeout_ms: Some(5_000),
                    })
                    .await?;
                Ok::<Entry, Error>(Entry::from_reversal(&original, rows)?)
            }
        })
        .description("Reverse a posted charge, in part or in full."),
    );
}`;

const WEBHOOK_STRIPE_RS = `use crate::types::{StripeEvent, WebhookAck};
use iii_sdk::{Error, IIIClient, RegisterFunction, TriggerRequest};
use serde_json::json;

/// Stripe retries. \`charge::record\` already dedupes on the provider event
/// id, so a replayed delivery posts nothing and returns the first entry.
pub fn register(iii: &IIIClient) {
    let engine = iii.clone();
    iii.register_function(
        "payments::webhook::stripe",
        RegisterFunction::new_async(move |event: StripeEvent| {
            let engine = engine.clone();
            async move {
                let Some(charge) = charge_from(&event) else {
                    return Ok::<WebhookAck, Error>(WebhookAck::ignored(&event.kind));
                };
                let entry = engine
                    .trigger(TriggerRequest {
                        function_id: "payments::charge::record".into(),
                        payload: json!(charge),
                        action: None,
                        timeout_ms: Some(10_000),
                    })
                    .await?;
                Ok(WebhookAck::posted(entry))
            }
        })
        .description("Post a Stripe charge event to the ledger, exactly once."),
    );
}`;

const RECONCILE_RS = `use crate::types::{ReconcileReport, Window};
use iii_sdk::{Error, IIIClient, RegisterFunction, TriggerRequest};
use serde_json::json;

/// Every account's entries must sum to its balance and every entry must have
/// a counterpart. Anything else is reported, never silently repaired.
pub fn register(iii: &IIIClient) {
    let db = iii.clone();
    iii.register_function(
        "payments::ledger::reconcile",
        RegisterFunction::new_async(move |window: Window| {
            let db = db.clone();
            async move {
                let rows = db
                    .trigger(TriggerRequest {
                        function_id: "database::query".into(),
                        payload: json!({ "sql": UNBALANCED, "params": [window.since] }),
                        action: None,
                        timeout_ms: Some(30_000),
                    })
                    .await?;
                Ok::<ReconcileReport, Error>(ReconcileReport::from_rows(rows)?)
            }
        })
        .description("Check the ledger balances and flag orphaned entries."),
    );
}`;

const TESTS_RS = `use crate::support::{ledger, TestEngine};

// The suite runs through the engine, against the contract, so it passes and
// fails the same way a caller does.

#[tokio::test]
async fn charge_record_writes_double_entry() {
    let engine = TestEngine::start().await;
    let entry = ledger::record(&engine, 4_200, "cus_8Kd2Qw", "evt_3PfL2m").await;

    assert_eq!(entry.balance, 4_200);
    assert_eq!(ledger::rows_for(&engine, &entry.entry_id).await.len(), 2);
    assert_eq!(ledger::sum_of(&engine, &entry.account).await, 0);
}

#[tokio::test]
async fn stripe_webhook_dedupes_by_event_id() {
    let engine = TestEngine::start().await;
    let first = ledger::webhook(&engine, "evt_3PfL2m").await;
    let replay = ledger::webhook(&engine, "evt_3PfL2m").await;

    assert_eq!(first.entry_id, replay.entry_id);
    assert!(replay.idempotent_replay);
}

#[tokio::test]
async fn refund_rejects_over_refund() {
    let engine = TestEngine::start().await;
    let entry = ledger::record(&engine, 4_200, "cus_8Kd2Qw", "evt_9Qm1Zx").await;
    let err = ledger::try_refund(&engine, &entry.entry_id, 5_000).await.unwrap_err();

    assert!(err.to_string().contains("exceeds"));
}

// 8 more: reversal maths, unknown event kinds, orphaned entries, reads of
// uncommitted rows, concurrent charges on one account, schema drift.`;

/** A `coder::create-file` exchange, as the batch card renders it. */
function writeFiles(
  files: Array<[path: string, content: string]>,
  durationMs: number,
): ChildEntry {
  return {
    role: "function-trigger",
    functionId: "coder::create-file",
    worker: "coder",
    input: {
      files: files.map(([path, content]) => ({ path, content, parents: true })),
    },
    output: {
      results: files.map(([path, content]) => ({
        path: `/workspace/payments-ledger/${path}`,
        success: true,
        bytes_written: content.length,
      })),
    },
    durationMs,
  };
}

/** A `database::create_table` exchange. */
function createTable(
  table: string,
  columns: Array<[name: string, type: string]>,
  durationMs: number,
): ChildEntry {
  return {
    role: "function-trigger",
    functionId: "database::create_table",
    worker: "database",
    input: {
      table,
      if_not_exists: true,
      columns: columns.map(([name, type]) => ({ name, type })),
    },
    output: { table, created: true, columns: columns.length },
    durationMs,
  };
}

const CHILDREN: ChildSpec[] = [
  {
    callId: "fc_spawn_ledger_core",
    sessionId: "console-sub-ledger-core",
    title: "ledger core",
    task: `Write the payments-ledger worker's core against the existing \`database\` worker.

\`payments::charge::record\` and \`payments::charge::refund\`, double-entry rows,
\`database::transaction\` for anything that writes two rows. Create the
\`ledger_entries\` and \`ledger_accounts\` tables.`,
    allow: ["coder::*", "database::*"],
    gated: true,
    work: [
      {
        at: 0,
        entry: {
          role: "assistant",
          content:
            "Double-entry, so a charge is never one row: debit the customer account, credit the house account, both inside one `database::transaction` or neither. That also gives the tests their invariant, every account summing to zero across its entries. Tables first, then the two writers.",
        },
      },
      {
        at: 1300,
        entry: createTable(
          "ledger_accounts",
          [
            ["id", "text primary key"],
            ["owner", "text not null"],
            ["currency", "text not null"],
            ["balance_minor", "bigint not null default 0"],
          ],
          18.7,
        ),
      },
      {
        at: 2500,
        entry: createTable(
          "ledger_entries",
          [
            ["id", "text primary key"],
            ["account_id", "text not null references ledger_accounts(id)"],
            ["amount_minor", "bigint not null"],
            ["provider_event_id", "text unique"],
            ["posted_at", "timestamptz not null default now()"],
          ],
          22.4,
        ),
      },
      {
        at: 3500,
        entry: {
          role: "thought",
          content:
            "Both tables are up, and the foreign key ties every entry to an account. Now the writers. Refund is the one with a rule attached: it reverses a posted entry and can never take out more than is left in it.",
          durationMs: 2400,
        },
      },
      {
        at: 4400,
        entry: writeFiles(
          [
            ["src/functions/charge_record.rs", CHARGE_RECORD_RS],
            ["src/functions/charge_refund.rs", CHARGE_REFUND_RS],
          ],
          61.5,
        ),
      },
    ],
    resultText:
      "ledger core done: charge::record + charge::refund over a double-entry schema, 2 tables created.",
    resultDetails: {
      functions: ["payments::charge::record", "payments::charge::refund"],
      tables: ["ledger_entries", "ledger_accounts"],
      turns_used: 7,
    },
    finishAfterMs: 7400,
  },
  {
    callId: "fc_spawn_ledger_webhook",
    sessionId: "console-sub-ledger-webhook",
    title: "stripe webhook",
    task: `Write \`payments::webhook::stripe\` and \`payments::ledger::reconcile\` for the
payments-ledger worker. Idempotent on the provider event id: a replayed event
must post nothing and return the original entry.`,
    allow: ["coder::*", "web::search"],
    work: [
      {
        at: 0,
        entry: {
          role: "assistant",
          content:
            "Stripe retries on any non-2xx, so this has to be idempotent at the ledger rather than at the HTTP layer. The provider event id is the natural key and `charge::record` is already unique on it, so the webhook stays thin: map the event, hand it over, return whatever comes back.",
        },
      },
      {
        at: 1500,
        entry: writeFiles(
          [["src/functions/webhook_stripe.rs", WEBHOOK_STRIPE_RS]],
          44.6,
        ),
      },
      {
        at: 2500,
        entry: {
          role: "thought",
          content:
            "Reconcile is the other half of trusting the ledger: read-only, so it can run on a cron without taking a lock, and it reports drift rather than repairing it. Silently fixing a payments discrepancy is how you lose the audit trail that made it findable.",
          durationMs: 1900,
        },
      },
      {
        at: 3300,
        entry: writeFiles([["src/functions/reconcile.rs", RECONCILE_RS]], 31.9),
      },
    ],
    resultText:
      "webhook + reconcile done: dedupe keyed on provider_event_id, replays return the original entry.",
    resultDetails: {
      functions: ["payments::webhook::stripe", "payments::ledger::reconcile"],
      idempotency_key: "provider_event_id",
      turns_used: 5,
    },
    finishAfterMs: 4600,
  },
  {
    callId: "fc_spawn_ledger_tests",
    sessionId: "console-sub-ledger-tests",
    title: "test suite",
    task: `Write the payments-ledger test suite. Cover double-entry balance, refunds
over the original amount, webhook replay, concurrent charges on one account,
and that the schema matches the migration.`,
    allow: ["coder::*", "shell::exec"],
    work: [
      {
        at: 0,
        entry: {
          role: "assistant",
          content:
            "The other two are still writing, so I will test the contract rather than their implementation: call every function through the engine the way a caller would. The suite is then ready the moment the worker installs, and it fails for the same reasons production would.",
        },
      },
      { at: 1100, entry: writeFiles([["tests/ledger.rs", TESTS_RS]], 38.2) },
    ],
    resultText:
      "test suite done: 11 tests over the ledger, webhook and schema.",
    resultDetails: { tests: 11, files: ["tests/ledger.rs"], turns_used: 4 },
    finishAfterMs: 2600,
  },
];

function spawnInput(child: ChildSpec) {
  return {
    task: child.task,
    model: "claude-opus-4-7",
    session_id: child.sessionId,
    parent_session_id: SESSION_ID,
    options: {
      mode: "agent",
      max_turns: 12,
      output: { type: "text" },
      functions: { allow: child.allow, deny: ["compose::*", "worker::remove"] },
    },
  };
}

function spawnResult(child: ChildSpec) {
  return {
    content: [{ type: "text" as const, text: child.resultText }],
    details: { session_id: child.sessionId, ...child.resultDetails },
  };
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

test result: ok. 11 passed; 0 failed; finished in 1.83s`;

const HTTP_TRIGGERS = [
  ["payments::charge::record", "POST", "/payments/charges"],
  ["payments::charge::refund", "POST", "/payments/refunds"],
  ["payments::webhook::stripe", "POST", "/payments/webhooks/stripe"],
  ["payments::ledger::reconcile", "POST", "/payments/reconcile"],
] as const;

const FINAL_ANSWER = `\`payments-ledger\` is live on the engine and answering.

| function | trigger | backed by |
| --- | --- | --- |
| \`payments::charge::record\` | \`POST /payments/charges\` | \`database::transaction\` |
| \`payments::charge::refund\` | \`POST /payments/refunds\` | \`database::transaction\` |
| \`payments::webhook::stripe\` | \`POST /payments/webhooks/stripe\` | \`database::execute\` |
| \`payments::ledger::reconcile\` | \`POST /payments/reconcile\` | \`database::query\` |

Nothing here needed to be integrated. The \`database\` worker was available on the registry,
so I installed it like a library. Unlike a library it was immediately ready to use. It joined
the same engine the rest of your workers are on.

**About those three subagents.** \`ledger core\`, \`stripe webhook\` and
\`test suite\` are still listed under this chat. They are real sessions
started in iii's native way: the sandbox worker. Each one had its own transcript,
its own turn budget and its own function policy, and you can open any of
them to read what it did. 

**About the pane on the right.** That is the
trace the observability worker recorded while the harness worker did its work.
There was no separate setup, as with any worker if it's added to the iii engine
it's immediately and completely observable. Every row above opened a span:
each \`harness::turn step\` is one durable step of my loop running on top of the queue worker, the
three \`harness::spawn\` branches are the subagents running at the same time, and
\`execute payments::charge::record\` near the bottom is the function they built
answering a live request. In the console you would click any bar for its
arguments, its result, and its logs. If this turn had crashed halfway, the loop
would have resumed from the last completed step, into the same trace.`;

/* ── the script ───────────────────────────────────────────────────────── */

export async function* runScenario(
  opts: ScenarioOptions,
): AsyncGenerator<DemoEvent> {
  const { signal, gate } = opts;
  spanSeq = 0;

  yield { kind: "turn-status", phase: "accepted" };
  await nap(420, signal);
  yield { kind: "turn-status", phase: "started" };
  await nap(260, signal);

  /* step 1 — look at what is already running */
  yield* step(
    1,
    thought(
      "The user wants a payments ledger with durable storage. Before I write a line of it I should look at what is already connected to this engine. iii keeps a live catalog of every running worker, so a durable database may already be here. If it is, there is nothing to scaffold and nothing to deploy alongside.",
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: "engine::workers::list",
        worker: "iii",
        input: { status: "connected" },
        output: WORKERS_LIST(),
        runMs: 900,
        reportedMs: 2.1,
        parentSpan: stepSpan,
        signal,
        callout: {
          anchor: "transcript",
          title: "discovery",
          text: "The agent reads the engine’s live worker catalog. Whatever is already running is already integrated and callable.",
        },
      }),
  );
  if (signal?.aborted) return;
  yield* readPause(signal);

  /* step 2 — read the database worker's contract */
  yield* step(
    2,
    thought(
      "`database` is available: durable postgres, nine functions. Let me read its contract so the ledger is written against the real signatures instead of guesses.",
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: "engine::workers::info",
        worker: "iii",
        input: { name: "database" },
        output: DATABASE_INFO(),
        runMs: 700,
        reportedMs: 1.4,
        parentSpan: stepSpan,
        signal,
        callout: {
          anchor: "waterfall",
          title: "every operation is traced",
          text: "Every call opens a span. This is the console’s own trace view, filling in live, recorded by the observability worker. Any worker added to the engine is immediately observable, no separate setup, and it is OTel compatible.",
        },
      }),
  );
  if (signal?.aborted) return;
  yield* readPause(signal);

  /* step 3 — fan out to three subagents; one of them stops at the gate */
  yield* step(
    3,
    assistant(
      "`database` gives me durable postgres with transactions, so the ledger can be double-entry without a second datastore. Three pieces here are independent, so I’ll run them as three subagents and stay on the wiring myself.",
      signal,
    ),
    signal,
    (stepSpan) => spawnFanOut(stepSpan, gate, signal),
  );
  if (signal?.aborted) return;
  yield* readPause(signal);

  /* step 4 — install the worker the subagents wrote */
  yield* step(
    4,
    thought(
      "All three subagents are back: core, webhook and tests. Install the worker so the engine can route to it.",
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: "worker::add",
        worker: "worker-manager",
        input: {
          source: { kind: "local", path: "./payments-ledger" },
          wait: true,
        },
        output: {
          name: "payments-ledger",
          version: "0.1.0",
          status: "installed",
          awaited_ready: true,
          config_path: "./config.yaml",
        },
        runMs: 1600,
        parentSpan: stepSpan,
        signal,
      }),
  );
  if (signal?.aborted) return;

  /* step 5 — one HTTP trigger per function, dispatched in a single step */
  yield* step(
    5,
    thought(
      "Now the entry points. I can use the http worker and each function gets an HTTP trigger.",
      signal,
    ),
    signal,
    async function* (stepSpan) {
      for (const [fn, method, path] of HTTP_TRIGGERS) {
        yield* call({
          fn: "engine::register_trigger",
          worker: "iii",
          input: {
            trigger_type: "http",
            function_id: fn,
            config: { method, path },
          },
          output: {
            id: `trg_${path.replace(/\W+/g, "_")}`,
            trigger_type: "http",
            function_id: fn,
            registered: true,
          },
          runMs: 320,
          reportedMs: 0.31,
          parentSpan: stepSpan,
          signal,
        });
        if (signal?.aborted) return;
      }
    },
  );
  if (signal?.aborted) return;

  /* step 6 — run the subagent's tests against the live worker */
  yield* step(
    6,
    thought(
      "Run the tests the subagent wrote against the installed worker.",
      signal,
    ),
    signal,
    (stepSpan) =>
      call({
        fn: "shell::exec",
        worker: "shell",
        input: {
          command: "iii test payments-ledger",
          cwd: "/workspace/payments-ledger",
          timeout_ms: 120000,
        },
        output: {
          exit_code: 0,
          stdout: TEST_STDOUT,
          stderr: "",
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
  );
  if (signal?.aborted) return;
  yield* readPause(signal);

  /* step 7 — call the thing it just built */
  yield* step(
    7,
    thought("Tests pass. Last check: call the function for real.", signal),
    signal,
    (stepSpan) =>
      call({
        fn: "payments::charge::record",
        worker: "payments-ledger",
        input: {
          amount: 4200,
          currency: "usd",
          customer_id: "cus_8Kd2Qw",
          provider_event_id: "evt_3PfL2m",
        },
        output: {
          entry_id: "led_01hq5r7t9m",
          account: "cus_8Kd2Qw",
          amount: 4200,
          currency: "usd",
          balance: 4200,
          posted_at: new Date().toISOString(),
          idempotent_replay: false,
        },
        runMs: 900,
        reportedMs: 12.4,
        parentSpan: stepSpan,
        signal,
        inner: [["execute database::transaction", "database", 0.6]],
        callout: {
          anchor: "waterfall",
          title: "a new payments worker, just added to the system",
          text: "The new worker’s span lands under the same trace, next to the `database` call it makes. Nothing was re-instrumented to get it there.",
        },
      }),
  );
  if (signal?.aborted) return;
  yield* readPause(signal);

  /* step 8 — the answer, and a tour of the pane on the right */
  yield* step(
    8,
    assistant(FINAL_ANSWER, signal, 22),
    signal,
    async function* () {
      yield {
        kind: "demo-callout",
        callout: {
          anchor: "waterfall",
          title: "one trace, eight durable steps",
          text: "Each `harness::turn step` row is one durable step of the loop, running on top of the queue worker. A crash mid-turn resumes from the last completed step, into the same trace. Agentic loops are a normal engineering pattern in iii. They work the same as any programmatic loop would in iii.",
        },
      };
    },
  );
}
