import type { ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';

export type ManagedProcess = Pick<
  ChildProcess,
  'pid' | 'exitCode' | 'signalCode' | 'kill' | 'once'
>;

export function processExited(child: Pick<ChildProcess, 'exitCode' | 'signalCode'>) {
  return child.exitCode !== null || child.signalCode !== null;
}

export function probePort(port: number, host: string) {
  return new Promise<boolean>((done) => {
    const probe = createServer();
    probe.once('error', () => done(false));
    probe.listen(port, host, () => probe.close(() => done(true)));
  });
}

export async function findFreePort(options: {
  min: number;
  max: number;
  host: string;
  taken?: Iterable<number>;
  probe?: typeof probePort;
}) {
  const taken = new Set(options.taken ?? []);
  const probe = options.probe ?? probePort;
  for (let port = options.min; port <= options.max; port++) {
    if (taken.has(port)) continue;
    if (await probe(port, options.host)) return port;
  }
  throw new Error(`no VS Code port available in ${options.min}-${options.max}`);
}

export function signalProcessGroup(
  child: ManagedProcess,
  signal: NodeJS.Signals,
  killGroup: (pid: number, signal: NodeJS.Signals) => void = (pid, sig) => process.kill(-pid, sig),
) {
  if (child.pid === undefined) return;
  try {
    killGroup(child.pid, signal);
  } catch {
    child.kill(signal);
  }
}

export function stopProcess(
  child: ManagedProcess,
  options: { graceMs: number; killGroup?: (pid: number, signal: NodeJS.Signals) => void },
) {
  return new Promise<void>((done) => {
    if (processExited(child)) return done();
    const timer = setTimeout(() => {
      signalProcessGroup(child, 'SIGKILL', options.killGroup);
      done();
    }, options.graceMs);
    child.once('exit', () => {
      clearTimeout(timer);
      done();
    });
    signalProcessGroup(child, 'SIGTERM', options.killGroup);
  });
}

export async function respondsOverHttp(url: string, fetchImpl: typeof fetch = fetch) {
  try {
    const response = await fetchImpl(url, { redirect: 'manual' });
    return response.status === 200 || (response.status >= 300 && response.status < 400);
  } catch {
    return false;
  }
}

export type ReadyOutcome = 'ready' | 'exited' | 'timeout';

export async function waitForHttp(options: {
  url: string;
  timeoutMs: number;
  intervalMs?: number;
  exited: () => boolean;
  fetch?: typeof fetch;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
}): Promise<ReadyOutcome> {
  const now = options.now ?? Date.now;
  const sleep =
    options.sleep ?? ((ms: number) => new Promise<void>((done) => setTimeout(done, ms)));
  const intervalMs = options.intervalMs ?? 100;
  const deadline = now() + options.timeoutMs;
  while (now() < deadline) {
    if (options.exited()) return 'exited';
    if (await respondsOverHttp(options.url, options.fetch)) return 'ready';
    await sleep(intervalMs);
  }
  return 'timeout';
}
