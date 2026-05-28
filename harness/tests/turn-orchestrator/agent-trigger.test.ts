import { afterEach, describe, expect, it, vi } from 'vitest';
import { IIIInvocationError, type ISdk } from '../../src/runtime/iii.js';
import {
  TOOL_NAME,
  agentTriggerTool,
  dispatchWithHook,
  functionNotFoundHint,
  isErrorResult,
  triggerFunctionCall,
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

describe('triggerFunctionCall', () => {
  it('returns gate_unavailable denial on trigger failure', async () => {
    const iii = {
      trigger: vi.fn().mockRejectedValue(new Error('handler error')),
    } as unknown as ISdk;
    const result = await triggerFunctionCall(iii, {
      id: 'fc-1',
      function_id: 'shell::fs::write',
      arguments: {},
    });
    expect(isErrorResult(result)).toBe(true);
    expect(result.details).toMatchObject({
      status: 'denied',
      denied_by: 'gate_unavailable',
      function_id: 'shell::fs::write',
    });
  });

  it('surfaces structured S-code handler errors verbatim, not gate_unavailable', async () => {
    // Mimic what `iii-worker`'s sandbox daemon emits: its `Display` impl
    // serialises the error envelope as JSON, which the engine then forwards
    // through `IIIInvocationError`. The harness must hand that envelope to
    // the agent untouched so it sees `code`, `docs_url`, `fix`, etc.
    const envelope = {
      code: 'S210',
      type: 'filesystem',
      message: 'path is required',
      docs_url: 'https://example.invalid/README.md#S210',
      retryable: false,
      fix: 'pass an absolute `path` argument',
    };
    const triggerError = new IIIInvocationError({
      code: 'HANDLER',
      message: JSON.stringify(envelope),
      function_id: 'sandbox::fs::write',
    });
    const iii = {
      trigger: vi.fn().mockRejectedValue(triggerError),
    } as unknown as ISdk;
    const result = await triggerFunctionCall(iii, {
      id: 'fc-1',
      function_id: 'sandbox::fs::write',
      arguments: {},
    });
    expect(isErrorResult(result)).toBe(true);
    // Envelope passes through verbatim alongside a `handler_error`
    // discriminator that lets isErrorResult and any retry gate
    // classify the result correctly.
    expect(result.details).toMatchObject({ error: 'handler_error', ...envelope });
    expect(result.details).not.toMatchObject({ denied_by: 'gate_unavailable' });
  });

  it('falls back to gate_unavailable when message is not structured JSON', async () => {
    const triggerError = new IIIInvocationError({
      code: 'HANDLER',
      message: 'opaque handler text',
      function_id: 'sandbox::fs::write',
    });
    const iii = {
      trigger: vi.fn().mockRejectedValue(triggerError),
    } as unknown as ISdk;
    const result = await triggerFunctionCall(iii, {
      id: 'fc-1',
      function_id: 'sandbox::fs::write',
      arguments: {},
    });
    expect(result.details).toMatchObject({ denied_by: 'gate_unavailable' });
  });

  it('falls back to gate_unavailable when JSON message lacks code/message fields', async () => {
    // A partial JSON payload (e.g. `{"hint": "..."}`) does NOT count as a
    // structured envelope — only `{code, message, ...}` shapes get the
    // verbatim treatment. Anything else stays in the gate path so we don't
    // silently misroute unrelated wire shapes.
    const triggerError = new IIIInvocationError({
      code: 'HANDLER',
      message: JSON.stringify({ hint: 'try again' }),
      function_id: 'sandbox::fs::write',
    });
    const iii = {
      trigger: vi.fn().mockRejectedValue(triggerError),
    } as unknown as ISdk;
    const result = await triggerFunctionCall(iii, {
      id: 'fc-1',
      function_id: 'sandbox::fs::write',
      arguments: {},
    });
    expect(result.details).toMatchObject({ denied_by: 'gate_unavailable' });
  });
});

describe('dispatchWithHook returns DispatchResult', () => {
  it('returns kind:pending when consultBefore returns pending', async () => {
    vi.spyOn(hookModule, 'consultBefore').mockResolvedValue({ kind: 'pending' });
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const out = await dispatchWithHook(iii, {
      id: 'fc-1',
      function_id: 'shell::run',
      arguments: { command: 'ls' },
    });
    expect(out.kind).toBe('pending');
  });

  it('returns kind:result with denied details on hard deny', async () => {
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
    const out = await dispatchWithHook(iii, {
      id: 'fc-1',
      function_id: 'shell::run',
      arguments: {},
    });
    expect(out.kind).toBe('result');
    if (out.kind === 'result') {
      expect(out.result.details).toMatchObject({ status: 'denied' });
    }
  });

  it('returns kind:result on allow + successful dispatch', async () => {
    vi.spyOn(hookModule, 'consultBefore').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockResolvedValue({ ok: true }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(iii, {
      id: 'fc-1',
      function_id: 'shell::run',
      arguments: {},
    });
    expect(out.kind).toBe('result');
  });

  it('attaches a "did you mean worker::fn" hint when function_id is a canonical skill path', async () => {
    // Observed in QA against google/gemma-4-e4b: model saw
    // `sandbox/skills/sandbox/create` in directory::skills::list,
    // assumed it was callable, retried 3× on `function_not_found`.
    // The hint must propose the canonical worker::fn form so the
    // recovery loop collapses to one turn.
    vi.spyOn(hookModule, 'consultBefore').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockRejectedValue({ code: 'function_not_found' }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(iii, {
      id: 'fc-1',
      function_id: 'sandbox/skills/sandbox/create',
      arguments: { image: 'node' },
    });
    expect(out.kind).toBe('result');
    if (out.kind !== 'result') return;
    const details = out.result.details as Record<string, unknown>;
    expect(details.error).toBe('function_not_found');
    expect(details.function).toBe('sandbox/skills/sandbox/create');
    expect(details.hint).toMatch(/Did you mean `sandbox::create`\?/);
    expect(details.hint).toMatch(/Skill ids are NOT function ids/);
  });

  it('attaches the generic skill-id hint when function_id has slashes but no clean rewrite', async () => {
    vi.spyOn(hookModule, 'consultBefore').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockRejectedValue({ code: 'function_not_found' }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(iii, {
      id: 'fc-1',
      function_id: 'some/odd/three-segment/id',
      arguments: {},
    });
    if (out.kind !== 'result') throw new Error('expected result kind');
    const details = out.result.details as Record<string, unknown>;
    // No "Did you mean" — three-segment ids don't match the
    // worker/skills/worker/fn shape and don't trip the weaker
    // two-segment rewrite either.
    expect(details.hint).not.toMatch(/Did you mean/);
    expect(details.hint).toMatch(/Skill ids are NOT function ids/);
  });

  it('falls back to the generic skill-load hint when function_id contains no slash', async () => {
    vi.spyOn(hookModule, 'consultBefore').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockRejectedValue({ code: 'function_not_found' }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(iii, {
      id: 'fc-1',
      function_id: 'misspelled',
      arguments: {},
    });
    if (out.kind !== 'result') throw new Error('expected result kind');
    const details = out.result.details as Record<string, unknown>;
    expect(details.hint).toBe(
      'load the relevant skill via directory::skills::get, or check the function id',
    );
  });
});

describe('functionNotFoundHint', () => {
  it('rewrites <w>/skills/<w>/<fn> → <w>::<fn>', () => {
    expect(functionNotFoundHint('sandbox/skills/sandbox/create')).toMatch(
      /Did you mean `sandbox::create`\?/,
    );
  });

  it('handles nested function ids: <w>/skills/<w>/<a>/<b> → <w>::<a>::<b>', () => {
    expect(functionNotFoundHint('directory/skills/directory/skills/get')).toMatch(
      /Did you mean `directory::skills::get`\?/,
    );
  });

  it('rewrites the weaker <w>/<fn> shorthand', () => {
    expect(functionNotFoundHint('sandbox/create')).toMatch(/Did you mean `sandbox::create`\?/);
  });

  it('does not rewrite <w>/index (would shadow the bare-name alias)', () => {
    // `sandbox/index` is a legitimate skill id (the bare-name alias
    // resolved by directory::skills::get); rewriting to `sandbox::index`
    // would be wrong.
    expect(functionNotFoundHint('sandbox/index')).not.toMatch(/Did you mean/);
  });

  it('returns the generic skill-load hint for slash-free ids', () => {
    expect(functionNotFoundHint('misspelled')).toBe(
      'load the relevant skill via directory::skills::get, or check the function id',
    );
  });

  it('never produces a suggestion containing a slash', () => {
    // Safety net: any rewrite we emit must be a valid worker::fn form.
    const cases = [
      'sandbox/skills/sandbox/create',
      'sandbox/create',
      'directory/skills/directory/engine/functions/list',
    ];
    for (const c of cases) {
      const hint = functionNotFoundHint(c);
      const match = hint.match(/Did you mean `([^`]+)`/);
      if (match) {
        expect(match[1]).not.toContain('/');
        expect(match[1]).toContain('::');
      }
    }
  });
});
