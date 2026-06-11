/**
 * Backward-compatibility integration test.
 *
 * Verifies that when session-tree::compactions returns a legacy entry with
 * a non-null summary string (anchored update path), the summariser receives
 * a system_prompt containing <previous-summary> with the prior summary text.
 *
 * Also verifies that session-tree::compact is called for the second compaction.
 */
import { describe, expect, it, vi } from 'vitest';
import { handleAsync } from '../../../src/context-compaction/handler-async.js';
import type { AssistantMessage } from '../../../src/types/agent-message.js';
import { loadFixture } from '../../fixtures/load.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type MockTriggerReq = { function_id: string; payload: Record<string, unknown>; timeoutMs?: number };

const PRIOR_SUMMARY = 'Prior compaction summary: user asked about X and we explored Y.';

function buildBackwardCompatMock(opts: {
  fixtureMessages: Array<{ entry_id: string; message: unknown }>;
  capturedSystemPrompts: string[];
  compactPayloads: unknown[];
}) {
  const { fixtureMessages, capturedSystemPrompts, compactPayloads } = opts;

  const stateStore = new Map<string, unknown>();
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

  const trigger = vi.fn(async (req: MockTriggerReq) => {
    const { function_id, payload } = req;

    if (function_id === 'session-tree::messages') {
      return { messages: fixtureMessages };
    }
    if (function_id === 'session-tree::compactions') {
      // Return a legacy-style entry: tail_start_id is absent (old schema)
      // with a free-form summary string — this is the backward-compat scenario.
      return {
        entries: [
          {
            id: 'comp-legacy-1',
            summary: PRIOR_SUMMARY,
            tokens_before: 80_000,
            timestamp: Date.now() - 120_000,
            // Intentionally omitting tail_start_id (old schema)
          },
        ],
      };
    }
    if (function_id === 'session-tree::compact') {
      compactPayloads.push(payload);
      return undefined;
    }
    if (function_id === 'session-tree::update_part') {
      return { ok: true };
    }
    if (function_id === 'models::get') {
      return {
        id: 'claude-haiku-4-5',
        context_window: 200_000,
        max_output_tokens: 4_096,
      };
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
    if (function_id.startsWith('provider::')) {
      // Capture the system_prompt sent to the summariser
      const systemPrompt = payload.system_prompt;
      if (typeof systemPrompt === 'string') {
        capturedSystemPrompts.push(systemPrompt);
      }

      // Deliver a successful summary via the channel
      if (channelCb) {
        const msg: AssistantMessage = {
          role: 'assistant',
          content: [{ type: 'text', text: 'Updated anchored summary.' }],
          stop_reason: 'end',
          model: 'claude-haiku-4-5',
          provider: 'anthropic',
          timestamp: Date.now(),
        };
        channelCb(JSON.stringify({ type: 'done', message: msg }));
      }
      return undefined;
    }

    return undefined;
  });

  const createChannel = vi.fn(async () => channel);

  const iii = { trigger, createChannel } as unknown as import('../../../src/runtime/iii.js').ISdk;

  return { iii, trigger };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const largeFixture = loadFixture('large-with-media');

describe('backward-compat: prior compaction (anchored update path)', () => {
  it('includes <previous-summary> in system_prompt when a legacy compaction entry exists', async () => {
    const capturedSystemPrompts: string[] = [];
    const compactPayloads: unknown[] = [];

    const fixtureMessages = largeFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message,
    }));

    const { iii } = buildBackwardCompatMock({
      fixtureMessages,
      capturedSystemPrompts,
      compactPayloads,
    });

    // Fire a turn_end that triggers overflow
    const frame = {
      session_id: `${largeFixture.session_id}-compat`,
      provider: 'anthropic',
      model: 'claude-haiku-4-5',
      usage: { input: 185_000, output: 0, cache_read: 0, cache_write: 0 },
    };

    await handleAsync(iii, frame);

    // The system_prompt must contain the anchored <previous-summary> block
    expect(capturedSystemPrompts).toHaveLength(1);
    const sysPrompt = capturedSystemPrompts[0]!;
    expect(sysPrompt).toContain('<previous-summary>');
    expect(sysPrompt).toContain(PRIOR_SUMMARY);
    expect(sysPrompt).toContain('</previous-summary>');
  });

  it('calls session-tree::compact for the second (updated) compaction', async () => {
    const capturedSystemPrompts: string[] = [];
    const compactPayloads: unknown[] = [];

    const fixtureMessages = largeFixture.entries.map((e) => ({
      entry_id: e.id,
      message: e.message,
    }));

    const { iii } = buildBackwardCompatMock({
      fixtureMessages,
      capturedSystemPrompts,
      compactPayloads,
    });

    const frame = {
      session_id: `${largeFixture.session_id}-compat2`,
      provider: 'anthropic',
      model: 'claude-haiku-4-5',
      usage: { input: 185_000, output: 0, cache_read: 0, cache_write: 0 },
    };

    await handleAsync(iii, frame);

    // Compact must be called to persist the second (updated) compaction entry
    expect(compactPayloads.length).toBeGreaterThan(0);
    const cp = compactPayloads[0] as Record<string, unknown>;
    expect(typeof cp.summary).toBe('string');
    expect((cp.summary as string).length).toBeGreaterThan(0);
  });
});
