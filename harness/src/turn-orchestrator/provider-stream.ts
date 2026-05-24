/**
 * Provider streaming. Turns an iii stream channel plus the provider trigger into
 * a single final `AssistantMessage`, hiding the pull-based message pump behind an
 * async iterator.
 *
 * `streamProviderTurn` owns channel creation, the concurrent provider trigger,
 * and the read loop. The caller supplies how to build the provider input (it
 * needs the channel's writer ref) and a per-delta callback used to emit UI
 * `message_update` events.
 */

import type { ISdk, StreamChannelRef } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AssistantMessage } from '../types/agent-message.js';
import type { ProviderStreamInput } from '../types/provider.js';
import type { AssistantMessageEvent } from '../types/stream-event.js';

const PROVIDER_STREAM_TIMEOUT_MS = 300_000;

type Channel = Awaited<ReturnType<ISdk['createChannel']>>;

/**
 * Bridges a push callback (`channel.reader.onMessage`) to async iteration.
 * `push` buffers a message and wakes a pending `drain`; `end` terminates the
 * iterator once the buffer is empty.
 */
class MessagePump {
  private readonly items: string[] = [];
  private wake: (() => void) | null = null;
  private ended = false;

  push(item: string): void {
    this.items.push(item);
    this.signal();
  }

  end(): void {
    this.ended = true;
    this.signal();
  }

  private signal(): void {
    if (this.wake) {
      const wake = this.wake;
      this.wake = null;
      wake();
    }
  }

  async *drain(): AsyncGenerator<string> {
    while (true) {
      while (this.items.length > 0) {
        const item = this.items.shift();
        if (item !== undefined) yield item;
      }
      if (this.ended) return;
      await new Promise<void>((resolve) => {
        this.wake = resolve;
      });
    }
  }
}

/** Outcome of a provider turn: the final message, or the reason none arrived. */
export type ProviderTurnResult = {
  final: AssistantMessage | null;
  /** Set when the provider trigger threw; null when the channel just closed. */
  error: string | null;
};

/** Strip iii invocation-error prefixes so the surfaced message reads cleanly. */
export function formatProviderError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  return raw
    .replace(/^IIIInvocationError:\s*/i, '')
    .replace(/^invocation_failed:\s*/i, '')
    .trim();
}

/** The assistant message a stream event carries (final for done/error, else the partial). */
function eventMessage(ev: AssistantMessageEvent): AssistantMessage | null {
  if (ev.type === 'done') return ev.message;
  if (ev.type === 'error') return ev.error;
  if ('partial' in ev) return ev.partial;
  return null;
}

function parseEvent(text: string, session_id: string): AssistantMessageEvent | null {
  try {
    return JSON.parse(text) as AssistantMessageEvent;
  } catch (err) {
    logger.warn('decode AssistantMessageEvent failed', { session_id, err: String(err) });
    return null;
  }
}

export async function streamProviderTurn(
  iii: ISdk,
  params: {
    session_id: string;
    targetFn: string;
    buildInput: (writerRef: StreamChannelRef) => ProviderStreamInput;
    onDelta: (partial: AssistantMessage, event: AssistantMessageEvent) => Promise<void>;
  },
): Promise<ProviderTurnResult> {
  let channel: Channel;
  try {
    channel = await iii.createChannel();
  } catch (err) {
    logger.warn('createChannel failed; falling back to synthetic error', { err: String(err) });
    return { final: null, error: `create_channel failed: ${String(err)}` };
  }

  const pump = new MessagePump();
  channel.reader.onMessage((msg: string) => pump.push(msg));
  channel.reader.stream.resume();

  let error: string | null = null;
  const triggerPromise = iii
    .trigger<unknown, unknown>({
      function_id: params.targetFn,
      payload: params.buildInput(channel.writerRef as StreamChannelRef),
      timeoutMs: PROVIDER_STREAM_TIMEOUT_MS,
    })
    .catch((err) => {
      logger.warn('provider stream trigger failed', { targetFn: params.targetFn, err: String(err) });
      error = formatProviderError(err);
      pump.end();
      return null;
    });

  let final: AssistantMessage | null = null;
  for await (const text of pump.drain()) {
    const event = parseEvent(text, params.session_id);
    if (!event) continue;
    if (event.type === 'done' || event.type === 'error') {
      final = eventMessage(event);
      break;
    }
    const partial = eventMessage(event);
    if (partial) {
      final = partial;
      await params.onDelta(partial, event);
    }
  }
  pump.end();
  await triggerPromise;
  return { final, error };
}
