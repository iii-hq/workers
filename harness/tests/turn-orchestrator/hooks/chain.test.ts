import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../../src/runtime/iii.js';
import { consultPreTrigger } from '../../../src/turn-orchestrator/hooks/chain.js';
import {
  addPreTriggerBindingForTests,
  resetPreTriggerBindingsForTests,
} from '../../../src/turn-orchestrator/hooks/registry.js';
import type { BindingConfig, HookInput } from '../../../src/turn-orchestrator/hooks/types.js';

function input(function_id = 'shell::run'): HookInput {
  return {
    point: 'pre_trigger',
    session_id: 's1',
    turn_id: 't_1',
    step: 1,
    depth: 0,
    call: { id: 'fc-1', function_id, arguments: { cmd: 'ls' } },
  };
}

function bind(function_id: string, config?: Partial<BindingConfig>): void {
  addPreTriggerBindingForTests({
    id: `b-${function_id}`,
    function_id,
    config: { timeout_ms: 5_000, on_error: 'fail_closed', ...config },
  });
}

function iiiAnswering(answers: Record<string, unknown | (() => unknown)>): ISdk {
  return {
    trigger: vi.fn(async ({ function_id }: { function_id: string }) => {
      const a = answers[function_id];
      if (a === undefined) throw new Error(`unexpected trigger ${function_id}`);
      return typeof a === 'function' ? (a as () => unknown)() : a;
    }),
  } as unknown as ISdk;
}

beforeEach(() => {
  resetPreTriggerBindingsForTests();
});

describe('consultPreTrigger', () => {
  it('allows when no bindings match (hooks narrow, never widen)', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    expect(await consultPreTrigger(iii, input())).toEqual({ kind: 'allow' });
    expect(iii.trigger).not.toHaveBeenCalled();
  });

  it('skips bindings whose functions globs do not match', async () => {
    bind('approval::gate', { functions: ['web::*'] });
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    expect(await consultPreTrigger(iii, input('shell::run'))).toEqual({ kind: 'allow' });
    expect(iii.trigger).not.toHaveBeenCalled();
  });

  it('continue from every hook resolves to allow, passing the HookInput verbatim', async () => {
    bind('approval::gate');
    const iii = iiiAnswering({ 'approval::gate': { decision: 'continue' } });

    expect(await consultPreTrigger(iii, input())).toEqual({ kind: 'allow' });
    expect(iii.trigger).toHaveBeenCalledWith({
      function_id: 'approval::gate',
      payload: input(),
      timeoutMs: 5_000,
    });
  });

  it('deny short-circuits with a denied envelope carrying the hook reason', async () => {
    bind('approval::gate');
    bind('zz::never-reached');
    const iii = iiiAnswering({
      'approval::gate': { decision: 'deny', reason: 'human_only_function' },
    });

    const out = await consultPreTrigger(iii, input());
    expect(out).toEqual({
      kind: 'deny',
      denial: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'hook',
        function_id: 'shell::run',
        reason: 'human_only_function',
      },
    });
    expect(iii.trigger).toHaveBeenCalledTimes(1);
  });

  it('hold short-circuits with the holding hook id and the timeout', async () => {
    bind('approval::gate');
    const iii = iiiAnswering({
      'approval::gate': { decision: 'hold', pending_timeout_ms: 1_800_000 },
    });

    expect(await consultPreTrigger(iii, input())).toEqual({
      kind: 'hold',
      held_by: 'approval::gate',
      pending_timeout_ms: 1_800_000,
    });
  });

  it('transport failure fails closed with gate_unavailable', async () => {
    bind('approval::gate');
    const iii = iiiAnswering({
      'approval::gate': () => {
        throw new Error('connection refused');
      },
    });

    const out = await consultPreTrigger(iii, input());
    expect(out.kind).toBe('deny');
    if (out.kind !== 'deny') return;
    expect(out.denial.denied_by).toBe('gate_unavailable');
    expect(out.denial.reason).toContain('approval::gate unavailable');
    expect(out.denial.reason).toContain('connection refused');
  });

  it('an unparseable hook reply fails closed', async () => {
    bind('approval::gate');
    const iii = iiiAnswering({ 'approval::gate': 'what even is this' });

    const out = await consultPreTrigger(iii, input());
    expect(out.kind).toBe('deny');
    if (out.kind !== 'deny') return;
    expect(out.denial.denied_by).toBe('gate_unavailable');
  });

  it('fail_open skips a broken hook and continues the chain', async () => {
    bind('alpha::broken', { on_error: 'fail_open' });
    bind('beta::gate');
    const iii = iiiAnswering({
      'alpha::broken': () => {
        throw new Error('down');
      },
      'beta::gate': { decision: 'hold', pending_timeout_ms: 60_000 },
    });

    expect(await consultPreTrigger(iii, input())).toEqual({
      kind: 'hold',
      held_by: 'beta::gate',
      pending_timeout_ms: 60_000,
    });
  });

  it('runs hooks in priority order and stops at the first non-continue', async () => {
    bind('beta::second', { priority: 1 });
    bind('alpha::first', { priority: 0 });
    const order: string[] = [];
    const iii = {
      trigger: vi.fn(async ({ function_id }: { function_id: string }) => {
        order.push(function_id);
        return function_id === 'alpha::first'
          ? { decision: 'continue' }
          : { decision: 'deny', reason: 'no' };
      }),
    } as unknown as ISdk;

    const out = await consultPreTrigger(iii, input());
    expect(order).toEqual(['alpha::first', 'beta::second']);
    expect(out.kind).toBe('deny');
  });
});
