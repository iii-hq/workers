import { EventEmitter } from 'node:events';
import { describe, expect, it, vi } from 'vitest';
import {
  findFreePort,
  type ManagedProcess,
  processExited,
  stopProcess,
  waitForHttp,
} from '../src/lifecycle.js';

function fakeChild(pid = 4242) {
  const emitter = new EventEmitter();
  const child = {
    pid,
    exitCode: null as number | null,
    signalCode: null as NodeJS.Signals | null,
    kill: vi.fn((signal?: NodeJS.Signals) => {
      child.signalCode = signal ?? 'SIGTERM';
      emitter.emit('exit');
      return true;
    }),
    once: (event: string, listener: () => void) => {
      emitter.once(event, listener);
      return child;
    },
  };
  return { child: child as unknown as ManagedProcess & typeof child, emitter };
}

describe('lifecycle', () => {
  it('reports exit from either exit code or signal', () => {
    expect(processExited({ exitCode: null, signalCode: null })).toBe(false);
    expect(processExited({ exitCode: 0, signalCode: null })).toBe(true);
    expect(processExited({ exitCode: null, signalCode: 'SIGTERM' })).toBe(true);
  });

  it('skips taken ports and returns the first free one', async () => {
    const probed: number[] = [];
    const port = await findFreePort({
      min: 100,
      max: 105,
      host: '127.0.0.1',
      taken: [100, 101],
      probe: async (candidate) => {
        probed.push(candidate);
        return candidate === 103;
      },
    });
    expect(port).toBe(103);
    expect(probed).toEqual([102, 103]);
  });

  it('throws when the whole range is busy', async () => {
    await expect(
      findFreePort({ min: 1, max: 2, host: '127.0.0.1', probe: async () => false }),
    ).rejects.toThrow(/no VS Code port available in 1-2/);
  });

  it('terminates the process group and resolves on exit', async () => {
    const { child, emitter } = fakeChild();
    const killGroup = vi.fn((_pid: number, _signal: NodeJS.Signals) => {
      setTimeout(() => emitter.emit('exit'), 0);
    });
    await stopProcess(child, { graceMs: 1000, killGroup });
    expect(killGroup).toHaveBeenCalledWith(4242, 'SIGTERM');
    expect(child.kill).not.toHaveBeenCalled();
  });

  it('falls back to a plain kill when the group signal fails', async () => {
    const { child } = fakeChild();
    const killGroup = vi.fn(() => {
      throw new Error('ESRCH');
    });
    await stopProcess(child, { graceMs: 1000, killGroup });
    expect(child.kill).toHaveBeenCalledWith('SIGTERM');
  });

  it('escalates to SIGKILL after the grace period', async () => {
    vi.useFakeTimers();
    const { child } = fakeChild();
    const signals: NodeJS.Signals[] = [];
    const pending = stopProcess(child, {
      graceMs: 50,
      killGroup: (_pid, signal) => {
        signals.push(signal);
      },
    });
    await vi.advanceTimersByTimeAsync(60);
    await pending;
    expect(signals).toEqual(['SIGTERM', 'SIGKILL']);
    vi.useRealTimers();
  });

  it('resolves immediately for a process that already exited', async () => {
    const { child } = fakeChild();
    child.exitCode = 0;
    const killGroup = vi.fn();
    await stopProcess(child, { graceMs: 1000, killGroup });
    expect(killGroup).not.toHaveBeenCalled();
  });

  it('waits for an HTTP answer and treats redirects as ready', async () => {
    let calls = 0;
    const fetchImpl = (async () => {
      calls += 1;
      if (calls < 3) throw new Error('ECONNREFUSED');
      return { status: 302 } as Response;
    }) as unknown as typeof fetch;
    const outcome = await waitForHttp({
      url: 'http://127.0.0.1:1/',
      timeoutMs: 10_000,
      exited: () => false,
      fetch: fetchImpl,
      sleep: async () => {},
    });
    expect(outcome).toBe('ready');
    expect(calls).toBe(3);
  });

  it('stops waiting when the process exits', async () => {
    let exited = false;
    const outcome = await waitForHttp({
      url: 'http://127.0.0.1:1/',
      timeoutMs: 10_000,
      exited: () => exited,
      fetch: (async () => {
        exited = true;
        throw new Error('ECONNREFUSED');
      }) as unknown as typeof fetch,
      sleep: async () => {},
    });
    expect(outcome).toBe('exited');
  });

  it('times out against a clock', async () => {
    let clock = 0;
    const outcome = await waitForHttp({
      url: 'http://127.0.0.1:1/',
      timeoutMs: 500,
      intervalMs: 100,
      exited: () => false,
      fetch: (async () => ({ status: 503 }) as Response) as unknown as typeof fetch,
      now: () => clock,
      sleep: async (ms) => {
        clock += ms;
      },
    });
    expect(outcome).toBe('timeout');
  });
});
