/**
 * `turn::assistant_streaming`. Start turn, stream provider response, finalize, and route onward.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk, StreamChannelRef } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { AssistantMessage } from '../../types/agent-message.js';
import type { AgentFunction } from '../../types/function.js';
import type { ProviderStreamInput } from '../../types/provider.js';
import type { AssistantMessageEvent } from '../../types/stream-event.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { runPreflight } from '../preflight.js';
import { buildInput, decide, targetFunctionId } from '../provider-router.js';
import { runTransition } from '../run-transition.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';

function eventPartial(ev: AssistantMessageEvent): AssistantMessage | null {
  if ('partial' in ev) return ev.partial;
  if (ev.type === 'done') return ev.message;
  if (ev.type === 'error') return ev.error;
  return null;
}

function syntheticErrorAssistant(
  provider: string,
  model: string,
  reason: string,
): AssistantMessage {
  return {
    role: 'assistant',
    content: [{ type: 'text', text: reason }],
    stop_reason: 'error',
    error_message: reason,
    error_kind: 'transient',
    usage: null,
    model,
    provider,
    timestamp: Date.now(),
  };
}

function formatProviderError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  return raw
    .replace(/^IIIInvocationError:\s*/i, '')
    .replace(/^invocation_failed:\s*/i, '')
    .trim();
}

function isErrorOrAborted(asst: AssistantMessage): boolean {
  return asst.stop_reason === 'error' || asst.stop_reason === 'aborted';
}

async function finalizeAssistant(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const asst = rec.last_assistant;
  if (!asst) throw new Error('assistant_streaming finalize without last_assistant');

  await emit(iii, rec.session_id, {
    type: 'message_complete',
    message: asst,
    body_streamed: rec.assistant_body_streamed === true,
  });

  const errored = isErrorOrAborted(asst);
  if (!errored) {
    const messages = await persistence.loadMessages(iii, rec.session_id);
    const last = messages[messages.length - 1];
    const dup =
      last &&
      last.role === 'assistant' &&
      last.timestamp === asst.timestamp &&
      last.model === asst.model &&
      last.provider === asst.provider;
    if (!dup) {
      messages.push(asst);
      await persistence.saveMessages(iii, rec.session_id, messages);
    } else {
      logger.warn('finalizeAssistant: skipping duplicate assistant push (re-entry detected)', {
        session_id: rec.session_id,
        timestamp: asst.timestamp,
      });
    }
  }

  if (errored) {
    await emit(iii, rec.session_id, { type: 'turn_end', message: asst, function_results: [] });
    rec.turn_end_emitted = true;
    transitionTo(rec, 'tearing_down');
    return;
  }
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
  if (rec.max_turns !== undefined && rec.turn_count >= rec.max_turns) {
    const cap = rec.max_turns ?? 0;
    const exhausted: AssistantMessage = {
      role: 'assistant',
      content: [{ type: 'text', text: `loop stopped: max_turns (${cap}) reached` }],
      stop_reason: 'end',
      error_message: null,
      error_kind: null,
      usage: null,
      model: '',
      provider: '',
      timestamp: Date.now(),
    };
    await emit(iii, rec.session_id, {
      type: 'message_complete',
      message: exhausted,
      body_streamed: false,
    });
    await emit(iii, rec.session_id, {
      type: 'turn_end',
      message: exhausted,
      function_results: [],
    });
    rec.turn_end_emitted = true;
    rec.last_assistant = exhausted;
    const messages = await persistence.loadMessages(iii, rec.session_id);
    messages.push(exhausted);
    await persistence.saveMessages(iii, rec.session_id, messages);
    transitionTo(rec, 'tearing_down');
    return;
  }
  rec.turn_count++;
  rec.turn_end_emitted = false;
  rec.assistant_body_streamed = false;

  const request = await persistence.loadRunRequest(iii, rec.session_id);
  let messages = await persistence.loadMessages(iii, rec.session_id);
  const schemas = await persistence.loadFunctionSchemas(iii, rec.session_id);

  const { provider, model, system_prompt } = request;
  const tools = (Array.isArray(schemas) ? schemas : []) as AgentFunction[];

  const decision = decide({ provider, model });
  const targetFn = targetFunctionId(decision);

  const preflightResult = await runPreflight(
    iii,
    rec.session_id,
    messages,
    decision.provider,
    model,
  );
  if (preflightResult === 'compacted') {
    messages = await persistence.loadMessages(iii, rec.session_id);
  }

  let channel: Awaited<ReturnType<ISdk['createChannel']>>;
  try {
    channel = await iii.createChannel();
  } catch (err) {
    logger.warn('createChannel failed; falling back to synthetic error', {
      err: String(err),
    });
    rec.last_assistant = syntheticErrorAssistant(
      decision.provider,
      decision.model,
      `create_channel failed: ${String(err)}`,
    );
    await finalizeAssistant(iii, rec);
    return;
  }

  const messageQueue: string[] = [];
  let done = false;
  let resolveNext: (() => void) | null = null;
  channel.reader.onMessage((msg: string) => {
    messageQueue.push(msg);
    if (resolveNext) {
      const fn = resolveNext;
      resolveNext = null;
      fn();
    }
  });
  channel.reader.stream.resume();

  const input: ProviderStreamInput = buildInput(
    decision,
    channel.writerRef as StreamChannelRef,
    system_prompt,
    messages,
    tools,
  );

  let triggerError: string | null = null;
  const triggerPromise = iii
    .trigger<unknown, unknown>({
      function_id: targetFn,
      payload: input,
      timeoutMs: 300_000,
    })
    .catch((err) => {
      logger.warn('provider stream trigger failed', { targetFn, err: String(err) });
      triggerError = formatProviderError(err);
      done = true;
      if (resolveNext) {
        const fn = resolveNext;
        resolveNext = null;
        fn();
      }
      return null;
    });

  const readPromise = (async (): Promise<AssistantMessage | null> => {
    let final: AssistantMessage | null = null;
    while (!done) {
      while (messageQueue.length > 0) {
        const text = messageQueue.shift();
        if (text === undefined) break;
        let event: AssistantMessageEvent | null = null;
        try {
          event = JSON.parse(text) as AssistantMessageEvent;
        } catch (err) {
          logger.warn('decode AssistantMessageEvent failed', {
            session_id: rec.session_id,
            err: String(err),
          });
          continue;
        }
        const partial = eventPartial(event);
        if (partial) final = partial;
        if (event.type !== 'done' && event.type !== 'error') {
          if (partial) {
            await emit(iii, rec.session_id, {
              type: 'message_update',
              message: partial,
              llm_event: event,
            });
            if (event.type === 'text_delta' || event.type === 'thinking_delta') {
              rec.assistant_body_streamed = true;
            }
          }
          continue;
        }
        if (event.type === 'done') final = event.message;
        else final = event.error;
        done = true;
        break;
      }
      if (done) break;
      await new Promise<void>((r) => {
        resolveNext = r;
      });
    }
    return final;
  })();

  const [, finalMsg] = await Promise.all([triggerPromise, readPromise]);
  if (finalMsg) {
    rec.last_assistant = finalMsg;
  } else {
    const errorText = triggerError ?? 'provider channel closed without final';
    const synthetic = syntheticErrorAssistant(decision.provider, decision.model, errorText);
    await emit(iii, rec.session_id, {
      type: 'message_update',
      message: synthetic,
      llm_event: { type: 'text_delta', partial: synthetic, delta: errorText },
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
