import { describe, expect, it, vi } from 'vitest';
import type { ApprovalSettings } from '../../src/approval-gate/schemas.js';
import type {
  CheckPermissionsPayload,
  PolicyCheckReply,
} from '../../src/harness/policy/check-permissions.js';
import type { ISdk } from '../../src/runtime/iii.js';
import { consultBefore } from '../../src/turn-orchestrator/hook.js';

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

/** Build an ApprovalSettings with sane defaults; override per test. */
function mkSettings(overrides: Partial<ApprovalSettings> = {}): ApprovalSettings {
  return {
    mode: 'manual',
    always_allow: [],
    approved_always: [],
    mode_set_at: 1,
    ...overrides,
  };
}

function allowEntry(function_id: string) {
  return { function_id, granted_at: 1, granted_by: 'user_click' as const };
}

/** Build an iii fake that returns approval_settings for `state::get` calls and routes policy checks through the supplied impl. */
function modedIii(
  settings: ApprovalSettings | null,
  policyImpl?: () => PolicyCheckReply | Promise<PolicyCheckReply>,
) {
  return {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      if (req.function_id === 'state::get') {
        return settings;
      }
      if (req.function_id === 'policy::check_permissions') {
        return policyImpl ? await policyImpl() : { decision: 'needs_approval' };
      }
      throw new Error(`unexpected trigger ${req.function_id}`);
    }),
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

describe('consultBefore (mode + always_allow)', () => {
  const fcWithSession = {
    id: 'fc-1',
    function_id: 'shell::fs::write',
    arguments: { path: '/tmp/x', session_id: 'sess-abc' },
  };
  const fcSafeWithSession = {
    id: 'fc-2',
    function_id: 'shell::fs::read',
    arguments: { path: '/tmp/x', session_id: 'sess-abc' },
  };

  it('full mode short-circuits to allow without calling policy', async () => {
    const iii = modedIii(mkSettings({ mode: 'full' }));
    const outcome = await consultBefore(iii, fcWithSession);
    expect(outcome.kind).toBe('allow');
    const calls = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.map((c) => c[0].function_id)).not.toContain('policy::check_permissions');
  });

  it('auto mode + allowlist hit short-circuits to allow even when policy would deny', async () => {
    const iii = modedIii(
      mkSettings({
        mode: 'auto',
        always_allow: [allowEntry('shell::fs::write')],
      }),
      () => ({
        decision: 'deny',
        rule_id: 'would-deny',
        rule_action: 'deny',
      }),
    );
    const outcome = await consultBefore(iii, fcWithSession);
    expect(outcome.kind).toBe('allow');
    const calls = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.map((c) => c[0].function_id)).not.toContain('policy::check_permissions');
  });

  it('manual mode treats the allowlist as dormant (does NOT short-circuit)', async () => {
    const iii = modedIii(
      mkSettings({
        mode: 'manual',
        always_allow: [allowEntry('shell::fs::write')],
      }),
      () => ({ decision: 'needs_approval' }),
    );
    const outcome = await consultBefore(iii, fcWithSession);
    expect(outcome.kind).toBe('pending');
    const calls = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.map((c) => c[0].function_id)).toContain('policy::check_permissions');
  });

  it('auto mode falls through to policy when function id is not on the allowlist', async () => {
    const iii = modedIii(mkSettings({ mode: 'auto' }), () => ({
      decision: 'needs_approval',
    }));
    const outcome = await consultBefore(iii, fcSafeWithSession);
    expect(outcome.kind).toBe('pending');
  });

  it('approved_always short-circuits in MANUAL mode (per-session grant honored everywhere)', async () => {
    const iii = modedIii(
      mkSettings({
        mode: 'manual',
        approved_always: [allowEntry('shell::fs::write')],
      }),
      () => ({ decision: 'needs_approval' }),
    );
    const outcome = await consultBefore(iii, fcWithSession);
    expect(outcome.kind).toBe('allow');
    const calls = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.map((c) => c[0].function_id)).not.toContain('policy::check_permissions');
  });

  it('approved_always short-circuits in AUTO mode too', async () => {
    const iii = modedIii(
      mkSettings({
        mode: 'auto',
        approved_always: [allowEntry('shell::fs::write')],
      }),
      () => ({ decision: 'deny', rule_id: 'would-deny', rule_action: 'deny' }),
    );
    const outcome = await consultBefore(iii, fcWithSession);
    expect(outcome.kind).toBe('allow');
  });

  it('approved_always does not affect a different function id', async () => {
    const iii = modedIii(
      mkSettings({
        mode: 'manual',
        approved_always: [allowEntry('shell::fs::read')],
      }),
      () => ({ decision: 'needs_approval' }),
    );
    const outcome = await consultBefore(iii, fcWithSession);
    expect(outcome.kind).toBe('pending');
  });

  it('denies human-only approval functions (self-escalation defense)', async () => {
    const iii = modedIii(null);
    const outcome = await consultBefore(iii, {
      id: 'fc-escalate',
      function_id: 'approval::set_mode',
      arguments: { session_id: 'sess-abc', mode: 'full' },
    });
    expect(outcome.kind).toBe('deny');
    if (outcome.kind !== 'deny') return;
    expect(outcome.denial.denied_by).toBe('permissions');
    expect(outcome.denial.rule_id).toBe('human_only_function');
    // No state::get and no policy::check_permissions — denial is upfront.
    const calls = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.length).toBe(0);
  });

  it('falls back to policy when settings are absent (legacy session)', async () => {
    const iii = modedIii(null, () => ({
      decision: 'allow',
      rule_id: 'r',
    }));
    const outcome = await consultBefore(iii, fcWithSession);
    expect(outcome.kind).toBe('allow');
    const calls = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.map((c) => c[0].function_id)).toContain('policy::check_permissions');
  });
});
