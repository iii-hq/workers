import type { ISdk, StreamChannelRef } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AssistantMessage } from '../types/agent-message.js';
import type { ProviderStreamInput } from '../types/provider.js';
import type { AssistantMessageEvent } from '../types/stream-event.js';

const SUMMARIZER_TIMEOUT_MS = 120_000;

export type StreamCollectInput = Omit<ProviderStreamInput, 'writer_ref'>;

export async function streamAndCollect(
  iii: ISdk,
  input: StreamCollectInput,
  providerFunctionId: string,
): Promise<AssistantMessage> {
  const channel = await iii.createChannel();
  const events: AssistantMessageEvent[] = [];
  let resolveNext: (() => void) | null = null;
  let terminal: AssistantMessageEvent | null = null;

  channel.reader.onMessage((raw: string) => {
    try {
      const ev = JSON.parse(raw) as AssistantMessageEvent;
      events.push(ev);
      if (ev.type === 'done' || ev.type === 'error') terminal = ev;
      if (resolveNext) {
        const fn = resolveNext;
        resolveNext = null;
        fn();
      }
    } catch (err) {
      logger.warn('streamAndCollect: decode failed', { err: String(err) });
    }
  });
  // iii-sdk@0.12.0: onMessage doesn't open the read-side; resume() does.
  channel.reader.stream.resume();

  await iii.trigger<unknown, unknown>({
    function_id: providerFunctionId,
    payload: {
      ...input,
      writer_ref: channel.writerRef satisfies StreamChannelRef,
    },
    timeoutMs: SUMMARIZER_TIMEOUT_MS,
  });

  // trigger() resolved, but the channel may not have delivered the
  // terminal event yet. Poll for up to GRACE_MS before giving up so a
  // slow IPC hop doesn't masquerade as "stream returned without a
  // terminal event".
  const GRACE_MS = 1_000;
  const deadline = Date.now() + GRACE_MS;
  while (!terminal && Date.now() < deadline) {
    await new Promise<void>((r) => {
      resolveNext = r;
      setTimeout(r, 25);
    });
  }

  if (!terminal) {
    throw new Error('summariser stream returned without a terminal event');
  }
  if ((terminal as AssistantMessageEvent).type === 'error') {
    // Surface provider errors as a thrown exception so summarizeAndAppend's
    // catch treats them as compaction failures. Without this, the error
    // AssistantMessage's text content got silently written as the summary.
    const errMsg = (terminal as { type: 'error'; error: AssistantMessage }).error;
    const detail =
      typeof errMsg.error_message === 'string' && errMsg.error_message.length > 0
        ? errMsg.error_message
        : extractTextFromMessage(errMsg);
    throw new Error(`summariser stream error: ${detail || 'unknown provider error'}`);
  }
  return (terminal as { type: 'done'; message: AssistantMessage }).message;
}

function extractTextFromMessage(msg: AssistantMessage): string {
  for (const block of msg.content ?? []) {
    if ((block as { type?: string }).type === 'text') {
      return (block as { type: 'text'; text: string }).text;
    }
  }
  return '';
}
