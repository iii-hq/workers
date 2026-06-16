import { describe, expect, it, vi } from 'vitest';
import {
  composeMigratedValue,
  migrateLlmRouterConfig,
  ROUTING_PARITY_SEED,
} from '../../src/harness/migrate-llm-router-config.js';
import type { ISdk } from '../../src/runtime/iii.js';

type TriggerReq = { function_id: string; payload: Record<string, unknown> };

function makeIii(opts: {
  marker?: unknown;
  routerValue?: unknown;
  harnessValue?: unknown;
  failSets?: number;
}) {
  const sets: unknown[] = [];
  const markerWrites: unknown[] = [];
  let setFailures = opts.failSets ?? 0;
  const trigger = vi.fn(async (req: TriggerReq) => {
    const { function_id, payload } = req;
    if (function_id === 'state::get') {
      return opts.marker ?? null;
    }
    if (function_id === 'state::set') {
      markerWrites.push(payload.value);
      return { ok: true };
    }
    if (function_id === 'configuration::get') {
      if (payload.id === 'llm-router') return { id: 'llm-router', value: opts.routerValue ?? null };
      if (payload.id === 'harness') return { id: 'harness', value: opts.harnessValue ?? null };
      return null;
    }
    if (function_id === 'configuration::set') {
      if (setFailures > 0) {
        setFailures -= 1;
        throw new Error('schema validation failed: unknown entry');
      }
      sets.push(payload);
      return { ok: true };
    }
    return null;
  });
  const iii = { trigger } as unknown as ISdk;
  return { iii, trigger, sets, markerWrites };
}

describe('composeMigratedValue', () => {
  it('copies providers and seeds routing parity', () => {
    const out = composeMigratedValue(null, { anthropic: { api_key: 'sk-1' } });
    expect(out.providers).toEqual({ anthropic: { api_key: 'sk-1' } });
    expect(out.default_provider).toBe(ROUTING_PARITY_SEED.default_provider);
    expect(out.routing_heuristics).toEqual(ROUTING_PARITY_SEED.routing_heuristics);
  });

  it('never clobbers operator-set routing or provider slices', () => {
    const out = composeMigratedValue(
      {
        default_provider: 'openai',
        routing_heuristics: [{ pattern: '^x-', provider: 'kimi' }],
        providers: { anthropic: { api_key: 'operator-key' } },
      },
      { anthropic: { api_key: 'legacy-key' }, openai: { api_key: 'sk-o' } },
    );
    expect(out.default_provider).toBe('openai');
    expect(out.routing_heuristics).toEqual([{ pattern: '^x-', provider: 'kimi' }]);
    // Operator slice wins over the legacy copy; missing slices are added.
    expect(out.providers).toEqual({
      anthropic: { api_key: 'operator-key' },
      openai: { api_key: 'sk-o' },
    });
  });
});

describe('migrateLlmRouterConfig', () => {
  it('no-ops when the marker is already set', async () => {
    const { iii, sets, trigger } = makeIii({ marker: { migrated_at: '2026-06-01' } });
    await migrateLlmRouterConfig(iii);
    expect(sets).toHaveLength(0);
    expect(
      trigger.mock.calls.some(([req]) => (req as TriggerReq).function_id === 'configuration::set'),
    ).toBe(false);
  });

  it('treats an operator-populated llm-router entry as migrated', async () => {
    const { iii, sets, markerWrites } = makeIii({
      routerValue: { providers: { anthropic: { api_key: 'sk-op' } } },
    });
    await migrateLlmRouterConfig(iii);
    expect(sets).toHaveLength(0);
    expect(markerWrites).toHaveLength(1);
  });

  it('copies the harness providers block, seeds parity, and writes the marker', async () => {
    const { iii, sets, markerWrites } = makeIii({
      harnessValue: {
        permissions: { default_mode: 'manual' },
        providers: { anthropic: { api_key: '${ANTHROPIC_API_KEY:}' } },
      },
    });
    await migrateLlmRouterConfig(iii);

    expect(sets).toHaveLength(1);
    const set = sets[0] as { id: string; value: Record<string, unknown> };
    expect(set.id).toBe('llm-router');
    // Raw template form survives the copy verbatim.
    expect(set.value.providers).toEqual({ anthropic: { api_key: '${ANTHROPIC_API_KEY:}' } });
    expect(set.value.default_provider).toBe('anthropic');
    expect(markerWrites).toHaveLength(1);
  });

  it('retries the set while the entry schema is still composing', async () => {
    vi.useFakeTimers();
    try {
      const { iii, sets, markerWrites } = makeIii({
        harnessValue: { providers: { anthropic: { api_key: 'sk-1' } } },
        failSets: 1,
      });
      const run = migrateLlmRouterConfig(iii);
      await vi.advanceTimersByTimeAsync(5_000);
      await run;
      expect(sets).toHaveLength(1);
      expect(markerWrites).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
