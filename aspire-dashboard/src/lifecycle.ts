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

export async function respondsOverHttp(url: string, fetchImpl: typeof fetch = fetch) {
  try {
    const response = await fetchImpl(url, { redirect: 'manual' })
    return response.status === 200 || (response.status >= 300 && response.status < 400)
  } catch {
    return false
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
    if (await respondsOverHttp(options.url, options.fetch)) return 'ready'
    await sleep(intervalMs)
  }
  return 'timeout'
}
