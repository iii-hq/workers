import { describe, expect, it, vi } from 'vitest';
import { parseOnTurnEnd, resolveModel } from '../../src/context-compaction/handler-async.js';
import type { ISdk } from '../../src/runtime/iii.js';

describe('parseOnTurnEnd', () => {
  it('parses a well-formed turn_end payload', () => {
    const out = parseOnTurnEnd({
      session_id: 'sess-1',
      usage: { input: 100, output: 50, cache_read: 800 },
      provider: 'anthropic',
      model: 'claude-haiku-4-5',
    });
    expect(out).toEqual({
      session_id: 'sess-1',
      usage: { input: 100, output: 50, cache_read: 800 },
      provider: 'anthropic',
      model: 'claude-haiku-4-5',
    });
  });

  it('defaults provider/model to empty strings when absent (handler falls back to session-tree)', () => {
    const out = parseOnTurnEnd({ session_id: 'sess-2', usage: { input: 10 } });
    expect(out).toEqual({ session_id: 'sess-2', usage: { input: 10 }, provider: '', model: '' });
  });

  it('returns null usage when usage is missing or malformed', () => {
    expect(parseOnTurnEnd({ session_id: 'sess-3' })?.usage).toBeNull();
    expect(parseOnTurnEnd({ session_id: 'sess-3', usage: 'nope' })?.usage).toBeNull();
  });

  it('returns null when session_id is missing', () => {
    expect(parseOnTurnEnd({ usage: { input: 1 } })).toBeNull();
    expect(parseOnTurnEnd({ session_id: '' })).toBeNull();
    expect(parseOnTurnEnd(null)).toBeNull();
    expect(parseOnTurnEnd(42)).toBeNull();
  });

  it('passes a threaded model_limit through and drops a malformed one', () => {
    const limit = { context: 200_000, input: 200_000, output: 64_000 };
    expect(parseOnTurnEnd({ session_id: 'sess-4', model_limit: limit })?.model_limit).toEqual(
      limit,
    );
    expect(parseOnTurnEnd({ session_id: 'sess-4', model_limit: 'nope' })).not.toHaveProperty(
      'model_limit',
    );
  });
});

describe('resolveModel', () => {
  function trackingIii(): { iii: ISdk; calls: string[] } {
    const calls: string[] = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string }) => {
        calls.push(req.function_id);
        return { model: { context_window: 200_000, max_output_tokens: 8_096 } };
      }),
    } as unknown as ISdk;
    return { iii, calls };
  }

  it('uses the threaded limit and skips router::models::get', async () => {
    const { iii, calls } = trackingIii();

    const resolved = await resolveModel(iii, 'sess-1', 'anthropic', 'claude-sonnet-4-6', {
      context: 1_000_000,
      input: 1_000_000,
      output: 64_000,
    });

    expect(calls).not.toContain('router::models::get');
    expect(resolved).toEqual({
      providerID: 'anthropic',
      modelID: 'claude-sonnet-4-6',
      modelLimit: { context: 1_000_000, input: 1_000_000, output: 64_000 },
    });
  });

  it('falls back to router::models::get when no threaded limit is present', async () => {
    const { iii, calls } = trackingIii();

    await resolveModel(iii, 'sess-1', 'anthropic', 'claude-sonnet-4-6');

    expect(calls).toContain('router::models::get');
  });
});
