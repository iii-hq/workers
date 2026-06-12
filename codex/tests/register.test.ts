import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@openai/codex-sdk', () => ({ Codex: vi.fn() }));

import { Codex } from '@openai/codex-sdk';
import { loadConfig } from '../src/config.js';
import { makeEmitter } from '../src/events.js';
import { register } from '../src/run.js';
import { type CodexCapture, fakeCodexClass, fullTurn } from './_helpers/fake-codex.js';
import { type FakeIii, fakeIii } from './_helpers/fake-iii.js';

const CodexMock = vi.mocked(Codex);

async function registeredWorker(): Promise<FakeIii> {
  const fake = fakeIii();
  const cfg = await loadConfig('/nonexistent/config.yaml');
  const emit = makeEmitter(fake.iii, cfg.events_stream);
  const emitRaw = makeEmitter(fake.iii, cfg.raw_events_stream);
  register(fake.iii, cfg, emit, emitRaw);
  return fake;
}

beforeEach(() => {
  CodexMock.mockReset();
});

describe('register', () => {
  it('registers the full codex::* surface', async () => {
    const fake = await registeredWorker();
    expect([...fake.registered.keys()].sort()).toEqual([
      'codex::run',
      'codex::sessions::list',
      'codex::start',
      'codex::status',
      'codex::stop',
    ]);
  });

  it('codex::run parses at the unknown boundary and runs a turn', async () => {
    const fake = await registeredWorker();
    const capture: CodexCapture = { aborted: false };
    CodexMock.mockImplementation(fakeCodexClass(fullTurn, capture) as never);
    const res = (await fake.registered.get('codex::run')?.({
      prompt: 'hi',
      session_id: 's1',
      iii_context: false,
    })) as Record<string, unknown>;
    expect(res.result).toBe('done');
    expect(capture.prompt).toBe('hi');
  });

  it('codex::run rejects invalid payloads', async () => {
    const fake = await registeredWorker();
    await expect(
      fake.registered.get('codex::run')?.({ prompt: 'x', sandbox_mode: 'yolo' }),
    ).rejects.toThrow();
    await expect(
      fake.registered.get('codex::run')?.({ prompt: 'x', approval_policy: 'always' }),
    ).rejects.toThrow();
  });

  it('codex::start returns immediately and the turn lands in the background', async () => {
    const fake = await registeredWorker();
    const capture: CodexCapture = { aborted: false };
    CodexMock.mockImplementation(fakeCodexClass(fullTurn, capture) as never);
    const res = (await fake.registered.get('codex::start')?.({ prompt: 'bg' })) as Record<
      string,
      unknown
    >;
    expect(res.started).toBe(true);
    expect(typeof res.session_id).toBe('string');
    await vi.waitFor(() => {
      const record = fake.state.get(`codex_sessions/${res.session_id}`) as
        | { status: string }
        | undefined;
      expect(record?.status).toBe('done');
    });
  });

  it('codex::stop without a live run reports stopped: false', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('codex::stop')?.({ session_id: 'ghost' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'ghost', stopped: false });
  });

  it('codex::stop aborts a live run', async () => {
    const fake = await registeredWorker();
    const capture: CodexCapture = { aborted: false };
    let release: (() => void) | undefined;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    CodexMock.mockImplementation((() => {
      const thread = {
        id: 'th-live',
        runStreamed: async (_prompt: string, turnOptions?: { signal?: AbortSignal }) => ({
          events: (async function* () {
            yield { type: 'thread.started', thread_id: 'th-live' };
            await gate;
            if (turnOptions?.signal?.aborted) {
              capture.aborted = true;
              throw new Error('aborted');
            }
          })(),
        }),
      };
      return { startThread: () => thread, resumeThread: () => thread };
    }) as never);

    const startRes = (await fake.registered.get('codex::start')?.({
      prompt: 'long',
      session_id: 'live-1',
    })) as Record<string, unknown>;
    await vi.waitFor(() => {
      expect(fake.state.has('codex_sessions/live-1')).toBe(true);
    });
    const stopRes = (await fake.registered.get('codex::stop')?.({
      session_id: String(startRes.session_id),
    })) as Record<string, unknown>;
    expect(stopRes.stopped).toBe(true);
    release?.();
    await vi.waitFor(() => {
      const record = fake.state.get('codex_sessions/live-1') as { status: string } | undefined;
      expect(record?.status).toBe('done');
    });
    expect(capture.aborted).toBe(true);
  });

  it('codex::status reflects the stored record and live flag', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('codex::status')?.({ session_id: 'none' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'none', live: false, record: null });
  });

  it('codex::sessions::list returns every stored record', async () => {
    const fake = await registeredWorker();
    const capture: CodexCapture = { aborted: false };
    CodexMock.mockImplementation(fakeCodexClass(fullTurn, capture) as never);
    await fake.registered.get('codex::run')?.({ prompt: 'a', session_id: 's1' });
    await fake.registered.get('codex::run')?.({ prompt: 'b', session_id: 's2' });
    const res = (await fake.registered.get('codex::sessions::list')?.({})) as {
      sessions: Array<{ session_id: string }>;
    };
    expect(res.sessions.map((s) => s.session_id).sort()).toEqual(['s1', 's2']);
  });
});
