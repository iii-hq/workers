/**
 * `turn::assistant_finished`. Persist assistant message and route to steering or function execute.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AgentEvent } from '../../types/agent-event.js';
import type { AssistantMessage } from '../../types/agent-message.js';
import type { FunctionCall } from '../../types/function.js';
import { TOOL_NAME } from '../agent-trigger.js';
import { emit } from '../events.js';
import type { PreparedEntry } from '../persistence.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import {
  TurnStepPayloadSchema,
  type TurnStepPayload,
  type TurnStepResult,
  staleSkipResult,
} from '../turn-step-payload.js';

function unwrapAgentTrigger(fc: FunctionCall): FunctionCall {
  if (fc.function_id !== TOOL_NAME) return fc;
  const args = (fc.arguments ?? {}) as Record<string, unknown>;
  const fn = typeof args.function === 'string' ? args.function : '';
  const payload = args.payload ?? {};
  return { id: fc.id, function_id: fn, arguments: payload };
}

function extractFunctionCalls(msg: AssistantMessage): FunctionCall[] {
  const out: FunctionCall[] = [];
  for (const b of msg.content) {
    if (b.type === 'function_call') {
      out.push({ id: b.id, function_id: b.function_id, arguments: b.arguments });
    }
  }
  return out;
}

function assistantLifecycleEvents(asst: AssistantMessage): AgentEvent[] {
  return [
    { type: 'message_start', message: asst },
    { type: 'message_end', message: asst },
  ];
}

export async function handleFinished(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const asst = rec.last_assistant;
  if (!asst) {
    throw new Error('assistant_finished without last_assistant');
  }
  for (const evt of assistantLifecycleEvents(asst)) {
    await emit(iii, rec.session_id, evt);
  }
  const isErrorOrAborted = asst.stop_reason === 'error' || asst.stop_reason === 'aborted';
  if (!isErrorOrAborted) {
    const messages = await persistence.loadMessages(iii, rec.session_id);
    messages.push(asst);
    await persistence.saveMessages(iii, rec.session_id, messages);
  }

  if (isErrorOrAborted) {
    await emit(iii, rec.session_id, {
      type: 'turn_end',
      message: asst,
      function_results: [],
    });
    rec.turn_end_emitted = true;
    transitionTo(rec, 'tearing_down');
    return;
  }
  const calls = extractFunctionCalls(asst);
  if (calls.length === 0) {
    transitionTo(rec, 'steering_check');
    return;
  }

  rec.function_results = [];
  rec.pending_function_calls = calls.map(unwrapAgentTrigger);

  const prepared: PreparedEntry[] = rec.pending_function_calls.map((fc) => ({
    function_call: fc,
    blocked: null,
  }));

  await persistence.saveExecutedCalls(iii, rec.session_id, []);
  await persistence.savePreparedCalls(iii, rec.session_id, prepared);
  transitionTo(rec, 'function_execute');
}

export async function execute(iii: ISdk, payload: TurnStepPayload): Promise<TurnStepResult> {
  const rec = await persistence.loadRecord(iii, payload.session_id);
  if (!rec) {
    throw new Error(`turn::assistant_finished invariant: missing session ${payload.session_id}`);
  }
  const skipped = staleSkipResult('assistant_finished', rec);
  if (skipped) return skipped;

  const from_state = rec.state;
  try {
    await handleFinished(iii, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec);
  return { ok: true, from_state, to_state: rec.state };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::assistant_finished',
    async (payload: unknown) => execute(iii, TurnStepPayloadSchema.parse(payload)),
    {
      description:
        'Run one durable FSM transition for session in state assistant_finished: finalize assistant and route onward.',
    },
  );
}
