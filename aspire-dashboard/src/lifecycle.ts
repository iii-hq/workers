import type { ChildProcess } from 'node:child_process'
import { createServer } from 'node:net'

export function processExited(child: Pick<ChildProcess, 'exitCode' | 'signalCode'>) {
  return child.exitCode !== null || child.signalCode !== null
}

export function probePort(port: number, host: string) {
  return new Promise<boolean>((done) => {
    const probe = createServer()
    probe.once('error', () => done(false))
    probe.listen(port, host, () => probe.close(() => done(true)))
  })
}

export async function assertPortsFree(ports: readonly number[], host: string) {
  for (const port of ports) {
    if (!(await probePort(port, host))) {
      throw new Error(`port ${host}:${port} is already in use`)
    }
  }
}

export async function respondsOverHttp(url: string, fetchImpl: typeof fetch = fetch, signal?: AbortSignal) {
  try {
    const response = await fetchImpl(url, { redirect: 'manual', signal })
    return response.status === 200 || (response.status >= 300 && response.status < 400)
  } catch {
    return false
  }
}

// Shares one run between overlapping callers. The dashboard has a single
// process and a fixed set of ports, so two concurrent starts would both clear
// the port check before either child binds, and the second would overwrite the
// first - leaving an untracked process that stop cannot reach.
export function singleFlight<T>(run: () => Promise<T>): () => Promise<T> {
  let inFlight: Promise<T> | null = null
  return () => {
    inFlight ??= run().finally(() => {
      inFlight = null
    })
    return inFlight
  }
}

export type ReadyOutcome = 'ready' | 'exited' | 'timeout'

export async function waitForHttp(options: {
  url: string
  timeoutMs: number
  intervalMs?: number
  exited: () => boolean
  fetch?: typeof fetch
  now?: () => number
  sleep?: (ms: number) => Promise<void>
}): Promise<ReadyOutcome> {
  const now = options.now ?? Date.now
  const sleep = options.sleep ?? ((ms: number) => new Promise<void>((done) => setTimeout(done, ms)))
  const intervalMs = options.intervalMs ?? 250
  const deadline = now() + options.timeoutMs
  while (now() < deadline) {
    if (options.exited()) return 'exited'
    // The probe carries the remaining deadline. Without it, an endpoint that
    // accepts the connection and never answers would hold this loop open past
    // timeoutMs, so the caller never reaches its own failure handling.
    if (await respondsOverHttp(options.url, options.fetch, AbortSignal.timeout(deadline - now()))) return 'ready'
    await sleep(intervalMs)
  }
  return 'timeout'
}
