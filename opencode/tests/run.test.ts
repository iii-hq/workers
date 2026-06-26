import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('node:child_process', () => ({ spawn: vi.fn() }));

import { spawn } from 'node:child_process';
import { type Config, loadConfig } from '../src/config.js';
import { makeEmitter } from '../src/events.js';
import { executeRun, RunPayloadSchema } from '../src/run.js';
import { fakeIii } from './_helpers/fake-iii.js';
import { ev, fullTurn, newSpawnCapture, scriptedSpawn } from './_helpers/fake-opencode.js';

const spawnMock = vi.mocked(spawn);

async function baseConfig(): Promise<Config> {
  return loadConfig('/nonexistent/config.yaml');
}

async function runTurn(
  payload: Record<string, unknown>,
  events = fullTurn,
  cfgOverrides: Partial<Config> = {},
) {
  const fake = fakeIii();
  const cfg = { ...(await baseConfig()), ...cfgOverrides };
  const capture = newSpawnCapture();
  spawnMock.mockImplementation(scriptedSpawn(events, capture) as never);
  const emit = makeEmitter(fake.iii, cfg.events_stream);
  const emitRaw = makeEmitter(fake.iii, cfg.raw_events_stream);
  const result = await executeRun(fake.iii, cfg, emit, emitRaw, RunPayloadSchema.parse(payload));
  return { fake, capture, result };
}

beforeEach(() => spawnMock.mockReset());

describe('executeRun', () => {
  it('returns the result with usage and cost from the event stream', async () => {
    const { result } = await runTurn({ prompt: 'do it', session_id: 's1' });
    expect(result).toMatchObject({
      session_id: 's1',
      opencode_session_id: 'ses_1',
      result: 'pong',
      stop_reason: 'end',
      is_error: false,
      total_cost_usd: 0.01,
    });
    expect((result.usage as { output_tokens: number }).output_tokens).toBe(5);
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
    expect(start).toMatchObject({ function_id: 'opencode::bash', args: { command: 'ls' } });
    expect(end).toMatchObject({ function_id: 'opencode::bash', is_error: false });
  });

  it('mirrors every raw OpenCode event onto opencode::events', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const raw = fake.streamFrames('opencode::events').map((f) => (f.data as { type: string }).type);
    expect(raw).toEqual(['step_start', 'text', 'tool_use', 'step_finish']);
  });

  it('persists working then done with the opencode session id', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const sets = fake.calls.filter(
      (c) =>
        c.function_id === 'state::set' &&
        (c.payload as { scope?: string }).scope === 'opencode_sessions',
    );
    const statuses = sets.map((c) => (c.payload.value as { status: string }).status);
    expect(statuses[0]).toBe('working');
    expect(statuses[statuses.length - 1]).toBe('done');
    expect(
      (sets[sets.length - 1].payload.value as { opencode_session_id: string }).opencode_session_id,
    ).toBe('ses_1');
  });

  it('prepends the iii context on a fresh session and resumes with --session after', async () => {
    const { capture } = await runTurn({ prompt: 'do it', session_id: 's1' });
    expect(capture.args?.[capture.args.length - 1]).toContain('# iii runtime');
    expect(capture.args).not.toContain('--session');
  });

  it('resumes the prior opencode session and skips the iii context', async () => {
    const fake = fakeIii();
    fake.state.set('opencode_sessions/s1', {
      session_id: 's1',
      opencode_session_id: 'ses_prior',
      cwd: '',
      model: '',
      status: 'done',
      turns: 1,
      total_cost_usd: 0.01,
      usage: null,
      updated_at_ms: 1,
    });
    const cfg = await baseConfig();
    const capture = newSpawnCapture();
    spawnMock.mockImplementation(scriptedSpawn(fullTurn, capture) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    const result = await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'again', session_id: 's1' }),
    );
    expect(capture.args).toContain('--session');
    expect(capture.args?.[capture.args.indexOf('--session') + 1]).toBe('ses_prior');
    expect(capture.args?.[capture.args.length - 1]).toBe('again');
    expect(result.num_turns).toBe(2);
  });

  it('marks the record error and still closes the turn on non-zero exit', async () => {
    const fake = fakeIii();
    const cfg = await baseConfig();
    const capture = newSpawnCapture();
    spawnMock.mockImplementation(
      scriptedSpawn([ev.step_start()], capture, { code: 1, stderr: 'boom' }) as never,
    );
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
    expect(String(result.result)).toContain('boom');
    expect((fake.state.get('opencode_sessions/s1') as { status: string }).status).toBe('error');
    const types = fake.streamFrames('agent::events').map((f) => (f.data as { type: string }).type);
    expect(types).toContain('turn_end');
    expect(types).toContain('agent_end');
  });

  it('flags a failed tool (non-zero exit) as is_error on the end frame', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' }, [
      ev.step_start(),
      ev.tool('bash', { command: 'false' }, '', 'ses_1', 1),
      ev.step_finish(),
    ]);
    const end = fake
      .streamFrames('agent::events')
      .map((f) => f.data as Record<string, unknown>)
      .find((d) => d.type === 'function_execution_end');
    expect(end).toMatchObject({ is_error: true });
  });

  it('rejects a second run while one is live for the session', async () => {
    const fake = fakeIii();
    const cfg = await baseConfig();
    const capture = newSpawnCapture();
    spawnMock.mockImplementation(scriptedSpawn(fullTurn, capture, { hang: true }) as never);
    const emit = makeEmitter(fake.iii, cfg.events_stream);
    const first = executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'a', session_id: 'busy' }),
    );
    await new Promise((r) => setTimeout(r, 20));
    const second = (await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'b', session_id: 'busy' }),
    )) as Record<string, unknown>;
    expect(second).toMatchObject({ session_id: 'busy', busy: true });
    // first never closes (hang); nothing to await
    void first;
  });
});
