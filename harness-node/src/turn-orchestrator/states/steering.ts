/**
 * `steering_check`. Drains the steering / followup inbox queues and the
 * abort flag, then routes onward. Mirrors
 * `turn-orchestrator/src/states/steering.rs`.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AgentMessage, AssistantMessage } from '../../types/agent-message.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, abortSignalKey, transitionTo } from '../state.js';

export type SteeringRoute =
  | 'abort'
  | 'steering'
  | 'followup'
  | 'continue_after_function'
  | 'end_turn';

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
      payload: { scope: 'agent', key: abortSignalKey(session_id) },
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

function abortedMessage(): AssistantMessage {
  return {
    role: 'assistant',
    content: [],
    stop_reason: 'aborted',
    error_message: 'aborted',
    error_kind: 'transient',
    usage: null,
    model: 'harness',
    provider: 'harness',
    timestamp: Date.now(),
  };
}

async function emitTurnEndOnce(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  if (rec.turn_end_emitted) return;
  const last =
    rec.last_assistant ??
    ({
      role: 'assistant',
      content: [],
      stop_reason: 'end',
      error_message: null,
      error_kind: null,
      usage: null,
      model: '',
      provider: '',
      timestamp: Date.now(),
    } as AssistantMessage);
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
      const aborted = abortedMessage();
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
    case 'steering': {
      await emitTurnEndOnce(iii, rec);
      const messages = await persistence.loadMessages(iii, rec.session_id);
      messages.push(...steering);
      await persistence.saveMessages(iii, rec.session_id, messages);
      rec.function_results = [];
      transitionTo(rec, 'awaiting_assistant');
      break;
    }
    case 'followup': {
      await emitTurnEndOnce(iii, rec);
      const messages = await persistence.loadMessages(iii, rec.session_id);
      messages.push(...followup);
      await persistence.saveMessages(iii, rec.session_id, messages);
      rec.function_results = [];
      transitionTo(rec, 'awaiting_assistant');
      break;
    }
    case 'continue_after_function': {
      // function_finalize already emitted TurnEnd; just move on.
      rec.function_results = [];
      transitionTo(rec, 'awaiting_assistant');
      break;
    }
    case 'end_turn': {
      await emitTurnEndOnce(iii, rec);
      transitionTo(rec, 'tearing_down');
      break;
    }
  }
}
