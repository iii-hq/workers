import { describe, expect, it } from 'vitest';
import { parseStateListValues } from '../../src/runtime/state.js';

describe('parseStateListValues', () => {
  it('accepts flat array (official iii shape)', () => {
    const rows = [{ session_id: 's1', state: 'stopped' }];
    expect(parseStateListValues(rows)).toEqual(rows);
  });

  it('unwraps { value } rows', () => {
    const inner = { session_id: 's1', state: 'function_awaiting_approval' };
    expect(parseStateListValues([{ value: inner }])).toEqual([inner]);
  });

  it('returns [] for non-array responses', () => {
    expect(parseStateListValues(null)).toEqual([]);
    expect(parseStateListValues({ ok: true })).toEqual([]);
    expect(parseStateListValues({ items: [{ id: 'm1' }] })).toEqual([]);
  });
});
