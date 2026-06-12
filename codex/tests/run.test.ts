import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@openai/codex-sdk', () => ({ Codex: vi.fn() }));

import { Codex } from '@openai/codex-sdk';
import { type Config, loadConfig } from '../src/config.js';
import { makeEmitter } from '../src/events.js';
import { executeRun, RunPayloadSchema } from '../src/run.js';
import { type CodexCapture, fakeCodexClass, fullTurn } from './_helpers/fake-codex.js';
import { fakeIii } from './_helpers/fake-iii.js';

const CodexMock = vi.mocked(Codex);

async function baseConfig(): Promise<Config> {
  return loadConfig('/nonexistent/config.yaml');
}

async function runTurn(
  payload: Record<string, unknown>,
  events: Array<Record<string, unknown>> = fullTurn,
  cfgOverrides: Partial<Config> = {},
) {
  const fake = fakeIii();
  const cfg = { ...(await baseConfig()), ...cfgOverrides };
  const capture: CodexCapture = { aborted: false };
  CodexMock.mockImplementation(fakeCodexClass(events, capture) as never);
  const emit = makeEmitter(fake.iii, cfg.events_stream);
  const emitRaw = makeEmitter(fake.iii, cfg.raw_events_stream);
  const result = await executeRun(fake.iii, cfg, emit, emitRaw, RunPayloadSchema.parse(payload));
  return { fake, capture, result };
}

beforeEach(() => {
  CodexMock.mockReset();
});

describe('executeRun', () => {
  it('returns the final agent message with mapped usage', async () => {
    const { result } = await runTurn({ prompt: 'do it', session_id: 's1' });
    expect(result).toMatchObject({
      session_id: 's1',
      codex_thread_id: 'th-1',
      result: 'done',
      stop_reason: 'end',
      is_error: false,
      num_turns: 1,
      usage: {
        input_tokens: 5,
        output_tokens: 2,
        cache_read_tokens: 100,
        reasoning_tokens: 7,
      },
    });
  });

  it('persists the session record working then done with the thread id', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const sets = fake.calls.filter(
      (c) =>
        c.function_id === 'state::set' &&
        (c.payload as { scope?: string }).scope === 'codex_sessions',
    );
    const statuses = sets.map((c) => (c.payload.value as { status: string }).status);
    expect(statuses[0]).toBe('working');
    expect(statuses[statuses.length - 1]).toBe('done');
    const final = sets[sets.length - 1].payload.value as Record<string, unknown>;
    expect(final.codex_thread_id).toBe('th-1');
  });

  it('mirrors every SDK event verbatim onto the raw stream', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const raw = fake.streamFrames('codex::events').map((f) => f.data);
    expect(raw).toEqual(fullTurn);
    const groupIds = fake.streamFrames('codex::events').map((f) => f.group_id);
    expect(new Set(groupIds)).toEqual(new Set(['s1']));
  });

  it('emits the translated AgentEvent sequence on agent::events', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const types = fake.streamFrames('agent::events').map((f) => (f.data as { type: string }).type);
    expect(types).toEqual([
      'function_execution_start',
      'function_execution_end',
      'message_complete',
      'turn_end',
      'agent_end',
    ]);
    const [start, end] = fake
      .streamFrames('agent::events')
      .map((f) => f.data as Record<string, unknown>)
      .filter((d) => String(d.type).startsWith('function_execution'));
    expect(start).toMatchObject({
      function_call_id: 'item-1',
      function_id: 'codex::shell',
      args: { command: 'ls' },
    });
    expect(end).toMatchObject({ function_call_id: 'item-1', is_error: false });
  });

  it('delivers the iii runtime context as developer_instructions by default', async () => {
    const { capture } = await runTurn({ prompt: 'do it', session_id: 's1' });
    const config = capture.codexOptions?.config as { developer_instructions?: string };
    expect(config.developer_instructions).toContain('# iii runtime');
    expect(config.developer_instructions).toContain('iii trigger engine::functions::list');
    expect(capture.input).toBe('do it');
  });

  it('keeps developer_instructions on resumed threads without touching the prompt', async () => {
    const fake = fakeIii();
    fake.state.set('codex_sessions/s1', {
      session_id: 's1',
      codex_thread_id: 'th-prior',
      cwd: '',
      model: '',
      status: 'done',
      turns: 1,
      usage: null,
      updated_at_ms: 1,
    });
    const cfg = await baseConfig();
    const capture: CodexCapture = { aborted: false };
    CodexMock.mockImplementation(fakeCodexClass(fullTurn, capture) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'again', session_id: 's1' }),
    );
    const config = capture.codexOptions?.config as { developer_instructions?: string };
    expect(config.developer_instructions).toContain('# iii runtime');
    expect(capture.input).toBe('again');
  });

  it('a caller-supplied developer_instructions wins over the iii block', async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      codex_config: { developer_instructions: 'house rules' },
    });
    const config = capture.codexOptions?.config as { developer_instructions?: string };
    expect(config.developer_instructions).toBe('house rules');
  });

  it('config-level iii_context: false disables the block for every turn', async () => {
    const { capture } = await runTurn({ prompt: 'plain', session_id: 's1' }, fullTurn, {
      iii_context: false,
    });
    expect(capture.codexOptions?.config).toBeUndefined();
    expect(capture.input).toBe('plain');
  });

  it('omits the context when disabled per turn', async () => {
    const { capture } = await runTurn({ prompt: 'plain', session_id: 's1', iii_context: false });
    expect(capture.codexOptions?.config).toBeUndefined();
    expect(capture.input).toBe('plain');
  });

  it('passes worker defaults and named fields to thread options', async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      iii_context: false,
      cwd: '/repo',
      model: 'gpt-5.2-codex',
      sandbox_mode: 'read-only',
      reasoning_effort: 'high',
    });
    expect(capture.prompt).toBe('x');
    expect(capture.threadOptions).toMatchObject({
      workingDirectory: '/repo',
      model: 'gpt-5.2-codex',
      sandboxMode: 'read-only',
      approvalPolicy: 'never',
      skipGitRepoCheck: true,
      modelReasoningEffort: 'high',
    });
  });

  it('forwards raw SDK options verbatim and lets them win over derived fields', async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      sandbox_mode: 'read-only',
      options: { networkAccessEnabled: true, sandboxMode: 'workspace-write' },
    });
    expect(capture.threadOptions).toMatchObject({
      networkAccessEnabled: true,
      sandboxMode: 'workspace-write',
    });
  });

  it('forwards output_schema as the turn outputSchema', async () => {
    const schema = { type: 'object', properties: { ok: { type: 'boolean' } } };
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1', output_schema: schema });
    expect(capture.turnOptions?.outputSchema).toEqual(schema);
  });

  it('forwards codex_config as SDK config overrides alongside the iii block', async () => {
    const codex_config = { mcp_servers: { github: { command: 'gh-mcp' } } };
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1', codex_config });
    expect(capture.codexOptions?.config).toMatchObject(codex_config);
    const config = capture.codexOptions?.config as { developer_instructions?: string };
    expect(config.developer_instructions).toContain('# iii runtime');
  });

  it('attaches local images to the prompt input', async () => {
    const { capture } = await runTurn({
      prompt: 'describe these',
      session_id: 's1',
      iii_context: false,
      images: ['/tmp/a.png', '/tmp/b.png'],
    });
    expect(capture.input).toEqual([
      { type: 'text', text: 'describe these' },
      { type: 'local_image', path: '/tmp/a.png' },
      { type: 'local_image', path: '/tmp/b.png' },
    ]);
  });

  it('resumes the prior thread for a known session_id', async () => {
    const fake = fakeIii();
    fake.state.set('codex_sessions/s1', {
      session_id: 's1',
      codex_thread_id: 'th-prior',
      cwd: '/repo',
      model: '',
      status: 'done',
      turns: 1,
      usage: null,
      updated_at_ms: 1,
    });
    const cfg = await baseConfig();
    const capture: CodexCapture = { aborted: false };
    CodexMock.mockImplementation(fakeCodexClass(fullTurn, capture) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    const result = await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'again', session_id: 's1' }),
    );
    expect(capture.resumedFrom).toBe('th-prior');
    expect(result.num_turns).toBe(2);
  });

  it('honors per-turn cwd and model overrides on a resumed session', async () => {
    const fake = fakeIii();
    fake.state.set('codex_sessions/s1', {
      session_id: 's1',
      codex_thread_id: 'th-prior',
      cwd: '/old/repo',
      model: 'old-model',
      status: 'done',
      turns: 1,
      usage: null,
      updated_at_ms: 1,
    });
    const cfg = await baseConfig();
    const capture: CodexCapture = { aborted: false };
    CodexMock.mockImplementation(fakeCodexClass(fullTurn, capture) as never);
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
    expect(capture.threadOptions).toMatchObject({
      workingDirectory: '/new/repo',
      model: 'new-model',
    });
  });

  it('extracts the prompt from the last user message of a messages payload', async () => {
    const { capture } = await runTurn({
      session_id: 's1',
      iii_context: false,
      messages: [
        { role: 'user', content: [{ type: 'text', text: 'first' }] },
        { role: 'assistant', content: [{ type: 'text', text: 'reply' }] },
        { role: 'user', content: [{ type: 'text', text: 'second' }] },
      ],
    });
    expect(capture.prompt).toBe('second');
  });

  it('marks the record error and still closes the turn when the stream fails', async () => {
    const turn = [
      { type: 'thread.started', thread_id: 'th-1' },
      { type: 'turn.failed', error: { message: 'model exploded' } },
    ];
    const { fake, result } = await runTurn({ prompt: 'x', session_id: 's1' }, turn);
    expect(result.is_error).toBe(true);
    expect(result.stop_reason).toBe('error');
    expect(String(result.result)).toContain('model exploded');
    const record = fake.state.get('codex_sessions/s1') as { status: string };
    expect(record.status).toBe('error');
    const types = fake.streamFrames('agent::events').map((f) => (f.data as { type: string }).type);
    expect(types).toContain('turn_end');
    expect(types).toContain('agent_end');
  });

  it('reports a failed command item as an error function result', async () => {
    const turn = [
      { type: 'thread.started', thread_id: 'th-1' },
      {
        type: 'item.completed',
        item: {
          id: 'item-9',
          type: 'command_execution',
          command: 'false',
          aggregated_output: '',
          exit_code: 1,
          status: 'failed',
        },
      },
      {
        type: 'turn.completed',
        usage: {
          input_tokens: 1,
          cached_input_tokens: 0,
          output_tokens: 1,
          reasoning_output_tokens: 0,
        },
      },
    ];
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' }, turn);
    const end = fake
      .streamFrames('agent::events')
      .map((f) => f.data as Record<string, unknown>)
      .find((d) => d.type === 'function_execution_end');
    expect(end).toMatchObject({ function_call_id: 'item-9', is_error: true });
  });
});
