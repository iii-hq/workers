/**
 * One browser handler for every terminal the page opens:
 * `iii::tmux-ui::pty-output::<browserId>`. It lives at script scope (set up
 * in `setup(host)`), not in React, so Strict Mode's double effects never
 * drop it. Output can arrive before the page subscribes (the tmux client
 * starts painting while `open` is still returning), so events for an
 * unknown terminal wait in a bounded queue until `drain`.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'
import type { TerminalOutputEvent } from './types'

export const OUTPUT_HANDLER = 'iii::tmux-ui::pty-output'
const MAX_QUEUED_EVENTS = 4096
const MAX_QUEUED_TERMINALS = 16

export type OutputListener = (event: TerminalOutputEvent) => void

export interface OutputRouter {
  /** What `tmux::terminal::open` must be told to deliver to. */
  outputFunctionId: string
  subscribe(terminalId: string, listener: OutputListener): () => void
  drain(terminalId: string): TerminalOutputEvent[]
  /** Forget whatever is queued for a terminal the page no longer wants. */
  discard(terminalId: string): void
  dispose(): void
}

export function createOutputRouter(iii: Pick<ExtensionIii, 'on' | 'browserId'>): OutputRouter {
  const listeners = new Map<string, Set<OutputListener>>()
  const queues = new Map<string, TerminalOutputEvent[]>()
  let disposed = false
  const off = iii.on<TerminalOutputEvent>(OUTPUT_HANDLER, (event) => {
    if (disposed || typeof event?.terminal_id !== 'string') return
    const set = listeners.get(event.terminal_id)
    if (set && set.size > 0) {
      for (const listener of set) listener(event)
      return
    }
    let queue = queues.get(event.terminal_id)
    if (!queue) {
      if (queues.size >= MAX_QUEUED_TERMINALS) {
        const oldest = queues.keys().next().value
        if (oldest !== undefined) queues.delete(oldest)
      }
      queue = []
      queues.set(event.terminal_id, queue)
    }
    queue.push(event)
    if (queue.length > MAX_QUEUED_EVENTS) queue.shift()
  })
  return {
    outputFunctionId: `${OUTPUT_HANDLER}::${iii.browserId}`,
    subscribe(terminalId, listener) {
      const set = listeners.get(terminalId) ?? new Set<OutputListener>()
      set.add(listener)
      listeners.set(terminalId, set)
      return () => {
        set.delete(listener)
        if (set.size === 0) listeners.delete(terminalId)
      }
    },
    drain(terminalId) {
      const queue = queues.get(terminalId) ?? []
      queues.delete(terminalId)
      return queue
    },
    discard(terminalId) {
      queues.delete(terminalId)
    },
    dispose() {
      if (disposed) return
      disposed = true
      off()
      listeners.clear()
      queues.clear()
    },
  }
}
