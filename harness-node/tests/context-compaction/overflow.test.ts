// tests/context-compaction/overflow.test.ts
import { describe, expect, it } from 'vitest';
import {
  usable,
  isOverflow,
  preserveRecentBudget,
} from '../../src/context-compaction/overflow.js';

const MODEL_INPUT_LIMITED = {
  id: 'm1',
  limit: { context: 200_000, input: 180_000, output: 20_000 },
};
const MODEL_CONTEXT_ONLY = {
  id: 'm2',
  limit: { context: 100_000, input: 0, output: 8_000 },
};

describe('usable', () => {
  it('returns input - reserved for input-limited models', () => {
    expect(usable({ model: MODEL_INPUT_LIMITED, reserved: 20_000 })).toBe(160_000);
  });
  it('returns context - maxOutput for context-only models', () => {
    expect(usable({ model: MODEL_CONTEXT_ONLY, reserved: 20_000 })).toBe(92_000);
  });
  it('returns 0 for a zero-context model', () => {
    const m = { id: 'm3', limit: { context: 0, input: 0, output: 0 } };
    expect(usable({ model: m, reserved: 1000 })).toBe(0);
  });
});

describe('isOverflow', () => {
  it('false when auto disabled', () => {
    expect(
      isOverflow({
        tokens: { input: 200_000, output: 0, cache_read: 0, cache_write: 0 },
        model: MODEL_INPUT_LIMITED,
        reserved: 20_000,
        autoDisabled: true,
      }),
    ).toBe(false);
  });
  it('false for zero-context model', () => {
    const m = { id: 'z', limit: { context: 0, input: 0, output: 0 } };
    expect(
      isOverflow({
        tokens: { input: 50_000 },
        model: m,
        reserved: 1000,
      }),
    ).toBe(false);
  });
  it('sums tokens.total when present', () => {
    expect(
      isOverflow({
        tokens: { total: 161_000 },
        model: MODEL_INPUT_LIMITED,
        reserved: 20_000,
      }),
    ).toBe(true);
  });
  it('falls back to input + output + cache_read + cache_write', () => {
    expect(
      isOverflow({
        tokens: { input: 100_000, output: 50_000, cache_read: 10_000, cache_write: 1_000 },
        model: MODEL_INPUT_LIMITED,
        reserved: 20_000,
      }),
    ).toBe(true);
  });
});

describe('preserveRecentBudget', () => {
  it('clamps to MIN_PRESERVE when usable * 0.25 too small', () => {
    const tiny = { id: 't', limit: { context: 8000, input: 6000, output: 2000 } };
    expect(preserveRecentBudget({ model: tiny, reserved: 1000 })).toBe(2_000);
  });
  it('clamps to MAX_PRESERVE when usable * 0.25 too large', () => {
    expect(preserveRecentBudget({ model: MODEL_INPUT_LIMITED, reserved: 20_000 })).toBe(8_000);
  });
  it('honors override', () => {
    expect(
      preserveRecentBudget({ model: MODEL_INPUT_LIMITED, reserved: 20_000, override: 5_000 }),
    ).toBe(5_000);
  });
});
