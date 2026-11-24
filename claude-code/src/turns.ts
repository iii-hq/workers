/**
 * Turn settles on engine state. Scope `claude_turns`, key = the session id:
 * every Claude Code turn this worker sees end (a headless run, or a session
 * whose iii plugin hooks reach it) writes one record there, so a caller
 * observes `state::on-changed` once and is woken when a turn settles, with no
 * polling.
 */

import type { IIIClient } from 'iii-sdk';
import { RESTART_REASON, startupRetry } from './session-link.js';
import { listSessions, loadSession, saveSession } from './state.js';
import type { SessionRecord } from './types.js';

export const TURN_SCOPE = 'claude_turns';

export type TurnSettle = {
  /** iii session_id for headless; Claude Code session id for hook sessions. */
  id: string;
  source: 'run' | 'hook' | 'boot';
  status: 'completed' | 'stopped' | 'error' | 'gone';
  turn_id?: string;
  claude_session_id?: string;
  stop_reason?: string;
  /** Final assistant text when known. */
  result?: string;
  error?: string;
  /** Stop hook: >0 means paused on background work, not finished. */
  background_tasks?: number;
  settled_at_ms: number;
};

/**
 * Publish a settle. Best-effort: a missing `state` worker costs the observer
 * its wake-up, never the turn.
 */
export async function recordTurn(iii: IIIClient, settle: TurnSettle): Promise<void> {
  try {
    await iii.call({
      function_id: 'state::set',
      payload: { scope: TURN_SCOPE, key: settle.id, value: settle },
      timeoutMs: 15_000,
    });
  } catch (err) {
    console.warn(`claude-code: turn settle for ${settle.id} was not published: ${String(err)}`);
  }
}

/**
 * Claude session ids of this worker's headless runs. They load the same
 * plugin, so their hooks reach the activity sink too; their settle comes from
 * executeRun instead.
 */
const headless = new Set<string>();
const HEADLESS_CAP = 1000;

export function markHeadless(id: string): void {
  headless.delete(id);
  headless.add(id);
  if (headless.size > HEADLESS_CAP) headless.delete(headless.values().next().value as string);
}

export function isHeadless(id: string): boolean {
  return headless.has(id);
}

/**
 * At startup, a run record still `working` belongs to a turn that died with
 * the previous process: save it as `error` and settle it, so an observer
 * waiting on that turn is woken. Each record is re-read just before it is
 * written: a run saves its final status before releasing its slot, so one
 * still `working` and not `ranHere` (live) belongs to no run of this process.
 * The listing is retried at the `startupRetry` delays while it fails; `null`
 * when every attempt failed.
 */
export async function settleStaleTurns(
  iii: IIIClient,
  ranHere: (sessionId: string) => boolean,
): Promise<number | null> {
  for (const delay of [0, ...startupRetry.delaysMs]) {
    if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
    let records: SessionRecord[];
    try {
      records = await listSessions(iii);
    } catch (err) {
      console.warn(`claude-code: could not list runs left working: ${String(err)}`);
      continue;
    }
    let settled = 0;
    for (const listed of records) {
      if (listed.status !== 'working') continue;
      // A turn may have started or ended here since the listing.
      let record: SessionRecord | null;
      try {
        record = await loadSession(iii, listed.session_id);
      } catch (err) {
        console.warn(`claude-code: could not re-read run ${listed.session_id}: ${String(err)}`);
        continue;
      }
      if (record?.status !== 'working' || ranHere(record.session_id)) continue;
      try {
        await saveSession(iii, { ...record, status: 'error', updated_at_ms: Date.now() });
      } catch (err) {
        console.warn(`claude-code: run ${record.session_id} still shows working: ${String(err)}`);
      }
      await recordTurn(iii, {
        id: record.session_id,
        source: 'boot',
        status: 'error',
        stop_reason: 'worker_restart',
        ...(record.turn_id ? { turn_id: record.turn_id } : {}),
        ...(record.claude_session_id ? { claude_session_id: record.claude_session_id } : {}),
        error: RESTART_REASON,
        settled_at_ms: Date.now(),
      });
      settled++;
    }
    if (settled > 0) console.info(`claude-code: settled ${settled} turn(s) left working`);
    return settled;
  }
  return null;
}
