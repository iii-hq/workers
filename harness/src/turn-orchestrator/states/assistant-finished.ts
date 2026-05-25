/**
 * `turn::assistant_finished`. Persist assistant message and route to steering or function execute.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { AgentEvent } from '../../types/agent-event.js';
import type { AssistantMessage } from '../../types/agent-message.js';
import type { FunctionCall } from '../../types/function.js';
import { missingFunctionResult, unwrapAgentTrigger } from '../agent-trigger.js';
import { emit } from '../events.js';
import type { PreparedEntry } from '../persistence.js';
import * as persistence from '../persistence.js';
import { runTransition } from '../run-transition.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';

function extractFunctionCalls(msg: AssistantMessage): FunctionCall[] {
  const out: FunctionCall[] = [];
  for (const b of msg.content) {
    if (b.type === 'function_call') {
      out.push({ id: b.id, function_id: b.function_id, arguments: b.arguments });
    }
  }
  return out;
}

function assistantMessageComplete(asst: AssistantMessage, body_streamed: boolean): AgentEvent {
  return { type: 'message_complete', message: asst, body_streamed };
}

export async function handleFinished(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const asst = rec.last_assistant;
  if (!asst) {
    throw new Error('assistant_finished without last_assistant');
  }
  await emit(
    iii,
    rec.session_id,
    assistantMessageComplete(asst, rec.assistant_body_streamed === true),
  );
  const isErrorOrAborted = asst.stop_reason === 'error' || asst.stop_reason === 'aborted';
  // Error/aborted assistant messages (e.g. provider auth failures,
  // network blips, user aborts) are surfaced to the UI via the
  // message_complete emitted above, but we deliberately
  // keep them out of the session's persisted message history so the
  // LLM's next-turn context doesn't accumulate transient infra noise.
  if (!isErrorOrAborted) {
    const messages = await persistence.loadMessages(iii, rec.session_id);
    // Idempotency guard: handleFinished can re-enter (durable trigger
    // retry, crash before transitionTo persists). Without this guard a
    // second run pushes the SAME assistant message again. If that
    // assistant has tool_calls, Anthropic rejects the next request with:
    //   "each tool_use must have a unique id".
    // Detect by comparing timestamp + content shape against the last
    // assistant message in flat-state; skip the push when they match.
    const last = messages[messages.length - 1];
    const alreadyPersisted =
      last &&
      last.role === 'assistant' &&
      last.timestamp === asst.timestamp &&
      last.model === asst.model &&
      last.provider === asst.provider;
    if (alreadyPersisted) {
      logger.warn('handleFinished: skipping duplicate assistant push (re-entry detected)', {
        session_id: rec.session_id,
        timestamp: asst.timestamp,
      });
    } else {
      messages.push(asst);
      await persistence.saveMessages(iii, rec.session_id, messages);
    }
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

  const prepared: PreparedEntry[] = calls.map((raw) => {
    const function_call = unwrapAgentTrigger(raw);
    if (!function_call.function_id) {
      return { function_call, blocked: missingFunctionResult() };
    }
    return { function_call, blocked: null };
  });

  await persistence.saveExecutedCalls(iii, rec.session_id, []);
  await persistence.savePreparedCalls(iii, rec.session_id, prepared);
  transitionTo(rec, 'function_execute');
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::assistant_finished',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'assistant_finished', handleFinished, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state assistant_finished: finalize assistant and route onward.',
    },
  );
}
