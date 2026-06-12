/**
 * Stream one provider turn, persist the assistant message, and route onward.
 */

import type { AgentMessage, AssistantMessage } from '../../types/agent-message.js';
import { syntheticAssistant } from '../synthetic-assistant.js';
import { emitTurnEndOnce, transitionToFinishing } from '../state-runtime/turn-end.js';
import { enterFunctionExecute } from '../function-execute/run.js';
import { transitionTo, type AssistantStreamingTurnRecord } from '../state.js';
import { createDeltaCoalescer } from './coalesce-deltas.js';
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
  const { provider, model, system_prompt, function_schemas, thinking_level } = request;
  // Provisioning pinned the routed provider; fall back to the raw request
  // provider for records provisioned before the router cutover.
  const routedProvider = request.routed_provider ?? provider;
  const tools = parseFunctionSchemas(function_schemas);
  const model_meta = rec.model_meta;

  if (
    (await ports.runPreflight(
      rec.session_id,
      messages,
      routedProvider || provider,
      model,
      model_meta,
    )) === 'compacted'
  ) {
    messages = await ports.loadMessages(rec.session_id);
  }

  return {
    session_id: rec.session_id,
    provider: routedProvider,
    model,
    system_prompt,
    tools,
    messages,
    ...(thinking_level ? { thinking_level } : {}),
    request_id: `${rec.session_id}:${rec.started_at_ms}`,
  };
}

export async function runStreamTurn(
  ports: AssistantStreamingPorts,
  session_id: string,
  ctx: StreamContext,
): Promise<StreamTurnOutcome> {
  let body_streamed = false;

  const coalescer = createDeltaCoalescer((partial, event) =>
    ports.emitMessageUpdate(session_id, partial, event),
  );

  const { final, error } = await ports.streamTurn(ctx, async (partial, event) => {
    if (event.type === 'text_delta' || event.type === 'thinking_delta') {
      body_streamed = true;
    }
    await coalescer.onEvent(partial, event);
  });
  await coalescer.flush();

  return { final, error, body_streamed };
}

export function resolveAssistantMessage(
  outcome: StreamTurnOutcome,
  ctx: Pick<StreamContext, 'provider' | 'model'>,
): AssistantMessage {
  if (outcome.final) return outcome.final;

  // Defense-in-depth behind the router's terminal-frame guarantee: the
  // router itself can die mid-relay, so a close-without-terminal still
  // synthesizes a visible error.
  const reason = outcome.error ?? 'provider channel closed without final';
  return syntheticAssistant({
    stop_reason: 'error',
    text: reason,
    provider: ctx.provider,
    model: ctx.model,
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
  return { kind: 'end_turn' };
}

export async function finalizeAssistantTurn(
  ports: AssistantStreamingPorts,
  rec: AssistantStreamingTurnRecord,
  asst: AssistantMessage,
  messages: AgentMessage[],
): Promise<void> {
  await ports.emitMessageComplete(rec.session_id, asst, rec.assistant_body_streamed === true);

  const route = routeAssistantTurn(asst);

  if (route.kind === 'stopped') {
    await emitTurnEndOnce(ports, rec, asst);
    transitionToFinishing(rec);
    return;
  }

  await ports.persistAssistantIfNew(rec.session_id, asst, messages);

  if (route.kind === 'function_execute') {
    rec.function_results = [];
    enterFunctionExecute(rec, asst);
    transitionTo(rec, 'function_execute');
    return;
  }

  await emitTurnEndOnce(ports, rec, asst);
  transitionToFinishing(rec);
}

export async function runAssistantStreaming(
  ports: AssistantStreamingPorts,
  rec: AssistantStreamingTurnRecord,
): Promise<void> {
  beginTurn(rec);
  const ctx = await prepareStreamContext(ports, rec);
  const outcome = await runStreamTurn(ports, rec.session_id, ctx);
  const asst = resolveAssistantMessage(outcome, ctx);
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

  await finalizeAssistantTurn(ports, rec, asst, ctx.messages);
}
