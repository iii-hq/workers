/**
 * `turn::steering_check`. Drains steering / followup inboxes and the abort flag, then routes onward.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import { type AgentMessage, emptyAssistant } from '../../types/agent-message.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { runTransition } from '../run-transition.js';
import { AGENT_SCOPE, type TurnStateRecord, abortSignalKey, transitionTo } from '../state.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { syntheticAssistant } from '../synthetic-assistant.js';

export type SteeringRoute =
  | 'abort'
  | 'steering'
  | 'followup'
  | 'continue_after_function'
  | 'end_turn';

/** Pure priority router — no I/O. */
export function route(
  abort: boolean,
  has_steering: boolean,
  has_followup: boolean,
  has_function_results: boolean,
): SteeringRoute {
  if (abort) return 'abort';
  if (has_steering) return 'steering';
  if (has_followup) return 'followup';
  if (has_function_results) return 'continue_after_function';
  return 'end_turn';
}

async function abortSet(iii: ISdk, session_id: string): Promise<boolean> {
  try {
    const v = await iii.trigger<unknown, unknown>({
      function_id: 'state::get',
      payload: { scope: AGENT_SCOPE, key: abortSignalKey(session_id) },
    });
    return v === true;
  } catch {
    return false;
  }
}

async function drainQueue(iii: ISdk, name: string, session_id: string): Promise<AgentMessage[]> {
  try {
    const resp = await iii.trigger<unknown, { items?: unknown }>({
      function_id: 'session-inbox::drain',
      payload: { name, session_id },
    });
    if (Array.isArray(resp?.items)) return resp.items as AgentMessage[];
  } catch {
    // ignore
  }
  return [];
}

function maxTurnsReached(rec: TurnStateRecord): boolean {
  return rec.max_turns !== undefined && rec.turn_count >= rec.max_turns;
}

async function endForMaxTurns(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const msg = syntheticAssistant({
    stop_reason: 'end',
    text: `loop stopped: max_turns (${rec.max_turns ?? 0}) reached`,
  });
  rec.last_assistant = msg;
  const messages = await persistence.loadMessages(iii, rec.session_id);
  messages.push(msg);
  await persistence.saveMessages(iii, rec.session_id, messages);
  await emit(iii, rec.session_id, { type: 'message_complete', message: msg, body_streamed: false });
  await emit(iii, rec.session_id, { type: 'turn_end', message: msg, function_results: [] });
  rec.turn_end_emitted = true;
  transitionTo(rec, 'tearing_down');
}

async function emitTurnEndOnce(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  if (rec.turn_end_emitted) return;
  const last = rec.last_assistant ?? emptyAssistant();
  await emit(iii, rec.session_id, {
    type: 'turn_end',
    message: last,
    function_results: [],
  });
  rec.turn_end_emitted = true;
}

export async function handleSteering(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const abort = await abortSet(iii, rec.session_id);
  const steering = abort ? [] : await drainQueue(iii, 'steering', rec.session_id);
  const followup =
    abort || steering.length > 0 ? [] : await drainQueue(iii, 'followup', rec.session_id);

  const decision = route(
    abort,
    steering.length > 0,
    followup.length > 0,
    rec.function_results.length > 0,
  );
  switch (decision) {
    case 'abort': {
      const aborted = syntheticAssistant({
        stop_reason: 'aborted',
        provider: 'harness',
        model: 'harness',
      });
      const messages = await persistence.loadMessages(iii, rec.session_id);
      messages.push(aborted);
      await persistence.saveMessages(iii, rec.session_id, messages);
      rec.last_assistant = aborted;
      if (!rec.turn_end_emitted) {
        await emit(iii, rec.session_id, {
          type: 'turn_end',
          message: aborted,
          function_results: [],
        });
        rec.turn_end_emitted = true;
      }
      transitionTo(rec, 'tearing_down');
      break;
    }
    case 'steering':
    case 'followup': {
      if (maxTurnsReached(rec)) {
        await endForMaxTurns(iii, rec);
        break;
      }
      const inbox = decision === 'steering' ? steering : followup;
      await emitTurnEndOnce(iii, rec);
      const messages = await persistence.loadMessages(iii, rec.session_id);
      messages.push(...inbox);
      await persistence.saveMessages(iii, rec.session_id, messages);
      rec.function_results = [];
      transitionTo(rec, 'assistant_streaming');
      break;
    }
    case 'continue_after_function': {
      if (maxTurnsReached(rec)) {
        await endForMaxTurns(iii, rec);
        break;
      }
      rec.function_results = [];
      transitionTo(rec, 'assistant_streaming');
      break;
    }
    case 'end_turn': {
      await emitTurnEndOnce(iii, rec);
      transitionTo(rec, 'tearing_down');
      break;
    }
  }
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::steering_check',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'steering_check', handleSteering, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state steering_check: drain inboxes and route onward.',
    },
  );
}
