import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@anthropic-ai/claude-agent-sdk', () => ({ query: vi.fn() }));

import { query } from '@anthropic-ai/claude-agent-sdk';
import { registerAgentFeeds } from '../src/agent-feed.js';
import { type Config, loadConfig } from '../src/config.js';
import { executeRun, RunPayloadSchema, register } from '../src/run.js';
import { toSessionMessage } from '../src/session-link.js';
import { type FakeIii, fakeIii } from './_helpers/fake-iii.js';
import {
  assistantMsg,
  fullTurn,
  initMsg,
  resultMsg,
  scriptedQuery,
  toolResultMsg,
} from './_helpers/fake-query.js';

/**
 * A sub-agent's chat is read from session-manager (`session::messages`) by
 * every window opened after the run: the event feeds are live-only. These pin
 * that EVERY run linked to a parent session persists its turn there, whichever
 * entrypoint started it — the Social posts children were started with
 * `claude::start` and rendered blank with `message_count: 0`.
 */

const queryMock = vi.mocked(query);

beforeEach(() => {
  queryMock.mockReset();
});

async function cfg(): Promise<Config> {
  return loadConfig('/nonexistent/config.yaml');
}

function appended(fake: FakeIii, sessionId?: string) {
  return fake.calls
    .filter(
      (c) =>
        c.function_id === 'session::append' &&
        (sessionId === undefined || c.payload.session_id === sessionId),
    )
    .map((c) => c.payload.message as Record<string, unknown>);
}

async function run(fake: FakeIii, payload: Record<string, unknown>, turn = fullTurn) {
  queryMock.mockImplementation(scriptedQuery(turn, { interrupted: false }) as never);
  const feeds = registerAgentFeeds(fake.iii);
  return executeRun(
    fake.iii,
    await cfg(),
    feeds.agent.emit,
    feeds.raw.emit,
    RunPayloadSchema.parse(payload),
  );
}

function seedSession(fake: FakeIii, sessionId: string, metadata: Record<string, unknown>) {
  fake.state.set(`session-manager/${sessionId}`, { session_id: sessionId, metadata });
}

describe('child session transcript', () => {
  it('a run with parent_session_id persists the prompt and every message of the turn', async () => {
    const fake = fakeIii();
    await run(fake, { prompt: 'make the film', session_id: 'child-1', parent_session_id: 'p-1' });

    const messages = appended(fake, 'child-1');
    expect(messages.map((m) => m.role)).toEqual(['user', 'assistant', 'function_result']);
    expect(messages[0]).toMatchObject({
      role: 'user',
      content: [{ type: 'text', text: 'make the film' }],
    });
    expect(messages[1]).toMatchObject({
      role: 'assistant',
      agent: 'claude-code',
      provider: 'anthropic',
      model: 'claude-opus-4-8',
      stop_reason: 'end',
    });
    expect(messages[2]).toMatchObject({
      role: 'function_result',
      function_call_id: 'toolu_1',
      function_id: 'claude-code::Bash',
    });
  });

  it('marks the session done only after the last append has completed', async () => {
    // The console opens a finished child on `done`; an empty read at that
    // moment is the blank pane. Hold the final append open: `done` must wait.
    const fake = fakeIii();
    const real = fake.iii.call.bind(fake.iii);
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let appends = 0;
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: Record<string, unknown>;
    }) => {
      if (req.function_id === 'session::append' && ++appends === 3) {
        const out = real(req as never);
        await held;
        return out;
      }
      return real(req as never);
    };
    const running = run(fake, { prompt: 'x', session_id: 'child-2', parent_session_id: 'p-1' });
    const statusDone = () =>
      fake.calls.some(
        (c) => c.function_id === 'session::set-status' && c.payload.status === 'done',
      );
    await vi.waitFor(() => expect(appends).toBe(3));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(statusDone()).toBe(false);
    release();
    await running;
    expect(statusDone()).toBe(true);
  });

  it('never writes into a session another agent owns, even when named as the parent link', async () => {
    // session::ensure adopts an existing session without comparing owners, so
    // a successful link is not proof the session is this worker's child.
    const fake = fakeIii();
    seedSession(fake, 'console-xyz', { surface: 'console' });
    await run(fake, { prompt: 'x', session_id: 'console-xyz', parent_session_id: 'p-1' });
    expect(appended(fake)).toEqual([]);
    expect(fake.calls.some((c) => c.function_id === 'session::set-status')).toBe(false);
  });

  it('keeps the assistant messages when the terminal record save fails', async () => {
    const fake = fakeIii();
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: Record<string, unknown>;
    }) => {
      const value = req.payload?.value as { status?: string } | undefined;
      if (req.function_id === 'state::set' && value?.status === 'done') {
        throw new Error('store down');
      }
      return real(req as never);
    };
    await run(fake, {
      prompt: 'x',
      session_id: 'child-7',
      parent_session_id: 'p-1',
    }).catch(() => {});
    const messages = appended(fake, 'child-7');
    expect(JSON.stringify(messages)).toContain('running ls');
    expect(messages[messages.length - 1]).toMatchObject({
      role: 'assistant',
      stop_reason: 'error',
    });
    const statuses = fake.calls
      .filter((c) => c.function_id === 'session::set-status')
      .map((c) => c.payload.status);
    expect(statuses).toEqual(['working', 'error']);
  });

  it('stores the terminal stop reason on the last assistant message when the turn hits a cap', async () => {
    const fake = fakeIii();
    const turn = [
      initMsg,
      assistantMsg,
      toolResultMsg,
      { ...resultMsg, subtype: 'error_max_turns', is_error: true },
    ];
    await run(fake, { prompt: 'x', session_id: 'child-8', parent_session_id: 'p-1' }, turn);
    const assistants = appended(fake, 'child-8').filter((m) => m.role === 'assistant');
    expect(assistants[assistants.length - 1].stop_reason).toBe('length');
  });

  it('marks the session error when the SDK reports an error result', async () => {
    const fake = fakeIii();
    const turn = [
      initMsg,
      assistantMsg,
      { ...resultMsg, subtype: 'error_during_execution', is_error: true },
    ];
    await run(fake, { prompt: 'x', session_id: 'child-9', parent_session_id: 'p-1' }, turn);
    const statuses = fake.calls
      .filter((c) => c.function_id === 'session::set-status')
      .map((c) => c.payload.status);
    expect(statuses).toEqual(['working', 'error']);
  });

  it('retries a failed ownership lookup before writing instead of dropping the turn', async () => {
    const fake = fakeIii();
    seedSession(fake, 'child-10', { agent: 'claude-code', parent_session_id: 'p-1' });
    const real = fake.iii.call.bind(fake.iii);
    let lookups = 0;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'session::get' && ++lookups === 1) throw new Error('timeout');
      return real(req as never);
    };
    await run(fake, { prompt: 'again', session_id: 'child-10' });
    expect(appended(fake, 'child-10').map((m) => m.role)).toEqual([
      'user',
      'assistant',
      'function_result',
    ]);
  });

  it('stores usage in the session-manager shape so the transcript keeps its token counts', async () => {
    const fake = fakeIii();
    await run(fake, { prompt: 'x', session_id: 'child-3', parent_session_id: 'p-1' });
    const assistant = appended(fake, 'child-3').find((m) => m.role === 'assistant');
    expect(assistant?.usage).toEqual({ input: 5, output: 2, cache_read: 0, cache_write: 0 });
  });

  it('a follow-up turn on an existing claude-code child persists without the parent id', async () => {
    // Orchestrators resume a child with `claude::start { session_id }` alone.
    const fake = fakeIii();
    seedSession(fake, 'child-4', { agent: 'claude-code', parent_session_id: 'p-1' });
    await run(fake, { prompt: 'now shorten it', session_id: 'child-4' });
    const messages = appended(fake, 'child-4');
    expect(messages[0]).toMatchObject({
      role: 'user',
      content: [{ type: 'text', text: 'now shorten it' }],
    });
    expect(messages.some((m) => m.role === 'assistant')).toBe(true);
    const statuses = fake.calls
      .filter((c) => c.function_id === 'session::set-status')
      .map((c) => c.payload.status);
    expect(statuses).toEqual(['working', 'done']);
  });

  it('never writes into a session another agent owns', async () => {
    // A harness chat that drives claude::run with its own session id keeps
    // its own transcript; this worker must not double it.
    const fake = fakeIii();
    seedSession(fake, 'console-abc', { surface: 'console' });
    await run(fake, { prompt: 'x', session_id: 'console-abc' });
    expect(appended(fake)).toEqual([]);
    expect(fake.calls.some((c) => c.function_id === 'session::set-status')).toBe(false);
  });

  it('a plain run with no parent and no session writes nothing to session-manager', async () => {
    const fake = fakeIii();
    await run(fake, { prompt: 'x', session_id: 'plain-1' });
    expect(appended(fake)).toEqual([]);
  });

  it('persists what the turn produced when the SDK throws mid-stream', async () => {
    const fake = fakeIii();
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        yield { type: 'system', subtype: 'init', session_id: 'cs-1' };
        throw new Error('spawn failed');
      },
      interrupt: async () => {},
    })) as never);
    const feeds = registerAgentFeeds(fake.iii);
    await executeRun(
      fake.iii,
      await cfg(),
      feeds.agent.emit,
      feeds.raw.emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'child-5', parent_session_id: 'p-1' }),
    );
    const messages = appended(fake, 'child-5');
    expect(messages.map((m) => m.role)).toEqual(['user', 'assistant']);
    expect(messages[1].stop_reason).toBe('error');
    expect(JSON.stringify(messages[1].content)).toContain('spawn failed');
  });

  it('maps SDK stop subtypes onto a stop reason session-manager accepts', async () => {
    // `error_max_turns` is not a session-manager StopReason: sent verbatim the
    // append is rejected and the rest of the transcript is lost.
    const fake = fakeIii();
    const turn = [initMsg, { ...resultMsg, subtype: 'error_max_turns', is_error: true }];
    await run(fake, { prompt: 'x', session_id: 'child-6', parent_session_id: 'p-1' }, turn);
    const assistant = appended(fake, 'child-6').find((m) => m.role === 'assistant');
    expect(assistant?.stop_reason).toBe('length');
  });
});

describe('toSessionMessage', () => {
  it('keeps valid stop reasons and maps the rest', () => {
    const base = {
      role: 'assistant',
      content: [],
      model: 'm',
      provider: 'anthropic',
      timestamp: 1,
    };
    for (const ok of ['end', 'length', 'function_call', 'aborted', 'error']) {
      expect(toSessionMessage({ ...base, stop_reason: ok }).stop_reason).toBe(ok);
    }
    expect(toSessionMessage({ ...base, stop_reason: 'error_max_turns' }).stop_reason).toBe(
      'length',
    );
    expect(toSessionMessage({ ...base, stop_reason: 'error_during_execution' }).stop_reason).toBe(
      'error',
    );
  });

  it('leaves user and function_result messages untouched', () => {
    const user = { role: 'user', content: [{ type: 'text', text: 'hi' }], timestamp: 1 };
    expect(toSessionMessage(user)).toEqual(user);
  });

  it('drops usage it cannot read rather than sending the worker shape', () => {
    const base = { role: 'assistant', content: [], model: 'm', provider: 'a', timestamp: 1 };
    expect(toSessionMessage({ ...base, stop_reason: 'end', usage: null }).usage).toBeNull();
  });
});

describe('entrypoints', () => {
  async function worker(): Promise<FakeIii> {
    const fake = fakeIii();
    const config = await cfg();
    const feeds = registerAgentFeeds(fake.iii);
    register(fake.iii, () => config, feeds.agent.emit, feeds.raw.emit);
    return fake;
  }

  it('claude::start with parent_session_id persists the child transcript', async () => {
    const fake = await worker();
    queryMock.mockImplementation(scriptedQuery(fullTurn, { interrupted: false }) as never);
    const res = (await fake.registered.get('claude::start')?.({
      prompt: 'film 1',
      session_id: 'child-start',
      parent_session_id: 'p-1',
    })) as { session_id: string };
    await vi.waitFor(() => {
      expect(fake.state.get(`claude_sessions/${res.session_id}`)).toMatchObject({
        status: 'done',
      });
      expect(appended(fake, 'child-start').map((m) => m.role)).toEqual([
        'user',
        'assistant',
        'function_result',
      ]);
    });
  });

  it('claude::run with parent_session_id persists the child transcript', async () => {
    const fake = await worker();
    queryMock.mockImplementation(scriptedQuery(fullTurn, { interrupted: false }) as never);
    await fake.registered.get('claude::run')?.({
      prompt: 'trawl slack',
      session_id: 'child-run',
      parent_session_id: 'p-1',
    });
    expect(appended(fake, 'child-run')[0]).toMatchObject({
      role: 'user',
      content: [{ type: 'text', text: 'trawl slack' }],
    });
  });

  it('claude::task writes the transcript exactly once', async () => {
    const fake = await worker();
    queryMock.mockImplementation(scriptedQuery(fullTurn, { interrupted: false }) as never);
    await fake.registered.get('claude::task')?.({
      task: 'review',
      session_id: 'child-task',
      parent_session_id: 'p-1',
    });
    await vi.waitFor(() => {
      expect(fake.state.get('agent_tasks/child-task')).toMatchObject({ status: 'done' });
    });
    const roles = appended(fake, 'child-task').map((m) => m.role);
    expect(roles).toEqual(['user', 'assistant', 'function_result']);
  });
});
