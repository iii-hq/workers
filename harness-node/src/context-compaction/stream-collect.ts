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

  while (!terminal) {
    if (events.length > 0) {
      const tail = events[events.length - 1];
      if (tail && (tail.type === 'done' || tail.type === 'error')) {
        terminal = tail;
        break;
      }
    }
    await new Promise<void>((r) => {
      resolveNext = r;
      setTimeout(r, 25);
    });
    break;
  }

  if (!terminal) {
    throw new Error('summariser stream returned without a terminal event');
  }
  if ((terminal as AssistantMessageEvent).type === 'error') {
    return (terminal as { type: 'error'; error: AssistantMessage }).error;
  }
  return (terminal as { type: 'done'; message: AssistantMessage }).message;
}
