import { describe, expect, it } from 'vitest';
import type { TurnState } from '../../src/turn-orchestrator/state.js';
import {
  isTerminal,
  messagesKey,
  newRecord,
  transitionTo,
  turnStateKey,
} from '../../src/turn-orchestrator/state.js';

describe('TurnStateRecord', () => {
  it('starts in provisioning', () => {
    const r = newRecord('s1', 32);
    expect(r.state).toBe('provisioning');
    expect(r.session_id).toBe('s1');
    expect(r.max_turns).toBe(32);
    expect(isTerminal(r)).toBe(false);
  });

  it('transitionTo stopped marks terminal', () => {
    const r = newRecord('s1');
    transitionTo(r, 'stopped');
    expect(isTerminal(r)).toBe(true);
  });
});

describe('function_awaiting_approval state', () => {
  it('accepts function_awaiting_approval as a TurnState value', () => {
    const rec = newRecord('s1');
    transitionTo(rec, 'function_awaiting_approval' as TurnState);
    expect(rec.state).toBe('function_awaiting_approval');
  });

  it('is non-terminal', () => {
    const rec = newRecord('s1');
    transitionTo(rec, 'function_awaiting_approval' as TurnState);
    expect(isTerminal(rec)).toBe(false);
  });
});

describe('state keys', () => {
  it('namespace by session', () => {
    expect(turnStateKey('abc')).toBe('session/abc/turn_state');
    expect(messagesKey('abc')).toBe('session/abc/messages');
  });
});
