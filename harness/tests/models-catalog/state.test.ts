import { describe, expect, it } from 'vitest';
import { isModel, providerStateKey } from '../../src/models-catalog/state.js';

describe('provider catalog state helpers', () => {
  it('providerStateKey uses bare provider id', () => {
    expect(providerStateKey('anthropic')).toBe('anthropic');
  });

  it('isModel guards the write-side boundary (object with a string id)', () => {
    const model = {
      id: 'm1',
      provider: 'anthropic',
      api: 'anthropic-messages',
      display_name: 'm1',
      context_window: 1,
    };
    expect(isModel(model)).toBe(true);
    expect(isModel({ provider: 'anthropic' })).toBe(false);
    expect(isModel(null)).toBe(false);
    expect(isModel('bad')).toBe(false);
  });
});
