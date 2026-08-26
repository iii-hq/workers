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
    await iii.trigger({
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
