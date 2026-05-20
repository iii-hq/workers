import { describe, expect, it } from 'vitest';
import { parseStateListKeyedEntries, parseStateListValues } from '../../src/runtime/state.js';

describe('parseStateListValues', () => {
  it('accepts flat array (official iii shape)', () => {
    const rows = [{ session_id: 's1', state: 'stopped' }];
    expect(parseStateListValues(rows)).toEqual(rows);
  });

  it('unwraps { value } rows', () => {
    const inner = { session_id: 's1', state: 'function_awaiting_approval' };
    expect(parseStateListValues([{ value: inner }])).toEqual([inner]);
  });

  it('accepts { items: [...] } envelope', () => {
    const inner = { id: 'm1' };
    expect(parseStateListValues({ items: [inner, { value: { id: 'm2' } }] })).toEqual([
      inner,
      { id: 'm2' },
    ]);
  });

  it('returns [] for non-array responses', () => {
    expect(parseStateListValues(null)).toEqual([]);
    expect(parseStateListValues({ ok: true })).toEqual([]);
  });
});

describe('parseStateListKeyedEntries', () => {
  it('preserves key when present', () => {
    expect(
      parseStateListKeyedEntries({
        items: [{ key: 'session/s1/turn_state', value: { state: 'stopped' } }],
      }),
    ).toEqual([{ key: 'session/s1/turn_state', value: { state: 'stopped' } }]);
  });
});
