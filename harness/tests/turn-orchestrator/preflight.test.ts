/**
 * Smoke tests for runPreflight.
 *
 * The integration (pre-flight wired into handleStreaming) is covered by the
 * phase-11 flow-sync integration tests. These unit tests validate the
 * preflight helper's branching logic in isolation.
 */

import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { CompactionBusyError, ContextOverflowError } from '../../src/turn-orchestrator/errors.js';
import { runPreflight } from '../../src/turn-orchestrator/preflight.js';

type TriggerCall = { function_id: string; payload: unknown };

function makeIii(overrides?: {
  modelsGetResult?: unknown;
  sessionTreeResult?: unknown;
  compactNowResult?: unknown;
}): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];

  const iii = {
    trigger: async <_T, R>(req: { function_id: string; payload: unknown }): Promise<R> => {
      calls.push({ function_id: req.function_id, payload: req.payload });

      if (req.function_id === 'router::models::get') {
        const m = overrides?.modelsGetResult ?? null;
        return (m ? { model: m } : null) as R;
      }
      if (req.function_id === 'session-tree::messages') {
        return (overrides?.sessionTreeResult ?? { messages: [] }) as R;
      }
      if (req.function_id === 'context-compaction::compact_now') {
        return (overrides?.compactNowResult ?? { status: 'ok' }) as R;
      }
      return null as R;
    },
  } as unknown as ISdk;

  return { iii, calls };
}

const smallMessage = { role: 'user' as const, content: [] as never[], timestamp: 0 };

describe('runPreflight', () => {
  it('returns ok without calling compact_now when under the limit', async () => {
    const { iii, calls } = makeIii({
      modelsGetResult: { context_window: 200_000, max_output_tokens: 8_096 },
    });

    const result = await runPreflight(iii, 'session-1', [smallMessage], 'anthropic', 'claude-3');

    expect(result).toBe('ok');
    expect(calls.some((c) => c.function_id === 'context-compaction::compact_now')).toBe(false);
  });

  it('returns ok and skips compact_now when router::models::get returns null', async () => {
    const { iii, calls } = makeIii({ modelsGetResult: null });

    const result = await runPreflight(iii, 'session-1', [smallMessage], 'anthropic', 'claude-3');

    expect(result).toBe('ok');
    expect(calls.some((c) => c.function_id === 'context-compaction::compact_now')).toBe(false);
  });

  it('calls compact_now and returns compacted when projected >= usable', async () => {
    const { iii, calls } = makeIii({
      modelsGetResult: { context_window: 10, max_output_tokens: 0 },
      compactNowResult: { status: 'ok' },
    });

    const result = await runPreflight(iii, 'session-1', [smallMessage], 'anthropic', 'claude-3');

    expect(result).toBe('compacted');
    expect(calls.some((c) => c.function_id === 'context-compaction::compact_now')).toBe(true);
  });

  it('throws ContextOverflowError when compact_now returns overflow', async () => {
    const { iii } = makeIii({
      modelsGetResult: { context_window: 10, max_output_tokens: 0 },
      compactNowResult: { status: 'overflow' },
    });

    await expect(
      runPreflight(iii, 'session-1', [smallMessage], 'anthropic', 'claude-3'),
    ).rejects.toBeInstanceOf(ContextOverflowError);
  });

  it('throws CompactionBusyError when compact_now returns busy', async () => {
    const { iii } = makeIii({
      modelsGetResult: { context_window: 10, max_output_tokens: 0 },
      compactNowResult: { status: 'busy' },
    });

    await expect(
      runPreflight(iii, 'session-1', [smallMessage], 'anthropic', 'claude-3'),
    ).rejects.toBeInstanceOf(CompactionBusyError);
  });

  it("returns ok without claiming compacted when compact_now reports 'empty'", async () => {
    const { iii } = makeIii({
      modelsGetResult: { context_window: 10, max_output_tokens: 0 },
      compactNowResult: { status: 'empty' },
    });

    const result = await runPreflight(iii, 'session-1', [smallMessage], 'anthropic', 'claude-3');

    expect(result).toBe('ok');
  });

  it('returns ok on an unknown compact_now status instead of claiming compacted', async () => {
    const { iii } = makeIii({
      modelsGetResult: { context_window: 10, max_output_tokens: 0 },
      compactNowResult: { status: 'something-new' },
    });

    const result = await runPreflight(iii, 'session-1', [smallMessage], 'anthropic', 'claude-3');

    expect(result).toBe('ok');
  });

  it('uses the pre-resolved model and skips router::models::get', async () => {
    const { iii, calls } = makeIii({
      modelsGetResult: { context_window: 1, max_output_tokens: 0 },
    });
    const modelMeta = {
      id: 'claude-3',
      provider: 'anthropic',
      api: 'anthropic-messages',
      display_name: 'Claude 3',
      context_window: 200_000,
      max_output_tokens: 8_096,
    };

    const result = await runPreflight(
      iii,
      'session-1',
      [smallMessage],
      'anthropic',
      'claude-3',
      modelMeta,
    );

    expect(result).toBe('ok');
    expect(calls.some((c) => c.function_id === 'router::models::get')).toBe(false);
    expect(calls.some((c) => c.function_id === 'context-compaction::compact_now')).toBe(false);
  });

  it('fetches router::models::get when no pre-resolved model is threaded', async () => {
    const { iii, calls } = makeIii({
      modelsGetResult: { context_window: 200_000, max_output_tokens: 8_096 },
    });

    const result = await runPreflight(iii, 'session-1', [smallMessage], 'anthropic', 'claude-3');

    expect(result).toBe('ok');
    expect(calls.some((c) => c.function_id === 'router::models::get')).toBe(true);
  });

  it('passes session_id and model info to compact_now', async () => {
    const { iii, calls } = makeIii({
      modelsGetResult: { context_window: 10, max_output_tokens: 0 },
      compactNowResult: { status: 'ok' },
    });

    await runPreflight(iii, 'my-session', [smallMessage], 'openai', 'gpt-4o');

    const compactCall = calls.find((c) => c.function_id === 'context-compaction::compact_now');
    expect(compactCall).toBeDefined();
    const p = compactCall?.payload as Record<string, unknown>;
    expect(p.session_id).toBe('my-session');
    expect((p.model as Record<string, unknown>).id).toBe('gpt-4o');
    expect((p.model as Record<string, unknown>).providerID).toBe('openai');
  });
});
