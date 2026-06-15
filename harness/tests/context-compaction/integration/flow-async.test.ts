/**
 * Integration tests: async (TurnEnd-driven) compaction flow via handleAsync.
 *
 * Tests:
 * 1. Below-threshold TurnEnd → no compaction.
 * 2. Above-threshold TurnEnd (large fixture) → compaction runs, compaction custom entry appended.
 * 3. Two concurrent TurnEnds → only one compaction runs (lease serializes).
 */
import { describe, expect, it, vi } from 'vitest';
import { handleAsync } from '../../../src/context-compaction/handler-async.js';
import { loadFixture } from '../../fixtures/load.js';

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

type MockTriggerReq = { function_id: string; payload: Record<string, unknown>; timeoutMs?: number };

/**
 * Build an ISdk-compatible mock for the async flow.
 *
 * @param fixtureMessages - entries to return from session::messages
 * @param compactPayloads - collector array for compaction custom-entry appends
 * @param modelCatalog - what models::get should return (null = not found)
 */
function buildAsyncMock(opts: {
  fixtureMessages: Array<{ entry_id: string; message: unknown }>;
  compactPayloads?: unknown[];
  modelCatalog?: {
    id?: string;
    context_window: number;
    max_output_tokens: number;
  } | null;
  stateStore?: Map<string, unknown>;
}) {
  const {
    fixtureMessages,
    compactPayloads = [],
    modelCatalog = {
      context_window: 200_000,
      max_output_tokens: 4_096,
    },
    stateStore = new Map<string, unknown>(),
  } = opts;

  const doneSummaryMessage = {
    role: 'assistant',
    content: [{ type: 'text', text: 'Async compaction summary.' }],
    stop_reason: 'end',
    model: 'claude-haiku-4-5',
    provider: 'anthropic',
    timestamp: Date.now(),
  };

  let appendSeq = 0;
  const trigger = vi.fn(async (req: MockTriggerReq) => {
    const { function_id, payload } = req;

    if (function_id === 'session::messages') {
      return { messages: fixtureMessages };
    }
    if (function_id === 'session::append') {
      if ((payload as { custom?: unknown }).custom) compactPayloads.push(payload);
      return { entry_id: `appended-${++appendSeq}`, parent_id: null, timestamp: Date.now() };
    }
    if (function_id === 'session::update_message') {
      return { updated: true, revision: 1 };
    }
    if (function_id === 'router::models::get') {
      return modelCatalog ? { model: modelCatalog } : null;
    }
    if (function_id === 'state::get') {
      const v = stateStore.get((payload as { key: string }).key);
      return v !== undefined ? v : null;
    }
    if (function_id === 'state::set') {
      const p = payload as { key: string; value: unknown };
      if (p.value === null || p.value === undefined) {
        stateStore.delete(p.key);
      } else {
        stateStore.set(p.key, p.value);
      }
      return { ok: true };
    }
    if (function_id === 'state::update') {
      const p = payload as { key: string; ops: Array<{ type: string; value?: unknown }> };
      const oldValue = stateStore.has(p.key) ? stateStore.get(p.key) : null;
      let newValue: unknown = oldValue;
      for (const op of p.ops ?? []) {
        if (op.type === 'set') newValue = op.value;
      }
      if (newValue === null || newValue === undefined) {
        stateStore.delete(p.key);
      } else {
        stateStore.set(p.key, newValue);
      }
      return { old_value: oldValue ?? null, new_value: newValue ?? null };
    }
    if (function_id === 'router::complete') {
      return { message: doneSummaryMessage };
    }

    return undefined;
  });

  const iii = { trigger } as unknown as import('../../../src/runtime/iii.js').ISdk;

  return { iii, trigger, compactPayloads };
}

/**
 * Build an on_turn_end queue payload that handleAsync can process.
 *
 * @param sessionId - the session identifier
 * @param totalTokens - total token usage to report (drives overflow check)
 * @param modelId - model identifier (default: claude-haiku-4-5)
 */
function makeTurnEndFrame(sessionId: string, totalTokens: number, modelId = 'claude-haiku-4-5') {
  return {
    session_id: sessionId,
    provider: 'anthropic',
    model: modelId,
    usage: { input: totalTokens, output: 0, cache_read: 0, cache_write: 0 },
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const smallFixture = loadFixture('tiny');
const largeFixture = loadFixture('large-with-media');

describe('flow-async: below-threshold TurnEnd', () => {
  it('does not append a compaction when token usage is below overflow threshold', async () => {
    const compactPayloads: unknown[] = [];
    const fixtureMessages = smallFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message,
    }));

    const { iii, trigger } = buildAsyncMock({
      fixtureMessages,
      compactPayloads,
      modelCatalog: {
        context_window: 200_000,
        max_output_tokens: 4_096,
      },
    });

    // Use very small token count (well below the ~180k threshold for 200k context)
    const frame = makeTurnEndFrame(smallFixture.session_id, 100);
    await handleAsync(iii, frame);

    // compact should NOT have been called
    expect(compactPayloads).toHaveLength(0);

    // Also, no session::append should appear in trigger calls at all
    const appendCalls = trigger.mock.calls.filter(([req]) => req.function_id === 'session::append');
    expect(appendCalls).toHaveLength(0);
  });
});

describe('flow-async: above-threshold TurnEnd with large fixture', () => {
  it('runs compaction and appends the compaction custom entry when tokens exceed overflow threshold', async () => {
    const compactPayloads: unknown[] = [];
    const fixtureMessages = largeFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message,
    }));

    const { iii } = buildAsyncMock({
      fixtureMessages,
      compactPayloads,
      modelCatalog: {
        context_window: 200_000,
        max_output_tokens: 4_096,
      },
    });

    // Simulate overflow: input tokens at 185k (above 200k - 20k reserved = 180k threshold)
    const frame = makeTurnEndFrame(largeFixture.session_id, 185_000);
    await handleAsync(iii, frame);

    // The compaction custom entry MUST have been appended at least once
    expect(compactPayloads.length).toBeGreaterThan(0);

    // Verify the compact payload shape
    const cp = compactPayloads[0] as {
      session_id: string;
      custom: { custom_type: string; data: { summary: string } };
    };
    expect(cp.session_id).toBe(largeFixture.session_id);
    expect(cp.custom.custom_type).toBe('compaction');
    expect(typeof cp.custom.data.summary).toBe('string');
    expect(cp.custom.data.summary.length).toBeGreaterThan(0);
  });
});

describe('flow-async: concurrent TurnEnds serialized by lease', () => {
  it('runs compaction only once when two TurnEnds fire concurrently', async () => {
    const compactPayloads: unknown[] = [];
    const fixtureMessages = largeFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message,
    }));

    // Shared state store so both concurrent calls see the same lease state
    const stateStore = new Map<string, unknown>();

    const { iii } = buildAsyncMock({
      fixtureMessages,
      compactPayloads,
      modelCatalog: {
        context_window: 200_000,
        max_output_tokens: 4_096,
      },
      stateStore,
    });

    // Fire both concurrently
    const sessionId = `${largeFixture.session_id}-concurrent`;
    const frame = makeTurnEndFrame(sessionId, 185_000);
    await Promise.all([handleAsync(iii, frame), handleAsync(iii, frame)]);

    // Only ONE compaction should have been appended despite two concurrent calls
    expect(compactPayloads).toHaveLength(1);
  });
});
