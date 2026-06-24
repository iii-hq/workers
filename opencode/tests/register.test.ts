import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('node:child_process', () => ({ spawn: vi.fn() }));

import { spawn } from 'node:child_process';
import { loadConfig } from '../src/config.js';
import { makeEmitter } from '../src/events.js';
import { register } from '../src/run.js';
import { fakeIii, type FakeIii } from './_helpers/fake-iii.js';
import { fullTurn, newSpawnCapture, scriptedSpawn } from './_helpers/fake-opencode.js';

const spawnMock = vi.mocked(spawn);

async function registeredWorker(): Promise<FakeIii> {
  const fake = fakeIii();
  const cfg = await loadConfig('/nonexistent/config.yaml');
  const emit = makeEmitter(fake.iii, cfg.events_stream);
  const emitRaw = makeEmitter(fake.iii, cfg.raw_events_stream);
  register(fake.iii, () => cfg, emit, emitRaw);
  return fake;
}

beforeEach(() => spawnMock.mockReset());

describe('register', () => {
  it('registers the full opencode::* surface plus the shared entrypoint', async () => {
    const fake = await registeredWorker();
    expect([...fake.registered.keys()].sort()).toEqual([
      'opencode::run',
      'opencode::sessions::list',
      'opencode::start',
      'opencode::status',
      'opencode::stop',
      'run::start_and_wait',
    ]);
  });

  it('opencode::run runs a turn at the unknown boundary', async () => {
    const fake = await registeredWorker();
    spawnMock.mockImplementation(scriptedSpawn(fullTurn, newSpawnCapture()) as never);
    const res = (await fake.registered.get('opencode::run')?.({
      prompt: 'hi',
      session_id: 's1',
    })) as Record<string, unknown>;
    expect(res.result).toBe('pong');
  });

  it('opencode::run rejects invalid payloads', async () => {
    const fake = await registeredWorker();
    await expect(fake.registered.get('opencode::run')?.({ timeout_ms: -1 })).rejects.toThrow();
  });

  it('run::start_and_wait shares the run handler', async () => {
    const fake = await registeredWorker();
    spawnMock.mockImplementation(scriptedSpawn(fullTurn, newSpawnCapture()) as never);
    const res = (await fake.registered.get('run::start_and_wait')?.({
      session_id: 's1',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'go' }] }],
    })) as Record<string, unknown>;
    expect(res.result).toBe('pong');
  });

  it('opencode::start returns immediately and the turn lands in the background', async () => {
    const fake = await registeredWorker();
    spawnMock.mockImplementation(scriptedSpawn(fullTurn, newSpawnCapture()) as never);
    const res = (await fake.registered.get('opencode::start')?.({
      prompt: 'bg',
      session_id: 'bg1',
    })) as Record<string, unknown>;
    expect(res.started).toBe(true);
    await vi.waitFor(() => {
      const rec = fake.state.get('opencode_sessions/bg1') as { status: string } | undefined;
      expect(rec?.status).toBe('done');
    });
  });

  it('opencode::start reports busy when a run is already live for the session', async () => {
    const fake = await registeredWorker();
    spawnMock.mockImplementation(
      scriptedSpawn(fullTurn, newSpawnCapture(), { hang: true }) as never,
    );
    await fake.registered.get('opencode::start')?.({ prompt: 'a', session_id: 'busy1' });
    await new Promise((r) => setTimeout(r, 20));
    const second = (await fake.registered.get('opencode::start')?.({
      prompt: 'b',
      session_id: 'busy1',
    })) as Record<string, unknown>;
    expect(second).toMatchObject({ session_id: 'busy1', started: false, busy: true });
  });

  it('opencode::stop kills a live run', async () => {
    const fake = await registeredWorker();
    const capture = newSpawnCapture();
    spawnMock.mockImplementation(scriptedSpawn(fullTurn, capture, { hang: true }) as never);
    await fake.registered.get('opencode::start')?.({ prompt: 'long', session_id: 'live1' });
    await new Promise((r) => setTimeout(r, 20));
    const res = (await fake.registered.get('opencode::stop')?.({ session_id: 'live1' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'live1', stopped: true });
    expect(capture.killed).toBe(true);
  });

  it('opencode::stop without a live run reports stopped: false', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('opencode::stop')?.({ session_id: 'ghost' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'ghost', stopped: false });
  });

  it('opencode::status reflects the stored record and live flag', async () => {
    const fake = await registeredWorker();
    const res = (await fake.registered.get('opencode::status')?.({ session_id: 'none' })) as Record<
      string,
      unknown
    >;
    expect(res).toMatchObject({ session_id: 'none', live: false, record: null });
  });

  it('opencode::sessions::list returns every stored record', async () => {
    const fake = await registeredWorker();
    spawnMock.mockImplementation(scriptedSpawn(fullTurn, newSpawnCapture()) as never);
    await fake.registered.get('opencode::run')?.({ prompt: 'a', session_id: 's1' });
    await fake.registered.get('opencode::run')?.({ prompt: 'b', session_id: 's2' });
    const res = (await fake.registered.get('opencode::sessions::list')?.({})) as {
      sessions: Array<{ session_id: string }>;
    };
    expect(res.sessions.map((s) => s.session_id).sort()).toEqual(['s1', 's2']);
  });
});
