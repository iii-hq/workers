import { describe, expect, it } from 'vitest';
import type { Model } from '../../src/models-catalog/types.js';
import { buildThinkingConfig, MIN_THINKING_BUDGET } from '../../src/provider-anthropic/thinking.js';

function model(max_output_tokens?: number, extra: Partial<Model> = {}): Model {
  return {
    id: 'claude-sonnet-4-6',
    provider: 'anthropic',
    api: 'anthropic-messages',
    display_name: 'Claude Sonnet 4.6',
    context_window: 200_000,
    ...(max_output_tokens !== undefined ? { max_output_tokens } : {}),
    ...extra,
  };
}

describe('buildThinkingConfig', () => {
  it.each([undefined, 'off'])('returns undefined for level %s', (level) => {
    expect(buildThinkingConfig(level, 32_000, model(64_000))).toBeUndefined();
  });

  it('returns undefined when the model output ceiling is unknown', () => {
    expect(buildThinkingConfig('high', 32_000, model())).toBeUndefined();
    expect(buildThinkingConfig('high', 32_000, undefined)).toBeUndefined();
  });

  describe('formula budgets (no catalog budgets)', () => {
    // high -> min(16_000, floor(output/2 - 1))
    it.each([
      [8_192, 4_095],
      [32_000, 15_999],
      [64_000, 16_000],
    ])('high with output %i -> budget %i', (output, budget) => {
      expect(buildThinkingConfig('high', 32_000, model(output))).toEqual({
        type: 'enabled',
        budget_tokens: budget,
      });
    });

    // max/xhigh -> min(31_999, output - 1), then clamped to leave output room
    it('xhigh with output 64_000 reserves output room below max_tokens', () => {
      expect(buildThinkingConfig('xhigh', 32_000, model(64_000))).toEqual({
        type: 'enabled',
        budget_tokens: 32_000 - 1_024,
      });
    });

    it('medium with output 64_000 -> min(8_000, output/4)', () => {
      expect(buildThinkingConfig('medium', 32_000, model(64_000))).toEqual({
        type: 'enabled',
        budget_tokens: 8_000,
      });
    });

    it('low with output 64_000 -> min(4_000, output/8)', () => {
      expect(buildThinkingConfig('low', 32_000, model(64_000))).toEqual({
        type: 'enabled',
        budget_tokens: 4_000,
      });
    });
  });

  it('prefers catalog thinking_budgets over the formula', () => {
    const m = model(64_000, { thinking_budgets: { high: 20_000 } });
    expect(buildThinkingConfig('high', 32_000, m)).toEqual({
      type: 'enabled',
      budget_tokens: 20_000,
    });
  });

  it('never returns budget >= max_tokens and always reserves output room', () => {
    const cfg = buildThinkingConfig('xhigh', 16_000, model(64_000));
    expect(cfg).toEqual({ type: 'enabled', budget_tokens: 16_000 - 1_024 });
  });

  it('drops thinking when the clamped budget falls below the API minimum', () => {
    // max_tokens 2047 leaves budget 1023 (< MIN_THINKING_BUDGET) after the reserve
    expect(buildThinkingConfig('high', 2_047, model(64_000))).toBeUndefined();
    expect(buildThinkingConfig('high', MIN_THINKING_BUDGET, model(64_000))).toBeUndefined();
  });

  it('drops thinking for unknown levels', () => {
    expect(buildThinkingConfig('turbo', 32_000, model(64_000))).toBeUndefined();
  });

  it('drops thinking when the catalog says the model cannot think', () => {
    expect(
      buildThinkingConfig('high', 32_000, model(64_000, { supports_thinking: false })),
    ).toBeUndefined();
    // unknown flag stays permissive
    expect(buildThinkingConfig('high', 32_000, model(64_000))).toBeDefined();
  });

  it('degrades xhigh to the high tier when the catalog says xhigh is unsupported', () => {
    expect(buildThinkingConfig('xhigh', 32_000, model(64_000, { supports_xhigh: false }))).toEqual({
      type: 'enabled',
      budget_tokens: 16_000,
    });
    // unknown flag keeps the xhigh formula
    expect(buildThinkingConfig('xhigh', 32_000, model(64_000))).toEqual({
      type: 'enabled',
      budget_tokens: 32_000 - 1_024,
    });
  });
});
