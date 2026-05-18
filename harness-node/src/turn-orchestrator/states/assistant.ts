/**
 * `awaiting_assistant`, `assistant_streaming`, `assistant_finished`. Phase 2.A
 * channel-based streaming lives in `handleStreaming`.
 */

import type { ISdk, StreamChannelRef } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { AgentEvent } from '../../types/agent-event.js';
import type { AgentMessage, AssistantMessage } from '../../types/agent-message.js';
import type { ContentBlock } from '../../types/content.js';
import type { AgentFunction, FunctionCall } from '../../types/function.js';
import type { ProviderStreamInput } from '../../types/provider.js';
import type { AssistantMessageEvent, StopReason } from '../../types/stream-event.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { buildInput, decide, targetFunctionId } from '../provider-router.js';
import { type TurnStateRecord, transitionTo } from '../state.js';

export async function handleAwaiting(iii: ISdk, rec: TurnStateRecord): Promise<void> {
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
  transitionTo(rec, 'assistant_streaming');
}

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

/**
 * Strip iii-sdk's `IIIInvocationError: invocation_failed: ` framing from
 * a thrown trigger error so the user-visible message is just the
 * underlying cause (e.g. "auth::get_token returned no credential for
 * provider=openai").
 */
function formatProviderError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  return raw
    .replace(/^IIIInvocationError:\s*/i, '')
    .replace(/^invocation_failed:\s*/i, '')
    .trim();
}

export async function handleStreaming(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const request = await persistence.loadRunRequest(iii, rec.session_id);
  const messages = await persistence.loadMessages(iii, rec.session_id);
  const schemas = await persistence.loadFunctionSchemas(iii, rec.session_id);

  const provider = typeof request.provider === 'string' ? (request.provider as string) : '';
  const model = typeof request.model === 'string' ? (request.model as string) : '';
  const system_prompt =
    typeof request.system_prompt === 'string' ? (request.system_prompt as string) : null;
  const tools = (Array.isArray(schemas) ? schemas : []) as AgentFunction[];

  const decision = decide({ provider, model });
  const targetFn = targetFunctionId(decision);

  // Open a channel; provider writes AssistantMessageEvent JSON into it.
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

  const input: ProviderStreamInput = buildInput(
    decision,
    channel.writerRef as StreamChannelRef,
    system_prompt,
    messages,
    tools,
  );

  // Capture the trigger error (if any) so the synthetic assistant message
  // below carries the *actual* cause (e.g. "no credential for
  // provider=openai") instead of a generic "channel closed without final".
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
        // Terminal event: capture final message and break out.
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
    // Trigger failed or channel closed without a terminal frame. The
    // provider didn't get to stream any text_delta events, so the UI
    // never populated its renderer. Emit a synthetic message_update
    // carrying the error as a text_delta so the existing UI translator
    // (which assumes deltas drive the chat text) shows the error.
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

function extractFunctionCalls(msg: AssistantMessage): FunctionCall[] {
  const out: FunctionCall[] = [];
  for (const b of msg.content) {
    if (b.type === 'function_call') {
      out.push({ id: b.id, function_id: b.function_id, arguments: b.arguments });
    }
  }
  return out;
}

export function assistantLifecycleEvents(asst: AssistantMessage): AgentEvent[] {
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
  // Error/aborted assistant messages (e.g. provider auth failures,
  // network blips, user aborts) are surfaced to the UI via the
  // MessageStart/MessageEnd events emitted above, but we deliberately
  // keep them out of the session's persisted message history so the
  // LLM's next-turn context doesn't accumulate transient infra noise.
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
  } else {
    rec.pending_function_calls = calls;
    transitionTo(rec, 'function_prepare');
  }
}
