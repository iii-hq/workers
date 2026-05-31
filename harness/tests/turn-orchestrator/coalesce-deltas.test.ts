import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createDeltaCoalescer } from '../../src/turn-orchestrator/assistant-streaming/coalesce-deltas.js';
import type {
  AssistantStreamingPorts,
  DeltaHandler,
  StreamContext,
} from '../../src/turn-orchestrator/assistant-streaming/ports.js';
import { runStreamTurn } from '../../src/turn-orchestrator/assistant-streaming/run.js';
import { emptyAssistant } from '../../src/types/agent-message.js';
import type { AssistantMessageEvent } from '../../src/types/stream-event.js';

const P = emptyAssistant('anthropic', 'claude');
const td = (delta: string): AssistantMessageEvent => ({ type: 'text_delta', partial: P, delta });
const thd = (delta: string): AssistantMessageEvent => ({
  type: 'thinking_delta',
  partial: P,
  delta,
});
const fcd = (delta: string): AssistantMessageEvent => ({
  type: 'functioncall_delta',
  partial: P,
  delta,
});

function recorder() {
  const emits: AssistantMessageEvent[] = [];
  const emit = async (_partial: unknown, event: AssistantMessageEvent) => {
    emits.push(event);
  };
  return { emit, emits };
}

describe('createDeltaCoalescer — lossless merge', () => {
  it('merges consecutive same-type deltas into one event whose delta is the concatenation', async () => {
    const { emit, emits } = recorder();
    const c = createDeltaCoalescer(emit, { flushMs: 60 });

    await c.onEvent(P, td('Hel'));
    await c.onEvent(P, td('lo, '));
    await c.onEvent(P, td('world'));
    expect(emits).toHaveLength(0); // still buffering

    await c.flush();

    expect(emits).toHaveLength(1);
    expect(emits[0]).toMatchObject({ type: 'text_delta', delta: 'Hello, world' });
  });

  it('preserves every byte across many deltas (Σ in === merged out)', async () => {
    const { emit, emits } = recorder();
    const c = createDeltaCoalescer(emit, { flushMs: 60 });
    const parts = Array.from({ length: 50 }, (_, i) => `t${i}-`);
    for (const p of parts) await c.onEvent(P, td(p));
    await c.flush();
    const merged = emits.map((e) => (e as { delta: string }).delta).join('');
    expect(merged).toBe(parts.join(''));
  });
});

describe('createDeltaCoalescer — boundaries', () => {
  it('flushes at a delta-type switch (text vs thinking vs functioncall never merge)', async () => {
    const { emit, emits } = recorder();
    const c = createDeltaCoalescer(emit, { flushMs: 60 });

    await c.onEvent(P, td('a'));
    await c.onEvent(P, td('b'));
    await c.onEvent(P, thd('x'));
    await c.onEvent(P, thd('y'));
    await c.onEvent(P, fcd('{'));
    await c.flush();

    expect(emits.map((e) => [e.type, (e as { delta: string }).delta])).toEqual([
      ['text_delta', 'ab'],
      ['thinking_delta', 'xy'],
      ['functioncall_delta', '{'],
    ]);
  });

  it('flushes pending deltas before a discrete event and emits the discrete event 1:1, in order', async () => {
    const { emit, emits } = recorder();
    const c = createDeltaCoalescer(emit, { flushMs: 60 });

    await c.onEvent(P, td('a'));
    await c.onEvent(P, td('b'));
    await c.onEvent(P, { type: 'text_end', partial: P });
    await c.onEvent(P, td('c'));
    await c.flush();

    expect(emits.map((e) => e.type)).toEqual(['text_delta', 'text_end', 'text_delta']);
    expect((emits[0] as { delta: string }).delta).toBe('ab');
    expect((emits[2] as { delta: string }).delta).toBe('c');
  });

  it('force-flushes when a single batch reaches the char cap', async () => {
    const { emit, emits } = recorder();
    const c = createDeltaCoalescer(emit, { flushMs: 60, maxChars: 5 });

    await c.onEvent(P, td('abc'));
    await c.onEvent(P, td('def')); // 6 >= 5 → flush
    expect(emits).toHaveLength(1);
    expect((emits[0] as { delta: string }).delta).toBe('abcdef');

    await c.flush(); // nothing buffered
    expect(emits).toHaveLength(1);
  });
});

describe('createDeltaCoalescer — flush triggers', () => {
  it('emits trailing buffered deltas on flush()', async () => {
    const { emit, emits } = recorder();
    const c = createDeltaCoalescer(emit, { flushMs: 60 });
    await c.onEvent(P, td('tail'));
    expect(emits).toHaveLength(0);
    await c.flush();
    expect(emits).toHaveLength(1);
    expect((emits[0] as { delta: string }).delta).toBe('tail');
  });

  describe('with fake timers', () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it('flushes on a periodic cadence while continuous deltas arrive (not a trailing debounce)', async () => {
      const { emit, emits } = recorder();
      const c = createDeltaCoalescer(emit, { flushMs: 60 });

      await c.onEvent(P, td('a'));
      await c.onEvent(P, td('b'));
      expect(emits).toHaveLength(0);

      await vi.advanceTimersByTimeAsync(60);
      expect(emits).toHaveLength(1);
      expect((emits[0] as { delta: string }).delta).toBe('ab');

      await c.onEvent(P, td('c'));
      await vi.advanceTimersByTimeAsync(60);
      expect(emits).toHaveLength(2);
      expect((emits[1] as { delta: string }).delta).toBe('c');
    });
  });

  it('passthrough mode (flushMs: 0) emits each delta 1:1 immediately', async () => {
    const { emit, emits } = recorder();
    const c = createDeltaCoalescer(emit, { flushMs: 0 });
    await c.onEvent(P, td('a'));
    await c.onEvent(P, td('b'));
    expect(emits.map((e) => (e as { delta: string }).delta)).toEqual(['a', 'b']);
  });
});

describe('runStreamTurn wires the coalescer', () => {
  function mkPorts(drive: (onDelta: DeltaHandler) => Promise<void>) {
    const emits: AssistantMessageEvent[] = [];
    const ports = {
      streamTurn: async (_ctx: StreamContext, onDelta: DeltaHandler) => {
        await drive(onDelta);
        return { final: emptyAssistant(), error: null };
      },
      emitMessageUpdate: async (
        _sid: string,
        _msg: unknown,
        event: AssistantMessageEvent,
      ) => {
        emits.push(event);
      },
    } as unknown as AssistantStreamingPorts;
    return { ports, emits };
  }

  it('collapses a run of same-type deltas into a single emitMessageUpdate (via the final flush)', async () => {
    const { ports, emits } = mkPorts(async (onDelta) => {
      await onDelta(P, td('a'));
      await onDelta(P, td('b'));
      await onDelta(P, td('c'));
    });

    const outcome = await runStreamTurn(ports, 'sid', {} as StreamContext);

    expect(emits).toHaveLength(1);
    expect((emits[0] as { delta: string }).delta).toBe('abc');
    expect(outcome.body_streamed).toBe(true);
  });

  it('passes discrete events through 1:1, in order, around coalesced text', async () => {
    const { ports, emits } = mkPorts(async (onDelta) => {
      await onDelta(P, td('a'));
      await onDelta(P, td('b'));
      await onDelta(P, { type: 'text_end', partial: P });
      await onDelta(P, td('c'));
    });

    await runStreamTurn(ports, 'sid', {} as StreamContext);

    expect(emits.map((e) => e.type)).toEqual(['text_delta', 'text_end', 'text_delta']);
    expect((emits[0] as { delta: string }).delta).toBe('ab');
    expect((emits[2] as { delta: string }).delta).toBe('c');
  });

  it('sets body_streamed=false when no text/thinking deltas were seen', async () => {
    const { ports, emits } = mkPorts(async (onDelta) => {
      await onDelta(P, { type: 'usage', usage: { output: 1 } });
    });

    const outcome = await runStreamTurn(ports, 'sid', {} as StreamContext);

    expect(outcome.body_streamed).toBe(false);
    expect(emits.map((e) => e.type)).toEqual(['usage']);
  });
});
