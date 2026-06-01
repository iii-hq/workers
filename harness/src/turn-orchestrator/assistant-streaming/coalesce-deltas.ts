/**
 * Coalesce consecutive same-type provider stream deltas into a single
 * `message_update` emit, to cut the per-token span/RPC explosion on the
 * streaming path (see docs/superpowers/specs/2026-05-31-harness-streaming-
 * delta-coalescing-design.md).
 *
 * The console renders streaming text by appending `llm_event.delta`
 * (console/web/src/lib/backend/translate.ts), so merging N same-type deltas
 * into one event whose `delta` is their concatenation — carrying the latest
 * `partial` — is byte-for-byte equivalent on the wire. Discrete events
 * (`*_start`/`*_end`, `usage`, `stop`, …) are flushed-through 1:1 and never
 * merged, preserving ordering.
 */

import type { AssistantMessage } from '../../types/agent-message.js';
import type { AssistantMessageEvent } from '../../types/stream-event.js';

const COALESCE_TYPES = new Set(['text_delta', 'thinking_delta', 'functioncall_delta']);
const DEFAULT_FLUSH_MS = 60;
const DEFAULT_MAX_CHARS = 4096;

type Emit = (partial: AssistantMessage, event: AssistantMessageEvent) => Promise<void>;

export interface DeltaCoalescer {
  /** Mirrors the provider onDelta contract: (partial, event). */
  onEvent(partial: AssistantMessage, event: AssistantMessageEvent): Promise<void>;
  /** Emit any buffered deltas now. Idempotent; safe to call on every exit path. */
  flush(): Promise<void>;
}

export function createDeltaCoalescer(
  emit: Emit,
  opts?: { flushMs?: number; maxChars?: number },
): DeltaCoalescer {
  const flushMs = opts?.flushMs ?? DEFAULT_FLUSH_MS;
  const maxChars = opts?.maxChars ?? DEFAULT_MAX_CHARS;

  let buf: {
    type: string;
    delta: string;
    last: AssistantMessageEvent;
    partial: AssistantMessage;
  } | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  function clearTimer(): void {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  }

  async function flush(): Promise<void> {
    clearTimer();
    if (!buf) return;
    const merged = { ...buf.last, delta: buf.delta } as AssistantMessageEvent;
    const { partial } = buf;
    buf = null;
    await emit(partial, merged);
  }

  // Periodic throttle, NOT a trailing debounce: a steady ~flushMs cadence keeps
  // text appearing smoothly while continuous deltas arrive. A trailing debounce
  // would withhold all text until the stream paused.
  function arm(): void {
    if (timer) return;
    timer = setTimeout(() => {
      void flush();
    }, flushMs);
  }

  async function onEvent(partial: AssistantMessage, event: AssistantMessageEvent): Promise<void> {
    const d = event as { type: string; delta?: string };
    const coalescable = COALESCE_TYPES.has(d.type) && typeof d.delta === 'string';

    if (coalescable) {
      if (buf && buf.type !== d.type) await flush(); // type switch is a boundary
      buf = buf
        ? { type: buf.type, delta: buf.delta + d.delta, last: event, partial }
        : { type: d.type, delta: d.delta as string, last: event, partial };
      if (flushMs <= 0 || buf.delta.length >= maxChars) {
        await flush();
      } else {
        arm();
      }
      return;
    }

    // Discrete event: flush pending deltas first (preserve order), then 1:1.
    await flush();
    await emit(partial, event);
  }

  return { onEvent, flush };
}
