/**
 * `turn::assistant_streaming`. Start turn, stream provider response, advance to finished.
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
import { type TurnStateRecord, transitionTo } from '../state.js';
import {
  TurnStepPayloadSchema,
  type TurnStepPayload,
  type TurnStepResult,
  staleSkipResult,
} from '../turn-step-payload.js';

function eventPartial(ev: AssistantMessageEvent): AssistantMessage | null {
  if ('partial' in ev) return ev.partial;
  if (ev.type === 'done') return ev.message;
  if (ev.type === 'error') return ev.error;
  return null;
}

function latestFunctionCall(
  msg: AssistantMessage,
): { id: string; function_id: string; args: unknown } | null {
  for (let i = msg.content.length - 1; i >= 0; i--) {
    const b = msg.content[i];
    if (b?.type === 'function_call') {
      return { id: b.id, function_id: b.function_id, args: b.arguments };
    }
  }
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
    await emit(iii, rec.session_id, { type: 'message_start', message: exhausted });
    await emit(iii, rec.session_id, { type: 'message_end', message: exhausted });
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
  await emit(iii, rec.session_id, { type: 'turn_start' });

  const request = await persistence.loadRunRequest(iii, rec.session_id);
  let messages = await persistence.loadMessages(iii, rec.session_id);
  const schemas = await persistence.loadFunctionSchemas(iii, rec.session_id);

  const provider = typeof request.provider === 'string' ? (request.provider as string) : '';
  const model = typeof request.model === 'string' ? (request.model as string) : '';
  const system_prompt =
    typeof request.system_prompt === 'string' ? (request.system_prompt as string) : null;
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
    transitionTo(rec, 'assistant_finished');
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
            if (event.type === 'functioncall_start' || event.type === 'functioncall_delta') {
              const fc = latestFunctionCall(partial);
              if (fc) {
                await emit(iii, rec.session_id, {
                  type: 'function_execution_update',
                  function_call_id: fc.id,
                  function_id: fc.function_id,
                  args: fc.args,
                  partial_result: null,
                });
              }
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
  transitionTo(rec, 'assistant_finished');
}

export async function execute(iii: ISdk, payload: TurnStepPayload): Promise<TurnStepResult> {
  const rec = await persistence.loadRecord(iii, payload.session_id);
  if (!rec) {
    throw new Error(`turn::assistant_streaming invariant: missing session ${payload.session_id}`);
  }
  const skipped = staleSkipResult('assistant_streaming', rec);
  if (skipped) return skipped;

  const from_state = rec.state;
  try {
    await handleStreaming(iii, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec);
  return { ok: true, from_state, to_state: rec.state };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::assistant_streaming',
    async (payload: unknown) => execute(iii, TurnStepPayloadSchema.parse(payload)),
    {
      description:
        'Run one durable FSM transition for session in state assistant_streaming: start turn and stream provider response.',
    },
  );
}
