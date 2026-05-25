/**
 * Stream one provider turn, persist the assistant message, route onward, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AssistantMessage } from '../../types/agent-message.js';
import { decide } from '../provider-router.js';
import { runTransition } from '../run-transition.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { syntheticAssistant } from '../synthetic-assistant.js';
import { emitTurnEndOnce } from '../state-runtime/turn-end.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import {
  AssistantStreamingInvariantError,
  createStreamingPorts,
  hasFunctionCalls,
  isErrorOrAborted,
  parseFunctionSchemas,
  type AssistantRoute,
  type AssistantStreamingPorts,
  type StreamContext,
  type StreamTurnOutcome,
} from './ports.js';

export function beginTurn(rec: TurnStateRecord): void {
  rec.turn_count++;
  rec.turn_end_emitted = false;
  rec.assistant_body_streamed = false;
}

export async function prepareStreamContext(
  ports: AssistantStreamingPorts,
  rec: TurnStateRecord,
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
  rec: TurnStateRecord,
): Promise<void> {
  const asst = rec.last_assistant;
  if (!asst) {
    throw new AssistantStreamingInvariantError(
      'assistant_streaming finalize without last_assistant',
    );
  }

  await ports.emitMessageComplete(
    rec.session_id,
    asst,
    rec.assistant_body_streamed === true,
  );

  const route = routeAssistantTurn(asst);

  if (route.kind === 'stopped') {
    await emitTurnEndOnce(ports, rec, asst);
    await ports.finishSession(rec);
    return;
  }

  await ports.persistAssistantIfNew(rec.session_id, asst);

  if (route.kind === 'function_execute') {
    rec.function_results = [];
    rec.work = undefined;
    transitionTo(rec, 'function_execute');
    return;
  }

  transitionTo(rec, 'steering_check');
}

export async function handleStreaming(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const ports = createStreamingPorts(iii);
  beginTurn(rec);
  const ctx = await prepareStreamContext(ports, rec);
  const outcome = await runStreamTurn(ports, rec.session_id, ctx);
  rec.last_assistant = resolveAssistantMessage(outcome, ctx.decision);
  rec.assistant_body_streamed = outcome.body_streamed;

  const syntheticReason = syntheticStreamReason(outcome);
  if (syntheticReason) {
    await ports.emitMessageUpdate(rec.session_id, rec.last_assistant, {
      type: 'text_delta',
      partial: rec.last_assistant,
      delta: syntheticReason,
    });
  }

  await finalizeAssistantTurn(ports, rec);
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
