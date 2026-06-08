/**
 * Run one steering_check transition: feed function results back to the model,
 * or end the turn.
 *
 * The turn_state record is persisted atomically by the shared runTransition
 * runner, so this only mutates `rec` and emits the things that write can't
 * carry — the live-stream events (turn_end / agent_end / message_complete) and
 * the session-tree history append.
 */

import type { ISdk } from '../../runtime/iii.js';
import { emptyAssistant } from '../../types/agent-message.js';
import { emit } from '../events.js';
import { createTurnStore } from '../state-runtime/store.js';
import { transitionTo, type SteeringCheckTurnRecord } from '../state.js';
import { syntheticAssistant } from '../synthetic-assistant.js';

function maxTurnsReached(rec: SteeringCheckTurnRecord): boolean {
  return rec.max_turns !== undefined && rec.turn_count >= rec.max_turns;
}

export async function runSteeringCheck(iii: ISdk, rec: SteeringCheckTurnRecord): Promise<void> {
  // Function results under the turn cap: feed them back to the model. Pure state
  // update — runTransition persists it and wakes turn::assistant_streaming.
  if (rec.function_results.length > 0 && !maxTurnsReached(rec)) {
    rec.function_results = [];
    transitionTo(rec, 'assistant_streaming');
    return;
  }

  // Function results but the turn cap is hit: append a synthetic notice so the
  // user sees why the loop stopped, then fall through to teardown.
  if (rec.function_results.length > 0) {
    const msg = syntheticAssistant({
      stop_reason: 'end',
      text: `loop stopped: max_turns (${rec.max_turns ?? 0}) reached`,
    });
    rec.last_assistant = msg;
    await createTurnStore(iii).appendMessages(rec.session_id, [msg]);
    await emit(iii, rec.session_id, {
      type: 'message_complete',
      message: msg,
      body_streamed: false,
    });
  }

  // Terminal: emit turn_end once, then agent_end (a signal carrying no
  // transcript) and settle in `stopped`.
  if (!rec.turn_end_emitted) {
    await emit(iii, rec.session_id, {
      type: 'turn_end',
      message: rec.last_assistant ?? emptyAssistant(),
      function_results: [],
    });
    rec.turn_end_emitted = true;
  }
  await emit(iii, rec.session_id, { type: 'agent_end', messages: [] });
  transitionTo(rec, 'stopped');
}
