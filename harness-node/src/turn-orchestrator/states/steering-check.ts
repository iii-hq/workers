/**
 * `turn::steering_check`. Drains steering / followup inboxes and the abort flag, then routes onward.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AgentMessage, AssistantMessage } from '../../types/agent-message.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, abortSignalKey, transitionTo } from '../state.js';
import {
  TurnStepPayloadSchema,
  type TurnStepPayload,
  type TurnStepResult,
  staleSkipResult,
} from '../turn-step-payload.js';

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
    case 'steering':
    case 'followup': {
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

export async function execute(iii: ISdk, payload: TurnStepPayload): Promise<TurnStepResult> {
  const rec = await persistence.loadRecord(iii, payload.session_id);
  if (!rec) {
    throw new Error(`turn::steering_check invariant: missing session ${payload.session_id}`);
  }
  const skipped = staleSkipResult('steering_check', rec);
  if (skipped) return skipped;

  const from_state = rec.state;
  try {
    await handleSteering(iii, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec);
  return { ok: true, from_state, to_state: rec.state };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::steering_check',
    async (payload: unknown) => execute(iii, TurnStepPayloadSchema.parse(payload)),
    {
      description:
        'Run one durable FSM transition for session in state steering_check: drain inboxes and route onward.',
    },
  );
}
