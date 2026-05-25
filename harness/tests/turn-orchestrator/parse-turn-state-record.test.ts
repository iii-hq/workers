import { describe, expect, it } from 'vitest';
import { newRecord, parseTurnStateRecord } from '../../src/turn-orchestrator/state.js';

describe('parseTurnStateRecord', () => {
  it('returns a valid record for a well-formed turn_state', () => {
    const rec = newRecord('sess-1');
    expect(parseTurnStateRecord(rec)).toEqual(rec);
  });

  it('returns null for null, undefined, and primitives', () => {
    expect(parseTurnStateRecord(null)).toBeNull();
    expect(parseTurnStateRecord(undefined)).toBeNull();
    expect(parseTurnStateRecord('nope')).toBeNull();
    expect(parseTurnStateRecord(42)).toBeNull();
  });

  it('returns null when required identity fields are missing', () => {
    expect(parseTurnStateRecord({ state: 'provisioning' })).toBeNull();
    expect(parseTurnStateRecord({ session_id: 's1' })).toBeNull();
    expect(parseTurnStateRecord({ messages: [] })).toBeNull();
  });

  it('applies defaults for missing scalar fields on partial records', () => {
    const parsed = parseTurnStateRecord({
      session_id: 's1',
      state: 'provisioning',
    });
    expect(parsed).toMatchObject({
      session_id: 's1',
      state: 'provisioning',
      turn_count: 0,
      turn_end_emitted: false,
    });
  });
});
