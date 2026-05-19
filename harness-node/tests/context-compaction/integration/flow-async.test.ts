/**
 * Integration tests: async (TurnEnd-driven) compaction flow via handleAsync.
 *
 * Tests:
 * 1. Below-threshold TurnEnd → no compaction.
 * 2. Above-threshold TurnEnd (large fixture) → compaction runs, session-tree::compact called.
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
 * @param fixtureMessages - entries to return from session-tree::messages
 * @param compactPayloads - collector array for session-tree::compact calls
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

  let channelCb: ((raw: string) => void) | null = null;

  const channel = {
    reader: {
      onMessage(cb: (raw: string) => void) {
        channelCb = cb;
      },
      stream: { resume: () => {} },
    },
    writerRef: 'mock-writer-ref',
  };

  const doneSummaryEvent = JSON.stringify({
    type: 'done',
    message: {
      role: 'assistant',
      content: [{ type: 'text', text: 'Async compaction summary.' }],
      stop_reason: 'end',
      model: 'claude-haiku-4-5',
      provider: 'anthropic',
      timestamp: Date.now(),
    },
  });

  const trigger = vi.fn(async (req: MockTriggerReq) => {
    const { function_id, payload } = req;

    if (function_id === 'session-tree::messages') {
      return { messages: fixtureMessages };
    }
    if (function_id === 'session-tree::compactions') {
      return { entries: [] };
    }
    if (function_id === 'session-tree::compact') {
      compactPayloads.push(payload);
      return undefined;
    }
    if (function_id === 'session-tree::update_part') {
      return { ok: true };
    }
    if (function_id === 'models::get') {
      return modelCatalog;
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
    if (function_id.startsWith('provider::')) {
      if (channelCb) {
        channelCb(doneSummaryEvent);
      }
      return undefined;
    }

    return undefined;
  });

  const createChannel = vi.fn(async () => channel);

  const iii = { trigger, createChannel } as unknown as import('../../../src/runtime/iii.js').ISdk;

  return { iii, trigger, createChannel, compactPayloads };
}

/**
 * Build a TurnEnd frame that handleAsync can process.
 *
 * @param sessionId - the session identifier
 * @param totalTokens - total token usage to report (drives overflow check)
 * @param modelId - model identifier (default: claude-haiku-4-5)
 */
function makeTurnEndFrame(sessionId: string, totalTokens: number, modelId = 'claude-haiku-4-5') {
  return {
    groupId: sessionId,
    event: {
      data: {
        type: 'TurnEnd',
        message: {
          role: 'assistant',
          model: modelId,
          provider: 'anthropic',
          usage: {
            input: totalTokens,
            output: 0,
            cache_read: 0,
            cache_write: 0,
          },
        },
      },
    },
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const smallFixture = loadFixture('tiny');
const largeFixture = loadFixture('large-with-media');

describe('flow-async: below-threshold TurnEnd', () => {
  it('does not call session-tree::compact when token usage is below overflow threshold', async () => {
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

    // Also, session-tree::compact should not appear in trigger calls
    const compactCalls = trigger.mock.calls.filter(
      ([req]) => req.function_id === 'session-tree::compact',
    );
    expect(compactCalls).toHaveLength(0);
  });
});

describe('flow-async: above-threshold TurnEnd with large fixture', () => {
  it('runs compaction and calls session-tree::compact when tokens exceed overflow threshold', async () => {
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

    // session-tree::compact MUST have been called at least once
    expect(compactPayloads.length).toBeGreaterThan(0);

    // Verify the compact payload shape
    const cp = compactPayloads[0] as Record<string, unknown>;
    expect(cp.session_id).toBe(largeFixture.session_id);
    expect(typeof cp.summary).toBe('string');
    expect((cp.summary as string).length).toBeGreaterThan(0);
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
