/**
 * The child-session link that makes a run a SUB-AGENT rather than a function
 * call that happened to return text.
 *
 * A harness sub-agent is not a special mechanism: it is a session whose
 * metadata names its parent, a transcript the console can read, and a status
 * that moves. Any worker that does those three things shows up in the console's
 * session tree and in the parent turn's chat — no harness change, no special
 * case for this worker (see `console/web/src/components/chat/agent-session`).
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
  return line.length > 60 ? `${line.slice(0, 59)}…` : line || 'pi task';
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
  const metadata: Record<string, unknown> = { agent: 'pi' };
  if (options.parentSessionId) metadata.parent_session_id = options.parentSessionId;
  try {
    await iii.trigger({
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
    console.warn(`pi: ${detail}`);
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
    await iii.trigger({
      function_id: 'session::set-status',
      payload: { session_id: sessionId, status, ...(reason ? { reason } : {}) },
      timeoutMs: TIMEOUT_MS,
    });
  } catch (err) {
    console.warn(`pi: session::set-status ${status} failed for ${sessionId}: ${String(err)}`);
  }
}

/**
 * The scope a delegated task's outcome lands in. An orchestrator binds a
 * `state` trigger on `{ scope: TASK_SCOPE, key: <child session id> }` and is
 * WOKEN when the task settles — no polling, no blocking call held open for the
 * length of an agent run. It is the same shape a harness sub-agent uses: the
 * child writes where it was told to write, and the parent reacts to that write.
 */
export const TASK_SCOPE = 'agent_tasks';

export type TaskOutcome = {
  session_id: string;
  parent_session_id?: string;
  /** Which agent ran it, so one binding can serve several. */
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
 * its events are on the stream either way.
 */
export async function recordTaskOutcome(iii: IIIClient, outcome: TaskOutcome): Promise<void> {
  try {
    await iii.trigger({
      function_id: 'state::set',
      payload: { scope: TASK_SCOPE, key: outcome.session_id, value: outcome },
      timeoutMs: TIMEOUT_MS,
    });
  } catch (err) {
    console.warn(`pi: task outcome for ${outcome.session_id} was not published: ${String(err)}`);
  }
}
