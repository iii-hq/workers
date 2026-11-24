import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@anthropic-ai/claude-agent-sdk', () => ({ query: vi.fn() }));

import { query } from '@anthropic-ai/claude-agent-sdk';
import { loadConfig, type Config } from '../src/config.js';
import { ON_AGENT_EVENT_FN, ON_RAW_EVENT_FN, registerAgentFeeds } from '../src/agent-feed.js';
import { localRun } from '../src/iii-plugin/local-run.js';
import { executeRun, RunPayloadSchema } from '../src/run.js';
import { fakeClaude } from './_helpers/fake-claude.js';
import { type FakeIii, fakeIii } from './_helpers/fake-iii.js';
import { fullTurn, scriptedQuery, type QueryCapture } from './_helpers/fake-query.js';

const queryMock = vi.mocked(query);

/** Both feeds, with an observer of each for `session_id`. */
async function observedFeeds(fake: FakeIii, session_id = 's1') {
  const feeds = registerAgentFeeds(fake.iii);
  await fake.observeFeed(ON_AGENT_EVENT_FN, session_id);
  await fake.observeFeed(ON_RAW_EVENT_FN, session_id);
  return feeds;
}

async function baseConfig(): Promise<Config> {
  return loadConfig('/nonexistent/config.yaml');
}

/** The engine answers like one running mcp-server behind the http worker. */
function withMcpServer(fake: FakeIii): void {
  const realCall = fake.iii.call.bind(fake.iii);
  (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
    if (req.function_id === 'http::status') return { url: 'http://127.0.0.1:3111' };
    if (req.function_id === 'engine::functions::list')
      return { functions: [{ function_id: 'mcp-server::handler' }] };
    return realCall(req as never);
  };
}

async function runTurn(
  payload: Record<string, unknown>,
  messages: Array<Record<string, unknown>> = fullTurn,
  cfgOverrides: Partial<Config> = {},
  mcpServer = false,
) {
  const fake = fakeIii();
  if (mcpServer) withMcpServer(fake);
  const cfg = { ...(await baseConfig()), ...cfgOverrides };
  const capture: QueryCapture = { interrupted: false };
  queryMock.mockImplementation(scriptedQuery(messages, capture) as never);
  const feeds = await observedFeeds(fake, String(payload.session_id ?? 's1'));
  const emit = feeds.agent.emit;
  const emitRaw = feeds.raw.emit;
  const result = await executeRun(fake.iii, cfg, emit, emitRaw, RunPayloadSchema.parse(payload));
  return { fake, capture, result };
}

beforeEach(() => {
  queryMock.mockReset();
});

describe('executeRun', () => {
  it('releases the live slot even when the working-save rejects (no stuck busy)', async () => {
    const fake = fakeIii();
    const cfg = await baseConfig();
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    // make every state::set reject — the working-save during setup throws
    const realCall = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'state::set') throw new Error('store down');
      return realCall(req as never);
    };
    const emit = (await observedFeeds(fake)).agent.emit;
    await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'leak-1' }),
    ).catch(() => {});
    // slot must be free — a second run is not falsely "busy"
    const second = (await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'leak-1' }),
    ).catch(() => ({ busy: undefined }))) as Record<string, unknown>;
    expect(second.busy).not.toBe(true);
  });

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

  it('mirrors every SDK message verbatim onto claude::on-raw-event', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const raw = fake.feedFrames(ON_RAW_EVENT_FN).map((f) => f.event);
    expect(raw).toEqual(fullTurn);
    const sessionIds = fake.feedFrames(ON_RAW_EVENT_FN).map((f) => f.session_id);
    expect(new Set(sessionIds)).toEqual(new Set(['s1']));
  });

  it('emits the translated AgentEvent sequence on claude::on-agent-event', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const types = fake.feedFrames(ON_AGENT_EVENT_FN).map((f) => (f.event as { type: string }).type);
    expect(types).toEqual([
      'message_complete',
      'function_execution_start',
      'function_execution_end',
      'turn_end',
      'agent_end',
    ]);
    const [start, end] = fake
      .feedFrames(ON_AGENT_EVENT_FN)
      .map((f) => f.event as Record<string, unknown>)
      .filter((d) => String(d.type).startsWith('function_execution'));
    expect(start).toMatchObject({
      function_call_id: 'toolu_1',
      function_id: 'claude-code::Bash',
      args: { command: 'ls' },
    });
    expect(end).toMatchObject({ function_call_id: 'toolu_1', is_error: false });
  });

  it('appends the iii runtime context to the system prompt by default', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1' });
    const sp = capture.options?.systemPrompt as { type: string; append?: string };
    expect(sp.type).toBe('preset');
    expect(sp.append).toContain('# iii runtime');
    expect(sp.append).toContain('iii call engine::functions::list');
    expect(capture.options?.allowedTools).toContain('Bash(iii *)');
  });

  it('stacks the iii context with a user append', async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      append_system_prompt: 'house rules',
    });
    const sp = capture.options?.systemPrompt as { type: string; append?: string };
    expect(sp.append).toContain('# iii runtime');
    expect(sp.append).toContain('house rules');
    expect(sp.append?.indexOf('# iii runtime')).toBeLessThan(
      sp.append?.indexOf('house rules') ?? -1,
    );
  });

  it('config-level iii_context: false disables the block for every turn', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1' }, fullTurn, {
      iii_context: false,
    });
    const sp = capture.options?.systemPrompt as { type: string; append?: string };
    expect(sp.append).toBeUndefined();
    expect(capture.options?.allowedTools).not.toContain('Bash(iii *)');
  });

  it('omits the iii context when disabled per turn and keeps the user append', async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      iii_context: false,
      append_system_prompt: 'house rules',
    });
    const sp = capture.options?.systemPrompt as { type: string; append?: string };
    expect(sp.append).toBe('house rules');
    expect(capture.options?.allowedTools).not.toContain('Bash(iii *)');
  });

  it('installs the published plugin in the folder the turn runs in, and passes no plugin copy', async () => {
    const claude = fakeClaude();
    vi.mocked(localRun).mockImplementation(claude.run);
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1', cwd: '/repo/a' });
    expect(capture.options?.plugins).toBeUndefined();
    expect(claude.claudeCalls()).toContain('plugin install iii@iii --scope local --json');
    expect(claude.installs).toEqual([
      { id: 'iii@iii', scope: 'local', enabled: true, projectPath: '/repo/a' },
    ]);
    // The install is in the folder's local settings: the turn has to read them.
    expect(capture.options?.settingSources).toEqual(['local']);
  });

  it('leaves a folder whose plugin is installed and enabled alone', async () => {
    const claude = fakeClaude({
      installs: [{ id: 'iii@iii', scope: 'local', enabled: true, projectPath: '/repo/a' }],
    });
    vi.mocked(localRun).mockImplementation(claude.run);
    await runTurn({ prompt: 'x', session_id: 's1', cwd: '/repo/a' });
    expect(claude.claudeCalls()).toEqual(['plugin list --json']);
  });

  it('checks the folder a turn without cwd runs in: the worker process folder', async () => {
    const claude = fakeClaude();
    vi.mocked(localRun).mockImplementation(claude.run);
    await runTurn({ prompt: 'x', session_id: 's1' });
    expect(claude.installs[0]?.projectPath).toBe(process.cwd());
  });

  it('uses the configured claude binary for the plugin commands', async () => {
    const claude = fakeClaude();
    vi.mocked(localRun).mockImplementation(claude.run);
    await runTurn({ prompt: 'x', session_id: 's1', cwd: '/repo/a' }, fullTurn, {
      claude_executable: '/usr/local/bin/claude',
    });
    expect(claude.runs.filter((r) => r.program !== 'pwd').map((r) => r.program)).toEqual(
      expect.arrayContaining(['/usr/local/bin/claude']),
    );
  });

  it("points the turn's Claude process at mcp-server with III_MCP_URL, and III_CLI for the hooks", async () => {
    const { capture } = await runTurn(
      { prompt: 'x', session_id: 's1', cwd: '/repo/a' },
      fullTurn,
      {},
      true,
    );
    expect(capture.options?.settings).toBeUndefined();
    const env = capture.options?.env as Record<string, string>;
    expect(env.III_MCP_URL).toBe('http://127.0.0.1:3111/mcp');
    expect(env.III_PLUGIN_MCP_URL).toBe('http://127.0.0.1:3111/mcp');
    expect(env.III_CLI).toBeTruthy();
    // The worker's own environment still reaches the CLI.
    expect(env.PATH).toBe(process.env.PATH);
  });

  it('reads the http address fresh for every turn', async () => {
    const fake = fakeIii();
    let port = 3111;
    const realCall = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'http::status') return { url: `http://127.0.0.1:${port}` };
      return realCall(req as never);
    };
    const cfg = await baseConfig();
    const emit = (await observedFeeds(fake)).agent.emit;
    const envs: Record<string, string>[] = [];
    for (const next of [3111, 4111]) {
      port = next;
      const capture: QueryCapture = { interrupted: false };
      queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
      await executeRun(
        fake.iii,
        cfg,
        emit,
        emit,
        RunPayloadSchema.parse({ prompt: 'x', session_id: `p${next}` }),
      );
      envs.push(capture.options?.env as Record<string, string>);
    }
    expect(envs.map((e) => e.III_MCP_URL)).toEqual([
      'http://127.0.0.1:3111/mcp',
      'http://127.0.0.1:4111/mcp',
    ]);
  });

  it('runs the turn without the plugin when it cannot be installed, reading no settings', async () => {
    const claude = fakeClaude();
    claude.fail.add('plugin install');
    vi.mocked(localRun).mockImplementation(claude.run);
    const { capture, result } = await runTurn({ prompt: 'x', session_id: 's1', cwd: '/repo/a' });
    expect(result).toMatchObject({ is_error: false });
    expect(capture.options?.settingSources).toEqual([]);
    expect(capture.options?.settings).toBeUndefined();
  });

  it("never replaces Claude Code's system prompt with an agent", async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1' }, fullTurn, {}, true);
    expect(capture.options?.agent).toBeUndefined();
    const sp = capture.options?.systemPrompt as { type: string; preset?: string; append?: string };
    expect(sp).toMatchObject({ type: 'preset', preset: 'claude_code' });
  });

  it('with mcp-server on the engine, appends default-mcp and allows the iii tools', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1' }, fullTurn, {}, true);
    const sp = capture.options?.systemPrompt as { append?: string };
    // default-mcp, not the CLI-only runtime text: that one says to act only
    // through the `iii` CLI, which contradicts default-mcp's iii_call.
    expect(sp.append).not.toContain('# iii runtime');
    expect(sp.append).toContain('You are an iii agent. Call the engine with iii_call.');
    const allowed = capture.options?.allowedTools as string[];
    for (const t of ['iii_call', 'iii_observe', 'iii_observations_poll', 'iii_status'])
      expect(allowed).toContain(`mcp__plugin_iii_iii__${t}`);
    expect(allowed).not.toContain('mcp__plugin_iii_iii__iii_secret_store');
  });

  it('with mcp-server, stacks default-mcp with a user append and no runtime text', async () => {
    const { capture } = await runTurn(
      { prompt: 'x', session_id: 's1', append_system_prompt: 'house rules' },
      fullTurn,
      {},
      true,
    );
    const sp = capture.options?.systemPrompt as { append?: string };
    expect(sp.append).toBe('You are an iii agent. Call the engine with iii_call.\n\nhouse rules');
  });

  it('with the approval gate on, iii_call and iii_observe go through the gate', async () => {
    const { capture } = await runTurn(
      { prompt: 'x', session_id: 's1' },
      fullTurn,
      { approval_gate: true },
      true,
    );
    const allowed = capture.options?.allowedTools as string[];
    expect(allowed).not.toContain('mcp__plugin_iii_iii__iii_call');
    expect(allowed).not.toContain('mcp__plugin_iii_iii__iii_observe');
    expect(allowed).toContain('mcp__plugin_iii_iii__iii_status');
  });

  it('without mcp-server, the turn works through the iii CLI as before', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1' });
    const sp = capture.options?.systemPrompt as { append?: string };
    expect(sp.append).toContain('# iii runtime');
    expect(sp.append).not.toContain('iii_call');
    const allowed = capture.options?.allowedTools as string[];
    expect(allowed).toContain('Bash(iii *)');
    expect(allowed.filter((t) => t.startsWith('mcp__plugin_iii_iii__'))).toEqual([]);
  });

  it('does not touch the plugin when the iii context is off', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1', iii_context: false });
    expect(vi.mocked(localRun)).not.toHaveBeenCalled();
    expect(capture.options?.plugins).toBeUndefined();
    expect(capture.options?.settingSources).toEqual([]);
    expect(capture.options?.agent).toBeUndefined();
  });

  it("lets a caller's own options.agent through", async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      options: { agent: 'reviewer' },
    });
    expect(capture.options?.agent).toBe('reviewer');
  });

  it('a caller-supplied system_prompt wins verbatim with nothing appended', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1', system_prompt: 'override' });
    expect(capture.options?.systemPrompt).toBe('override');
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
      settingSources: ['local'],
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
    const emit = (await observedFeeds(fake)).agent.emit;
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
    const emit = (await observedFeeds(fake)).agent.emit;
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
    const emit = (await observedFeeds(fake)).agent.emit;
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
    const types = fake.feedFrames(ON_AGENT_EVENT_FN).map((f) => (f.event as { type: string }).type);
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
    const originalCall = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: Record<string, unknown>;
    }) => {
      if (req.function_id === 'policy::check_permissions') return policyReply();
      return originalCall(req as never);
    };
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const emit = (await observedFeeds(fake)).agent.emit;
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
    const emit = (await observedFeeds(fake)).agent.emit;
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

  it('the named permission_mode field cannot bypass the gate either', async () => {
    const fake = fakeIii();
    const cfg = { ...(await baseConfig()), approval_gate: true };
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const emit = (await observedFeeds(fake)).agent.emit;
    await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({
        prompt: 'x',
        session_id: 's1',
        permission_mode: 'bypassPermissions',
      }),
    );
    expect(capture.options?.permissionMode).toBe('default');
    expect(capture.options?.canUseTool).toBeTypeOf('function');
  });
});

describe('event feed delivery during a turn', () => {
  it('a refused feed call never fails the turn', async () => {
    const fake = fakeIii();
    const cfg = await baseConfig();
    queryMock.mockImplementation(scriptedQuery(fullTurn, { interrupted: false }) as never);
    const feeds = await observedFeeds(fake);
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === ON_AGENT_EVENT_FN || req.function_id === ON_RAW_EVENT_FN) {
        throw new Error('OVERFLOW');
      }
      return real(req as never);
    };
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const result = await executeRun(
      fake.iii,
      cfg,
      feeds.agent.emit,
      feeds.raw.emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 's1' }),
    );
    expect(result).toMatchObject({ session_id: 's1', result: 'done', is_error: false });
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(ON_AGENT_EVENT_FN));
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(ON_RAW_EVENT_FN));
    warn.mockRestore();
  });

  it('does not touch iii-stream at all', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    expect(fake.calls.some((c) => c.function_id.startsWith('stream::'))).toBe(false);
    const feedCalls = fake.calls.filter(
      (c) => c.function_id === ON_AGENT_EVENT_FN || c.function_id === ON_RAW_EVENT_FN,
    );
    expect(feedCalls.length).toBeGreaterThan(0);
    for (const call of feedCalls) {
      expect(call.action).toEqual({ type: 'void' });
    }
  });
});
