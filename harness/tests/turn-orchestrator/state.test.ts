import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { TurnOrchestratorConfig } from '../../src/turn-orchestrator/config.js';
import type {
  AwaitingApprovalEntry,
  TurnState,
  TurnStateRecord,
} from '../../src/turn-orchestrator/state.js';
import {
  isTerminal,
  messagesKey,
  newRecord,
  transitionTo,
  turnStateKey,
} from '../../src/turn-orchestrator/state.js';
import { step } from '../../src/turn-orchestrator/transitions.js';

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

describe('awaiting_approval field', () => {
  it('defaults to undefined on fresh records', () => {
    const rec: TurnStateRecord = newRecord('s1');
    expect(rec.awaiting_approval).toBeUndefined();
  });

  it('accepts AwaitingApprovalEntry items', () => {
    const rec: TurnStateRecord = newRecord('s1');
    const entry: AwaitingApprovalEntry = {
      function_call_id: 'fc-1',
      function_id: 'shell::run',
      args: { command: 'ls' },
    };
    rec.awaiting_approval = [entry];
    expect(rec.awaiting_approval).toHaveLength(1);
    expect(rec.awaiting_approval[0].function_call_id).toBe('fc-1');
  });
});

describe('step dispatches function_awaiting_approval', () => {
  it('runs the awaiting-approval handler for that state', async () => {
    const cfg = {} as TurnOrchestratorConfig;
    const rec = newRecord('s1');
    transitionTo(rec, 'function_awaiting_approval');
    rec.awaiting_approval = [];

    await step({} as ISdk, cfg, rec);

    expect(rec.state).toBe('function_execute');
  });
});

describe('state keys', () => {
  it('namespace by session', () => {
    expect(turnStateKey('abc')).toBe('session/abc/turn_state');
    expect(messagesKey('abc')).toBe('session/abc/messages');
  });
});
