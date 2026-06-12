import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  _resetOutputTokenCapForTests,
  clampOutputTokens,
  getCatalogModel,
  OUTPUT_TOKEN_MAX,
  OUTPUT_TOKEN_MAX_ENV,
  outputTokenCap,
} from '../../src/runtime/output-tokens.js';
import type { ISdk } from '../../src/runtime/iii.js';

afterEach(() => {
  delete process.env[OUTPUT_TOKEN_MAX_ENV];
  _resetOutputTokenCapForTests();
});

describe('outputTokenCap', () => {
  it('defaults to 32_000', () => {
    expect(outputTokenCap()).toBe(OUTPUT_TOKEN_MAX);
  });

  it('reads a valid env override', () => {
    process.env[OUTPUT_TOKEN_MAX_ENV] = '16000';
    expect(outputTokenCap()).toBe(16_000);
  });

  it.each(['abc', '0', '-5', 'NaN'])('falls back to 32_000 on invalid env %s', (v) => {
    process.env[OUTPUT_TOKEN_MAX_ENV] = v;
    expect(outputTokenCap()).toBe(OUTPUT_TOKEN_MAX);
  });
});

describe('clampOutputTokens precedence', () => {
  const workerDefault = 8_192;

  it('defaults to min(modelMax, cap): model below cap', () => {
    expect(clampOutputTokens({ modelMaxOutput: 16_000, userOverride: null, workerDefault })).toBe(
      16_000,
    );
  });

  it('defaults to min(modelMax, cap): model above cap clamps to 32k', () => {
    expect(clampOutputTokens({ modelMaxOutput: 64_000, userOverride: null, workerDefault })).toBe(
      32_000,
    );
  });

  it('user override wins below the model ceiling', () => {
    expect(clampOutputTokens({ modelMaxOutput: 64_000, userOverride: 50_000, workerDefault })).toBe(
      50_000,
    );
  });

  it('user override is clamped down to the model ceiling', () => {
    expect(
      clampOutputTokens({ modelMaxOutput: 64_000, userOverride: 100_000, workerDefault }),
    ).toBe(64_000);
  });

  it('user override is NOT capped to 32k (deliberate choice)', () => {
    expect(
      clampOutputTokens({ modelMaxOutput: 128_000, userOverride: 64_000, workerDefault }),
    ).toBe(64_000);
  });

  it('user override passes through when model is unknown', () => {
    expect(
      clampOutputTokens({ modelMaxOutput: undefined, userOverride: 4_096, workerDefault }),
    ).toBe(4_096);
  });

  it.each([0, undefined, null])('unknown model max (%s) falls back to workerDefault', (v) => {
    expect(clampOutputTokens({ modelMaxOutput: v, userOverride: null, workerDefault })).toBe(
      workerDefault,
    );
  });

  it('honors an explicit cap argument over the env cap', () => {
    expect(
      clampOutputTokens({ modelMaxOutput: 64_000, userOverride: null, workerDefault, cap: 16_000 }),
    ).toBe(16_000);
  });

  it('env cap raises the default for high-output models', () => {
    process.env[OUTPUT_TOKEN_MAX_ENV] = '64000';
    expect(clampOutputTokens({ modelMaxOutput: 128_000, userOverride: null, workerDefault })).toBe(
      64_000,
    );
  });

  it('floors fractional values so token counts stay integers', () => {
    expect(clampOutputTokens({ modelMaxOutput: 64_000, userOverride: 1024.5, workerDefault })).toBe(
      1024,
    );
    expect(clampOutputTokens({ modelMaxOutput: undefined, userOverride: 1.5, workerDefault })).toBe(
      1,
    );
  });

  it.each([0, -1])('ignores a non-positive userOverride (%s)', (ov) => {
    expect(clampOutputTokens({ modelMaxOutput: 64_000, userOverride: ov, workerDefault })).toBe(
      32_000,
    );
    expect(clampOutputTokens({ modelMaxOutput: undefined, userOverride: ov, workerDefault })).toBe(
      workerDefault,
    );
  });

  it('rejects scientific-notation env values instead of truncating them', () => {
    // parseInt("1e9") === 1 — the strict digits-only parse must fall back to 32k.
    process.env[OUTPUT_TOKEN_MAX_ENV] = '1e9';
    expect(outputTokenCap()).toBe(OUTPUT_TOKEN_MAX);
  });
});

describe('getCatalogModel', () => {
  function fakeIii(trigger: (req: unknown) => Promise<unknown>): ISdk {
    return { trigger } as unknown as ISdk;
  }

  it('returns the catalog entry from router::models::get', async () => {
    const entry = { id: 'claude-sonnet-4-6', provider: 'anthropic', max_output_tokens: 64_000 };
    const trigger = vi.fn().mockResolvedValue({ model: entry });
    const out = await getCatalogModel(fakeIii(trigger), 'anthropic', 'claude-sonnet-4-6');
    expect(out).toEqual(entry);
    expect(trigger).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'router::models::get',
        payload: { provider: 'anthropic', id: 'claude-sonnet-4-6' },
      }),
    );
  });

  it('returns null when the model is unknown', async () => {
    const out = await getCatalogModel(fakeIii(vi.fn().mockResolvedValue(null)), 'anthropic', 'x');
    expect(out).toBeNull();
  });

  it('returns null when the bus call throws', async () => {
    const out = await getCatalogModel(
      fakeIii(vi.fn().mockRejectedValue(new Error('timeout'))),
      'anthropic',
      'x',
    );
    expect(out).toBeNull();
  });
});
