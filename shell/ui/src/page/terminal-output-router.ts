import type { Host } from '@iii-dev/console-ui'

const OUTPUT_HANDLER = 'iii::shell-ui::pty-output'
const MAX_UNSUBSCRIBED_OUTPUT_BYTES = 2 * 1024 * 1024

interface OutputQueue {
  events: PtyOutputEvent[]
  rawBytes: number
}

export interface PtyOutputEvent {
  session_id: string
  sequence: number
  data: string | null
  eof: boolean
  exit_code: number | null
  signal: string | null
  error: string | null
}

export type OutputListener = (event: PtyOutputEvent) => void

export interface TerminalOutputRouter {
  outputFunctionId: string
  subscribe(sessionId: string, listener: OutputListener): () => void
  drain(sessionId: string): PtyOutputEvent[]
  dispose(): void
}

const routerHosts = new WeakMap<TerminalOutputRouter, Host>()

export function terminalOutputRouterHost(router: TerminalOutputRouter): Host {
  const host = routerHosts.get(router)
  if (!host) throw new Error('terminal output router is disposed')
  return host
}

function outputBytes(data: string | null): number {
  if (!data) return 0
  let padding = 0
  if (data.endsWith('==')) padding = 2
  else if (data.endsWith('=')) padding = 1
  return Math.floor((data.length * 3) / 4) - padding
}

export function createTerminalOutputRouter(host: Host): TerminalOutputRouter {
  const listeners = new Map<string, Set<OutputListener>>()
  const queues = new Map<string, OutputQueue>()
  let disposed = false
  const offOutput = host.iii.on<PtyOutputEvent>(OUTPUT_HANDLER, (event) => {
    if (disposed) return
    const sessionListeners = listeners.get(event.session_id)
    if (sessionListeners) {
      for (const listener of sessionListeners) listener(event)
      return
    }
    const queue = queues.get(event.session_id) ?? { events: [], rawBytes: 0 }
    queue.events.push(event)
    queue.rawBytes += outputBytes(event.data)
    while (
      queue.rawBytes > MAX_UNSUBSCRIBED_OUTPUT_BYTES &&
      queue.events.length > 0
    ) {
      const removed = queue.events.shift()
      if (removed) queue.rawBytes -= outputBytes(removed.data)
    }
    if (queue.events.length > 0) queues.set(event.session_id, queue)
  })
  const router: TerminalOutputRouter = {
    outputFunctionId: `${OUTPUT_HANDLER}::${host.iii.browserId}`,
    subscribe(sessionId, listener) {
      if (disposed) throw new Error('terminal output router is disposed')
      const sessionListeners = listeners.get(sessionId) ?? new Set()
      sessionListeners.add(listener)
      listeners.set(sessionId, sessionListeners)
      return () => {
        sessionListeners.delete(listener)
        if (sessionListeners.size === 0) listeners.delete(sessionId)
      }
    },
    drain(sessionId) {
      const queue = queues.get(sessionId)
      queues.delete(sessionId)
      return queue?.events ?? []
    },
    dispose() {
      if (disposed) return
      disposed = true
      offOutput()
      listeners.clear()
      queues.clear()
      routerHosts.delete(router)
    },
  }
  routerHosts.set(router, host)
  return router
}
