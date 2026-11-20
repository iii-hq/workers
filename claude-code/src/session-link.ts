/**
 * The child-session link that makes a run a SUB-AGENT rather than a function
 * call that happened to return text.
 *
 * A harness sub-agent is not a special mechanism: it is a session whose
 * metadata names its parent, a transcript the console can read, and a status
 * that moves. Any worker that does those three things shows up in the console's
 * session tree and in the parent turn's chat — no harness change, no special
 * case for this worker.
 *
 * Everything here is best-effort. `session-manager` is a dependency of the
 * console's views, not of running an agent: a task must still run when nobody
 * is watching.
 */

import type { IIIClient } from 'iii-sdk';

const TIMEOUT_MS = 15_000;

/** One line of a title, so a session tree stays readable. */
function title(task: string): string {
  const line = task.replace(/\s+/g, ' ').trim();
  return line.length > 60 ? `${line.slice(0, 59)}…` : line || 'claude task';
}

/**
 * Create or adopt the session a task runs in, linked to the session that asked
 * for it. `parent_session_id` is what the console nests on; it is applied on
 * CREATION only, which is why this runs before the first turn rather than after.
 */
export async function linkChildSession(
  iii: IIIClient,
  options: { sessionId: string; parentSessionId?: string; task: string },
): Promise<{ linked: boolean; detail: string; owner: Ownership }> {
  const metadata: Record<string, unknown> = { agent: 'claude-code' };
  if (options.parentSessionId) metadata.parent_session_id = options.parentSessionId;
  try {
    // `ensure` answers with the session's STORED metadata — for a session it
    // just created that is ours, for an existing one it is whoever made it.
    // That is the ownership evidence; the call succeeding is not.
    const res = await iii.call<unknown, { meta?: { metadata?: Record<string, unknown> } } | null>({
      function_id: 'session::ensure',
      payload: {
        session_id: options.sessionId,
        title: title(options.task),
        metadata,
      },
      timeoutMs: TIMEOUT_MS,
    });
    const stored = res?.meta?.metadata;
    const owner: Ownership = !stored
      ? 'unknown'
      : stored.agent === 'claude-code'
        ? 'child'
        : 'other';
    return { linked: true, detail: '', owner };
  } catch (err) {
    // A missing session-manager costs the console its session tree, not the run.
    const detail = `session::ensure failed for ${options.sessionId}: ${String(err)}`;
    console.warn(`claude-code: ${detail}`);
    return { linked: false, detail, owner: 'unknown' };
  }
}

/** Move the session's coarse status, so a tree shows working / done / error. */
/** Waits before each retry of a terminal status write. `[0, 0]` in tests. */
export const statusRetry = { delaysMs: [500, 2_000] };

/**
 * Terminal statuses whose every retry failed, by session. Written the next
 * time any status write of this worker succeeds (session-manager answers
 * again), with no timer; a session's own next write replaces its entry.
 */
const unsettled = new Map<string, { status: 'done' | 'error'; reason?: string }>();
let reconciling = false;
/** A success arrived during a sweep: run another pass once it ends. */
let reconcileRequested = false;

/** Each session's status writes, replays included, go out one at a time, in order. */
const statusQueues = new Map<string, Promise<unknown>>();

function inOrder<T>(sessionId: string, op: () => Promise<T>): Promise<T> {
  const next = (statusQueues.get(sessionId) ?? Promise.resolve()).then(op, op);
  const settled = next.catch(() => undefined);
  statusQueues.set(sessionId, settled);
  void settled.then(() => {
    if (statusQueues.get(sessionId) === settled) statusQueues.delete(sessionId);
  });
  return next;
}

async function writeStatus(
  iii: IIIClient,
  sessionId: string,
  status: 'working' | 'done' | 'error',
  reason?: string,
): Promise<void> {
  await iii.call({
    function_id: 'session::set-status',
    payload: { session_id: sessionId, status, ...(reason ? { reason } : {}) },
    timeoutMs: TIMEOUT_MS,
  });
}

async function reconcileUnsettled(iii: IIIClient): Promise<void> {
  if (unsettled.size === 0) return;
  if (reconciling) {
    // Only a successful write asks, so a store that keeps failing never loops.
    reconcileRequested = true;
    return;
  }
  reconciling = true;
  try {
    do {
      reconcileRequested = false;
      for (const [sessionId, pending] of [...unsettled]) {
        try {
          await inOrder(sessionId, async () => {
            // A newer write for this session was requested meanwhile: it wins,
            // and this replay is never sent.
            if (unsettled.get(sessionId) !== pending) return;
            await writeStatus(iii, sessionId, pending.status, pending.reason);
            if (unsettled.get(sessionId) === pending) unsettled.delete(sessionId);
          });
        } catch {
          // Still failing: it stays pending for the next success; try the rest.
        }
      }
    } while (reconcileRequested && unsettled.size > 0);
  } finally {
    reconciling = false;
  }
}

export async function setSessionStatus(
  iii: IIIClient,
  sessionId: string,
  status: 'working' | 'done' | 'error',
  reason?: string,
): Promise<boolean> {
  // A newer status for this session replaces a lost one at once, so a replay
  // not yet sent is skipped; one already sent finishes first (in order).
  unsettled.delete(sessionId);
  try {
    await inOrder(sessionId, () => writeStatus(iii, sessionId, status, reason));
    // The store answers again: write any other session's lost status, in the
    // background, without holding this caller.
    void reconcileUnsettled(iii);
    return true;
  } catch (err) {
    console.warn(
      `claude-code: session::set-status ${status} failed for ${sessionId}: ${String(err)}`,
    );
    return false;
  }
}

export const RESTART_REASON = 'claude-code restarted while this run was active';

/**
 * At startup, settle every session this worker created that still says
 * `working`: no run of its own can be live yet, so each one lost its final
 * status (a write that never landed, or a worker that died mid-run). Each
 * namespace has at most one claude-code, and its calls reach only its own
 * namespace's session-manager, so every `agent: claude-code` session listed
 * here is this worker's. `ranHere` spares every session this worker has run a turn in since it started,
 * so a turn that starts and finishes during the scan keeps its own status. Every
 * page is read before anything is written: a write moves a session out of the
 * `working` filter under the cursor. Returns how many were settled.
 */
export async function settleStaleSessions(
  iii: IIIClient,
  ranHere: (sessionId: string) => boolean,
): Promise<number> {
  const stale: string[] = [];
  try {
    let cursor: string | null | undefined;
    do {
      const page = await iii.call<
        unknown,
        { sessions?: Array<{ session_id?: string }>; next_cursor?: string | null } | null
      >({
        function_id: 'session::list',
        payload: {
          status: 'working',
          metadata: { agent: 'claude-code' },
          limit: 100,
          ...(cursor ? { cursor } : {}),
        },
        timeoutMs: TIMEOUT_MS,
      });
      for (const s of page?.sessions ?? []) {
        if (typeof s.session_id === 'string') stale.push(s.session_id);
      }
      cursor = page?.next_cursor;
    } while (cursor);
  } catch (err) {
    console.warn(`claude-code: could not list sessions left working: ${String(err)}`);
    return 0;
  }
  let settled = 0;
  for (const sessionId of stale) {
    if (ranHere(sessionId)) continue;
    if (await setSessionStatus(iii, sessionId, 'error', RESTART_REASON)) settled++;
    // Not written: replayed once the store answers, unless a run on this
    // session writes its own status first (which drops the replay).
    else unsettled.set(sessionId, { status: 'error', reason: RESTART_REASON });
  }
  return settled;
}

/**
 * A run's last status write: retried, since nothing writes this session's
 * status again until its next run, and a lost `done` or `error` would leave it
 * reporting `working` with no run behind it.
 */
export async function settleSessionStatus(
  iii: IIIClient,
  sessionId: string,
  status: 'done' | 'error',
  reason?: string,
): Promise<boolean> {
  if (await setSessionStatus(iii, sessionId, status, reason)) return true;
  for (const delay of statusRetry.delaysMs) {
    await new Promise((resolve) => setTimeout(resolve, delay));
    if (await setSessionStatus(iii, sessionId, status, reason)) return true;
  }
  console.error(
    `claude-code: session ${sessionId} still shows working: its ${status} status could not be written; it is written once session-manager answers again`,
  );
  unsettled.set(sessionId, { status, ...(reason ? { reason } : {}) });
  return false;
}

/**
 * The scope a delegated task's outcome lands in. An orchestrator observes
 * `state::on-changed` with filters on `input.scope` (TASK_SCOPE) and
 * `input.key` (the child session id) and is WOKEN when the task settles — no polling, no blocking call held open for the
 * length of an agent run. It is the same shape a harness sub-agent uses: the
 * child writes where it was told to write, and the parent reacts to that write.
 */
export const TASK_SCOPE = 'agent_tasks';

export type TaskOutcome = {
  session_id: string;
  parent_session_id?: string;
  /** Which agent ran it, so one observer can serve several. */
  agent: string;
  task: string;
  status: 'done' | 'error';
  /** The agent's answer, when it produced one. */
  result?: string;
  error?: string;
  updated_at_ms: number;
};

/**
 * Publish a delegated task's outcome. Best-effort: a missing `state` worker
 * costs the parent its wake-up, not the work — the run already happened, and
 * its events are on the claude::on-agent-event feed either way.
 */
export async function recordTaskOutcome(iii: IIIClient, outcome: TaskOutcome): Promise<void> {
  try {
    await iii.call({
      function_id: 'state::set',
      payload: { scope: TASK_SCOPE, key: outcome.session_id, value: outcome },
      timeoutMs: TIMEOUT_MS,
    });
  } catch (err) {
    console.warn(
      `claude-code: task outcome for ${outcome.session_id} was not published: ${String(err)}`,
    );
  }
}

/**
 * Who owns `sessionId` in session-manager. `child` is a session this worker
 * created (`metadata.agent: 'claude-code'`) — a run with `parent_session_id`
 * links one, and an orchestrator resumes it with `claude::start { session_id }`
 * alone. `other` is a session another agent owns (a harness chat driving
 * `claude::run` with its own id) or none at all: never written to. `unknown`
 * is a failed lookup, which the caller asks again before writing rather than
 * dropping the turn.
 */
export type Ownership = 'child' | 'other' | 'unknown';

export async function childSessionOwnership(iii: IIIClient, sessionId: string): Promise<Ownership> {
  try {
    const res = await iii.call<unknown, { meta?: { metadata?: Record<string, unknown> } } | null>({
      function_id: 'session::get',
      payload: { session_id: sessionId },
      timeoutMs: TIMEOUT_MS,
    });
    return res?.meta?.metadata?.agent === 'claude-code' ? 'child' : 'other';
  } catch (err) {
    console.warn(`claude-code: session::get failed for ${sessionId}: ${String(err)}`);
    return 'unknown';
  }
}

/** session-manager's StopReason; anything else fails the append. */
const STOP_REASONS = new Set(['end', 'length', 'function_call', 'aborted', 'error']);

function sessionStopReason(raw: unknown): string {
  const reason = String(raw ?? 'end');
  if (STOP_REASONS.has(reason)) return reason;
  // A `claude::stop` that ended the turn: stopped by request, not failed.
  if (reason === 'interrupted') return 'aborted';
  // SDK result subtypes: `error_max_turns`, `error_max_budget_usd`, … hit a
  // cap; every other subtype (`error_during_execution`) is a failure.
  return reason.startsWith('error_max') ? 'length' : 'error';
}

/**
 * Shape one transcript message for `session::append`. The feed's assistant
 * message carries the worker's usage (`input_tokens`, …) and the SDK's stop
 * subtype; session-manager wants its own `Usage` and `StopReason`, and rejects
 * an unknown stop reason outright — which would cut the transcript short.
 */
export function toSessionMessage(message: Record<string, unknown>): Record<string, unknown> {
  if (message.role !== 'assistant') return message;
  const usage = message.usage as Record<string, number | undefined> | null | undefined;
  return {
    ...message,
    stop_reason: sessionStopReason(message.stop_reason),
    usage: usage
      ? {
          input: usage.input_tokens ?? 0,
          output: usage.output_tokens ?? 0,
          cache_read: usage.cache_read_tokens ?? 0,
          cache_write: usage.cache_write_tokens ?? 0,
        }
      : null,
  };
}
