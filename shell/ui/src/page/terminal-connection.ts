import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '../lib/format'
import type { LocalTerminalLease } from './terminal-leases'
import type {
  OutputListener,
  PtyOutputEvent,
  TerminalOutputRouter,
} from './terminal-output-router'
import type { PtySessionStatus } from './terminal-session-state'
import { mergeTerminalFrames, type TerminalFrame } from './terminal-stream'

interface PtyAttachResponse {
  access_key: string
  reconnect_token: string
  frames: TerminalFrame[]
  truncated: boolean
  next_sequence: number
  cwd: string
  status: PtySessionStatus
}

interface PtyOpenResponse {
  session_id: string
  access_key: string
  reconnect_token: string
  pid: number | null
  cwd: string
}

export interface ActivatedTerminalConnection {
  frames: TerminalFrame[]
  events: PtyOutputEvent[]
  requiresReplay: boolean
  replayThrough: number
}

export interface TerminalConnection {
  sessionId: string
  accessKey: string
  reconnectToken: string
  cwd: string
  status: PtySessionStatus
  truncated: boolean
  nextSequence: number
  activate(listener: OutputListener): ActivatedTerminalConnection
  unsubscribe(): void
}

interface ConnectTerminalSessionOptions {
  host: Host
  router: TerminalOutputRouter
  root: string
  lease: LocalTerminalLease | null
  requestId: string
  cols: number
  rows: number
}

export interface ReclaimableTerminalLease extends LocalTerminalLease {
  update: (lease: LocalTerminalLease) => void
  remove: () => void
}

const MAX_PENDING_OUTPUT_BYTES = 2 * 1024 * 1024
const MAX_PENDING_OUTPUT_EVENTS = 4096

function outputBytes(event: PtyOutputEvent): number {
  if (!event.data) return 0
  let padding = 0
  if (event.data.endsWith('==')) padding = 2
  else if (event.data.endsWith('=')) padding = 1
  return Math.floor((event.data.length * 3) / 4) - padding
}

function stopRoutingOutput(
  router: TerminalOutputRouter,
  connection: TerminalConnection,
): void {
  connection.unsubscribe()
  router.drain(connection.sessionId)
}

export function backendConfirmedSessionMissing(error: unknown): boolean {
  const message = errorMessage(error)
  return (
    message.includes('terminal session does not exist') ||
    message.includes('terminal session is closed')
  )
}

export function legacyTerminalStorageKey(root: string): string {
  return `iii::shell-ui::terminal-leases::legacy::${root}`
}

export async function connectTerminalSession({
  host,
  router,
  root,
  lease,
  requestId,
  cols,
  rows,
}: ConnectTerminalSessionOptions): Promise<TerminalConnection> {
  let liveListener: OutputListener | null = null
  const pending: PtyOutputEvent[] = []
  let pendingBytes = 0
  let pendingTruncated = false
  const enqueue = (event: PtyOutputEvent) => {
    pending.push(event)
    pendingBytes += outputBytes(event)
    while (
      pending.length > MAX_PENDING_OUTPUT_EVENTS ||
      pendingBytes > MAX_PENDING_OUTPUT_BYTES
    ) {
      const removed = pending.shift()
      if (!removed) break
      pendingBytes -= outputBytes(removed)
      pendingTruncated = true
    }
  }
  const receive: OutputListener = (event) => {
    if (liveListener) {
      liveListener(event)
      return
    }
    enqueue(event)
  }
  let sessionId: string
  let accessKey: string
  let reconnectToken: string
  let cwd: string
  let status: PtySessionStatus
  let replay: TerminalFrame[]
  let truncated: boolean
  let nextSequence: number
  const afterSequence = 0
  let unsubscribe: () => void

  if (lease) {
    sessionId = lease.sessionId
    unsubscribe = router.subscribe(sessionId, receive)
    for (const event of router.drain(sessionId)) enqueue(event)
    let attached: PtyAttachResponse
    try {
      attached = await host.iii.trigger<PtyAttachResponse>(
        'shell::pty::attach',
        {
          request_id: requestId,
          session_id: sessionId,
          reconnect_token: lease.reconnectToken,
          output_function_id: router.outputFunctionId,
          cols,
          rows,
          after_sequence: afterSequence,
        },
        { timeoutMs: 5_000 },
      )
    } catch (error) {
      unsubscribe()
      throw error
    }
    accessKey = attached.access_key
    reconnectToken = attached.reconnect_token
    cwd = attached.cwd
    status = attached.status
    replay = attached.frames
    truncated = attached.truncated
    nextSequence = attached.next_sequence
  } else {
    const opened = await host.iii.trigger<PtyOpenResponse>(
      'shell::pty::open',
      {
        request_id: requestId,
        cwd: root,
        cols,
        rows,
        output_function_id: router.outputFunctionId,
      },
      { timeoutMs: 5_000 },
    )
    sessionId = opened.session_id
    accessKey = opened.access_key
    reconnectToken = opened.reconnect_token
    cwd = opened.cwd
    status = 'attached'
    replay = []
    truncated = false
    nextSequence = 1
    unsubscribe = router.subscribe(sessionId, receive)
    for (const event of router.drain(sessionId)) enqueue(event)
  }

  return {
    sessionId,
    accessKey,
    reconnectToken,
    cwd,
    status,
    truncated,
    nextSequence,
    activate(listener) {
      liveListener = listener
      const queued = pending.splice(0)
      const queuedFrames = queued.flatMap((event) =>
        event.data ? [{ sequence: event.sequence, data: event.data }] : [],
      )
      return {
        frames: pendingTruncated
          ? replay
          : mergeTerminalFrames(replay, queuedFrames, afterSequence),
        events: queued.filter((event) => event.eof),
        requiresReplay: pendingTruncated,
        replayThrough: Math.max(0, nextSequence - 1),
      }
    },
    unsubscribe,
  }
}

export async function disposeTerminalConnection(
  host: Host,
  router: TerminalOutputRouter,
  connection: TerminalConnection,
): Promise<void> {
  stopRoutingOutput(router, connection)
  await host.iii.trigger('shell::pty::detach', {
    session_id: connection.sessionId,
    access_key: connection.accessKey,
  })
}

export async function closeTerminalConnection(
  host: Host,
  router: TerminalOutputRouter,
  connection: TerminalConnection,
): Promise<void> {
  stopRoutingOutput(router, connection)
  await host.iii.trigger('shell::pty::close', {
    session_id: connection.sessionId,
    access_key: connection.accessKey,
  })
}

export async function reclaimTerminalLease(
  host: Host,
  router: TerminalOutputRouter,
  lease: ReclaimableTerminalLease,
): Promise<string | null> {
  const unsubscribe = router.subscribe(lease.sessionId, () => undefined)
  router.drain(lease.sessionId)
  try {
    const attached = await host.iii.trigger<PtyAttachResponse>(
      'shell::pty::attach',
      {
        session_id: lease.sessionId,
        reconnect_token: lease.reconnectToken,
        output_function_id: router.outputFunctionId,
        cols: 80,
        rows: 24,
        after_sequence: lease.lastSequence,
      },
    )
    let warning: string | null = null
    try {
      lease.update({
        paneId: lease.paneId,
        sessionId: lease.sessionId,
        reconnectToken: attached.reconnect_token,
        lastSequence: lease.lastSequence,
      })
    } catch (error) {
      warning = errorMessage(error)
    }
    await host.iii.trigger('shell::pty::close', {
      session_id: lease.sessionId,
      access_key: attached.access_key,
    })
    try {
      lease.remove()
    } catch (error) {
      warning = warning
        ? `${warning}; ${errorMessage(error)}`
        : errorMessage(error)
    }
    return warning
  } catch (error) {
    if (!backendConfirmedSessionMissing(error)) throw error
    try {
      lease.remove()
      return null
    } catch (storageError) {
      return errorMessage(storageError)
    }
  } finally {
    unsubscribe()
    router.drain(lease.sessionId)
  }
}
