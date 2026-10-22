/**
 * The VM side of the wire (design §5): `__vm_receive` for host→VM json-v0
 * text, one batch of entries per VM turn handed to `__host_send` as an
 * array (the native VM encodes it as bin-v1, the test harness as json-v0;
 * the SDK never serializes, D16), requests and their responses, trigger
 * subscriptions, and error reporting.
 */

import { batch } from 'solid-js'
import { IN, METHOD, OP } from '../generated/catalog'
import type { JsonObject, JsonValue } from '../types'

/** One VM→host entry: `[code, ...fields]`, plain JSON values only. */
export type Entry = unknown[]

/** A failed request: `code` is one of `protocol.errorCodes` or the engine's own code. */
export class IiiError extends Error {
  readonly code: string
  constructor(code: string, message: string) {
    super(message)
    this.name = 'IiiError'
    this.code = code
  }
}

const g = globalThis as unknown as Record<string, unknown>

// The SDK keeps the only reference to the VM→host channel and removes it from
// the global, so app code cannot bypass the runtime (design §4.1).
const hostSend = g.__host_send as ((entries: Entry[]) => unknown) | undefined
delete g.__host_send

/**
 * Where id allocation starts. Production always starts at 1; the test
 * harness may seed other values so a conformance flow reproduces the exact
 * ids of a design example (`createTestHost({ ids })`).
 */
const seed = (g.__IIIUI_TEST_IDS__ ?? {}) as { node?: number; req?: number; sub?: number }
delete g.__IIIUI_TEST_IDS__

/** Next node, request and subscription ids (design §5.2): from 1, never reused while the VM lives. */
export const ids = { node: seed.node ?? 1, req: seed.req ?? 1, sub: seed.sub ?? 1 }

let out: Entry[] = []
let scheduled = false
let flushing = false
const flushHooks: (() => void)[] = []

/**
 * How many microtask hops the batch waits before it goes out. The first op
 * of a turn starts the chain; promise continuations that the same turn set
 * off (a response resolving `iii.call`, then the app's `await`) run in
 * between and land in the same batch instead of a second message.
 */
const FLUSH_HOPS = 8

/** Adds one entry to the current batch. */
export function push(entry: Entry): void {
  out.push(entry)
  schedule()
}

/** The last entry of the current batch (the tree folds a remove and a re-insert into a move). */
export function lastEntry(): Entry | undefined {
  return out[out.length - 1]
}

export function dropLastEntry(): void {
  out.pop()
}

/** Makes sure the batch of this VM turn goes out (a short chain of microtasks started by the first op). */
export function schedule(): void {
  if (scheduled || flushing) return
  scheduled = true
  let hops = FLUSH_HOPS
  const hop = () => {
    if (--hops > 0) queueMicrotask(hop)
    else flush()
  }
  queueMicrotask(hop)
}

/** Runs right before a batch goes out; the tree destroys the nodes left detached (they go last). */
export function onFlush(hook: () => void): void {
  flushHooks.push(hook)
}

function flush(): void {
  flushing = true
  try {
    for (const hook of flushHooks) hook()
  } catch (err) {
    if (isVmStop(err)) throw err
    console.error('native-ui: end of batch failed', describe(err))
  } finally {
    flushing = false
    scheduled = false
  }
  if (!out.length) return
  const entries = out
  out = []
  send(entries)
}

function send(entries: Entry[]): void {
  if (!hostSend) {
    console.error('native-ui: no __host_send in this VM; the batch is dropped')
    return
  }
  try {
    hostSend(entries)
  } catch (err) {
    if (isVmStop(err)) throw err
    // Too big for limits.maxMessageBytes (or too deep): the host takes
    // nothing. Halves keep the order of the ops; a single entry that cannot
    // fit is a fatal error, since the host's tree would no longer match ours.
    if (err instanceof RangeError && entries.length > 1) {
      const half = entries.length >> 1
      send(entries.slice(0, half))
      send(entries.slice(half))
      return
    }
    console.error('native-ui: __host_send refused a batch', describe(err))
    if (entries.length === 1 && entries[0][0] === OP.request && entries[0][2] === METHOD.reportError) return
    reportError(err, { fatal: true })
  }
}

type InHandler = (entry: unknown[]) => void
const inHandlers: Record<number, InHandler> = {}

/** Registers the handler of a host→VM code (design §5.6). */
export function onEntry(code: number, handler: InHandler): void {
  inHandlers[code] = handler
}

/**
 * The engine's own stops: QuickJS's out-of-memory error must reach the
 * host as IIIUI_OUT_OF_MEMORY (and the deadline's interrupt cannot be caught
 * anyway), so the SDK never swallows them.
 */
export function isVmStop(err: unknown): boolean {
  return err instanceof Error && err.name === 'InternalError' && /out of memory|interrupted/.test(err.message)
}

g.__vm_receive = (message: unknown): void => {
  if (typeof message !== 'string') {
    console.error('native-ui: the host sent a binary message; host→VM is json-v0 text')
    return
  }
  let entries: unknown
  try {
    entries = JSON.parse(message)
  } catch (err) {
    console.error('native-ui: the host sent invalid JSON', describe(err))
    return
  }
  if (!Array.isArray(entries)) return
  for (const entry of entries) {
    if (!Array.isArray(entry)) continue
    // An unknown code is ignored (design §5.6).
    const handler = inHandlers[entry[0] as number]
    if (!handler) continue
    try {
      handler(entry)
    } catch (err) {
      if (isVmStop(err)) throw err
      reportError(err)
    }
  }
}

// ── Requests ────────────────────────────────────────────────────────────────

interface Pending {
  resolve: (value: JsonValue) => void
  reject: (error: IiiError) => void
}
const pending = new Map<number, Pending>()

/** Sends a request (design §5.4); resolves with the response value, rejects with an {@link IiiError}. */
export function request(method: number, args: JsonObject): Promise<JsonValue> {
  const reqId = ids.req++
  push([OP.request, reqId, method, args])
  return new Promise((resolve, reject) => pending.set(reqId, { resolve, reject }))
}

/** A request whose result nobody waits for; a failure only logs. */
export function notify(method: number, args: JsonObject): void {
  request(method, args).catch((err) => console.warn(`native-ui: request ${method} failed`, describe(err)))
}

onEntry(IN.response, ([, reqId, ok, value]) => {
  const p = pending.get(reqId as number)
  if (!p) return
  pending.delete(reqId as number)
  if (ok === 1) p.resolve((value ?? null) as JsonValue)
  else {
    const v = (value ?? {}) as { code?: unknown; message?: unknown }
    p.reject(
      new IiiError(
        typeof v.code === 'string' ? v.code : 'invalid',
        typeof v.message === 'string' ? v.message : 'The request failed',
      ),
    )
  }
})

// ── Trigger subscriptions ───────────────────────────────────────────────────

const subscriptions = new Map<number, (payload: JsonValue) => void>()

/** Binds a trigger (request `on`); the returned function unbinds it (request `off`). */
export function subscribe(
  trigger: string,
  config: JsonObject,
  handler: (payload: JsonValue) => void,
): { sub: number; ready: Promise<void>; off: () => void } {
  const sub = ids.sub++
  subscriptions.set(sub, handler)
  const ready = request(METHOD.on, { sub, trigger, config }).then(() => undefined)
  ready.catch((err) => console.warn(`native-ui: binding ${trigger} failed`, describe(err)))
  let bound = true
  const off = () => {
    if (!bound) return
    bound = false
    subscriptions.delete(sub)
    notify(METHOD.off, { sub })
  }
  return { sub, ready, off }
}

onEntry(IN.trigger, ([, sub, payload]) => {
  const handler = subscriptions.get(sub as number)
  if (handler) run(() => handler((payload ?? null) as JsonValue))
})

// ── Errors ──────────────────────────────────────────────────────────────────

/** `Name: message` (and the stack) of anything thrown, for the log. */
export function describe(err: unknown): string {
  if (err instanceof Error) return `${err.name}: ${err.message}${err.stack ? `\n${err.stack}` : ''}`
  try {
    return typeof err === 'string' ? err : JSON.stringify(err)
  } catch {
    return String(err)
  }
}

/** Not fatal: the log (and a toast in a debug build of the app). Fatal: the failure screen with Restart. */
export function reportError(error: unknown, opts?: { fatal?: boolean }): void {
  const args: JsonObject = {
    message:
      error instanceof Error ? `${error.name === 'Error' ? '' : `${error.name}: `}${error.message}` : describe(error),
  }
  if (error instanceof Error && error.stack) args.stack = error.stack
  if (opts?.fatal) args.fatal = true
  console.error('native-ui:', describe(error))
  notify(METHOD.reportError, args)
}

/**
 * Runs app code from the host (an event, a trigger, a text edit) in one
 * Solid batch; a throw is reported, not fatal, and what the code changed
 * before it threw still renders.
 */
export function run(fn: () => void): void {
  let failure: unknown
  let failed = false
  try {
    batch(() => {
      try {
        fn()
      } catch (err) {
        if (isVmStop(err)) throw err
        failed = true
        failure = err
      }
    })
  } catch (err) {
    if (isVmStop(err)) throw err
    failed = true
    failure = err
  }
  if (failed) reportError(failure)
}
