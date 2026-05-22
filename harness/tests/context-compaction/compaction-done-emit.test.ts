/**
 * Asserts that both the sync (handleSync) and async (handleAsync) compaction
 * paths publish a `compaction_done` AgentEvent on `agent::events` after a
 * successful flat-state rewrite. The UI consumes this event to drop the
 * post-compaction CTX bar to its correct value.
 *
 * Strategy: vi.mock the summarize + flat-state + prune + replay helpers so
 * the handler reaches the emit call deterministically without needing a
 * real provider stream. We capture stream::set payloads via a stub ISdk.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';

vi.mock('../../src/context-compaction/summarize.js', () => ({
  summarizeAndAppend: vi.fn(async () => ({
    kind: 'ok',
    tail_start_id: 'entry-tail-1',
    tokens_before: 12_345,
    compaction_entry_id: 'entry-compact-1',
    summary_text: 'condensed summary of older turns',
    tail_messages: [],
  })),
}));

vi.mock('../../src/context-compaction/flat-state.js', () => ({
  buildSummaryMessage: (text: string) => ({
    role: 'system',
    kind: 'compaction',
    content: [{ type: 'text', text }],
  }),
  rewriteFlatMessages: vi.fn(async () => undefined),
}));

vi.mock('../../src/context-compaction/prune.js', () => ({
  prune: vi.fn(async () => ({ pruned_tokens: 0, pruned_parts: 0, scanned_parts: 0 })),
}));

vi.mock('../../src/context-compaction/replay.js', () => ({
  extractReplayTarget: () => ({ replay: null, truncatedMessages: [] }),
  reinjectReplay: vi.fn(async () => 'entry-replay'),
}));

vi.mock('../../src/context-compaction/model-resolver.js', () => ({
  fetchModelLimit: vi.fn(async () => ({
    providerID: 'anthropic',
    modelID: 'claude-haiku-4-5',
    modelLimit: { context: 200_000, input: 200_000, output: 4_096 },
  })),
  resolveModelFromSession: vi.fn(),
  resolveModelFromRunRequest: vi.fn(),
}));

type StreamSetCall = {
  stream_name: string;
  group_id: string;
  item_id: string;
  data: unknown;
};

function makeStubIii(extra: Record<string, unknown> = {}): {
  iii: ISdk;
  streamSetCalls: StreamSetCall[];
} {
  const state = new Map<string, unknown>();
  const streamSetCalls: StreamSetCall[] = [];

  const trigger = vi.fn(
    async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (Object.prototype.hasOwnProperty.call(extra, function_id)) {
        const v = extra[function_id];
        return typeof v === 'function' ? (v as () => unknown)() : v;
      }
      const p = (payload ?? {}) as Record<string, unknown>;

      if (function_id === 'stream::set') {
        streamSetCalls.push(p as unknown as StreamSetCall);
        return { ok: true };
      }
      if (function_id === 'state::get') {
        const v = state.get(p.key as string);
        return v !== undefined ? v : null;
      }
      if (function_id === 'state::set') {
        const v = p.value;
        if (v === null || v === undefined) state.delete(p.key as string);
        else state.set(p.key as string, v);
        return { ok: true };
      }
      if (function_id === 'state::update') {
        const key = p.key as string;
        const ops = (p.ops ?? []) as Array<{ type: string; by?: number; value?: unknown }>;
        const oldValue = state.has(key) ? state.get(key) : null;
        let newValue: unknown = oldValue;
        for (const op of ops) {
          if (op.type === 'set') newValue = op.value;
          if (op.type === 'increment') {
            const prev = typeof oldValue === 'number' ? oldValue : 0;
            newValue = prev + (op.by ?? 1);
          }
        }
        if (newValue === null || newValue === undefined) state.delete(key);
        else state.set(key, newValue);
        return { old_value: oldValue ?? null, new_value: newValue ?? null };
      }
      if (function_id === 'session-tree::messages') return { messages: [] };
      if (function_id === 'session-tree::compactions') return { entries: [] };
      if (function_id === 'session-tree::compact') return { entry_id: 'entry-compact-1' };
      if (function_id === 'session-tree::append_synthetic') return { entry_id: 'entry-syn-1' };
      return null;
    },
  );

  return {
    iii: {
      trigger,
      registerFunction: vi.fn(),
      registerTrigger: vi.fn(),
      publish: vi.fn(),
      subscribe: vi.fn(),
      enqueue: vi.fn(),
    } as unknown as ISdk,
    streamSetCalls,
  };
}

const model = {
  id: 'claude-haiku-4-5',
  providerID: 'anthropic',
  limit: { context: 200_000, input: 200_000, output: 4_096 },
};

describe('handleSync emits compaction_done', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    delete process.env.COMPACT_BUSY_TIMEOUT_MS;
  });

  it('publishes a compaction_done event on agent::events with mode="sync" after a successful rewrite', async () => {
    const { handleSync } = await import('../../src/context-compaction/handler-sync.js');
    const { iii, streamSetCalls } = makeStubIii();

    const result = await handleSync(iii, {
      session_id: 'sess-1',
      projected_tokens: 250_000,
      last_user_message_id: '',
      model,
    });

    expect(result.status).toBe('ok');
    const events = streamSetCalls.filter((c) => c.stream_name === 'agent::events');
    expect(events).toHaveLength(1);
    expect(events[0]?.group_id).toBe('sess-1');
    const data = events[0]?.data as Record<string, unknown>;
    expect(data.type).toBe('compaction_done');
    expect(data.mode).toBe('sync');
    expect(data.summary_text).toBe('condensed summary of older turns');
    expect(data.tokens_before).toBe(12_345);
    expect(data.compaction_entry_id).toBe('entry-compact-1');
    expect(data.tail_start_id).toBe('entry-tail-1');
  });

  it('swallows a publish failure and still returns ok to the orchestrator', async () => {
    const { handleSync } = await import('../../src/context-compaction/handler-sync.js');
    const { iii } = makeStubIii({
      'stream::set': () => {
        throw new Error('stream worker offline');
      },
    });

    const result = await handleSync(iii, {
      session_id: 'sess-publish-fail',
      projected_tokens: 250_000,
      last_user_message_id: '',
      model,
    });

    // Publish failure must not bubble — the orchestrator's turn is
    // blocked on this RPC return.
    expect(result.status).toBe('ok');
  });
});

describe('handleAsync emits compaction_done', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('publishes a compaction_done event on agent::events with mode="async" when an overflowing TurnEnd triggers compaction', async () => {
    const { handleAsync } = await import('../../src/context-compaction/handler-async.js');
    const { iii, streamSetCalls } = makeStubIii();

    // 200k usable (= 200k input − 0 reserved for this test) → overflow at 250k.
    process.env.COMPACT_RESERVED_TOKENS = '0';

    try {
      await handleAsync(iii, {
        groupId: 'sess-async-1',
        event: {
          data: {
            type: 'TurnEnd',
            message: {
              provider: 'anthropic',
              model: 'claude-haiku-4-5',
              usage: { input: 200_000, output: 50_000 },
            },
          },
        },
      });

      const events = streamSetCalls.filter((c) => c.stream_name === 'agent::events');
      expect(events).toHaveLength(1);
      const data = events[0]?.data as Record<string, unknown>;
      expect(data.type).toBe('compaction_done');
      expect(data.mode).toBe('async');
      expect(events[0]?.group_id).toBe('sess-async-1');
    } finally {
      delete process.env.COMPACT_RESERVED_TOKENS;
    }
  });

  it('does NOT emit when the turn did not overflow (under threshold)', async () => {
    const { handleAsync } = await import('../../src/context-compaction/handler-async.js');
    const { iii, streamSetCalls } = makeStubIii();

    await handleAsync(iii, {
      groupId: 'sess-async-noop',
      event: {
        data: {
          type: 'TurnEnd',
          message: {
            provider: 'anthropic',
            model: 'claude-haiku-4-5',
            usage: { input: 100, output: 50 },
          },
        },
      },
    });

    const events = streamSetCalls.filter((c) => c.stream_name === 'agent::events');
    expect(events).toHaveLength(0);
  });
});
