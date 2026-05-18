import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  consumeResolvedApprovalEntries,
  failClosedBlockReply,
  nextStateAfterFinalize,
  prefilledResultForBlock,
  prefilledResultIsError,
  preparedCallsFromApprovalEntries,
  publishFailureFromResponse,
  replacePendingApprovalPlaceholders,
} from '../../src/turn-orchestrator/states/functions.js';
import type { FunctionResultMessage } from '../../src/types/agent-message.js';

describe('prefilledResultForBlock', () => {
  it('returns a terminating pending placeholder when status is pending', () => {
    const result = prefilledResultForBlock(
      { block: true, status: 'pending', reason: 'approval required' },
      'tc-1',
      'shell::exec',
    );
    expect(result.terminate).toBe(true);
    expect((result.details as Record<string, unknown>).pending_approval).toBe(true);
    expect((result.details as Record<string, unknown>).call_id).toBe('tc-1');
    expect(result.content[0]?.type).toBe('text');
  });

  it('returns a non-terminating blocked placeholder for hard denials', () => {
    const result = prefilledResultForBlock(
      { block: true, status: 'denied', reason: 'blocked by policy' },
      'tc-2',
      'shell::exec',
    );
    expect(result.terminate).toBe(false);
    expect((result.details as Record<string, unknown>).blocked).toBe(true);
    expect((result.content[0] as { text: string }).text).toBe('blocked by policy');
  });
});

describe('prefilledResultIsError', () => {
  it('returns false for pending placeholders', () => {
    expect(
      prefilledResultIsError({
        content: [],
        details: { pending_approval: true },
        terminate: true,
      }),
    ).toBe(false);
  });

  it('returns true for hard-block placeholders', () => {
    expect(
      prefilledResultIsError({
        content: [],
        details: { blocked: true },
        terminate: false,
      }),
    ).toBe(true);
  });
});

describe('failClosedBlockReply', () => {
  it('builds a state_error denial envelope', () => {
    const reply = failClosedBlockReply('hook_publish', 'ws closed');
    expect(reply.block).toBe(true);
    expect(reply.status).toBe('denied');
    const denial = reply.denial as Record<string, unknown>;
    expect(denial.kind).toBe('state_error');
    const detail = denial.detail as Record<string, unknown>;
    expect(detail.phase).toBe('hook_publish');
    expect(detail.error).toBe('ws closed');
    expect((reply.reason as string).includes('hook_publish')).toBe(true);
  });
});

describe('publishFailureFromResponse', () => {
  it('returns the publish error when publish.ok is false', () => {
    expect(
      publishFailureFromResponse({ publish: { ok: false, error: 'ws closed' }, replies: [] }, true),
    ).toBe('ws closed');
  });

  it('returns a generic message when publish_failed is true with no error', () => {
    expect(publishFailureFromResponse({ publish_failed: true, replies: [] }, false)).toMatch(
      /publish failed/,
    );
  });

  it('returns missing-gate error when requireApprovalGateReply and no gate reply', () => {
    expect(
      publishFailureFromResponse(
        { publish: { ok: true }, replies: [{ subscriber: 'policy-denylist' }] },
        true,
      ),
    ).toMatch(/approval-gate did not reply/);
  });

  it('returns undefined when publish ok and a gate reply is present', () => {
    expect(
      publishFailureFromResponse(
        {
          publish: { ok: true },
          replies: [{ subscriber: 'approval-gate', approval_gate: true }],
        },
        true,
      ),
    ).toBeUndefined();
  });

  it('does not require a gate reply when requireApprovalGateReply is false', () => {
    expect(
      publishFailureFromResponse({ publish: { ok: true }, replies: [] }, false),
    ).toBeUndefined();
  });
});

describe('preparedCallsFromApprovalEntries', () => {
  it('maps allow entries to dispatchable FunctionCalls without prefilled result', () => {
    const out = preparedCallsFromApprovalEntries([
      {
        function_call_id: 'tc-1',
        function_id: 'shell::exec',
        args: { command: 'date' },
        decision: 'allow',
      },
    ]);
    expect(out).toHaveLength(1);
    expect(out[0]?.[0]).toEqual({
      id: 'tc-1',
      function_id: 'shell::exec',
      arguments: { command: 'date' },
    });
    expect(out[0]?.[1]).toBeNull();
  });

  it('maps deny entries to prefilled denial FunctionResult', () => {
    const out = preparedCallsFromApprovalEntries([
      {
        function_call_id: 'tc-1',
        function_id: 'shell::exec',
        args: {},
        decision: 'deny',
        reason: 'user',
      },
    ]);
    const result = out[0]?.[1];
    expect(result).not.toBeNull();
    if (!result) return;
    expect((result.content[0] as { text: string }).text).toMatch(/approval denied/);
    expect((result.details as Record<string, unknown>).approval_denied).toBe(true);
    expect((result.details as Record<string, unknown>).reason).toBe('user');
  });

  it('uses "approval timed out" text when reason is timed_out', () => {
    const out = preparedCallsFromApprovalEntries([
      {
        function_call_id: 'tc-1',
        function_id: 'shell::exec',
        args: {},
        decision: 'deny',
        reason: 'timed_out',
      },
    ]);
    const result = out[0]?.[1];
    expect(result).not.toBeNull();
    if (!result) return;
    expect((result.content[0] as { text: string }).text).toMatch(/approval timed out/);
  });
});

describe('consumeResolvedApprovalEntries', () => {
  type TriggerCall = { function_id: string; payload: unknown };
  function fakeIii(handler: (call: TriggerCall) => unknown): {
    iii: ISdk;
    calls: TriggerCall[];
  } {
    const calls: TriggerCall[] = [];
    const iii = {
      trigger: async <T, R>(req: { function_id: string; payload: T }): Promise<R> => {
        const call = { function_id: req.function_id, payload: req.payload };
        calls.push(call);
        const v = handler(call);
        if (v instanceof Error) throw v;
        return v as R;
      },
    } as unknown as ISdk;
    return { iii, calls };
  }

  it('returns prepared calls when consume returns ok:true with entries', async () => {
    const { iii } = fakeIii(() => ({
      ok: true,
      entries: [
        { function_call_id: 'tc-1', function_id: 'shell::exec', args: {}, decision: 'allow' },
      ],
    }));
    const out = await consumeResolvedApprovalEntries(iii, 's1');
    expect(out).toHaveLength(1);
    expect(out[0]?.[1]).toBeNull(); // allow → no prefilled
  });

  it('throws when the trigger throws', async () => {
    const { iii } = fakeIii(() => new Error('trigger boom'));
    await expect(consumeResolvedApprovalEntries(iii, 's1')).rejects.toThrow(/trigger boom/);
  });

  it('throws when consume returns ok:false', async () => {
    const { iii } = fakeIii(() => ({ ok: false, error: 'list_failed', entries: [] }));
    await expect(consumeResolvedApprovalEntries(iii, 's1')).rejects.toThrow(/list_failed/);
  });
});

describe('replacePendingApprovalPlaceholders', () => {
  it('removes any prior pending_approval FunctionResult sharing a call_id with a replacement', () => {
    const messages = [
      {
        role: 'function_result',
        function_call_id: 'tc-1',
        function_id: 'shell::exec',
        content: [],
        details: { pending_approval: true },
        is_error: false,
        timestamp: 1,
      } satisfies FunctionResultMessage,
      {
        role: 'function_result',
        function_call_id: 'tc-2',
        function_id: 'shell::fs::ls',
        content: [],
        details: {},
        is_error: false,
        timestamp: 2,
      } satisfies FunctionResultMessage,
    ];
    const replacement: FunctionResultMessage = {
      role: 'function_result',
      function_call_id: 'tc-1',
      function_id: 'shell::exec',
      content: [],
      details: { allowed: true },
      is_error: false,
      timestamp: 3,
    };
    replacePendingApprovalPlaceholders(messages, [replacement]);
    expect(messages).toHaveLength(1);
    expect((messages[0] as FunctionResultMessage).function_call_id).toBe('tc-2');
  });

  it('keeps non-placeholder FunctionResults with the same call_id', () => {
    const messages = [
      {
        role: 'function_result',
        function_call_id: 'tc-1',
        function_id: 'shell::exec',
        content: [],
        details: { created: true },
        is_error: false,
        timestamp: 1,
      } satisfies FunctionResultMessage,
    ];
    const replacement: FunctionResultMessage = {
      role: 'function_result',
      function_call_id: 'tc-1',
      function_id: 'shell::exec',
      content: [],
      details: { created: true },
      is_error: false,
      timestamp: 2,
    };
    replacePendingApprovalPlaceholders(messages, [replacement]);
    expect(messages).toHaveLength(1);
  });

  it('is a no-op when replacements is empty', () => {
    const messages = [
      {
        role: 'function_result',
        function_call_id: 'tc-1',
        function_id: 'shell::exec',
        content: [],
        details: { pending_approval: true },
        is_error: false,
        timestamp: 1,
      } satisfies FunctionResultMessage,
    ];
    replacePendingApprovalPlaceholders(messages, []);
    expect(messages).toHaveLength(1);
  });
});

describe('nextStateAfterFinalize', () => {
  it('returns tearing_down when all_terminate is true', () => {
    expect(nextStateAfterFinalize(false, true)).toBe('tearing_down');
  });
  it('returns steering_check otherwise', () => {
    expect(nextStateAfterFinalize(true, false)).toBe('steering_check');
    expect(nextStateAfterFinalize(false, false)).toBe('steering_check');
  });
});
