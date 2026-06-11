import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@anthropic-ai/claude-agent-sdk', () => ({ query: vi.fn() }));

import { query } from '@anthropic-ai/claude-agent-sdk';
import { loadConfig, type Config } from '../src/config.js';
import { makeEmitter } from '../src/events.js';
import { executeRun, RunPayloadSchema } from '../src/run.js';
import { fakeIii } from './_helpers/fake-iii.js';
import { fullTurn, scriptedQuery, type QueryCapture } from './_helpers/fake-query.js';

const queryMock = vi.mocked(query);

async function baseConfig(): Promise<Config> {
  return loadConfig('/nonexistent/config.yaml');
}

async function runTurn(
  payload: Record<string, unknown>,
  messages: Array<Record<string, unknown>> = fullTurn,
  cfgOverrides: Partial<Config> = {},
) {
  const fake = fakeIii();
  const cfg = { ...(await baseConfig()), ...cfgOverrides };
  const capture: QueryCapture = { interrupted: false };
  queryMock.mockImplementation(scriptedQuery(messages, capture) as never);
  const emit = makeEmitter(fake.iii, cfg.events_stream);
  const emitRaw = makeEmitter(fake.iii, cfg.raw_events_stream);
  const result = await executeRun(fake.iii, cfg, emit, emitRaw, RunPayloadSchema.parse(payload));
  return { fake, capture, result };
}

beforeEach(() => {
  queryMock.mockReset();
});

describe('executeRun', () => {
  it('returns the result with mapped usage and cost', async () => {
    const { result } = await runTurn({ prompt: 'do it', session_id: 's1' });
    expect(result).toMatchObject({
      session_id: 's1',
      claude_session_id: 'cs-1',
      result: 'done',
      stop_reason: 'end',
      is_error: false,
      num_turns: 2,
      total_cost_usd: 0.01,
      usage: { input_tokens: 5, output_tokens: 2 },
    });
  });

  it('persists the session record working then done with the Claude session id', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const sets = fake.calls.filter(
      (c) =>
        c.function_id === 'state::set' &&
        (c.payload as { scope?: string }).scope === 'claude_sessions',
    );
    const statuses = sets.map((c) => (c.payload.value as { status: string }).status);
    expect(statuses[0]).toBe('working');
    expect(statuses[statuses.length - 1]).toBe('done');
    const final = sets[sets.length - 1].payload.value as Record<string, unknown>;
    expect(final.claude_session_id).toBe('cs-1');
    expect(final.total_cost_usd).toBe(0.01);
  });

  it('mirrors every SDK message verbatim onto the raw stream', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const raw = fake.streamFrames('claude::events').map((f) => f.data);
    expect(raw).toEqual(fullTurn);
    const groupIds = fake.streamFrames('claude::events').map((f) => f.group_id);
    expect(new Set(groupIds)).toEqual(new Set(['s1']));
  });

  it('emits the translated AgentEvent sequence on agent::events', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const types = fake.streamFrames('agent::events').map((f) => (f.data as { type: string }).type);
    expect(types).toEqual([
      'message_complete',
      'function_execution_start',
      'function_execution_end',
      'turn_end',
      'agent_end',
    ]);
    const [start, end] = fake
      .streamFrames('agent::events')
      .map((f) => f.data as Record<string, unknown>)
      .filter((d) => String(d.type).startsWith('function_execution'));
    expect(start).toMatchObject({
      function_call_id: 'toolu_1',
      function_id: 'claude-code::Bash',
      args: { command: 'ls' },
    });
    expect(end).toMatchObject({ function_call_id: 'toolu_1', is_error: false });
  });

  it('passes worker defaults and named fields to query options', async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      cwd: '/repo',
      model: 'claude-sonnet-4-6',
      permission_mode: 'plan',
      max_turns: 7,
    });
    expect(capture.prompt).toBe('x');
    expect(capture.options).toMatchObject({
      cwd: '/repo',
      model: 'claude-sonnet-4-6',
      permissionMode: 'plan',
      maxTurns: 7,
      settingSources: [],
    });
  });

  it('forwards raw SDK options verbatim and lets them win over derived fields', async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      max_turns: 7,
      options: { forkSession: true, includePartialMessages: true, maxTurns: 3 },
    });
    expect(capture.options).toMatchObject({
      forkSession: true,
      includePartialMessages: true,
      maxTurns: 3,
    });
  });

  it('resumes the prior Claude session for a known session_id', async () => {
    const fake = fakeIii();
    fake.state.set('claude_sessions/s1', {
      session_id: 's1',
      claude_session_id: 'cs-prior',
      cwd: '/repo',
      model: '',
      status: 'done',
      turns: 1,
      total_cost_usd: 0.01,
      usage: null,
      updated_at_ms: 1,
    });
    const cfg = await baseConfig();
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    const result = await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'again', session_id: 's1' }),
    );
    expect(capture.options?.resume).toBe('cs-prior');
    expect(result.num_turns).toBe(3);
  });

  it('honors per-turn cwd and model overrides on a resumed session', async () => {
    const fake = fakeIii();
    fake.state.set('claude_sessions/s1', {
      session_id: 's1',
      claude_session_id: 'cs-prior',
      cwd: '/old/repo',
      model: 'old-model',
      status: 'done',
      turns: 1,
      total_cost_usd: 0,
      usage: null,
      updated_at_ms: 1,
    });
    const cfg = await baseConfig();
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({
        prompt: 'x',
        session_id: 's1',
        cwd: '/new/repo',
        model: 'new-model',
      }),
    );
    expect(capture.options).toMatchObject({
      cwd: '/new/repo',
      model: 'new-model',
      resume: 'cs-prior',
    });
  });

  it('extracts the prompt from the last user message of a messages payload', async () => {
    const { capture } = await runTurn({
      session_id: 's1',
      messages: [
        { role: 'user', content: [{ type: 'text', text: 'first' }] },
        { role: 'assistant', content: [{ type: 'text', text: 'reply' }] },
        { role: 'user', content: [{ type: 'text', text: 'second' }] },
      ],
    });
    expect(capture.prompt).toBe('second');
  });

  it('marks the record error and still closes the turn when the SDK throws', async () => {
    const fake = fakeIii();
    const cfg = await baseConfig();
    queryMock.mockImplementation((() => {
      return {
        async *[Symbol.asyncIterator]() {
          yield { type: 'system', subtype: 'init', session_id: 'cs-1' };
          throw new Error('spawn failed');
        },
        interrupt: async () => {},
      };
    }) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    const result = await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 's1' }),
    );
    expect(result.is_error).toBe(true);
    expect(result.stop_reason).toBe('error');
    expect(String(result.result)).toContain('spawn failed');
    const record = fake.state.get('claude_sessions/s1') as { status: string };
    expect(record.status).toBe('error');
    const types = fake.streamFrames('agent::events').map((f) => (f.data as { type: string }).type);
    expect(types).toContain('turn_end');
    expect(types).toContain('agent_end');
  });

  it('reports non-success result subtypes as the stop reason', async () => {
    const turn = [
      { type: 'system', subtype: 'init', session_id: 'cs-1' },
      { ...fullTurn[3], subtype: 'error_max_turns', is_error: true },
    ];
    const { result } = await runTurn({ prompt: 'x', session_id: 's1' }, turn);
    expect(result.stop_reason).toBe('error_max_turns');
    expect(result.is_error).toBe(true);
  });

  it('omits the resume option for a fresh session', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 'fresh' });
    expect(capture.options).not.toHaveProperty('resume');
  });
});

describe('approval gate', () => {
  async function captureCanUseTool(policyReply: () => Promise<unknown>) {
    const fake = fakeIii();
    const cfg = { ...(await baseConfig()), approval_gate: true };
    const originalTrigger = fake.iii.trigger.bind(fake.iii);
    (fake.iii as { trigger: unknown }).trigger = async (req: {
      function_id: string;
      payload: Record<string, unknown>;
    }) => {
      if (req.function_id === 'policy::check_permissions') return policyReply();
      return originalTrigger(req as never);
    };
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 's1' }),
    );
    return capture.options?.canUseTool as (
      tool: string,
      input: Record<string, unknown>,
    ) => Promise<{ behavior: string; message?: string }>;
  }

  it('allows when the policy says allow', async () => {
    const canUseTool = await captureCanUseTool(async () => ({ decision: 'allow' }));
    expect(canUseTool).toBeTypeOf('function');
    await expect(canUseTool('Bash', { command: 'ls' })).resolves.toMatchObject({
      behavior: 'allow',
    });
  });

  it('denies when the policy says deny', async () => {
    const canUseTool = await captureCanUseTool(async () => ({ decision: 'deny' }));
    await expect(canUseTool('Bash', { command: 'rm' })).resolves.toMatchObject({
      behavior: 'deny',
    });
  });

  it('fails closed when the policy is unreachable', async () => {
    const canUseTool = await captureCanUseTool(async () => {
      throw new Error('gate down');
    });
    const verdict = await canUseTool('Bash', { command: 'ls' });
    expect(verdict.behavior).toBe('deny');
    expect(verdict.message).toContain('fail-closed');
  });

  it('does not install canUseTool when the gate is off', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1' });
    expect(capture.options).not.toHaveProperty('canUseTool');
  });

  it('caller options cannot shadow the gate', async () => {
    const fake = fakeIii();
    const cfg = { ...(await baseConfig()), approval_gate: true };
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({
        prompt: 'x',
        session_id: 's1',
        options: { canUseTool: 'overridden', permissionMode: 'bypassPermissions' },
      }),
    );
    expect(capture.options?.canUseTool).toBeTypeOf('function');
    expect(capture.options?.permissionMode).not.toBe('bypassPermissions');
  });
});
