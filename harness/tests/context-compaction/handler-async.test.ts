import { describe, expect, it, vi } from 'vitest';
import {
  extractEventPayload,
  resolveModelFromEvent,
  turnEndUsage,
} from '../../src/context-compaction/handler-async.js';
import type { ISdk } from '../../src/runtime/iii.js';

describe('extractEventPayload', () => {
  it('handles camelCase envelope (event.data shape)', () => {
    const env = {
      groupId: 'sess-1',
      event: { data: { type: 'TurnEnd', message: {} } },
    };
    const out = extractEventPayload(env);
    expect(out?.session_id).toBe('sess-1');
    expect((out?.event as { type: string }).type).toBe('TurnEnd');
  });

  it('handles snake_case envelope (top-level data shape)', () => {
    const env = { group_id: 'sess-2', data: { type: 'TurnEnd' } };
    const out = extractEventPayload(env);
    expect(out?.session_id).toBe('sess-2');
    expect((out?.event as { type: string }).type).toBe('TurnEnd');
  });

  it('returns null when session id is missing', () => {
    expect(extractEventPayload({ data: { type: 'TurnEnd' } })).toBeNull();
    expect(extractEventPayload(null)).toBeNull();
    expect(extractEventPayload(42)).toBeNull();
  });
});

describe('turnEndUsage', () => {
  it('extracts usage on TurnEnd', () => {
    const event = {
      type: 'TurnEnd',
      message: { usage: { input: 100, output: 50, cache_read: 800 } },
    };
    expect(turnEndUsage(event)).toEqual({ input: 100, output: 50, cache_read: 800 });
  });

  it('extracts usage on turn_end (snake_case variant)', () => {
    const event = {
      type: 'turn_end',
      message: { usage: { input: 200, output: 30 } },
    };
    expect(turnEndUsage(event)).toEqual({ input: 200, output: 30 });
  });

  it('returns null for non-TurnEnd events', () => {
    for (const kind of ['TurnStart', 'MessageStart', 'AgentStart']) {
      expect(turnEndUsage({ type: kind, message: { usage: { input: 9999 } } })).toBeNull();
    }
  });

  it('returns null when usage is missing', () => {
    expect(turnEndUsage({ type: 'TurnEnd', message: {} })).toBeNull();
    expect(turnEndUsage({ type: 'TurnEnd' })).toBeNull();
  });
});

describe('resolveModelFromEvent', () => {
  function trackingIii(): { iii: ISdk; calls: string[] } {
    const calls: string[] = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string }) => {
        calls.push(req.function_id);
        // Stand-in for a models::get fallback result.
        return { context_window: 200_000, max_output_tokens: 8_096 };
      }),
    } as unknown as ISdk;
    return { iii, calls };
  }

  it('uses the inline limit and skips models::get', async () => {
    const { iii, calls } = trackingIii();
    const event = {
      type: 'turn_end',
      message: { provider: 'anthropic', model: 'claude-sonnet-4-6' },
      model_limit: { context: 1_000_000, input: 1_000_000, output: 64_000 },
    };

    const resolved = await resolveModelFromEvent(iii, 'sess-1', event);

    expect(calls).not.toContain('models::get');
    expect(resolved).toEqual({
      providerID: 'anthropic',
      modelID: 'claude-sonnet-4-6',
      modelLimit: { context: 1_000_000, input: 1_000_000, output: 64_000 },
    });
  });

  it('falls back to models::get when no inline limit is present', async () => {
    const { iii, calls } = trackingIii();
    const event = {
      type: 'turn_end',
      message: { provider: 'anthropic', model: 'claude-sonnet-4-6' },
    };

    await resolveModelFromEvent(iii, 'sess-1', event);

    expect(calls).toContain('models::get');
  });
});
