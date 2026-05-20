import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { DispatchResult } from '../../src/turn-orchestrator/agent-trigger.js';
import {
  TOOL_NAME,
  agentTriggerTool,
  dispatchWithHook,
  isErrorResult,
} from '../../src/turn-orchestrator/agent-trigger.js';
import * as hookModule from '../../src/turn-orchestrator/hook.js';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('agent_trigger tool schema', () => {
  it('returns a stable schema with the required `function` field', () => {
    const tool = agentTriggerTool() as Record<string, unknown>;
    expect(tool.name).toBe('agent_trigger');
    const params = tool.parameters as Record<string, unknown>;
    expect(params.type).toBe('object');
    expect(params.required).toEqual(['function']);
  });

  it('TOOL_NAME is stable', () => {
    expect(TOOL_NAME).toBe('agent_trigger');
  });
});

describe('DispatchResult shape', () => {
  it('result variant carries a FunctionResult', () => {
    const r: DispatchResult = {
      kind: 'result',
      result: { content: [], details: {}, terminate: false },
    };
    expect(r.kind).toBe('result');
  });

  it('deny variant carries a denial FunctionResult', () => {
    const r: DispatchResult = {
      kind: 'deny',
      result: { content: [], details: { status: 'denied' }, terminate: false },
    };
    expect(r.kind).toBe('deny');
  });

  it('pending variant carries no result', () => {
    const r: DispatchResult = { kind: 'pending' };
    expect(r.kind).toBe('pending');
  });
});

describe('isErrorResult', () => {
  it('treats details.error as error', () => {
    expect(
      isErrorResult({
        content: [],
        details: { error: 'boom' },
        terminate: false,
      }),
    ).toBe(true);
  });

  it('treats details.status === "denied" as error', () => {
    expect(
      isErrorResult({
        content: [],
        details: { schema_version: 1, status: 'denied', denied_by: 'permissions' },
        terminate: false,
      }),
    ).toBe(true);
  });

  it('does not flag normal envelopes', () => {
    expect(isErrorResult({ content: [], details: { ok: true }, terminate: false })).toBe(false);
  });
});

describe('dispatchWithHook returns DispatchResult', () => {
  it('returns kind:pending when consultBefore returns pending', async () => {
    vi.spyOn(hookModule, 'consultBefore').mockResolvedValue({ kind: 'pending' });
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const out = await dispatchWithHook(
      iii,
      { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
      's1',
    );
    expect(out.kind).toBe('pending');
  });

  it('returns kind:deny on hard deny', async () => {
    vi.spyOn(hookModule, 'consultBefore').mockResolvedValue({
      kind: 'deny',
      denial: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'permissions',
        function_id: 'x',
        reason: 'nope',
      },
    });
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const out = await dispatchWithHook(
      iii,
      { id: 'fc-1', function_id: 'shell::run', arguments: {} },
      's1',
    );
    expect(out.kind).toBe('deny');
    if (out.kind === 'deny') {
      expect(out.result.details).toMatchObject({ status: 'denied' });
    }
  });

  it('returns kind:result on allow + successful dispatch', async () => {
    vi.spyOn(hookModule, 'consultBefore').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockResolvedValue({ ok: true }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(
      iii,
      { id: 'fc-1', function_id: 'shell::run', arguments: {} },
      's1',
    );
    expect(out.kind).toBe('result');
  });
});
