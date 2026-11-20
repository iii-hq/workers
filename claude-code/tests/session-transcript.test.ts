import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@anthropic-ai/claude-agent-sdk', () => ({ query: vi.fn() }));

import { query } from '@anthropic-ai/claude-agent-sdk';
import { registerAgentFeeds } from '../src/agent-feed.js';
import { type Config, loadConfig } from '../src/config.js';
import { executeRun, RunPayloadSchema, ranSinceStart, register } from '../src/run.js';
import {
  RESTART_REASON,
  setSessionStatus,
  settleStaleSessions,
  statusRetry,
  toSessionMessage,
} from '../src/session-link.js';
import { streamTiming } from '../src/session-writer.js';
import { type FakeIii, fakeIii } from './_helpers/fake-iii.js';
import {
  assistantMsg,
  fullTurn,
  initMsg,
  resultMsg,
  type QueryCapture,
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
const defaultThrottleMs = streamTiming.throttleMs;

beforeEach(() => {
  queryMock.mockReset();
  // Deterministic streaming: every change is written at once, no timer.
  streamTiming.throttleMs = 0;
});

afterEach(() => {
  streamTiming.throttleMs = defaultThrottleMs;
});

async function cfg(): Promise<Config> {
  return loadConfig('/nonexistent/config.yaml');
}

/** The child's transcript as session-manager holds it (appends + updates applied). */
function appended(fake: FakeIii, sessionId?: string) {
  return fake.sessionMessages(sessionId);
}

function statuses(fake: FakeIii) {
  return fake.calls
    .filter((c) => c.function_id === 'session::set-status')
    .map((c) => c.payload.status);
}

const stream = (event: Record<string, unknown>) => ({ type: 'stream_event', event });

/** One model message streamed in two blocks (text, then a tool call), which
 *  the SDK then reports as TWO assistant messages sharing the message id. */
const streamedTurn = [
  initMsg,
  stream({ type: 'message_start', message: { id: 'msg_1', model: 'claude-opus-4-8' } }),
  stream({ type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } }),
  stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Hel' } }),
  stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'lo' } }),
  stream({ type: 'content_block_stop', index: 0 }),
  {
    type: 'assistant',
    message: { id: 'msg_1', model: 'claude-opus-4-8', content: [{ type: 'text', text: 'Hello' }] },
  },
  stream({
    type: 'content_block_start',
    index: 1,
    content_block: { type: 'tool_use', id: 'toolu_1', name: 'Bash', input: {} },
  }),
  stream({
    type: 'content_block_delta',
    index: 1,
    delta: { type: 'input_json_delta', partial_json: '{"command":' },
  }),
  stream({
    type: 'content_block_delta',
    index: 1,
    delta: { type: 'input_json_delta', partial_json: '"ls"}' },
  }),
  stream({ type: 'content_block_stop', index: 1 }),
  {
    type: 'assistant',
    message: {
      id: 'msg_1',
      model: 'claude-opus-4-8',
      usage: { input_tokens: 5, output_tokens: 2 },
      content: [{ type: 'tool_use', id: 'toolu_1', name: 'Bash', input: { command: 'ls' } }],
    },
  },
  // The SDK's per-message usage is a snapshot from the message start; the
  // final output count arrives on the stream's message_delta.
  stream({ type: 'message_delta', usage: { output_tokens: 42 } }),
  stream({ type: 'message_stop' }),
  toolResultMsg,
  resultMsg,
];

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

describe('live child session', () => {
  it('shows the prompt and a working status while the turn is still running', async () => {
    const fake = fakeIii();
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        yield initMsg;
        await gate;
        yield* fullTurn.slice(1);
      },
      interrupt: async () => {},
    })) as never);
    const feeds = registerAgentFeeds(fake.iii);
    const running = executeRun(
      fake.iii,
      await cfg(),
      feeds.agent.emit,
      feeds.raw.emit,
      RunPayloadSchema.parse({
        prompt: 'make the film',
        session_id: 'live-1',
        parent_session_id: 'p-1',
      }),
    );
    await vi.waitFor(() => {
      expect(appended(fake, 'live-1')).toEqual([
        expect.objectContaining({
          role: 'user',
          content: [{ type: 'text', text: 'make the film' }],
        }),
      ]);
    });
    expect(statuses(fake)).toEqual(['working']);
    release();
    await running;
    expect(appended(fake, 'live-1').map((m) => m.role)).toEqual([
      'user',
      'assistant',
      'function_result',
    ]);
    expect(statuses(fake)).toEqual(['working', 'done']);
  });

  it('streams a model message into ONE assistant entry and finishes it with usage', async () => {
    const fake = fakeIii();
    await run(
      fake,
      { prompt: 'x', session_id: 'stream-1', parent_session_id: 'p-1' },
      streamedTurn,
    );

    const assistantAppends = fake.calls.filter(
      (c) =>
        c.function_id === 'session::append' &&
        c.payload.session_id === 'stream-1' &&
        (c.payload.message as { role?: string }).role === 'assistant',
    );
    expect(assistantAppends).toHaveLength(1);

    // The text grew in place before the message was complete.
    const texts = fake.calls
      .filter((c) => c.function_id === 'session::update-message')
      .map((c) => (c.payload.content as Array<{ text?: string }>)[0]?.text);
    expect(texts).toContain('Hello');
    const first = (assistantAppends[0].payload.message as { content: Array<{ text: string }> })
      .content[0].text;
    expect(first).toBe('Hel');

    const messages = appended(fake, 'stream-1');
    expect(messages.map((m) => m.role)).toEqual(['user', 'assistant', 'function_result']);
    expect(messages[1]).toMatchObject({
      role: 'assistant',
      content: [
        { type: 'text', text: 'Hello' },
        {
          type: 'function_call',
          id: 'toolu_1',
          function_id: 'claude-code::Bash',
          arguments: { command: 'ls' },
        },
      ],
      usage: { input: 5, output: 42, cache_read: 0, cache_write: 0 },
    });
  });

  it('asks the SDK for partial messages only when a child session is streamed', async () => {
    const child = fakeIii();
    const childCapture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, childCapture) as never);
    const childFeeds = registerAgentFeeds(child.iii);
    await executeRun(
      child.iii,
      await cfg(),
      childFeeds.agent.emit,
      childFeeds.raw.emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'c-1', parent_session_id: 'p-1' }),
    );
    expect(childCapture.options?.includePartialMessages).toBe(true);

    const plain = fakeIii();
    const plainCapture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, plainCapture) as never);
    const plainFeeds = registerAgentFeeds(plain.iii);
    await executeRun(
      plain.iii,
      await cfg(),
      plainFeeds.agent.emit,
      plainFeeds.raw.emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'plain-2' }),
    );
    expect(plainCapture.options).not.toHaveProperty('includePartialMessages');
  });
});

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

describe('review: ownership, status and write completeness', () => {
  function failing(fake: FakeIii, fn: string, times = Number.POSITIVE_INFINITY) {
    const real = fake.iii.call.bind(fake.iii);
    let n = 0;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === fn && n++ < times) {
        fake.calls.push({ function_id: req.function_id, payload: {} });
        throw new Error('timeout');
      }
      return real(req as never);
    };
  }

  it('a new child still saves when session::get keeps failing: ensure proved ownership', async () => {
    const fake = fakeIii();
    failing(fake, 'session::get');
    await run(fake, { prompt: 'x', session_id: 'new-1', parent_session_id: 'p-1' });
    expect(appended(fake, 'new-1').map((m) => m.role)).toEqual([
      'user',
      'assistant',
      'function_result',
    ]);
    expect(statuses(fake)).toEqual(['working', 'done']);
  });

  it('sets working before the first write when an ownership retry confirms the child', async () => {
    const fake = fakeIii();
    seedSession(fake, 'resume-1', { agent: 'claude-code', parent_session_id: 'p-1' });
    failing(fake, 'session::get', 1);
    await run(fake, { prompt: 'again', session_id: 'resume-1' });
    const working = fake.calls.findIndex(
      (c) => c.function_id === 'session::set-status' && c.payload.status === 'working',
    );
    const firstAppend = fake.calls.findIndex((c) => c.function_id === 'session::append');
    expect(working).toBeGreaterThan(-1);
    expect(working).toBeLessThan(firstAppend);
    expect(statuses(fake)).toEqual(['working', 'done']);
  });

  it('keeps the failure text as the status reason when the turn fails after partial output', async () => {
    const fake = fakeIii();
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        yield initMsg;
        yield assistantMsg;
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
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'partial-1', parent_session_id: 'p-1' }),
    );
    const error = fake.calls.find(
      (c) => c.function_id === 'session::set-status' && c.payload.status === 'error',
    );
    expect(String(error?.payload.reason)).toContain('spawn failed');
    const messages = appended(fake, 'partial-1');
    expect(JSON.stringify(messages)).toContain('running ls');
    expect(JSON.stringify(messages[messages.length - 1])).toContain('spawn failed');
  });

  it('retries a failed append with the same entry id and does not duplicate it', async () => {
    const fake = fakeIii();
    const real = fake.iii.call.bind(fake.iii);
    let appends = 0;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      // The second append commits, then times out on the way back.
      if (req.function_id === 'session::append' && ++appends === 2) {
        await real(req as never);
        throw new Error('timeout');
      }
      return real(req as never);
    };
    await run(fake, { prompt: 'x', session_id: 'retry-1', parent_session_id: 'p-1' });
    expect(appended(fake, 'retry-1').map((m) => m.role)).toEqual([
      'user',
      'assistant',
      'function_result',
    ]);
    expect(statuses(fake)).toEqual(['working', 'done']);
  });

  it('never marks the child done when its transcript could not be written in full', async () => {
    const fake = fakeIii();
    const real = fake.iii.call.bind(fake.iii);
    let appends = 0;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'session::append' && ++appends > 1) throw new Error('down');
      return real(req as never);
    };
    await run(fake, { prompt: 'x', session_id: 'broken-1', parent_session_id: 'p-1' });
    expect(statuses(fake)).not.toContain('done');
    expect(statuses(fake)[statuses(fake).length - 1]).toBe('error');
  });
});

describe('claude::start busy slot', () => {
  it('reports busy instead of started when the session is still running', async () => {
    const fake = fakeIii();
    const config = await cfg();
    const feeds = registerAgentFeeds(fake.iii);
    register(fake.iii, () => config, feeds.agent.emit, feeds.raw.emit);
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        yield initMsg;
        await gate;
        yield resultMsg;
      },
      interrupt: async () => {},
    })) as never);
    await fake.registered.get('claude::start')?.({ prompt: 'one', session_id: 'busy-1' });
    await vi.waitFor(() => expect(queryMock).toHaveBeenCalledTimes(1));
    const second = (await fake.registered.get('claude::start')?.({
      prompt: 'two',
      session_id: 'busy-1',
    })) as Record<string, unknown>;
    expect(second).toMatchObject({ session_id: 'busy-1', started: false, busy: true });
    release();
  });
});

describe('feed failures', () => {
  it('a rejecting raw feed loses nothing: the child transcript and the turn complete', async () => {
    const fake = fakeIii();
    queryMock.mockImplementation(scriptedQuery(fullTurn, { interrupted: false }) as never);
    const feeds = registerAgentFeeds(fake.iii);
    const rejecting = async () => {
      throw new Error('raw feed down');
    };
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const result = await executeRun(
      fake.iii,
      await cfg(),
      feeds.agent.emit,
      rejecting,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'raw-1', parent_session_id: 'p-1' }),
    );
    warn.mockRestore();
    expect(result).toMatchObject({ result: 'done', is_error: false });
    expect(appended(fake, 'raw-1').map((m) => m.role)).toEqual([
      'user',
      'assistant',
      'function_result',
    ]);
    expect(statuses(fake)).toEqual(['working', 'done']);
  });
});

describe('append retry race', () => {
  it('a retried append that hit an already-committed older write still ends with the full message', async () => {
    // The first assistant append commits its snapshot ("Hel") but its reply
    // is held until the stream has finished, then times out. The idempotent
    // retry returns the OLD stored entry, so the writer must still write the
    // current content before the session reads done.
    const fake = fakeIii();
    const real = fake.iii.call.bind(fake.iii);
    let release: () => void = () => {};
    const streamDone = new Promise<void>((resolve) => {
      release = resolve;
    });
    let committed: () => void = () => {};
    const firstCommitted = new Promise<void>((resolve) => {
      committed = resolve;
    });
    let held = false;
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: Record<string, unknown>;
    }) => {
      const role = (req.payload?.message as { role?: string } | undefined)?.role;
      if (req.function_id === 'session::append' && role === 'assistant' && !held) {
        held = true;
        await real(req as never);
        committed();
        await streamDone;
        // Fail only once the run is already waiting in finish(), with its
        // final updates queued behind this append.
        await new Promise((resolve) => setTimeout(resolve, 20));
        throw new Error('timeout');
      }
      return real(req as never);
    };
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        // Up to the first text delta ("Hel"), then wait until that snapshot
        // has committed, then stream the rest while its reply is held.
        const firstDelta = streamedTurn.findIndex(
          (m) => (m as { event?: { delta?: { text?: string } } }).event?.delta?.text === 'Hel',
        );
        yield* streamedTurn.slice(0, firstDelta + 1);
        await firstCommitted;
        yield* streamedTurn.slice(firstDelta + 1);
        release();
      },
      interrupt: async () => {},
    })) as never);
    const feeds = registerAgentFeeds(fake.iii);
    await executeRun(
      fake.iii,
      await cfg(),
      feeds.agent.emit,
      feeds.raw.emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: 'race-1', parent_session_id: 'p-1' }),
    );
    const assistant = appended(fake, 'race-1').find((m) => m.role === 'assistant');
    expect(assistant?.content).toEqual([
      { type: 'text', text: 'Hello' },
      {
        type: 'function_call',
        id: 'toolu_1',
        function_id: 'claude-code::Bash',
        arguments: { command: 'ls' },
      },
    ]);
    expect(statuses(fake)).toEqual(['working', 'done']);
  });
});

describe('review: terminal status, settled entries, partial blocks, stop and errors', () => {
  const defaultDelays = statusRetry.delaysMs;
  beforeEach(() => {
    statusRetry.delaysMs = [0, 0];
  });
  afterEach(() => {
    statusRetry.delaysMs = defaultDelays;
  });

  async function worker(): Promise<FakeIii> {
    const fake = fakeIii();
    const config = await cfg();
    const feeds = registerAgentFeeds(fake.iii);
    register(fake.iii, () => config, feeds.agent.emit, feeds.raw.emit);
    return fake;
  }

  /** Fails the first `times` set-status writes of `status`, without recording them. */
  function failStatus(fake: FakeIii, status: string, times: number) {
    const real = fake.iii.call.bind(fake.iii);
    let n = 0;
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: { status?: string };
    }) => {
      if (
        req.function_id === 'session::set-status' &&
        req.payload.status === status &&
        n++ < times
      ) {
        throw new Error('timeout');
      }
      return real(req as never);
    };
  }

  it('retries the terminal done status when its first write fails', async () => {
    const fake = fakeIii();
    failStatus(fake, 'done', 1);
    await run(fake, { prompt: 'x', session_id: 'st-1', parent_session_id: 'p-1' });
    expect(statuses(fake)).toEqual(['working', 'done']);
  });

  it('retries the terminal error status of an incomplete transcript', async () => {
    const fake = fakeIii();
    failStatus(fake, 'error', 2);
    const real = fake.iii.call.bind(fake.iii);
    let appends = 0;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'session::append' && ++appends > 1) throw new Error('down');
      return real(req as never);
    };
    await run(fake, { prompt: 'x', session_id: 'st-2', parent_session_id: 'p-1' });
    expect(statuses(fake)).toEqual(['working', 'error']);
  });

  function statusesOf(fake: FakeIii, sessionId: string) {
    return fake.calls
      .filter((c) => c.function_id === 'session::set-status' && c.payload.session_id === sessionId)
      .map((c) => c.payload.status);
  }

  it('writes a status that exhausted its retries once session-manager answers again', async () => {
    const fake = fakeIii();
    // Every attempt at `done` fails during the first run: retries exhausted.
    failStatus(fake, 'done', 3);
    await run(fake, { prompt: 'x', session_id: 'lost-1', parent_session_id: 'p-1' });
    expect(statusesOf(fake, 'lost-1')).toEqual(['working']);
    // The next status write that succeeds, for any session, proves the
    // store is back: the lost `done` is written then.
    await run(fake, { prompt: 'y', session_id: 'other-1', parent_session_id: 'p-1' });
    // The replay runs in the background, without holding that run.
    await vi.waitFor(() => expect(statusesOf(fake, 'lost-1')).toEqual(['working', 'done']));
    expect(statusesOf(fake, 'other-1')).toEqual(['working', 'done']);
    // Written once: a later success does not repeat it.
    await run(fake, { prompt: 'z', session_id: 'other-2', parent_session_id: 'p-1' });
    expect(statusesOf(fake, 'lost-1')).toEqual(['working', 'done']);
  });

  it('a new run on the same session replaces a status that was never written', async () => {
    const fake = fakeIii();
    failStatus(fake, 'done', 3);
    await run(fake, { prompt: 'x', session_id: 'lost-2', parent_session_id: 'p-1' });
    // The follow-up turn owns the status now: the old `done` must not land
    // after its `working`.
    await run(fake, { prompt: 'again', session_id: 'lost-2' });
    expect(statusesOf(fake, 'lost-2')).toEqual(['working', 'working', 'done']);
  });

  /** Holds race-1's first `done` before it commits; records what commits for it. */
  function holdReplay(fake: FakeIii, sessionId: string) {
    const committed: unknown[] = [];
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const state = { held: false };
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: { session_id?: string; status?: string };
    }) => {
      const ours =
        req.function_id === 'session::set-status' && req.payload.session_id === sessionId;
      if (ours && req.payload.status === 'done' && !state.held) {
        state.held = true;
        await gate;
      }
      const out = await real(req as never);
      if (ours) committed.push(req.payload.status);
      return out;
    };
    return { committed, release, state };
  }

  it('a replay in flight never overwrites a newer status of the same session', async () => {
    const fake = fakeIii();
    failStatus(fake, 'done', 3);
    await run(fake, { prompt: 'x', session_id: 'race-1', parent_session_id: 'p-1' });
    const hold = holdReplay(fake, 'race-1');
    // Another session's write succeeds: the replay starts and is held.
    await setSessionStatus(fake.iii, 'other-3', 'working');
    await vi.waitFor(() => expect(hold.state.held).toBe(true));
    // The newer write is requested while the replay is in flight: it queues.
    const newer = setSessionStatus(fake.iii, 'race-1', 'working');
    hold.release();
    expect(await newer).toBe(true);
    expect(hold.committed).toEqual(['done', 'working']);
  });

  it('a success during a sweep runs the sweep again for what was lost meanwhile', async () => {
    const fake = fakeIii();
    // `done` fails for the sessions in `down`; sweep-a's replay is then held.
    const down = new Set(['sweep-a', 'sweep-c']);
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let held = false;
    let holding = false;
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: { session_id?: string; status?: string };
    }) => {
      const id = String(req.payload.session_id);
      if (req.function_id === 'session::set-status' && req.payload.status === 'done') {
        if (holding && id === 'sweep-a' && !held) {
          held = true;
          await gate;
        } else if (down.has(id)) {
          throw new Error('timeout');
        }
      }
      return real(req as never);
    };
    const { settleSessionStatus } = await import('../src/session-link.js');
    expect(await settleSessionStatus(fake.iii, 'sweep-a', 'done')).toBe(false);
    // A success starts a sweep, held on sweep-a's replay.
    holding = true;
    down.delete('sweep-a');
    await setSessionStatus(fake.iii, 'other-4', 'working');
    await vi.waitFor(() => expect(held).toBe(true));
    // During the sweep, sweep-c loses its `done`, then a write succeeds.
    expect(await settleSessionStatus(fake.iii, 'sweep-c', 'done')).toBe(false);
    down.delete('sweep-c');
    await setSessionStatus(fake.iii, 'other-5', 'working');
    release();
    // The pass that was running took its list before sweep-c was lost: only
    // a second pass writes it.
    await vi.waitFor(() => {
      const done = fake.calls.filter(
        (c) =>
          c.function_id === 'session::set-status' &&
          c.payload.session_id === 'sweep-c' &&
          c.payload.status === 'done',
      );
      expect(done.length).toBeGreaterThan(0);
    });
  });

  it('a replay that fails does not stop the sweep for the others', async () => {
    const fake = fakeIii();
    // Both lose their `done`; first-bad keeps failing, second-ok recovers.
    const down = new Set(['first-bad', 'second-ok']);
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: { session_id?: string; status?: string };
    }) => {
      if (
        req.function_id === 'session::set-status' &&
        req.payload.status === 'done' &&
        down.has(String(req.payload.session_id))
      ) {
        throw new Error('timeout');
      }
      return real(req as never);
    };
    await run(fake, { prompt: 'x', session_id: 'first-bad', parent_session_id: 'p-1' });
    await run(fake, { prompt: 'x', session_id: 'second-ok', parent_session_id: 'p-1' });
    down.delete('second-ok');
    await setSessionStatus(fake.iii, 'other-6', 'working');
    await vi.waitFor(() => expect(statusesOf(fake, 'second-ok')).toEqual(['working', 'done']));
  });

  /** A session-manager record for `id`, owned by claude-code and `working`. */
  function staleSession(fake: FakeIii, id: string) {
    seedSession(fake, id, { agent: 'claude-code' });
    const meta = fake.state.get(`session-manager/${id}`) as Record<string, unknown>;
    fake.state.set(`session-manager/${id}`, { ...meta, status: 'working' });
  }

  /** Holds the first restart `error` write for `id`, then fails it. */
  function holdThenFailRestart(fake: FakeIii, id: string) {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const state = { held: false };
    const committed: unknown[] = [];
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: { session_id?: string; status?: string };
    }) => {
      const ours = req.function_id === 'session::set-status' && req.payload.session_id === id;
      if (ours && req.payload.status === 'error' && !state.held) {
        state.held = true;
        await gate;
        throw new Error('timeout');
      }
      const out = await real(req as never);
      if (ours) committed.push(req.payload.status);
      return out;
    };
    return { release, state, committed };
  }

  it('a restart error that fails after a success meanwhile is replayed at once', async () => {
    const fake = fakeIii();
    staleSession(fake, 'pend-1');
    const hold = holdThenFailRestart(fake, 'pend-1');
    const scan = settleStaleSessions(fake.iii, () => false);
    await vi.waitFor(() => expect(hold.state.held).toBe(true));
    // The store answers while the restart write is pending...
    await setSessionStatus(fake.iii, 'other-8', 'working');
    hold.release();
    await scan;
    // ...so the failed write is replayed without waiting for another success.
    await vi.waitFor(() => expect(hold.committed).toEqual(['error']));
  });

  it('never saves a restart replay for a session a turn started in meanwhile', async () => {
    const fake = fakeIii();
    staleSession(fake, 'pend-2');
    const hold = holdThenFailRestart(fake, 'pend-2');
    const scan = settleStaleSessions(fake.iii, ranSinceStart);
    await vi.waitFor(() => expect(hold.state.held).toBe(true));
    // A turn starts on that session while its restart write is pending.
    const turn = run(fake, { prompt: 'x', session_id: 'pend-2' });
    await vi.waitFor(() => expect(ranSinceStart('pend-2')).toBe(true));
    hold.release();
    await scan;
    await turn;
    await setSessionStatus(fake.iii, 'other-9', 'working');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(hold.committed).toEqual(['working', 'done']);
  });

  it('a stop that arrives after an SDK error result does not hide the failure', async () => {
    const fake = await worker();
    let release: () => void = () => {};
    const after = new Promise<void>((resolve) => {
      release = resolve;
    });
    let resultSeen = false;
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        yield initMsg;
        yield {
          type: 'result',
          subtype: 'error_during_execution',
          is_error: true,
          session_id: 'cs-1',
          num_turns: 1,
          errors: ['real failure'],
        };
        resultSeen = true;
        // The stop lands after the error result was received.
        await after;
      },
      interrupt: async () => release(),
    })) as never);
    const started = fake.registered.get('claude::run')?.({ prompt: 'x', session_id: 'late-stop' });
    await vi.waitFor(() => expect(resultSeen).toBe(true));
    await fake.registered.get('claude::stop')?.({ session_id: 'late-stop' });
    const out = (await started) as { stop_reason?: string; is_error?: boolean; result?: string };
    expect(out).toMatchObject({ is_error: true, stop_reason: 'error_during_execution' });
    expect(out.result).toBe('real failure');
  });

  it('the startup scan never settles a session this worker ran since it started', async () => {
    // The scan lists `ran-b` as working, a turn on it then starts AND finishes
    // while the scan is still busy with an earlier session: `done` must stay.
    const fake = fakeIii();
    seedSession(fake, 'stale-a', { agent: 'claude-code' });
    seedSession(fake, 'ran-b', { agent: 'claude-code' });
    for (const id of ['stale-a', 'ran-b']) {
      const meta = fake.state.get(`session-manager/${id}`) as Record<string, unknown>;
      fake.state.set(`session-manager/${id}`, { ...meta, status: 'working' });
    }
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let listed = false;
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: { session_id?: string; status?: string };
    }) => {
      const out = await real(req as never);
      if (req.function_id === 'session::list') listed = true;
      if (req.function_id === 'session::set-status' && req.payload.session_id === 'stale-a') {
        await gate;
      }
      return out;
    };
    const scan = settleStaleSessions(fake.iii, ranSinceStart);
    await vi.waitFor(() => expect(listed).toBe(true));
    await run(fake, { prompt: 'x', session_id: 'ran-b' });
    release();
    await scan;
    const meta = (id: string) => fake.state.get(`session-manager/${id}`) as Record<string, unknown>;
    expect(meta('ran-b').status).toBe('done');
    expect(meta('stale-a')).toMatchObject({ status: 'error', status_reason: RESTART_REASON });
  });

  it('a restart error that could not be written is replayed once the store answers', async () => {
    const fake = fakeIii();
    seedSession(fake, 'stale-c', { agent: 'claude-code' });
    const meta = fake.state.get('session-manager/stale-c') as Record<string, unknown>;
    fake.state.set('session-manager/stale-c', { ...meta, status: 'working' });
    failStatus(fake, 'error', 1);
    expect(await settleStaleSessions(fake.iii, () => false)).toBe(0);
    await setSessionStatus(fake.iii, 'other-7', 'working');
    await vi.waitFor(() =>
      expect(fake.state.get('session-manager/stale-c')).toMatchObject({
        status: 'error',
        status_reason: RESTART_REASON,
      }),
    );
  });

  it('uses the SDK errors when a failed result has empty result text', async () => {
    const fake = fakeIii();
    const turn = [
      initMsg,
      {
        ...resultMsg,
        subtype: 'error_during_execution',
        is_error: true,
        result: '',
        errors: ['quota exceeded'],
      },
    ];
    const out = (await run(fake, { prompt: 'x' }, turn)) as { result?: string };
    expect(out.result).toBe('quota exceeded');
  });

  it('never rewrites an entry whose content and usage are already stored', async () => {
    // A rewrite of a settled entry is a write that can fail twice and turn a
    // complete transcript into an `error` session; refuse every such no-op.
    const fake = fakeIii();
    const real = fake.iii.call.bind(fake.iii);
    const stored = new Map<string, string>();
    let noops = 0;
    (fake.iii as { call: unknown }).call = async (req: {
      function_id: string;
      payload: Record<string, unknown>;
    }) => {
      const p = req.payload;
      if (req.function_id === 'session::append') {
        const m = p.message as Record<string, unknown>;
        const out = await real(req as never);
        stored.set(String(p.entry_id), JSON.stringify([m.content, m.usage ?? null]));
        return out;
      }
      if (req.function_id === 'session::update-message') {
        const now = JSON.stringify([p.content, p.usage ?? null]);
        if (stored.get(String(p.entry_id)) === now) {
          noops++;
          throw new Error('timeout');
        }
        const out = await real(req as never);
        stored.set(String(p.entry_id), now);
        return out;
      }
      return real(req as never);
    };
    await run(
      fake,
      { prompt: 'x', session_id: 'settled-1', parent_session_id: 'p-1' },
      streamedTurn,
    );
    expect(noops).toBe(0);
    expect(statuses(fake)).toEqual(['working', 'done']);
    const assistant = appended(fake, 'settled-1').find((m) => m.role === 'assistant');
    expect(assistant?.content).toEqual([
      { type: 'text', text: 'Hello' },
      {
        type: 'function_call',
        id: 'toolu_1',
        function_id: expect.any(String),
        arguments: { command: 'ls' },
      },
    ]);
  });

  /** One block completes, the next is cut off mid-stream by a failure. */
  function cutOffTurn(second: Record<string, unknown>, delta: Record<string, unknown>) {
    return (() => ({
      async *[Symbol.asyncIterator]() {
        yield initMsg;
        yield stream({ type: 'message_start', message: { id: 'msg_c', model: 'm' } });
        yield stream({
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'text', text: '' },
        });
        yield stream({
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'text_delta', text: 'Done part' },
        });
        yield stream({ type: 'content_block_stop', index: 0 });
        yield {
          type: 'assistant',
          message: { id: 'msg_c', model: 'm', content: [{ type: 'text', text: 'Done part' }] },
        };
        yield stream({ type: 'content_block_start', index: 1, content_block: second });
        yield stream({ type: 'content_block_delta', index: 1, delta });
        throw new Error('connection reset');
      },
      interrupt: async () => {},
    })) as never;
  }

  async function runCutOff(fake: FakeIii, sessionId: string) {
    const feeds = registerAgentFeeds(fake.iii);
    await executeRun(
      fake.iii,
      await cfg(),
      feeds.agent.emit,
      feeds.raw.emit,
      RunPayloadSchema.parse({ prompt: 'x', session_id: sessionId, parent_session_id: 'p-1' }),
    );
    return appended(fake, sessionId).filter((m) => m.role === 'assistant');
  }

  it('keeps a streamed block that was cut off before it completed, after the completed ones', async () => {
    const fake = fakeIii();
    queryMock.mockImplementation(
      cutOffTurn({ type: 'text', text: '' }, { type: 'text_delta', text: 'Half wri' }),
    );
    const [message, note] = await runCutOff(fake, 'cut-1');
    expect(message.content).toEqual([
      { type: 'text', text: 'Done part' },
      { type: 'text', text: 'Half wri' },
    ]);
    expect(JSON.stringify(note.content)).toContain('connection reset');
  });

  it('drops a tool call that was cut off: it never ran and its arguments are incomplete', async () => {
    const fake = fakeIii();
    queryMock.mockImplementation(
      cutOffTurn(
        { type: 'tool_use', id: 'toolu_cut', name: 'Bash', input: {} },
        { type: 'input_json_delta', partial_json: '{"comm' },
      ),
    );
    const [message] = await runCutOff(fake, 'cut-2');
    expect(message.content).toEqual([{ type: 'text', text: 'Done part' }]);
  });

  it('keeps a completed block whose SDK message never arrived, without duplicating others', async () => {
    const fake = fakeIii();
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        yield initMsg;
        yield stream({ type: 'message_start', message: { id: 'msg_k', model: 'm' } });
        yield stream({
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'thinking', thinking: '' },
        });
        yield stream({
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'thinking_delta', thinking: 'Plan' },
        });
        yield stream({ type: 'content_block_stop', index: 0 });
        yield {
          type: 'assistant',
          message: { id: 'msg_k', model: 'm', content: [{ type: 'thinking', thinking: 'Plan' }] },
        };
        yield stream({
          type: 'content_block_start',
          index: 1,
          content_block: { type: 'text', text: '' },
        });
        yield stream({
          type: 'content_block_delta',
          index: 1,
          delta: { type: 'text_delta', text: 'Complete answer' },
        });
        yield stream({ type: 'content_block_stop', index: 1 });
        // The SDK fails before it reports block 1 as an assistant message.
        throw new Error('connection reset');
      },
      interrupt: async () => {},
    })) as never);
    const [message] = await runCutOff(fake, 'cut-4');
    const texts = JSON.stringify(message.content);
    expect(texts).toContain('Complete answer');
    expect(texts.match(/Plan/g)).toHaveLength(1);
  });

  it('clears a cut-off tool call that was the only block, even after it was shown live', async () => {
    const fake = fakeIii();
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        yield initMsg;
        yield stream({ type: 'message_start', message: { id: 'msg_t', model: 'm' } });
        yield stream({
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'tool_use', id: 'toolu_first', name: 'Bash', input: {} },
        });
        yield stream({
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'input_json_delta', partial_json: '{"comm' },
        });
        throw new Error('connection reset');
      },
      interrupt: async () => {},
    })) as never);
    const assistants = await runCutOff(fake, 'cut-3');
    // Shown live while it streamed, so an entry exists: it ends empty.
    expect(JSON.stringify(assistants)).not.toContain('toolu_first');
    expect(JSON.stringify(assistants[assistants.length - 1].content)).toContain('connection reset');
  });

  it('a claude::stop mid-query ends the turn as interrupted, not an error', async () => {
    // Live behaviour: an interrupted query answers with an error result
    // (`error_during_execution` and an `[ede_diagnostic]` line), which alone
    // would mark a requested stop as a failure.
    const fake = await worker();
    let release: () => void = () => {};
    const interrupted = new Promise<void>((resolve) => {
      release = resolve;
    });
    let streaming = false;
    queryMock.mockImplementation((() => ({
      async *[Symbol.asyncIterator]() {
        yield initMsg;
        yield stream({ type: 'message_start', message: { id: 'msg_s', model: 'm' } });
        yield stream({
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'text', text: '' },
        });
        yield stream({
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'text_delta', text: 'Once upon' },
        });
        streaming = true;
        await interrupted;
        yield {
          type: 'result',
          subtype: 'error_during_execution',
          is_error: true,
          session_id: 'cs-1',
          num_turns: 1,
          errors: ['[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null'],
        };
      },
      interrupt: async () => release(),
    })) as never);
    await fake.registered.get('claude::start')?.({
      prompt: 'story',
      session_id: 'stop-mid',
      parent_session_id: 'p-1',
    });
    await vi.waitFor(() => expect(streaming).toBe(true));
    expect(await fake.registered.get('claude::stop')?.({ session_id: 'stop-mid' })).toMatchObject({
      stopped: true,
    });
    await vi.waitFor(() => expect(statuses(fake)).toEqual(['working', 'done']));
    const done = fake.calls.find(
      (c) => c.function_id === 'session::set-status' && c.payload.status === 'done',
    );
    expect(done?.payload.reason).toBe('stopped');
    const messages = appended(fake, 'stop-mid');
    expect(JSON.stringify(messages)).toContain('Once upon');
    expect(messages[messages.length - 1].stop_reason).toBe('aborted');
  });

  it('stores a stop before the query exists as aborted, not error', () => {
    const msg = toSessionMessage({ role: 'assistant', content: [], stop_reason: 'interrupted' });
    expect(msg.stop_reason).toBe('aborted');
  });

  it('carries the SDK errors text when a failed result has no result field', async () => {
    const fake = await worker();
    const { result: _drop, ...failed } = resultMsg;
    queryMock.mockImplementation(
      scriptedQuery(
        [
          initMsg,
          {
            ...failed,
            subtype: 'error_during_execution',
            is_error: true,
            errors: ['API quota exceeded', 'retry later'],
          },
        ],
        { interrupted: false },
      ) as never,
    );
    await fake.registered.get('claude::task')?.({
      task: 'review',
      session_id: 'err-1',
      parent_session_id: 'p-1',
    });
    await vi.waitFor(() => {
      expect(fake.state.get('agent_tasks/err-1')).toMatchObject({ status: 'error' });
    });
    const outcome = fake.state.get('agent_tasks/err-1') as { error: string };
    expect(outcome.error).toContain('API quota exceeded');
    expect(outcome.error).toContain('retry later');
  });

  it('keeps preferring the result text over errors', async () => {
    const fake = fakeIii();
    const turn = [
      initMsg,
      {
        ...resultMsg,
        subtype: 'error_during_execution',
        is_error: true,
        result: 'own words',
        errors: ['raw'],
      },
    ];
    const out = (await run(fake, { prompt: 'x' }, turn)) as { result?: string };
    expect(out.result).toBe('own words');
  });

  it('publishes the busy fields in the claude::start response format', async () => {
    const fake = await worker();
    const format = fake.functions.get('claude::start')?.options.response_format as {
      properties: Record<string, unknown>;
    };
    for (const key of ['session_id', 'started', 'busy', 'reason']) {
      expect(format.properties, key).toHaveProperty(key);
    }
  });
});
