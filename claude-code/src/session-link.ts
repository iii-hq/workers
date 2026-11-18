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
): Promise<{ linked: boolean; detail: string }> {
  const metadata: Record<string, unknown> = { agent: 'claude-code' };
  if (options.parentSessionId) metadata.parent_session_id = options.parentSessionId;
  try {
    await iii.call({
      function_id: 'session::ensure',
      payload: {
        session_id: options.sessionId,
        title: title(options.task),
        metadata,
      },
      timeoutMs: TIMEOUT_MS,
    });
    return { linked: true, detail: '' };
  } catch (err) {
    // A missing session-manager costs the console its session tree, not the run.
    const detail = `session::ensure failed for ${options.sessionId}: ${String(err)}`;
    console.warn(`claude-code: ${detail}`);
    return { linked: false, detail };
  }
}

/** Move the session's coarse status, so a tree shows working / done / error. */
export async function setSessionStatus(
  iii: IIIClient,
  sessionId: string,
  status: 'working' | 'done' | 'error',
  reason?: string,
): Promise<void> {
  try {
    await iii.call({
      function_id: 'session::set-status',
      payload: { session_id: sessionId, status, ...(reason ? { reason } : {}) },
      timeoutMs: TIMEOUT_MS,
    });
  } catch (err) {
    console.warn(
      `claude-code: session::set-status ${status} failed for ${sessionId}: ${String(err)}`,
    );
  }
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
 * Persist one message into the child session's transcript.
 *
 * The `claude::on-agent-event` feed is a live tape: a console window opened after the run has
 * nothing to replay from it, which is why a finished sub-agent used to render
 * blank with `message_count: 0`. The session manager is the durable side, and
 * the console reads it with `session::messages`. Best-effort, like the rest of
 * this module: a missing session manager costs the replay, not the run.
 */
export async function appendMessage(
  iii: IIIClient,
  sessionId: string,
  message: unknown,
): Promise<boolean> {
  try {
    await iii.call({
      function_id: 'session::append',
      payload: { session_id: sessionId, message },
      timeoutMs: TIMEOUT_MS,
    });
    return true;
  } catch (err) {
    console.warn(`claude-code: session::append failed for ${sessionId}: ${String(err)}`);
    return false;
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

/**
 * Write a finished turn into the child session: what was asked, then every
 * message the turn produced, in order. Called once per turn, before the
 * session reads `done` — the live view is the claude::on-agent-event feed,
 * this is what a window opened later replays. `terminalStop` is how the turn
 * ended (the SDK result subtype); it lands on the LAST assistant message only,
 * so a turn that hit `error_max_turns` after an assistant reply does not read
 * as a clean `end`. Earlier messages keep their own reason.
 */
export async function appendTranscript(
  iii: IIIClient,
  sessionId: string,
  task: string,
  messages: unknown[],
  terminalStop?: string,
): Promise<void> {
  const asked = {
    role: 'user',
    content: [{ type: 'text', text: task }],
    timestamp: Date.now(),
  };
  if (!(await appendMessage(iii, sessionId, asked))) return;
  const records = messages as Array<Record<string, unknown>>;
  let last = -1;
  records.forEach((message, index) => {
    if (message.role === 'assistant') last = index;
  });
  for (const [index, message] of records.entries()) {
    const ended =
      index === last && terminalStop && terminalStop !== 'end'
        ? { ...message, stop_reason: terminalStop }
        : message;
    if (!(await appendMessage(iii, sessionId, toSessionMessage(ended)))) return;
  }
}
