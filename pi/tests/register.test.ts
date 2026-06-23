import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../src/session.js', () => ({ buildSession: vi.fn() }));

import { loadConfig } from '../src/config.js';
import { makeEmitter } from '../src/events.js';
import { register } from '../src/run.js';
import { buildSession } from '../src/session.js';
import { fakeIii, type FakeIii } from './_helpers/fake-iii.js';
import {
  fullTurnEvents,
  newCapture,
  type PiEvent,
  scriptedSession,
  type SessionCapture,
} from './_helpers/fake-session.js';

const buildMock = vi.mocked(buildSession);

/** Point buildSession at a scripted session; returns the shared capture. */
function script(events: PiEvent[] = fullTurnEvents, gate?: Promise<void>): SessionCapture {
  const capture = newCapture();
  buildMock.mockImplementation(async (opts) => {
    capture.buildOpts = opts;
    return scriptedSession({ events, gate }, capture);
  });
  return capture;
}

async function registeredWorker(): Promise<FakeIii> {
  const fake = fakeIii();
  const cfg = await loadConfig('/nonexistent/config.yaml');
  const emit = makeEmitter(fake.iii, cfg.events_stream);
  const emitRaw = makeEmitter(fake.iii, cfg.raw_events_stream);
  register(fake.iii, () => cfg, emit, emitRaw);
  return fake;
}

beforeEach(() => {
  buildMock.mockReset();
});

describe('register', () => {
  it('registers the full pi::* surface plus the shared entrypoint', async () => {
    const fake = await registeredWorker();
    expect([...fake.registered.keys()].sort()).toEqual([
      'pi::follow_up',
      'pi::run',
      'pi::sessions::list',
      'pi::start',
      'pi::status',
      'pi::steer',
      'pi::stop',
      'run::start_and_wait',
    ]);
  });

  it('pi::run parses at the unknown boundary and runs a turn', async () => {
    const fake = await registeredWorker();
    const capture = script();
    const res = (await fake.registered.get('pi::run')?.({
      prompt: 'hi',
      session_id: 's1',
    })) as Record<string, unknown>;
    expect(res.result).toBe('done');
    expect(capture.promptText).toContain('hi');
  });

  it('pi::run rejects invalid payloads', async () => {
    const fake = await registeredWorker();
    await expect(
      fake.registered.get('pi::run')?.({ prompt: 'x', thinking_level: 'ultra' }),
    ).rejects.toThrow();
    await expect(fake.registered.get('pi::run')?.({ timeout_ms: -1 })).rejects.toThrow();
  });

  it('run::start_and_wait shares the pi::run handler behaviour', async () => {
    const fake = await registeredWorker();
    const capture = script();
    const res = (await fake.registered.get('run::start_and_wait')?.({
      session_id: 's1',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'go' }] }],
    })) as Record<string, unknown>;
    expect(res.result).toBe('done');
    expect(capture.promptText).toContain('go');
  });

  it('pi::start returns immediately and the turn lands in the background', async () => {
    const fake = await registeredWorker();
    script();
    const res = (await fake.registered.get('pi::start')?.({ prompt: 'bg' })) as Record<
      string,
      unknown
    >;
    expect(res.started).toBe(true);
    expect(typeof res.session_id).toBe('string');
    await vi.waitFor(() => {
      const record = fake.state.get(`pi_sessions/${res.session_id}`) as
        | { status: string }
        | undefined;
      expect(record?.status).toBe('done');
    });
  });

  it('marks the session error when a background run throws mid-stream', async () => {
    const fake = await registeredWorker();
    const capture = newCapture();
    buildMock.mockImplementation(async (opts) => {
      capture.buildOpts = opts;
      return scriptedSession({ events: [], throwOnPrompt: new Error('stream died') }, capture);
    });
    const res = (await fake.registered.get('pi::start')?.({
      prompt: 'bg',
      session_id: 'bg-1',
    })) as Record<string, unknown>;
    expect(res.started).toBe(true);
    await vi.waitFor(() => {
      const record = fake.state.get('pi_sessions/bg-1') as { status: string } | undefined;
      expect(record?.status).toBe('error');
    });
  });

  it('pi::stop without a live run reports stopped: false', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('pi::stop')?.({ session_id: 'ghost' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'ghost', stopped: false });
  });

  it('pi::stop aborts a live run', async () => {
    const fake = await registeredWorker();
    let release: (() => void) | undefined;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const capture = script(fullTurnEvents, gate);
    const startRes = (await fake.registered.get('pi::start')?.({
      prompt: 'long',
      session_id: 'live-1',
    })) as Record<string, unknown>;
    await vi.waitFor(() => {
      expect(fake.state.has('pi_sessions/live-1')).toBe(true);
    });
    const stopRes = (await fake.registered.get('pi::stop')?.({
      session_id: String(startRes.session_id),
    })) as Record<string, unknown>;
    expect(stopRes.stopped).toBe(true);
    expect(capture.aborted).toBe(true);
    release?.();
  });

  it('pi::steer and pi::follow_up inject into a live run', async () => {
    const fake = await registeredWorker();
    let release: (() => void) | undefined;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const capture = script(fullTurnEvents, gate);
    await fake.registered.get('pi::start')?.({ prompt: 'long', session_id: 'live-2' });
    await vi.waitFor(() => {
      expect(fake.state.has('pi_sessions/live-2')).toBe(true);
    });
    const steerRes = (await fake.registered.get('pi::steer')?.({
      session_id: 'live-2',
      prompt: 'do this instead',
    })) as Record<string, unknown>;
    const followRes = (await fake.registered.get('pi::follow_up')?.({
      session_id: 'live-2',
      prompt: 'then this',
    })) as Record<string, unknown>;
    expect(steerRes).toMatchObject({ steered: true });
    expect(followRes).toMatchObject({ queued: true });
    expect(capture.steered).toEqual(['do this instead']);
    expect(capture.followUps).toEqual(['then this']);
    release?.();
  });

  it('pi::steer without a live run reports steered: false', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('pi::steer')?.({
      session_id: 'ghost',
      prompt: 'x',
    })) as Record<string, unknown>;
    expect(res).toMatchObject({ session_id: 'ghost', steered: false });
  });

  it('rejects a second run while one is already live for the session', async () => {
    const fake = await registeredWorker();
    let release: (() => void) | undefined;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    script(fullTurnEvents, gate);
    await fake.registered.get('pi::start')?.({ prompt: 'first', session_id: 'busy-1' });
    await vi.waitFor(() => {
      expect(fake.state.has('pi_sessions/busy-1')).toBe(true);
    });
    const second = (await fake.registered.get('pi::run')?.({
      prompt: 'second',
      session_id: 'busy-1',
    })) as Record<string, unknown>;
    expect(second).toMatchObject({ session_id: 'busy-1', busy: true });
    release?.();
  });

  it('pi::start reports busy instead of started when a run is already live', async () => {
    const fake = await registeredWorker();
    let release: (() => void) | undefined;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    script(fullTurnEvents, gate);
    await fake.registered.get('pi::start')?.({ prompt: 'first', session_id: 'busy-2' });
    await vi.waitFor(() => {
      expect(fake.state.has('pi_sessions/busy-2')).toBe(true);
    });
    const second = (await fake.registered.get('pi::start')?.({
      prompt: 'second',
      session_id: 'busy-2',
    })) as Record<string, unknown>;
    expect(second).toMatchObject({ session_id: 'busy-2', busy: true, started: false });
    release?.();
  });

  it('pi::status reflects the stored record and live flag', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('pi::status')?.({ session_id: 'none' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'none', live: false, record: null });
  });

  it('pi::sessions::list returns every stored record', async () => {
    const fake = await registeredWorker();
    script();
    await fake.registered.get('pi::run')?.({ prompt: 'a', session_id: 's1' });
    await fake.registered.get('pi::run')?.({ prompt: 'b', session_id: 's2' });
    const res = (await fake.registered.get('pi::sessions::list')?.({})) as {
      sessions: Array<{ session_id: string }>;
    };
    expect(res.sessions.map((s) => s.session_id).sort()).toEqual(['s1', 's2']);
  });
});
