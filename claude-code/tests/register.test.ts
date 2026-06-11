import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@anthropic-ai/claude-agent-sdk', () => ({ query: vi.fn() }));

import { query } from '@anthropic-ai/claude-agent-sdk';
import { loadConfig } from '../src/config.js';
import { makeEmitter } from '../src/events.js';
import { register } from '../src/run.js';
import { fakeIii, type FakeIii } from './_helpers/fake-iii.js';
import { fullTurn, scriptedQuery, type QueryCapture } from './_helpers/fake-query.js';

const queryMock = vi.mocked(query);

async function registeredWorker(): Promise<FakeIii> {
  const fake = fakeIii();
  const cfg = await loadConfig('/nonexistent/config.yaml');
  const emit = makeEmitter(fake.iii, cfg.events_stream);
  const emitRaw = makeEmitter(fake.iii, cfg.raw_events_stream);
  register(fake.iii, cfg, emit, emitRaw);
  return fake;
}

beforeEach(() => {
  queryMock.mockReset();
});

describe('register', () => {
  it('registers the full claude::* surface plus the shared entrypoint', async () => {
    const fake = await registeredWorker();
    expect([...fake.registered.keys()].sort()).toEqual([
      'claude::run',
      'claude::sessions::list',
      'claude::start',
      'claude::status',
      'claude::stop',
      'run::start_and_wait',
    ]);
  });

  it('claude::run parses at the unknown boundary and runs a turn', async () => {
    const fake = await registeredWorker();
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const res = (await fake.registered.get('claude::run')?.({
      prompt: 'hi',
      session_id: 's1',
    })) as Record<string, unknown>;
    expect(res.result).toBe('done');
    expect(capture.prompt).toBe('hi');
  });

  it('claude::run rejects invalid payloads', async () => {
    const fake = await registeredWorker();
    await expect(
      fake.registered.get('claude::run')?.({ prompt: 'x', permission_mode: 'yolo' }),
    ).rejects.toThrow();
    await expect(fake.registered.get('claude::run')?.({ max_turns: -1 })).rejects.toThrow();
  });

  it('run::start_and_wait shares the claude::run handler behaviour', async () => {
    const fake = await registeredWorker();
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const res = (await fake.registered.get('run::start_and_wait')?.({
      session_id: 's1',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'go' }] }],
    })) as Record<string, unknown>;
    expect(res.result).toBe('done');
    expect(capture.prompt).toBe('go');
  });

  it('claude::start returns immediately and the turn lands in the background', async () => {
    const fake = await registeredWorker();
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    const res = (await fake.registered.get('claude::start')?.({ prompt: 'bg' })) as Record<
      string,
      unknown
    >;
    expect(res.started).toBe(true);
    expect(typeof res.session_id).toBe('string');
    await vi.waitFor(() => {
      const record = fake.state.get(`claude_sessions/${res.session_id}`) as
        | { status: string }
        | undefined;
      expect(record?.status).toBe('done');
    });
  });

  it('claude::stop without a live run reports stopped: false', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('claude::stop')?.({ session_id: 'ghost' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'ghost', stopped: false });
  });

  it('claude::stop interrupts a live run', async () => {
    const fake = await registeredWorker();
    const capture: QueryCapture = { interrupted: false };
    let release: (() => void) | undefined;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    queryMock.mockImplementation(((args: { options: Record<string, unknown> }) => {
      capture.options = args.options;
      return {
        async *[Symbol.asyncIterator]() {
          yield { type: 'system', subtype: 'init', session_id: 'cs-1' };
          await gate;
        },
        interrupt: async () => {
          capture.interrupted = true;
          release?.();
        },
      };
    }) as never);

    const startRes = (await fake.registered.get('claude::start')?.({
      prompt: 'long',
      session_id: 'live-1',
    })) as Record<string, unknown>;
    await vi.waitFor(() => {
      expect(fake.state.has('claude_sessions/live-1')).toBe(true);
    });
    const stopRes = (await fake.registered.get('claude::stop')?.({
      session_id: String(startRes.session_id),
    })) as Record<string, unknown>;
    expect(stopRes.stopped).toBe(true);
    expect(capture.interrupted).toBe(true);
  });

  it('claude::status reflects the stored record and live flag', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('claude::status')?.({ session_id: 'none' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'none', live: false, record: null });
  });

  it('claude::sessions::list returns every stored record', async () => {
    const fake = await registeredWorker();
    const capture: QueryCapture = { interrupted: false };
    queryMock.mockImplementation(scriptedQuery(fullTurn, capture) as never);
    await fake.registered.get('claude::run')?.({ prompt: 'a', session_id: 's1' });
    await fake.registered.get('claude::run')?.({ prompt: 'b', session_id: 's2' });
    const res = (await fake.registered.get('claude::sessions::list')?.({})) as {
      sessions: Array<{ session_id: string }>;
    };
    expect(res.sessions.map((s) => s.session_id).sort()).toEqual(['s1', 's2']);
  });
});
