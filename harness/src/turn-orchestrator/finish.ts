/**
 * Terminal teardown: emit the final `agent_end` with the full transcript and
 * stop the session. Called inline by the FSM paths that end a turn (replaces
 * the former standalone `tearing_down` state).
 */

import type { ISdk } from '../runtime/iii.js';
import { emit } from './events.js';
import * as persistence from './persistence.js';
import { type TurnStateRecord, transitionTo } from './state.js';

export async function finishSession(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const messages = await persistence.loadMessages(iii, rec.session_id);
  await emit(iii, rec.session_id, { type: 'agent_end', messages });
  transitionTo(rec, 'stopped');
}
