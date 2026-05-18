import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { consultBefore } from '../../src/turn-orchestrator/hook.js';

function fakeIii(triggerImpl: (req: { function_id: string; payload: unknown }) => unknown) {
  return {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => triggerImpl(req)),
  } as unknown as ISdk;
}

describe('consultBefore (direct policy call)', () => {
  const fc = { id: 'fc-1', function_id: 'shell::fs::write', arguments: { path: '/tmp/x' } };

  it('returns allow when policy decides allow', async () => {
    const iii = fakeIii(({ function_id }) => {
      expect(function_id).toBe('policy::check_permissions');
      return { decision: 'allow', rule_id: 'allow-tmp' };
    });
    const outcome = await consultBefore(iii, fc, [], 'sess-a', 'policy::check_permissions');
    expect(outcome.kind).toBe('allow');
  });

  it('returns deny with a permissions envelope when policy decides deny', async () => {
    const iii = fakeIii(() => ({
      decision: 'deny',
      rule_id: 'deny-rm-rf',
      matched_constraint: { field: 'path', operator: 'matches', value: '^/$' },
    }));
    const outcome = await consultBefore(iii, fc, [], 'sess-a', 'policy::check_permissions');
    expect(outcome.kind).toBe('deny');
    if (outcome.kind !== 'deny') return;
    expect(outcome.denial.denied_by).toBe('permissions');
    expect(outcome.denial.rule_id).toBe('deny-rm-rf');
  });

  it('returns pending when policy says needs_approval', async () => {
    const iii = fakeIii(() => ({ decision: 'needs_approval' }));
    const outcome = await consultBefore(iii, fc, [], 'sess-a', 'policy::check_permissions');
    expect(outcome.kind).toBe('pending');
  });

  it('falls back to legacy approval_required substring when policy is unreachable', async () => {
    const iii = fakeIii(() => {
      throw new Error('policy worker down');
    });
    const outcome = await consultBefore(
      iii,
      fc,
      ['shell::fs::write'],
      'sess-a',
      'policy::check_permissions',
    );
    expect(outcome.kind).toBe('pending');
  });

  it('fails closed (deny) when policy unreachable AND not in approval_required list', async () => {
    const iii = fakeIii(() => {
      throw new Error('policy worker down');
    });
    const outcome = await consultBefore(iii, fc, [], 'sess-a', 'policy::check_permissions');
    expect(outcome.kind).toBe('deny');
    if (outcome.kind !== 'deny') return;
    expect(outcome.denial.denied_by).toBe('gate_unavailable');
  });

  it('does NOT call hook-fanout::publish_collect', async () => {
    const trigger = vi.fn(async () => ({ decision: 'allow', rule_id: 'r' }));
    const iii = { trigger } as unknown as ISdk;
    await consultBefore(iii, fc, [], 'sess-a', 'policy::check_permissions');
    expect(trigger).toHaveBeenCalledTimes(1);
    expect(trigger.mock.calls[0][0].function_id).toBe('policy::check_permissions');
  });
});
