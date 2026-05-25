import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type {
  AwaitingApprovalEntry,
  TurnState,
  TurnStateRecord,
} from '../../src/turn-orchestrator/state.js';
import {
  AGENT_SCOPE,
  messagesKey,
  newRecord,
  transitionTo,
  turnStateKey,
} from '../../src/turn-orchestrator/state.js';
import { handleAwaitingApproval } from '../../src/turn-orchestrator/states/function-awaiting-approval.js';

describe('TurnStateRecord', () => {
  it('starts in provisioning', () => {
    const r = newRecord('s1', 32);
    expect(r.state).toBe('provisioning');
    expect(r.session_id).toBe('s1');
    expect(r.max_turns).toBe(32);
    expect(r.state).not.toBe('stopped');
  });

  it('transitionTo stopped marks terminal', () => {
    const r = newRecord('s1');
    transitionTo(r, 'stopped');
    expect(r.state).toBe('stopped');
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
    expect(rec.state).not.toBe('stopped');
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

describe('handleAwaitingApproval with empty queue', () => {
  it('advances to function_execute when awaiting_approval is empty', async () => {
    const rec = newRecord('s1');
    transitionTo(rec, 'function_awaiting_approval');
    rec.awaiting_approval = [];

    await handleAwaitingApproval({} as ISdk, rec);

    expect(rec.state).toBe('function_execute');
  });
});

describe('state keys', () => {
  it('namespace by session under agent scope', () => {
    expect(AGENT_SCOPE).toBe('agent');
    expect(turnStateKey('abc')).toBe('session/abc/turn_state');
    expect(messagesKey('abc')).toBe('session/abc/messages');
  });
});

describe('state record', () => {
  it('newRecord starts in provisioning, non-terminal, no work', () => {
    const r = newRecord('s1', 5);
    expect(r.state).toBe('provisioning');
    expect(r.state).not.toBe('stopped');
    expect(r.state).not.toBe('failed');
    expect(r.work).toBeUndefined();
    expect(r.max_turns).toBe(5);
  });

  it('failed is terminal', () => {
    const r: TurnStateRecord = { ...newRecord('s1'), state: 'failed', error: { kind: 'bug', message: 'x' } };
    expect(r.state).toBe('failed');
  });
});
