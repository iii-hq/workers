import { beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  CheckPermissionsPayload,
  PolicyCheckReply,
} from '../../src/harness/policy/check-permissions.js';
import type { ISdk } from '../../src/runtime/iii.js';
import { TOPIC_AFTER, consultBefore, publishAfter } from '../../src/turn-orchestrator/hook.js';
import { resetSubscriberCache } from '../../src/turn-orchestrator/subscriber-presence.js';

function fakeIii(
  triggerImpl: (req: {
    function_id: string;
    payload: CheckPermissionsPayload;
  }) => PolicyCheckReply | Promise<PolicyCheckReply>,
) {
  return {
    trigger: vi.fn(async (req: { function_id: string; payload: CheckPermissionsPayload }) =>
      triggerImpl(req),
    ),
  } as unknown as ISdk;
}

describe('consultBefore (direct policy call)', () => {
  const fc = { id: 'fc-1', function_id: 'shell::fs::write', arguments: { path: '/tmp/x' } };

  it('returns allow when policy decides allow', async () => {
    const iii = fakeIii(({ function_id }) => {
      expect(function_id).toBe('policy::check_permissions');
      return { decision: 'allow', rule_id: 'allow-tmp' };
    });
    const outcome = await consultBefore(iii, fc);
    expect(outcome.kind).toBe('allow');
  });

  it('returns deny with a permissions envelope when policy decides deny', async () => {
    const iii = fakeIii(() => ({
      decision: 'deny',
      rule_id: 'deny-rm-rf',
      rule_action: 'deny',
      matched_constraint: { field: 'path', operator: 'matches', value: '^/$' },
    }));
    const outcome = await consultBefore(iii, fc);
    expect(outcome.kind).toBe('deny');
    if (outcome.kind !== 'deny') return;
    expect(outcome.denial.denied_by).toBe('permissions');
    expect(outcome.denial.rule_id).toBe('deny-rm-rf');
  });

  it('returns pending when policy says needs_approval', async () => {
    const iii = fakeIii(() => ({ decision: 'needs_approval' }));
    const outcome = await consultBefore(iii, fc);
    expect(outcome.kind).toBe('pending');
  });

  it('fails closed (deny gate_unavailable) when policy is unreachable', async () => {
    const iii = fakeIii(() => {
      throw new Error('policy worker down');
    });
    const outcome = await consultBefore(iii, fc);
    expect(outcome.kind).toBe('deny');
    if (outcome.kind !== 'deny') return;
    expect(outcome.denial.denied_by).toBe('gate_unavailable');
  });

  it('does NOT call hook-fanout::publish_collect', async () => {
    const trigger = vi.fn(
      async () => ({ decision: 'allow', rule_id: 'r' }) satisfies PolicyCheckReply,
    );
    const iii = { trigger } as unknown as ISdk;
    await consultBefore(iii, fc);
    expect(trigger).toHaveBeenCalledTimes(1);
    expect(trigger.mock.calls[0][0].function_id).toBe('policy::check_permissions');
  });
});

describe('publishAfter (subscriber-aware after-hook)', () => {
  beforeEach(() => resetSubscriberCache());

  const fc = { id: 'fc-1', function_id: 'shell::fs::write', arguments: { path: '/tmp/x' } };
  const result = { content: [{ type: 'text', text: 'ok' }] };

  it('skips publish_collect when no subscriber is registered for the topic', async () => {
    const trigger = vi.fn(async (req: { function_id: string }) => {
      if (req.function_id === 'engine::triggers::list') return { triggers: [] };
      throw new Error(`should not call ${req.function_id}`);
    });
    const iii = { trigger } as unknown as ISdk;

    const merged = await publishAfter(iii, fc, result);

    expect(merged).toBeUndefined();
    const fns = trigger.mock.calls.map((c) => c[0].function_id);
    expect(fns).not.toContain('hook-fanout::publish_collect');
  });

  it('calls publish_collect when a subscriber is registered for the topic', async () => {
    const trigger = vi.fn(async (req: { function_id: string }) => {
      if (req.function_id === 'engine::triggers::list') {
        return {
          triggers: [{ trigger_type: 'durable:subscriber', config: { topic: TOPIC_AFTER } }],
        };
      }
      if (req.function_id === 'hook-fanout::publish_collect') return { merged: { rewritten: true } };
      throw new Error(`unexpected ${req.function_id}`);
    });
    const iii = { trigger } as unknown as ISdk;

    const merged = await publishAfter(iii, fc, result);

    expect(merged).toEqual({ rewritten: true });
    const fns = trigger.mock.calls.map((c) => c[0].function_id);
    // Proves the subscriber gate ran before delegating to the primitive.
    expect(fns).toContain('engine::triggers::list');
    expect(fns).toContain('hook-fanout::publish_collect');
  });
});
