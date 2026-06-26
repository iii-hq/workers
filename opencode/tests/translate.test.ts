import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('node:child_process', () => ({ spawn: vi.fn() }));

import { spawn } from 'node:child_process';
import { type Config, loadConfig } from '../src/config.js';
import { makeEmitter } from '../src/events.js';
import { executeRun, RunPayloadSchema } from '../src/run.js';
import { fakeIii } from './_helpers/fake-iii.js';
import { ev, newSpawnCapture, scriptedSpawn } from './_helpers/fake-opencode.js';

const spawnMock = vi.mocked(spawn);
const cfg = async (): Promise<Config> => loadConfig('/nonexistent/config.yaml');

async function run(events: object[], payload: Record<string, unknown> = {}) {
  const fake = fakeIii();
  const c = await cfg();
  spawnMock.mockImplementation(scriptedSpawn(events, newSpawnCapture()) as never);
  const emit = makeEmitter(fake.iii, c.events_stream);
  const emitRaw = makeEmitter(fake.iii, c.raw_events_stream);
  const result = await executeRun(
    fake.iii,
    c,
    emit,
    emitRaw,
    RunPayloadSchema.parse({ prompt: 'x', session_id: 's1', iii_context: false, ...payload }),
  );
  const agent = fake.streamFrames('agent::events').map((f) => f.data as Record<string, unknown>);
  return { fake, result, agent };
}

beforeEach(() => spawnMock.mockReset());

describe('event translation', () => {
  it('emits a message_complete per text event', async () => {
    const { agent } = await run([
      ev.step_start(),
      ev.text('one'),
      ev.text('two'),
      ev.step_finish(),
    ]);
    const msgs = agent.filter((d) => d.type === 'message_complete');
    expect(msgs).toHaveLength(2);
  });

  it('keeps the last text as the result', async () => {
    const { result } = await run([ev.text('first'), ev.text('final')]);
    expect(result.result).toBe('final');
  });

  it('translates each tool_use to a start + end pair in order', async () => {
    const { agent } = await run([
      ev.step_start(),
      ev.tool('bash', { command: 'ls' }, 'a\n'),
      ev.tool('read', { path: '/x' }, 'data'),
      ev.text('done'),
      ev.step_finish(),
    ]);
    const fexec = agent.filter((d) => String(d.type).startsWith('function_execution'));
    expect(fexec.map((d) => d.type)).toEqual([
      'function_execution_start',
      'function_execution_end',
      'function_execution_start',
      'function_execution_end',
    ]);
    expect((fexec[0] as { function_id: string }).function_id).toBe('opencode::bash');
    expect((fexec[2] as { function_id: string }).function_id).toBe('opencode::read');
  });

  it('carries tool input as args and output as result content', async () => {
    const { agent } = await run([
      ev.tool('bash', { command: 'echo hi' }, 'hi\n'),
      ev.step_finish(),
    ]);
    const start = agent.find((d) => d.type === 'function_execution_start') as { args: unknown };
    const end = agent.find((d) => d.type === 'function_execution_end') as {
      result: { content: unknown[] };
    };
    expect(start.args).toEqual({ command: 'echo hi' });
    expect(end.result.content).toEqual([{ type: 'text', text: 'hi\n' }]);
  });

  it('accumulates usage and cost across multiple step_finish events', async () => {
    const { result } = await run([ev.text('a'), ev.step_finish(), ev.text('b'), ev.step_finish()]);
    // two steps, each input 3 / output 5 / cost 0.01
    expect((result.usage as { input_tokens: number }).input_tokens).toBe(6);
    expect((result.usage as { output_tokens: number }).output_tokens).toBe(10);
    expect(result.total_cost_usd).toBeCloseTo(0.02, 6);
  });

  it('captures the opencode session id from the first event', async () => {
    const { result } = await run([
      ev.step_start('ses_abc'),
      ev.text('ok'),
      ev.step_finish('ses_abc'),
    ]);
    expect(result.opencode_session_id).toBe('ses_abc');
  });

  it('always closes with turn_end then agent_end', async () => {
    const { agent } = await run([ev.text('x'), ev.step_finish()]);
    const tail = agent.slice(-2).map((d) => d.type);
    expect(tail).toEqual(['turn_end', 'agent_end']);
  });

  it('handles an empty event stream without throwing', async () => {
    const { agent, result } = await run([]);
    expect(result.is_error).toBe(false);
    expect(agent.map((d) => d.type)).toEqual(['turn_end', 'agent_end']);
  });

  it('surfaces usage on the agent_end frame', async () => {
    const { agent } = await run([ev.text('x'), ev.step_finish()]);
    const end = agent.find((d) => d.type === 'agent_end') as {
      messages: Array<{ usage?: unknown }>;
    };
    expect(end.messages[0].usage).toMatchObject({ output_tokens: 5 });
  });

  it('drops a malformed JSON line but still completes', async () => {
    const fake = fakeIii();
    const c = await cfg();
    // a raw (non-JSON) line is emitted verbatim before a valid event; the
    // parser must skip it and the turn still finishes from the good events.
    spawnMock.mockImplementation(
      scriptedSpawn(
        [
          '{ this is not json',
          { type: 'text', sessionID: 'ses_1', part: { type: 'text', text: 'ok' } },
          ev.step_finish(),
        ],
        newSpawnCapture(),
      ) as never,
    );
    const emit = makeEmitter(fake.iii, c.events_stream);
    const result = await executeRun(
      fake.iii,
      c,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 's1', iii_context: false }),
    );
    expect(result.result).toBe('ok');
    expect(result.is_error).toBe(false);
    // the garbage line is not mirrored as a parsed event
    const raw = fake.streamFrames('opencode::events').map((f) => f.data as { type?: string });
    expect(raw.every((d) => typeof d.type === 'string')).toBe(true);
  });
});
