/**
 * Stream one provider turn, persist the assistant message, and route onward.
 */

import type { AssistantMessage } from '../../types/agent-message.js';
import { decide } from '../provider-router.js';
import { syntheticAssistant } from '../synthetic-assistant.js';
import { emitTurnEndOnce } from '../state-runtime/turn-end.js';
import { enterFunctionExecute } from '../function-execute/run.js';
import { transitionTo, type AssistantStreamingTurnRecord } from '../state.js';
import {
  hasFunctionCalls,
  isErrorOrAborted,
  parseFunctionSchemas,
  type AssistantRoute,
  type AssistantStreamingPorts,
  type StreamContext,
  type StreamTurnOutcome,
} from './ports.js';

export function beginTurn(rec: AssistantStreamingTurnRecord): void {
  rec.turn_count++;
  rec.turn_end_emitted = false;
  rec.assistant_body_streamed = false;
}

export async function prepareStreamContext(
  ports: AssistantStreamingPorts,
  rec: AssistantStreamingTurnRecord,
): Promise<StreamContext> {
  const request = await ports.loadRunRequest(rec.session_id);
  let messages = await ports.loadMessages(rec.session_id);
  const { provider, model, system_prompt, function_schemas } = request;
  const decision = decide({ provider, model });
  const tools = parseFunctionSchemas(function_schemas);

  if (
    (await ports.runPreflight(rec.session_id, messages, decision.provider, model)) === 'compacted'
  ) {
    messages = await ports.loadMessages(rec.session_id);
  }

  return {
    session_id: rec.session_id,
    decision,
    system_prompt,
    tools,
    messages,
  };
}

export async function runStreamTurn(
  ports: AssistantStreamingPorts,
  session_id: string,
  ctx: StreamContext,
): Promise<StreamTurnOutcome> {
  let body_streamed = false;

  const { final, error } = await ports.streamTurn(ctx, async (partial, event) => {
    await ports.emitMessageUpdate(session_id, partial, event);
    if (event.type === 'text_delta' || event.type === 'thinking_delta') {
      body_streamed = true;
    }
  });

  return { final, error, body_streamed };
}

export function resolveAssistantMessage(
  outcome: StreamTurnOutcome,
  decision: StreamContext['decision'],
): AssistantMessage {
  if (outcome.final) return outcome.final;

  const reason = outcome.error ?? 'provider channel closed without final';
  return syntheticAssistant({
    stop_reason: 'error',
    text: reason,
    provider: decision.provider,
    model: decision.model,
  });
}

/** Reason text for a synthetic error update when the provider did not return a final message. */
export function syntheticStreamReason(outcome: StreamTurnOutcome): string | null {
  if (outcome.final) return null;
  return outcome.error ?? 'provider channel closed without final';
}

export function routeAssistantTurn(asst: AssistantMessage): AssistantRoute {
  if (isErrorOrAborted(asst)) {
    return {
      kind: 'stopped',
      reason: asst.stop_reason === 'aborted' ? 'aborted' : 'error',
    };
  }
  if (hasFunctionCalls(asst)) {
    return { kind: 'function_execute' };
  }
  return { kind: 'steering_check' };
}

export async function finalizeAssistantTurn(
  ports: AssistantStreamingPorts,
  rec: AssistantStreamingTurnRecord,
  asst: AssistantMessage,
): Promise<void> {
  await ports.emitMessageComplete(rec.session_id, asst, rec.assistant_body_streamed === true);

  const route = routeAssistantTurn(asst);

  if (route.kind === 'stopped') {
    await emitTurnEndOnce(ports, rec, asst);
    await ports.finishSession(rec);
    return;
  }

  await ports.persistAssistantIfNew(rec.session_id, asst);

  if (route.kind === 'function_execute') {
    rec.function_results = [];
    enterFunctionExecute(rec, asst);
    transitionTo(rec, 'function_execute');
    return;
  }

  transitionTo(rec, 'steering_check');
}

export async function runAssistantStreaming(
  ports: AssistantStreamingPorts,
  rec: AssistantStreamingTurnRecord,
): Promise<void> {
  beginTurn(rec);
  const ctx = await prepareStreamContext(ports, rec);
  const outcome = await runStreamTurn(ports, rec.session_id, ctx);
  const asst = resolveAssistantMessage(outcome, ctx.decision);
  rec.last_assistant = asst;
  rec.assistant_body_streamed = outcome.body_streamed;

  const syntheticReason = syntheticStreamReason(outcome);
  if (syntheticReason) {
    await ports.emitMessageUpdate(rec.session_id, asst, {
      type: 'text_delta',
      partial: asst,
      delta: syntheticReason,
    });
  }

  await finalizeAssistantTurn(ports, rec, asst);
}
