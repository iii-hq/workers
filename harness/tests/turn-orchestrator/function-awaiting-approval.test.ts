import { describe, expect, it, vi } from 'vitest';
import {
  applyAwaitingApprovalOutcome,
  applyDecisionToPrepared,
  foldDecisionsIntoPrepared,
  processAwaitingApproval,
} from '../../src/turn-orchestrator/function-awaiting-approval/process.js';
import type { AwaitingApprovalPorts } from '../../src/turn-orchestrator/function-awaiting-approval/ports.js';
import type { PreparedCall, TurnStateRecord, TurnWork } from '../../src/turn-orchestrator/state.js';

const dispatchCall = {
  route: 'dispatch' as const,
  call: { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
};

function recordWith(
  awaiting: { function_call_id: string; function_id: string; args: unknown }[],
  work?: TurnWork,
): TurnStateRecord {
  return {
    session_id: 's1',
    state: 'function_awaiting_approval',
    turn_count: 0,
    max_turns: undefined,
    last_assistant: null,
    function_results: [],
    turn_end_emitted: false,
    started_at_ms: 0,
    updated_at_ms: 0,
    awaiting_approval: awaiting,
    work,
  };
}

function stubPorts(
  decisions: Record<string, { decision: 'allow' | 'deny' | 'aborted'; reason: string | null }>,
): AwaitingApprovalPorts {
  return {
    readDecision: vi.fn(async (_session_id, function_call_id) => {
      const decision = decisions[function_call_id];
      return decision ?? null;
    }),
  };
}

describe('applyDecisionToPrepared', () => {
  it('maps allow to pre_approved', () => {
    const next = applyDecisionToPrepared(dispatchCall, { decision: 'allow', reason: null });
    expect(next).toEqual({
      route: 'pre_approved',
      call: dispatchCall.call,
    });
  });

  it('maps deny to synthetic with denial result', () => {
    const next = applyDecisionToPrepared(dispatchCall, { decision: 'deny', reason: 'policy' });
    expect(next.route).toBe('synthetic');
    if (next.route === 'synthetic') {
      expect(next.result.details).toMatchObject({
        approval_denied: true,
        decision: 'deny',
        reason: 'policy',
      });
    }
  });

  it('maps aborted to synthetic with aborted details', () => {
    const next = applyDecisionToPrepared(dispatchCall, {
      decision: 'aborted',
      reason: 'session_aborted',
    });
    expect(next.route).toBe('synthetic');
    if (next.route === 'synthetic') {
      expect(next.result.details).toMatchObject({ decision: 'aborted' });
    }
  });
});

describe('foldDecisionsIntoPrepared', () => {
  it('folds each awaiting entry by function_call_id', () => {
    const prepared: PreparedCall[] = [
      dispatchCall,
      {
        route: 'dispatch',
        call: { id: 'fc-2', function_id: 'shell::fs::write', arguments: {} },
      },
    ];
    const awaiting = [
      { function_call_id: 'fc-1', function_id: 'shell::run', args: {} },
      { function_call_id: 'fc-2', function_id: 'shell::fs::write', args: {} },
    ];
    const decisions = [
      { decision: 'allow' as const, reason: null },
      { decision: 'deny' as const, reason: 'policy' },
    ];

    const folded = foldDecisionsIntoPrepared(prepared, awaiting, decisions);

    expect(folded[0]?.route).toBe('pre_approved');
    expect(folded[1]?.route).toBe('synthetic');
  });

  it('skips awaiting entries not found in prepared', () => {
    const prepared: PreparedCall[] = [dispatchCall];
    const awaiting = [{ function_call_id: 'fc-missing', function_id: 'x', args: {} }];
    const folded = foldDecisionsIntoPrepared(prepared, awaiting, [
      { decision: 'allow', reason: null },
    ]);
    expect(folded).toEqual(prepared);
  });
});

describe('processAwaitingApproval', () => {
  it('returns resume_empty when awaiting is empty', async () => {
    const rec = recordWith([]);
    const outcome = await processAwaitingApproval(stubPorts({}), rec);
    expect(outcome).toEqual({ kind: 'resume_empty' });
  });

  it('returns parked when any decision is missing', async () => {
    const rec = recordWith([{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }], {
      prepared: [dispatchCall],
      executed: {},
    });
    const outcome = await processAwaitingApproval(stubPorts({}), rec);
    expect(outcome).toEqual({ kind: 'parked' });
  });

  it('returns resume with folded prepared when all decisions present', async () => {
    const rec = recordWith([{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }], {
      prepared: [dispatchCall],
      executed: {},
    });
    const outcome = await processAwaitingApproval(
      stubPorts({ 'fc-1': { decision: 'allow', reason: null } }),
      rec,
    );
    expect(outcome.kind).toBe('resume');
    if (outcome.kind === 'resume') {
      expect(outcome.prepared[0]?.route).toBe('pre_approved');
    }
  });
});

describe('applyAwaitingApprovalOutcome', () => {
  it('no-ops when parked', () => {
    const rec = recordWith([{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }], {
      prepared: [dispatchCall],
      executed: {},
    });
    applyAwaitingApprovalOutcome(rec, { kind: 'parked' });
    expect(rec.state).toBe('function_awaiting_approval');
    expect(rec.awaiting_approval).toHaveLength(1);
    expect(rec.work?.prepared[0]?.route).toBe('dispatch');
  });

  it('clears awaiting and transitions on resume_empty', () => {
    const rec = recordWith([{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }]);
    applyAwaitingApprovalOutcome(rec, { kind: 'resume_empty' });
    expect(rec.state).toBe('function_execute');
    expect(rec.awaiting_approval).toEqual([]);
  });

  it('updates prepared and transitions on resume', () => {
    const rec = recordWith([{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }], {
      prepared: [dispatchCall],
      executed: {},
    });
    const prepared: PreparedCall[] = [{ route: 'pre_approved', call: dispatchCall.call }];
    applyAwaitingApprovalOutcome(rec, { kind: 'resume', prepared });
    expect(rec.state).toBe('function_execute');
    expect(rec.awaiting_approval).toEqual([]);
    expect(rec.work?.prepared).toEqual(prepared);
  });
});
