import { describe, expect, it } from 'vitest';
import { parseModelArray, providerStateKey } from '../../src/models-catalog/state.js';

describe('provider catalog state helpers', () => {
  it('providerStateKey uses bare provider id', () => {
    expect(providerStateKey('anthropic')).toBe('anthropic');
  });

  it('parseModelArray accepts only Model[]', () => {
    const model = {
      id: 'm1',
      provider: 'anthropic',
      api: 'anthropic-messages',
      display_name: 'm1',
      context_window: 1,
    };
    expect(parseModelArray([model])).toEqual([model]);
    expect(parseModelArray(model)).toEqual([]);
    expect(parseModelArray(null)).toEqual([]);
    expect(parseModelArray('bad')).toEqual([]);
  });
});
