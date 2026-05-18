import { describe, expect, it } from 'vitest';
import type { DispatchResult } from '../../src/turn-orchestrator/agent-call.js';
import {
  FUNCTION_ID,
  TOOL_NAME,
  agentCallTool,
  isErrorResult,
} from '../../src/turn-orchestrator/agent-call.js';

describe('agent_call tool schema', () => {
  it('returns a stable schema with the required `function` field', () => {
    const tool = agentCallTool() as Record<string, unknown>;
    expect(tool.name).toBe('agent_call');
    const params = tool.parameters as Record<string, unknown>;
    expect(params.type).toBe('object');
    expect(params.required).toEqual(['function']);
  });

  it('TOOL_NAME and FUNCTION_ID are stable', () => {
    expect(TOOL_NAME).toBe('agent_call');
    expect(FUNCTION_ID).toBe('agent::call');
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
