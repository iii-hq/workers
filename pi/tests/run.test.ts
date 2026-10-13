import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../src/session.js', () => ({ buildSession: vi.fn() }));

import { type Config, loadConfig } from '../src/config.js';
import { AGENT_EVENT_TRIGGER, RAW_EVENT_TRIGGER, registerAgentFeeds } from '../src/agent-feed.js';
import { executeRun, RunPayloadSchema } from '../src/run.js';
import { buildSession } from '../src/session.js';
import { type FakeIii, fakeIii } from './_helpers/fake-iii.js';
import {
  fullTurnEvents,
  newCapture,
  type PiEvent,
  scriptedSession,
  type SessionCapture,
} from './_helpers/fake-session.js';

const buildMock = vi.mocked(buildSession);

/** Both feeds, with a consumer bound to each for `session_id`. */
async function boundFeeds(fake: FakeIii, session_id = 's1') {
  const feeds = registerAgentFeeds(fake.iii);
  await fake.bindFeed(AGENT_EVENT_TRIGGER, session_id);
  await fake.bindFeed(RAW_EVENT_TRIGGER, session_id);
  return feeds;
}

async function baseConfig(): Promise<Config> {
  return loadConfig('/nonexistent/config.yaml');
}

function script(events: PiEvent[] = fullTurnEvents, throwOnPrompt?: Error): SessionCapture {
  const capture = newCapture();
  buildMock.mockImplementation(async (opts) => {
    capture.buildOpts = opts;
    return scriptedSession({ events, throwOnPrompt }, capture);
  });
  return capture;
}

async function runTurn(
  payload: Record<string, unknown>,
  events: PiEvent[] = fullTurnEvents,
  cfgOverrides: Partial<Config> = {},
) {
  const fake = fakeIii();
  const cfg = { ...(await baseConfig()), ...cfgOverrides };
  const capture = script(events);
  const feeds = await boundFeeds(fake, String(payload.session_id ?? 's1'));
  const emit = feeds.agent.emit;
  const emitRaw = feeds.raw.emit;
  const result = await executeRun(fake.iii, cfg, emit, emitRaw, RunPayloadSchema.parse(payload));
  return { fake, capture, result };
}

beforeEach(() => {
  buildMock.mockReset();
});

describe('executeRun', () => {
  it('releases the live slot even when the working-save rejects (no stuck busy)', async () => {
    const fake = fakeIii();
    const cfg = await baseConfig();
    script();
    // make every state::set reject — the working-save during setup throws
    const realTrigger = fake.iii.trigger.bind(fake.iii);
    (fake.iii as { trigger: unknown }).trigger = async (req: { function_id: string }) => {
      if (req.function_id === 'state::set') throw new Error('store down');
      return realTrigger(req as never);
    };
    const emit = (await boundFeeds(fake)).agent.emit;
    await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'leak-1' }),
    ).catch(() => {});
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
      pi_session_id: 'cs-1',
      result: 'done',
      stop_reason: 'end',
      is_error: false,
      num_turns: 1,
      total_cost_usd: 0.01,
      usage: { input_tokens: 5, output_tokens: 2 },
    });
  });

  it('persists the session record working then done with the Pi session id and file', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const sets = fake.calls.filter(
      (c) =>
        c.function_id === 'state::set' && (c.payload as { scope?: string }).scope === 'pi_sessions',
    );
    const statuses = sets.map((c) => (c.payload.value as { status: string }).status);
    expect(statuses[0]).toBe('working');
    expect(statuses[statuses.length - 1]).toBe('done');
    const final = sets[sets.length - 1].payload.value as Record<string, unknown>;
    expect(final.pi_session_id).toBe('cs-1');
    expect(final.session_file).toBe('/sessions/cs-1.jsonl');
    expect(final.total_cost_usd).toBe(0.01);
  });

  it('mirrors every Pi event verbatim onto pi::raw-event', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const raw = fake.feedFrames(RAW_EVENT_TRIGGER).map((f) => f.event);
    expect(raw).toEqual(fullTurnEvents);
    const sessionIds = fake.feedFrames(RAW_EVENT_TRIGGER).map((f) => f.session_id);
    expect(new Set(sessionIds)).toEqual(new Set(['s1']));
  });

  it('emits the translated AgentEvent sequence on pi::agent-event', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    const types = fake
      .feedFrames(AGENT_EVENT_TRIGGER)
      .map((f) => (f.event as { type: string }).type);
    expect(types).toEqual([
      'function_execution_start',
      'function_execution_end',
      'message_complete',
      'turn_end',
      'agent_end',
    ]);
    const [start, end] = fake
      .feedFrames(AGENT_EVENT_TRIGGER)
      .map((f) => f.event as Record<string, unknown>)
      .filter((d) => String(d.type).startsWith('function_execution'));
    expect(start).toMatchObject({
      function_call_id: 'call_1',
      function_id: 'pi::bash',
      args: { command: 'ls' },
    });
    expect(end).toMatchObject({ function_call_id: 'call_1', is_error: false });
  });

  it('prepends the iii runtime context to the prompt by default on a fresh session', async () => {
    const { capture } = await runTurn({ prompt: 'do it', session_id: 's1' });
    expect(capture.promptText).toContain('# iii runtime');
    expect(capture.promptText).toContain('iii trigger engine::functions::list');
    expect(capture.promptText?.trimEnd().endsWith('do it')).toBe(true);
  });

  it('config-level iii_context: false disables the block for every turn', async () => {
    const { capture } = await runTurn({ prompt: 'do it', session_id: 's1' }, fullTurnEvents, {
      iii_context: false,
    });
    expect(capture.promptText).toBe('do it');
  });

  it('omits the iii context when disabled per turn', async () => {
    const { capture } = await runTurn({
      prompt: 'do it',
      session_id: 's1',
      iii_context: false,
    });
    expect(capture.promptText).toBe('do it');
  });

  it('passes worker defaults and named fields to the session builder', async () => {
    const { capture } = await runTurn({
      prompt: 'x',
      session_id: 's1',
      cwd: '/repo',
      model: 'anthropic/claude-sonnet-4',
      thinking_level: 'high',
      tools: ['read', 'bash'],
    });
    expect(capture.buildOpts).toMatchObject({
      cwd: '/repo',
      model: 'anthropic/claude-sonnet-4',
      thinkingLevel: 'high',
      tools: ['read', 'bash'],
      resumeFile: null,
    });
  });

  it('resumes the prior Pi session file for a known session_id', async () => {
    const fake = fakeIii();
    fake.state.set('pi_sessions/s1', {
      session_id: 's1',
      pi_session_id: 'cs-prior',
      session_file: '/sessions/prior.jsonl',
      cwd: '/repo',
      model: '',
      status: 'done',
      turns: 1,
      total_cost_usd: 0.01,
      usage: null,
      updated_at_ms: 1,
    });
    const cfg = await baseConfig();
    const capture = script();
    const emit = (await boundFeeds(fake)).agent.emit;
    const result = await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({ prompt: 'again', session_id: 's1' }),
    );
    expect(capture.buildOpts?.resumeFile).toBe('/sessions/prior.jsonl');
    // resume skips the iii context prepend — already in conversation history
    expect(capture.promptText).toBe('again');
    expect(result.num_turns).toBe(2);
  });

  it('honors per-turn cwd and model overrides on a resumed session', async () => {
    const fake = fakeIii();
    fake.state.set('pi_sessions/s1', {
      session_id: 's1',
      pi_session_id: 'cs-prior',
      session_file: '/sessions/prior.jsonl',
      cwd: '/old/repo',
      model: 'old/model',
      status: 'done',
      turns: 1,
      total_cost_usd: 0,
      usage: null,
      updated_at_ms: 1,
    });
    const cfg = await baseConfig();
    const capture = script();
    const emit = (await boundFeeds(fake)).agent.emit;
    await executeRun(
      fake.iii,
      cfg,
      emit,
      emit,
      RunPayloadSchema.parse({
        prompt: 'x',
        session_id: 's1',
        cwd: '/new/repo',
        model: 'new/model',
      }),
    );
    expect(capture.buildOpts).toMatchObject({
      cwd: '/new/repo',
      model: 'new/model',
      resumeFile: '/sessions/prior.jsonl',
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
    expect(capture.promptText).toBe('second');
  });

  it('marks the record error and still closes the turn when the prompt throws', async () => {
    const fake = fakeIii();
    const cfg = await baseConfig();
    script([], new Error('spawn failed'));
    const emit = (await boundFeeds(fake)).agent.emit;
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
    const record = fake.state.get('pi_sessions/s1') as { status: string };
    expect(record.status).toBe('error');
    const types = fake
      .feedFrames(AGENT_EVENT_TRIGGER)
      .map((f) => (f.event as { type: string }).type);
    expect(types).toContain('turn_end');
    expect(types).toContain('agent_end');
  });

  it('disposes the session after the turn', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 's1' });
    expect(capture.disposed).toBe(true);
  });

  it('starts a fresh session with no resume file', async () => {
    const { capture } = await runTurn({ prompt: 'x', session_id: 'fresh' });
    expect(capture.buildOpts?.resumeFile).toBeNull();
  });
});

describe('event feed delivery during a turn', () => {
  it('a failing consumer never fails the turn', async () => {
    const fake = fakeIii();
    const cfg = await baseConfig();
    script();
    const feeds = registerAgentFeeds(fake.iii);
    await fake.bindFeed(AGENT_EVENT_TRIGGER, 's1', { function_id: 'gone::agent' });
    await fake.bindFeed(RAW_EVENT_TRIGGER, 's1', { function_id: 'gone::raw' });
    const real = fake.iii.trigger.bind(fake.iii);
    (fake.iii as { trigger: unknown }).trigger = async (req: { function_id: string }) => {
      if (req.function_id.startsWith('gone::')) throw new Error('function_not_found');
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
    expect(result).toMatchObject({ session_id: 's1', result: 'done' });
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('gone::agent'));
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('gone::raw'));
    warn.mockRestore();
  });

  it('does not touch iii-stream at all', async () => {
    const { fake } = await runTurn({ prompt: 'x', session_id: 's1' });
    expect(fake.calls.some((c) => c.function_id.startsWith('stream::'))).toBe(false);
    for (const call of fake.calls.filter((c) => c.function_id.startsWith('test::on::'))) {
      expect(call.action).toEqual({ type: 'void' });
    }
  });
});
