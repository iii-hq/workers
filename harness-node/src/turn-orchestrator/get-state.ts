/**
 * `turn::get_state` — one-shot reader for a session's current
 * turn_state record. Exists so UI clients can recover pending modals
 * after a page reload without reaching into iii state from the
 * browser. The orchestrator owns the turn_state schema and key
 * layout; clients call this and get back the record (or null for
 * an unknown session).
 */

import { requireString } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';
import * as persistence from './persistence.js';
import type { TurnStateRecord } from './state.js';

export async function execute(iii: ISdk, payload: unknown): Promise<TurnStateRecord | null> {
  const obj = (payload ?? {}) as Record<string, unknown>;
  const session_id = requireString(obj, 'session_id');
  return persistence.loadRecord(iii, session_id);
}

export function register(iii: ISdk): void {
  iii.registerFunction('turn::get_state', async (payload: unknown) => execute(iii, payload), {
    description:
      'Read the current turn_state record for a session. Returns null if the session is unknown. UI clients use this on page reload to recover any in-progress modals (e.g. function_awaiting_approval) without reading iii state directly.',
  });
}
