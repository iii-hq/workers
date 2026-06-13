import { afterEach, describe, expect, it, vi } from 'vitest';
import { IIIInvocationError, type ISdk } from '../../src/runtime/iii.js';
import {
  TOOL_NAME,
  agentTriggerTool,
  coercePayload,
  dispatchWithHook,
  extractFirstJsonObject,
  fetchContract,
  functionNotFoundHint,
  isArgumentDecodeError,
  isErrorResult,
  triggerFunctionCall,
  unwrapAgentTrigger,
} from '../../src/turn-orchestrator/agent-trigger.js';
import * as chainModule from '../../src/turn-orchestrator/hooks/chain.js';

const CTX = { session_id: 's1', turn_id: 't_test' };

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

describe('coercePayload (H1: double-encoded JSON args)', () => {
  it('parses a stringified JSON object into an object', () => {
    expect(coercePayload('{"sandbox_id":"abc","cmd":"bash"}')).toEqual({
      sandbox_id: 'abc',
      cmd: 'bash',
    });
  });

  it('parses a stringified JSON array', () => {
    expect(coercePayload('["-c", "echo hi"]')).toEqual(['-c', 'echo hi']);
  });

  it('tolerates surrounding whitespace', () => {
    expect(coercePayload('  { "a": 1 } ')).toEqual({ a: 1 });
  });

  it('leaves a real object untouched (identity)', () => {
    const obj = { a: 1 };
    expect(coercePayload(obj)).toBe(obj);
  });

  it('leaves a non-JSON string untouched', () => {
    expect(coercePayload('just a string')).toBe('just a string');
  });

  it('does not coerce a JSON scalar string (only object/array)', () => {
    // "5" is valid JSON but a scalar — a string that does not start with {/[
    // is left verbatim so genuine string payloads survive.
    expect(coercePayload('5')).toBe('5');
    expect(coercePayload('"quoted"')).toBe('"quoted"');
  });

  it('leaves malformed JSON-looking strings untouched', () => {
    expect(coercePayload('{not valid json')).toBe('{not valid json');
  });
});

describe('unwrapAgentTrigger (H1)', () => {
  it('passes an object payload through unchanged', () => {
    const out = unwrapAgentTrigger({
      id: 'fc-1',
      function_id: '',
      arguments: { function: 'sandbox::exec', payload: { sandbox_id: 'x', cmd: 'ls' } },
    });
    expect(out.function_id).toBe('sandbox::exec');
    expect(out.arguments).toEqual({ sandbox_id: 'x', cmd: 'ls' });
  });

  it('coerces a stringified payload (the sandbox::exec failure mode)', () => {
    const out = unwrapAgentTrigger({
      id: 'fc-1',
      function_id: '',
      arguments: {
        function: 'sandbox::exec',
        payload: '{"sandbox_id":"x","cmd":"bash","args":["-c","echo hi"]}',
      },
    });
    expect(out.function_id).toBe('sandbox::exec');
    expect(out.arguments).toEqual({ sandbox_id: 'x', cmd: 'bash', args: ['-c', 'echo hi'] });
  });

  it('coerces when the entire arguments object was JSON-encoded', () => {
    const out = unwrapAgentTrigger({
      id: 'fc-1',
      function_id: '',
      arguments: '{"function":"sandbox::list","payload":{}}',
    });
    expect(out.function_id).toBe('sandbox::list');
    expect(out.arguments).toEqual({});
  });

  it('defaults payload to {} when absent', () => {
    const out = unwrapAgentTrigger({
      id: 'fc-1',
      function_id: '',
      arguments: { function: 'sandbox::list' },
    });
    expect(out.arguments).toEqual({});
  });
});

describe('isArgumentDecodeError (H2)', () => {
  it('matches serde "missing field" / "serialization error"', () => {
    expect(
      isArgumentDecodeError(
        new Error('invocation_failed: serialization error: missing field `sandbox_id`'),
      ),
    ).toBe(true);
  });

  it('matches untagged-enum decode failures (content_b64 mistake)', () => {
    expect(
      isArgumentDecodeError(
        new Error('data did not match any variant of untagged enum WriteContent'),
      ),
    ).toBe(true);
  });

  it('matches "invalid type" serde errors', () => {
    expect(isArgumentDecodeError(new Error('invalid type: string, expected a map'))).toBe(true);
  });

  it('does not match opaque handler text', () => {
    expect(isArgumentDecodeError(new Error('opaque handler text'))).toBe(false);
  });

  it('reads .message off plain error-shaped objects', () => {
    expect(isArgumentDecodeError({ message: 'missing field `path`' })).toBe(true);
    expect(isArgumentDecodeError({ message: 'all good' })).toBe(false);
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

  it('classifies serde decode failures as invalid_arguments, not gate_unavailable (H2)', async () => {
    // The exact failure from session rjgvxowc: a double-encoded sandbox::exec
    // payload reached the daemon as a string, serde could not read the struct
    // fields, and the harness mislabeled it gate_unavailable.
    const triggerError = new IIIInvocationError({
      code: 'HANDLER',
      message: 'invocation_failed: serialization error: missing field `sandbox_id`',
      function_id: 'sandbox::exec',
    });
    const iii = {
      trigger: vi.fn().mockRejectedValue(triggerError),
    } as unknown as ISdk;
    const result = await triggerFunctionCall(iii, {
      id: 'fc-1',
      function_id: 'sandbox::exec',
      arguments: {},
    });
    expect(isErrorResult(result)).toBe(true);
    expect(result.details).toMatchObject({ error: 'invalid_arguments', function: 'sandbox::exec' });
    expect(result.details).not.toMatchObject({ denied_by: 'gate_unavailable' });
  });

  it('auto-attaches the target contract to an invalid_arguments error (chosen fix)', async () => {
    // When a call fails to decode, the harness fetches the target's contract via
    // engine::functions::info and embeds it so the model's NEXT attempt is
    // correctly shaped without spending a turn on info.
    const decodeError = new IIIInvocationError({
      code: 'HANDLER',
      message: 'invocation_failed: serialization error: invalid type: string, expected struct',
      function_id: 'sandbox::fs::write',
    });
    const contractDetails = {
      function_id: 'sandbox::fs::write',
      worker_name: 'iii-sandbox',
      request_schema: { sandbox_id: 'string', path: 'string', content: 'string' },
    };
    const trigger = vi.fn().mockImplementation(async ({ function_id }) => {
      if (function_id === 'engine::functions::info') return { details: contractDetails };
      throw decodeError;
    });
    const iii = { trigger } as unknown as ISdk;
    const result = await triggerFunctionCall(iii, {
      id: 'fc-1',
      function_id: 'sandbox::fs::write',
      arguments: 'not-an-object',
    });
    expect(result.details).toMatchObject({
      error: 'invalid_arguments',
      function: 'sandbox::fs::write',
      contract: contractDetails,
    });
    expect(JSON.stringify(result.details)).toContain('build your payload from it');
    // It fetched the contract for the TARGET, not for the info call itself.
    expect(trigger).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'engine::functions::info',
        payload: { function_id: 'sandbox::fs::write' },
      }),
    );
  });

  it('omits contract and keeps the plain hint when the info fetch also fails', async () => {
    // Best-effort: if engine::functions::info itself errors, fall back to the
    // generic message rather than failing the error path.
    const decodeError = new IIIInvocationError({
      code: 'HANDLER',
      message: 'invocation_failed: serialization error: missing field `path`',
      function_id: 'sandbox::fs::write',
    });
    const trigger = vi.fn().mockRejectedValue(decodeError);
    const iii = { trigger } as unknown as ISdk;
    const result = await triggerFunctionCall(iii, {
      id: 'fc-1',
      function_id: 'sandbox::fs::write',
      arguments: {},
    });
    expect(result.details).toMatchObject({ error: 'invalid_arguments' });
    expect(result.details).not.toHaveProperty('contract');
    expect(JSON.stringify(result.details)).toContain('engine::functions::info');
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

  it('surfaces a structured handler error (S211 + fix hint) embedded after a human prefix', async () => {
    // Live failure from session jg7u4c7i: the engine wrapped the structured
    // envelope as `invocation_failed: handler error: {json}`, so the old
    // extractor (pure JSON.parse) failed and the {code, fix} hint was buried in
    // gate_unavailable. The extractor must now pull the embedded JSON and the
    // model must receive code + fix.
    const s211 = {
      code: 'S211',
      docs_url:
        'https://github.com/iii-hq/iii/blob/main/crates/iii-worker/src/sandbox_daemon/README.md#S211',
      fix: { parents: true },
      fix_note:
        'merge `fix` into the original request and resubmit: `parents: true` auto-creates missing intermediate directories',
      message: 'parent directory not found: parent not found: /app',
      retryable: false,
      type: 'filesystem',
    };
    const wireError = new IIIInvocationError({
      code: 'HANDLER',
      message: `invocation_failed: handler error: ${JSON.stringify(s211)}`,
      function_id: 'sandbox::fs::write',
    });
    const iii = { trigger: vi.fn().mockRejectedValue(wireError) } as unknown as ISdk;
    const result = await triggerFunctionCall(iii, {
      id: 'fc-1',
      function_id: 'sandbox::fs::write',
      arguments: { path: '/app/index.js', content: 'x', sandbox_id: 's1' },
    });
    expect(result.details).toMatchObject({
      error: 'handler_error',
      code: 'S211',
      fix: { parents: true },
      message: 'parent directory not found: parent not found: /app',
    });
    expect(result.details).not.toMatchObject({ denied_by: 'gate_unavailable' });
    // The fix hint is in the text the model reads, too.
    const text = (result.content[0] as { text: string }).text;
    expect(text).toContain('"fix":{"parents":true}');
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

describe('extractFirstJsonObject', () => {
  it('pulls the JSON object out from behind a human prefix', () => {
    const out = extractFirstJsonObject('invocation_failed: handler error: {"code":"S211"}');
    expect(out).toBe('{"code":"S211"}');
  });

  it('handles nested objects (depth tracking)', () => {
    const out = extractFirstJsonObject('x: {"code":"S211","fix":{"parents":true},"n":1}');
    expect(out).toBe('{"code":"S211","fix":{"parents":true},"n":1}');
    expect(JSON.parse(out as string)).toMatchObject({ fix: { parents: true } });
  });

  it('ignores braces inside string values (string-aware)', () => {
    const out = extractFirstJsonObject('p: {"message":"weird } and { in text","code":"S1"}');
    expect(JSON.parse(out as string)).toMatchObject({
      message: 'weird } and { in text',
      code: 'S1',
    });
  });

  it('handles escaped quotes inside strings', () => {
    const out = extractFirstJsonObject('p: {"message":"say \\"hi\\" }now","code":"S1"}');
    expect(JSON.parse(out as string)).toMatchObject({ message: 'say "hi" }now', code: 'S1' });
  });

  it('ignores trailing text after the closing brace', () => {
    const out = extractFirstJsonObject('{"a":1} trailing junk }');
    expect(out).toBe('{"a":1}');
  });

  it('returns null when there is no object', () => {
    expect(extractFirstJsonObject('no json here')).toBeNull();
  });

  it('returns null on an unbalanced/truncated object', () => {
    expect(extractFirstJsonObject('handler error: {"code":"S211","fix":{')).toBeNull();
  });
});

describe('fetchContract', () => {
  it('returns the structured details of the info response', async () => {
    const details = { function_id: 'shell::fs::ls', request_schema: { path: 'string' } };
    const trigger = vi.fn().mockResolvedValue({ content: [], details, terminate: false });
    const out = await fetchContract({ trigger } as unknown as ISdk, 'shell::fs::ls');
    expect(out).toEqual(details);
  });

  it('returns the raw value when there is no details envelope', async () => {
    const raw = { function_id: 'shell::fs::ls', schema: {} };
    const trigger = vi.fn().mockResolvedValue(raw);
    const out = await fetchContract({ trigger } as unknown as ISdk, 'shell::fs::ls');
    expect(out).toEqual(raw);
  });

  it('never introspects the info call itself (no self-fetch / loop)', async () => {
    const trigger = vi.fn();
    const out = await fetchContract({ trigger } as unknown as ISdk, 'engine::functions::info');
    expect(out).toBeNull();
    expect(trigger).not.toHaveBeenCalled();
  });

  it('returns null on an empty function id', async () => {
    const trigger = vi.fn();
    const out = await fetchContract({ trigger } as unknown as ISdk, '');
    expect(out).toBeNull();
    expect(trigger).not.toHaveBeenCalled();
  });

  it('returns null (not throw) when the info fetch rejects', async () => {
    const trigger = vi.fn().mockRejectedValue(new Error('bus down'));
    const out = await fetchContract({ trigger } as unknown as ISdk, 'shell::fs::ls');
    expect(out).toBeNull();
  });
});

describe('dispatchWithHook returns DispatchResult', () => {
  it('returns kind:pending when the hook chain holds the call', async () => {
    vi.spyOn(chainModule, 'consultPreDispatch').mockResolvedValue({
      kind: 'hold',
      held_by: 'approval::gate',
      pending_timeout_ms: 1_800_000,
    });
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const out = await dispatchWithHook(iii, CTX, {
      id: 'fc-1',
      function_id: 'shell::run',
      arguments: { command: 'ls' },
    });
    expect(out).toEqual({
      kind: 'pending',
      held_by: 'approval::gate',
      pending_timeout_ms: 1_800_000,
    });
  });

  it('returns kind:result with denied details on hard deny', async () => {
    vi.spyOn(chainModule, 'consultPreDispatch').mockResolvedValue({
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
    const out = await dispatchWithHook(iii, CTX, {
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
    vi.spyOn(chainModule, 'consultPreDispatch').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockResolvedValue({ ok: true }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(iii, CTX, {
      id: 'fc-1',
      function_id: 'shell::run',
      arguments: {},
    });
    expect(out.kind).toBe('result');
  });

  it('builds the HookInput from the ctx and the ENVELOPED call', async () => {
    const consult = vi
      .spyOn(chainModule, 'consultPreDispatch')
      .mockResolvedValue({ kind: 'allow' });
    const iii = { trigger: vi.fn().mockResolvedValue({ ok: true }) } as unknown as ISdk;
    await dispatchWithHook(
      iii,
      { session_id: 's1', turn_id: 't_9', step: 3, metadata: { message_id: 'm1' } },
      { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls', session_id: 's1' } },
    );
    expect(consult).toHaveBeenCalledWith(iii, {
      point: 'pre_dispatch',
      session_id: 's1',
      turn_id: 't_9',
      step: 3,
      depth: 0,
      metadata: { message_id: 'm1' },
      call: {
        id: 'fc-1',
        function_id: 'shell::run',
        arguments: { command: 'ls', session_id: 's1' },
      },
    });
  });

  it('triggers the TARGET with clean args, not the routing envelope (function_id clobber fix)', async () => {
    // Regression: engine::functions::info { function_id: "sandbox::create" } was
    // arriving with function_id overwritten by the routing envelope (which
    // spreads function_id = the target id), so the engine described itself. The
    // hook sees the envelope; the target must get the caller's untouched args.
    vi.spyOn(chainModule, 'consultPreDispatch').mockResolvedValue({ kind: 'allow' });
    const trigger = vi.fn().mockResolvedValue({ ok: true });
    const iii = { trigger } as unknown as ISdk;
    const hookCall = {
      id: 'fc-1',
      function_id: 'engine::functions::info',
      arguments: { function_id: 'engine::functions::info', session_id: 's1' },
    };
    const targetCall = {
      id: 'fc-1',
      function_id: 'engine::functions::info',
      arguments: { function_id: 'sandbox::create' },
    };
    const out = await dispatchWithHook(iii, CTX, hookCall, targetCall);
    expect(out.kind).toBe('result');
    expect(trigger).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'engine::functions::info',
        payload: { function_id: 'sandbox::create' },
      }),
    );
  });

  it('attaches a "did you mean worker::fn" hint when function_id is a slash path', async () => {
    // Observed in QA against google/gemma-4-e4b: model invented the
    // slash path `sandbox/skills/sandbox/create`, assumed it was
    // callable, retried 3× on `function_not_found`. The hint must
    // propose the canonical worker::fn form so the recovery loop
    // collapses to one turn.
    vi.spyOn(chainModule, 'consultPreDispatch').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockRejectedValue({ code: 'function_not_found' }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(iii, CTX, {
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
    expect(details.hint).toMatch(/Slash-separated paths are NOT function ids/);
  });

  it('attaches the generic slash-path hint when function_id has slashes but no clean rewrite', async () => {
    vi.spyOn(chainModule, 'consultPreDispatch').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockRejectedValue({ code: 'function_not_found' }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(iii, CTX, {
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
    expect(details.hint).toMatch(/Slash-separated paths are NOT function ids/);
  });

  it('falls back to the engine discovery hint when function_id contains no slash', async () => {
    vi.spyOn(chainModule, 'consultPreDispatch').mockResolvedValue({ kind: 'allow' });
    const iii = {
      trigger: vi.fn().mockRejectedValue({ code: 'function_not_found' }),
    } as unknown as ISdk;
    const out = await dispatchWithHook(iii, CTX, {
      id: 'fc-1',
      function_id: 'misspelled',
      arguments: {},
    });
    if (out.kind !== 'result') throw new Error('expected result kind');
    const details = out.result.details as Record<string, unknown>;
    expect(details.hint).toBe(
      'check the function id with engine::functions::list { search: "<name>" }',
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
    expect(functionNotFoundHint('queue/skills/queue/jobs/get')).toMatch(
      /Did you mean `queue::jobs::get`\?/,
    );
  });

  it('rewrites the weaker <w>/<fn> shorthand', () => {
    expect(functionNotFoundHint('sandbox/create')).toMatch(/Did you mean `sandbox::create`\?/);
  });

  it('does not rewrite <w>/index (no reliable worker::fn reading)', () => {
    // `<w>/index` has no trustworthy function-id rewrite; suggesting
    // `sandbox::index` would be wrong more often than right.
    expect(functionNotFoundHint('sandbox/index')).not.toMatch(/Did you mean/);
  });

  it('returns the engine discovery hint for slash-free ids', () => {
    expect(functionNotFoundHint('misspelled')).toBe(
      'check the function id with engine::functions::list { search: "<name>" }',
    );
  });

  it('never produces a suggestion containing a slash', () => {
    // Safety net: any rewrite we emit must be a valid worker::fn form.
    const cases = [
      'sandbox/skills/sandbox/create',
      'sandbox/create',
      'queue/skills/queue/engine/functions/list',
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
