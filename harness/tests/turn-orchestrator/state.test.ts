import { describe, expect, it } from 'vitest';
import type { TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import {
  AGENT_SCOPE,
  messagesKey,
  newRecord,
  transitionTo,
  turnStateKey,
} from '../../src/turn-orchestrator/state.js';

describe('TurnStateRecord', () => {
  it('starts in provisioning with no work and the given max_turns', () => {
    const r = newRecord('s1', 32);
    expect(r.state).toBe('provisioning');
    expect(r.session_id).toBe('s1');
    expect(r.max_turns).toBe(32);
    expect(r.work).toBeUndefined();
  });

  it('transitionTo stopped marks terminal', () => {
    const r = newRecord('s1');
    transitionTo(r, 'stopped');
    expect(r.state).toBe('stopped');
  });

  it('awaiting_approval defaults to undefined on fresh records', () => {
    const rec: TurnStateRecord = newRecord('s1');
    expect(rec.awaiting_approval).toBeUndefined();
  });
});

describe('state keys', () => {
  it('namespace by session under agent scope', () => {
    expect(AGENT_SCOPE).toBe('agent');
    expect(turnStateKey('abc')).toBe('session/abc/turn_state');
    expect(messagesKey('abc')).toBe('session/abc/messages');
  });
});
