/**
 * `turn::assistant_streaming`. Stream one provider turn, persist the assistant
 * message, and route onward.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { AssistantMessage } from '../../types/agent-message.js';
import type { AgentFunction } from '../../types/function.js';
import { emit } from '../events.js';
import { finishSession } from '../finish.js';
import * as persistence from '../persistence.js';
import { runPreflight } from '../preflight.js';
import { buildInput, decide, targetFunctionId } from '../provider-router.js';
import { streamProviderTurn } from '../provider-stream.js';
import { runTransition } from '../run-transition.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { syntheticAssistant } from '../synthetic-assistant.js';

function isErrorOrAborted(asst: AssistantMessage): boolean {
  return asst.stop_reason === 'error' || asst.stop_reason === 'aborted';
}

/** Append the assistant message unless a re-entry already persisted it. */
async function persistAssistantOnce(
  iii: ISdk,
  rec: TurnStateRecord,
  asst: AssistantMessage,
): Promise<void> {
  const messages = await persistence.loadMessages(iii, rec.session_id);
  const last = messages[messages.length - 1];
  const dup =
    last &&
    last.role === 'assistant' &&
    last.timestamp === asst.timestamp &&
    last.model === asst.model &&
    last.provider === asst.provider;
  if (dup) {
    logger.warn('finalizeAssistant: skipping duplicate assistant push (re-entry detected)', {
      session_id: rec.session_id,
      timestamp: asst.timestamp,
    });
    return;
  }
  messages.push(asst);
  await persistence.saveMessages(iii, rec.session_id, messages);
}

async function finalizeAssistant(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const asst = rec.last_assistant;
  if (!asst) throw new Error('assistant_streaming finalize without last_assistant');

  await emit(iii, rec.session_id, {
    type: 'message_complete',
    message: asst,
    body_streamed: rec.assistant_body_streamed === true,
  });

  if (isErrorOrAborted(asst)) {
    await emit(iii, rec.session_id, { type: 'turn_end', message: asst, function_results: [] });
    rec.turn_end_emitted = true;
    await finishSession(iii, rec);
    return;
  }

  await persistAssistantOnce(iii, rec, asst);

  const hasCalls = asst.content.some((b) => b.type === 'function_call');
  if (!hasCalls) {
    transitionTo(rec, 'steering_check');
    return;
  }
  rec.function_results = [];
  rec.work = undefined; // function_execute builds the batch from last_assistant
  transitionTo(rec, 'function_execute');
}

export async function handleStreaming(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  rec.turn_count++;
  rec.turn_end_emitted = false;
  rec.assistant_body_streamed = false;

  const request = await persistence.loadRunRequest(iii, rec.session_id);
  let messages = await persistence.loadMessages(iii, rec.session_id);
  const { provider, model, system_prompt } = request;
  const tools = (
    Array.isArray(request.function_schemas) ? request.function_schemas : []
  ) as AgentFunction[];
  const decision = decide({ provider, model });

  if (
    (await runPreflight(iii, rec.session_id, messages, decision.provider, model)) === 'compacted'
  ) {
    messages = await persistence.loadMessages(iii, rec.session_id);
  }

  const { final, error } = await streamProviderTurn(iii, {
    session_id: rec.session_id,
    targetFn: targetFunctionId(decision),
    buildInput: (writerRef) => buildInput(decision, writerRef, system_prompt, messages, tools),
    onDelta: async (partial, event) => {
      await emit(iii, rec.session_id, {
        type: 'message_update',
        message: partial,
        llm_event: event,
      });
      if (event.type === 'text_delta' || event.type === 'thinking_delta') {
        rec.assistant_body_streamed = true;
      }
    },
  });

  if (final) {
    rec.last_assistant = final;
  } else {
    const reason = error ?? 'provider channel closed without final';
    const synthetic = syntheticAssistant({
      stop_reason: 'error',
      text: reason,
      provider: decision.provider,
      model: decision.model,
    });
    await emit(iii, rec.session_id, {
      type: 'message_update',
      message: synthetic,
      llm_event: { type: 'text_delta', partial: synthetic, delta: reason },
    });
    rec.last_assistant = synthetic;
  }
  await finalizeAssistant(iii, rec);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::assistant_streaming',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'assistant_streaming', handleStreaming, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state assistant_streaming: start turn, stream provider response, finalize, and route onward.',
    },
  );
}
