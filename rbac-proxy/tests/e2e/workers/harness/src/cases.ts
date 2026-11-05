/**
 * The RBAC e2e cases, each driven through the live proxy (`down`) and, where
 * the assertion needs the ground-truth engine state, verified via the
 * unfiltered admin connection (`support`).
 */

import { randomUUID } from 'node:crypto'
import { type IIIClient, registerWorker } from 'iii-sdk'
import { createChannel } from 'iii-sdk/helpers'
import WebSocket from 'ws'
import { middlewareCalls, TENANT1_TOKEN, TENANT2_TOKEN } from './support.ts'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const PROXY_URL = process.env.PROXY_URL ?? 'ws://127.0.0.1:49271'

export interface CaseContext {
  /** Admin connection straight to the engine (unfiltered ground truth). */
  support: IIIClient
  /** Downstream connection THROUGH the rbac-proxy port. */
  down: IIIClient
  /** Assert `fn` throws with an InvocationError whose code matches. */
  expectError: (fn: () => Promise<unknown>, code: string) => Promise<void>
}

export interface TestCase {
  name: string
  run: (ctx: CaseContext) => Promise<void>
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

/** The calls `shared::source` (a private function of `down`) handled, by payload. */
const sourceCalls: Json[] = []
let sharedSourceRegistered = false

/** Resolve once the engine lists `functionId` (asked on the unfiltered connection). */
async function waitForFunction(support: IIIClient, functionId: string): Promise<void> {
  for (let i = 0; i < 25; i++) {
    try {
      const detail: Json = await support.call({
        function_id: 'engine::functions::info',
        payload: { function_id: functionId },
      })
      if (detail?.function_id === functionId) return
    } catch {
      /* not indexed yet */
    }
    await sleep(200)
  }
  throw new Error(`the engine did not list ${functionId}`)
}

/**
 * Register `shared::source`, a private function of `down` that records each
 * call's payload in `sourceCalls`, unless an earlier case did; resolve once
 * the engine lists it as `tenant1::shared::source`.
 */
async function ensureSharedSource(down: IIIClient, support: IIIClient): Promise<void> {
  if (!sharedSourceRegistered) {
    down.registerFunction(
      'shared::source',
      async (input: Json) => {
        sourceCalls.push(input)
        return { handled: true }
      },
      { private: true },
    )
    sharedSourceRegistered = true
  }
  await waitForFunction(support, 'tenant1::shared::source')
}

/** Resolve with the engine's `ObserverInfo` for `id` once `ready` accepts it. */
async function waitForObserver(support: IIIClient, id: string, ready: (info: Json) => boolean): Promise<Json> {
  let info: Json
  for (let i = 0; i < 50; i++) {
    try {
      info = await support.call({ function_id: 'engine::observers::get', payload: { id } })
      if (ready(info)) return info
    } catch (e) {
      info = { error: String(e) }
    }
    await sleep(100)
  }
  throw new Error(`observer ${id} did not reach the expected state: ${JSON.stringify(info)}`)
}

/** A raw worker connection through the proxy, read frame by frame. */
interface RawSession {
  send(frame: Json): void
  /** The first frame (already received or still to come) that `pick` accepts. */
  next(pick: (frame: Json) => boolean, timeoutMs?: number): Promise<Json>
  close(): void
}

function openRaw(token = TENANT1_TOKEN): Promise<RawSession> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(PROXY_URL, { headers: { authorization: token } })
    const inbox: Json[] = []
    const waiters: { pick: (frame: Json) => boolean; done: (frame: Json) => void }[] = []
    ws.on('message', (raw) => {
      let frame: Json
      try {
        frame = JSON.parse(String(raw))
      } catch {
        return
      }
      const i = waiters.findIndex((w) => w.pick(frame))
      if (i >= 0) waiters.splice(i, 1)[0].done(frame)
      else inbox.push(frame)
    })
    ws.on('error', reject)
    ws.on('open', () =>
      resolve({
        send: (frame) => ws.send(JSON.stringify(frame)),
        next: (pick, timeoutMs = 5_000) => {
          const i = inbox.findIndex(pick)
          if (i >= 0) return Promise.resolve(inbox.splice(i, 1)[0])
          return new Promise((done, fail) => {
            const waiter = {
              pick,
              done: (frame: Json) => {
                clearTimeout(timer)
                done(frame)
              },
            }
            const timer = setTimeout(() => {
              waiters.splice(waiters.indexOf(waiter), 1)
              fail(
                new Error(`no matching frame within ${timeoutMs}ms; received: ${JSON.stringify(inbox).slice(0, 500)}`),
              )
            }, timeoutMs)
            waiters.push(waiter)
          })
        },
        close: () => ws.close(),
      }),
    )
  })
}

/** Call `functionId` on a raw session and resolve with its `invocationresult`. */
async function rawCall(raw: RawSession, functionId: string, data: Json): Promise<Json> {
  const invocationId = randomUUID()
  raw.send({ type: 'invokefunction', function_id: functionId, invocation_id: invocationId, data })
  return raw.next((f) => f?.type === 'invocationresult' && f.invocation_id === invocationId)
}

/** A raw session of `token`'s tenant that declared `name` on `engine::workers::register`. */
async function openDeclared(token: string, name: string): Promise<RawSession> {
  const raw = await openRaw(token)
  try {
    await raw.next((f) => f?.type === 'workerregistered')
    const declared = await rawCall(raw, 'engine::workers::register', {
      runtime: 'node',
      name,
      features: ['observe.v1', 'observe.v2'],
    })
    if (declared.error) throw new Error(`engine::workers::register failed: ${JSON.stringify(declared.error)}`)
    return raw
  } catch (e) {
    raw.close()
    throw e
  }
}

/**
 * Invoke `functionId` on a fresh session and resolve with its invocation
 * result. The frame goes out in 1 MiB fragments, the way browsers send large
 * messages, so a message over 16 MiB clears the proxy's own per-frame limit.
 */
function rawInvoke(functionId: string, data: Json, timeoutMs = 10_000): Promise<Json> {
  const invocationId = randomUUID()
  const frame = JSON.stringify({ type: 'invokefunction', function_id: functionId, invocation_id: invocationId, data })
  const fragment = 1024 * 1024
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(PROXY_URL, { headers: { authorization: TENANT1_TOKEN } })
    const fail = (e: Error) => {
      clearTimeout(timer)
      ws.terminate()
      reject(e)
    }
    const timer = setTimeout(() => fail(new Error(`no result for ${functionId} within ${timeoutMs}ms`)), timeoutMs)
    ws.on('open', () => {
      for (let i = 0; i < frame.length; i += fragment) {
        ws.send(frame.slice(i, i + fragment), { fin: i + fragment >= frame.length })
      }
    })
    ws.on('message', (raw) => {
      let msg: Json
      try {
        msg = JSON.parse(String(raw))
      } catch (e) {
        return fail(e as Error)
      }
      if (msg?.type !== 'invocationresult' || msg.invocation_id !== invocationId) return
      clearTimeout(timer)
      ws.close()
      resolve(msg)
    })
    ws.on('error', fail)
    // After a result this rejects an already-settled promise, a no-op.
    ws.on('close', (code) => fail(new Error(`socket closed (${code}) before the result for ${functionId}`)))
  })
}

export const CASES: TestCase[] = [
  // (a) An exposed call succeeds — and flows through middleware, which wraps
  //     the result and invokes the real target.
  {
    name: 'exposed-call-through-middleware',
    async run({ down }) {
      const r: Json = await down.call({ function_id: 'api::echo', payload: { hi: 1 } })
      if (r?.mw !== true) throw new Error(`middleware did not wrap the call: ${JSON.stringify(r)}`)
      if (r?.result?.hi !== 1) throw new Error(`target did not echo through middleware: ${JSON.stringify(r)}`)
    },
  },

  // (b) A forbidden (non-exposed) call rejects with FORBIDDEN, before
  //     middleware runs.
  {
    name: 'forbidden-call-rejected',
    async run({ down, expectError }) {
      await expectError(() => down.call({ function_id: 'secret::echo', payload: {} }), 'FORBIDDEN')
    },
  },

  // (f) engine::functions::list is result-filtered to the exposed surface.
  {
    name: 'discovery-functions-filtered',
    async run({ down }) {
      const res: Json = await down.call({ function_id: 'engine::functions::list', payload: {} })
      const ids: string[] = (res?.functions ?? []).map((f: Json) => f.function_id)
      if (!ids.includes('api::echo')) throw new Error(`exposed api::echo missing from discovery: ${JSON.stringify(ids)}`)
      if (ids.includes('secret::echo')) throw new Error(`forbidden secret::echo leaked into discovery: ${JSON.stringify(ids)}`)
    },
  },

  // engine::workers::list strips operational internals (expose_worker_internals
  // is false in the seed).
  {
    name: 'discovery-workers-internals-stripped',
    async run({ down }) {
      const res: Json = await down.call({ function_id: 'engine::workers::list', payload: {} })
      const workers: Json[] = res?.workers ?? []
      for (const w of workers) {
        if ('ip_address' in w) throw new Error(`ip_address leaked on worker ${w?.name}`)
        if ('isolation' in w) throw new Error(`isolation leaked on worker ${w?.name}`)
      }
    },
  },

  // (d) function_registration_prefix: a bare `myfn` registration is namespaced
  //     to `tenant1::myfn` by the proxy, and the on_function_registration hook
  //     stamps metadata — both verified against the engine's unfiltered view.
  {
    name: 'prefix-applied-and-hook-ran-on-registration',
    async run({ down, support }) {
      down.registerFunction('myfn', async () => ({ pong: true }))

      let detail: Json
      for (let i = 0; i < 25; i++) {
        await sleep(200)
        try {
          detail = await support.call({
            function_id: 'engine::functions::info',
            payload: { function_id: 'tenant1::myfn' },
          })
          if (detail?.function_id === 'tenant1::myfn') break
        } catch {
          /* not indexed yet */
        }
      }
      if (detail?.function_id !== 'tenant1::myfn') {
        throw new Error(`proxy did not prefix the registration to tenant1::myfn (admin saw: ${JSON.stringify(detail)})`)
      }
      if (detail?.metadata?.hooked !== true) {
        throw new Error(`on_function_registration hook did not stamp metadata.hooked: ${JSON.stringify(detail?.metadata)}`)
      }
    },
  },

  // The same session invokes its own bare-named `myfn`: the proxy resolves it
  // to tenant1::myfn, the engine dispatches back down (prefix stripped so the
  // SDK finds the local handler), and the result returns through middleware.
  {
    name: 'prefix-self-invoke-roundtrip',
    async run({ down }) {
      let r: Json
      let lastErr: unknown
      for (let i = 0; i < 25; i++) {
        try {
          r = await down.call({ function_id: 'myfn', payload: {} })
          break
        } catch (e) {
          lastErr = e
          await sleep(200)
        }
      }
      if (r === undefined) throw new Error(`self-invoke of own prefixed function failed: ${lastErr}`)
      if (r?.result?.pong !== true) throw new Error(`own handler did not run via the dispatch round-trip: ${JSON.stringify(r)}`)
      if (!middlewareCalls.includes('tenant1::myfn')) {
        throw new Error(`the middleware did not receive the own call: ${JSON.stringify(middlewareCalls)}`)
      }
    },
  },

  // With middleware configured, `functionregistered` offers local dispatch
  // for a function the connection hosts privately and for no other: the
  // worker's own calls of a private function skip the middleware (its forward
  // would come from the proxy's control connection, which a private function
  // refuses), and the ones of an ordinary function go through it.
  {
    name: 'local-dispatch-offered-for-own-private-functions',
    async run() {
      const raw = await openRaw()
      try {
        const registered = await raw.next((f) => f?.type === 'workerregistered')
        if (!registered.features?.includes('local-dispatch.v1')) {
          throw new Error(`workerregistered lost local-dispatch.v1: ${JSON.stringify(registered)}`)
        }
        const invocationId = randomUUID()
        raw.send({
          type: 'invokefunction',
          function_id: 'engine::workers::register',
          invocation_id: invocationId,
          data: {
            runtime: 'node',
            name: 'rbac-e2e-raw-dispatch',
            features: ['observe.v1', 'observe.v2', 'local-dispatch.v1'],
          },
        })
        const declared = await raw.next((f) => f?.type === 'invocationresult' && f.invocation_id === invocationId)
        if (declared.error) throw new Error(`engine::workers::register failed: ${JSON.stringify(declared.error)}`)

        raw.send({ type: 'registerfunction', id: 'raw::on-event', private: true })
        raw.send({ type: 'registerfunction', id: 'raw::handle' })
        const privateAck = await raw.next((f) => f?.type === 'functionregistered' && f.id === 'raw::on-event')
        const plainAck = await raw.next((f) => f?.type === 'functionregistered' && f.id === 'raw::handle')
        if (privateAck.local_dispatch !== true) {
          throw new Error(
            `a private function of the connection should be offered local dispatch: ${JSON.stringify(privateAck)}`,
          )
        }
        if (plainAck.local_dispatch !== false) {
          throw new Error(`an ordinary function should not be offered local dispatch: ${JSON.stringify(plainAck)}`)
        }
      } finally {
        raw.close()
      }
    },
  },

  // The worker's own call of its private function runs in-process: the
  // handler receives the caller's payload object itself (a private function's
  // local call passes it by reference), and the middleware never sees it.
  {
    name: 'own-private-call-runs-in-process-under-middleware',
    async run({ down, support }) {
      await ensureSharedSource(down, support)
      const before = middlewareCalls.length
      let local = false
      // Until `functionregistered` reaches the SDK, the call goes through the
      // proxy to the engine and back.
      for (let i = 0; i < 25 && !local; i++) {
        const payload = { attempt: i }
        const r: Json = await down.call({ function_id: 'shared::source', payload, timeoutMs: 10_000 })
        if (r?.handled !== true) throw new Error(`own private call came back wrapped: ${JSON.stringify(r)}`)
        local = sourceCalls.includes(payload)
        if (!local) await sleep(200)
      }
      if (!local) throw new Error('the own call of a private function never ran in-process')
      const wrapped = middlewareCalls.slice(before)
      if (wrapped.includes('tenant1::shared::source')) {
        throw new Error(`the middleware received an own call of a private function: ${JSON.stringify(wrapped)}`)
      }
    },
  },

  // Another worker's call of that private function goes through the
  // middleware (the seed exposes `tenant1::shared::*`), whose forward the
  // engine refuses: only the host may call a private function.
  {
    name: 'another-workers-call-of-a-private-function-goes-through-middleware',
    async run({ down, support, expectError }) {
      await ensureSharedSource(down, support)
      const other = registerWorker(PROXY_URL, {
        workerName: 'rbac-e2e-other',
        headers: { authorization: TENANT1_TOKEN },
        otel: { enabled: false },
      })
      try {
        await sleep(800)
        const before = middlewareCalls.length
        await expectError(
          () => other.call({ function_id: 'tenant1::shared::source', payload: { from: 'other' }, timeoutMs: 10_000 }),
          'FORBIDDEN',
        )
        const wrapped = middlewareCalls.slice(before)
        if (!wrapped.includes('tenant1::shared::source')) {
          throw new Error(`the middleware did not receive another worker's call: ${JSON.stringify(wrapped)}`)
        }
        if (sourceCalls.some((input) => input?.from === 'other')) {
          throw new Error('another worker ran a private function')
        }
      } finally {
        await other.shutdown()
      }
    },
  },

  // (c) An observer whose target is a forbidden function is refused with
  //     FORBIDDEN: the proxy never forwards it, so the engine holds no
  //     observer targeting it.
  {
    name: 'observe-to-forbidden-target-denied',
    async run({ down, support, expectError }) {
      const handle = down.observe('api::echo', 'secret::echo')
      await expectError(() => handle.ready(), 'FORBIDDEN')
      const res: Json = await support.call({ function_id: 'engine::observers::list', payload: {} })
      const targets: string[] = (res?.observers ?? []).map((o: Json) => o?.target?.function_id)
      if (targets.includes('secret::echo')) {
        throw new Error(`proxy forwarded an observer targeting a forbidden function: ${JSON.stringify(targets)}`)
      }
    },
  },

  // The on_observer_registration hook runs before an observer reaches the
  // engine; it denies observers whose args ask for it. (api::echo takes no
  // args, so an observer the hook let through would fail with INVALID_ARGS.)
  {
    name: 'observer-registration-hook-denies',
    async run({ down, expectError }) {
      const handle = down.observe({ function_id: 'api::echo', args: { deny: true } }, 'api::sink')
      await expectError(() => handle.ready(), 'FORBIDDEN')
    },
  },

  // An observer of an exposed function, targeting the session's own function,
  // goes active; a call of the observed function runs the target through the
  // proxy with the `Observed` envelope (the prefix stripped from the target).
  {
    name: 'observer-of-exposed-function-fires-through-proxy',
    async run({ down }) {
      let seen: Json
      down.registerFunction('on-echo', async (observed: Json) => {
        seen = observed
        return null
      })
      const handle = down.observe('api::echo', 'on-echo')
      await handle.ready()
      try {
        await down.call({ function_id: 'api::echo', payload: { ping: 'observed' } })
        for (let i = 0; i < 25 && seen === undefined; i++) await sleep(200)
      } finally {
        await handle.dispose()
      }
      if (seen?.function_id !== 'api::echo' || seen?.input?.ping !== 'observed') {
        throw new Error(`the observer's target did not receive the call of api::echo: ${JSON.stringify(seen)}`)
      }
    },
  },

  // Decision 51 through the proxy: a worker back on a new connection observes
  // under the id of its suspended observer and takes it over. The engine knows
  // a proxied connection as `rbac-proxy:<prefix>::<declared name>`, so a
  // tenant2 session declaring the same name is another worker: its `observe`
  // of the id gets OBSERVER_ID_CONFLICT and the observer stays tenant1's. The
  // tenant1 worker, back on a third connection, takes the observer over.
  {
    name: 'suspended-observer-replaced-only-by-its-own-tenants-worker',
    async run({ support }) {
      const id = randomUUID()
      const workerName = 'rbac-e2e-restarting'
      const observe = {
        type: 'observe',
        id,
        source: { function_id: 'api::echo' },
        target: { function_id: 'api::sink' },
      }
      const answer = (raw: RawSession) => raw.next((f) => f?.type === 'observeresult' && f.id === id)

      const first = await openDeclared(TENANT1_TOKEN, workerName)
      try {
        first.send(observe)
        const created = await answer(first)
        if (created.state === 'error') throw new Error(`the first observe failed: ${JSON.stringify(created)}`)
        await waitForObserver(support, id, (o) => o?.state === 'active')
      } finally {
        first.close()
      }
      const suspended = await waitForObserver(support, id, (o) => o?.state === 'suspended')

      const other = await openDeclared(TENANT2_TOKEN, workerName)
      try {
        other.send(observe)
        const refused = await answer(other)
        if (refused.state !== 'error' || refused.error?.code !== 'OBSERVER_ID_CONFLICT') {
          throw new Error(`another tenant's worker took the suspended observer over: ${JSON.stringify(refused)}`)
        }
      } finally {
        other.close()
      }
      const kept: Json = await support.call({ function_id: 'engine::observers::get', payload: { id } })
      if (kept?.state !== 'suspended' || kept?.worker?.worker_id !== suspended.worker?.worker_id) {
        throw new Error(`the suspended observer did not stay the first connection's: ${JSON.stringify(kept)}`)
      }

      const back = await openDeclared(TENANT1_TOKEN, workerName)
      try {
        back.send(observe)
        const replaced = await answer(back)
        if (replaced.state === 'error') {
          throw new Error(`the tenant's own worker could not take its observer over: ${JSON.stringify(replaced)}`)
        }
        const active = await waitForObserver(
          support,
          id,
          (o) => o?.state === 'active' && o?.worker?.worker_id !== suspended.worker?.worker_id,
        )
        if (active.worker?.worker_name !== `rbac-proxy:tenant1::${workerName}`) {
          throw new Error(`the engine knows the worker under another name: ${JSON.stringify(active.worker)}`)
        }
        back.send({ type: 'unobserve', id })
      } finally {
        back.close()
      }
    },
  },

  // (e) A channel round-trips through the proxy's /ws/channels bridge. The SDK
  //     builds the channel URL from the address `down` connected to (the
  //     proxy), so this exercises the bridge end to end.
  {
    name: 'channel-roundtrip-through-proxy',
    async run({ down }) {
      const ch = await createChannel(down)
      // Read to completion via the Writable stream API: `stream.end()` flushes
      // the buffered frame before closing (its `final` hook delays the close),
      // avoiding the send/close race that drops a fire-and-forget sendMessage.
      const readPromise = ch.reader.readAll()
      ch.writer.stream.write(Buffer.from('ping'))
      ch.writer.stream.end()
      const buf = await Promise.race([
        readPromise,
        new Promise<never>((_, rej) => setTimeout(() => rej(new Error('channel read timed out')), 8000)),
      ])
      const text = Buffer.isBuffer(buf) ? buf.toString('utf8') : String(buf)
      if (!text.includes('ping')) throw new Error(`channel did not relay the frame: got ${JSON.stringify(text)}`)
      ch.reader.close()
    },
  },

  // A call over 16 MiB reaches middleware on the shared control connection.
  // The proxy's SDK and the engine both declare large-messages.v1, so it
  // round-trips; it must never be replayed ahead of every later message on
  // each reconnect, which left the proxy unable to authenticate sessions or
  // answer any call. Kept last: on failure the proxy stays wedged for the
  // cases after it.
  {
    name: 'oversized-call-without-poisoning',
    async run({ down }) {
      const size = 17 * 1024 * 1024
      const big: Json = await rawInvoke('api::echo', { blob: 'x'.repeat(size) })
      if (big?.result?.result?.blob?.length !== size) {
        const summary = JSON.stringify(big, (key, value) => (key === 'blob' ? `<${value.length} chars>` : value))
        throw new Error(`oversized call should round-trip through middleware: ${summary.slice(0, 300)}`)
      }
      const r: Json = await down.call({ function_id: 'api::echo', payload: { hi: 2 }, timeoutMs: 10_000 })
      if (r?.result?.hi !== 2) {
        throw new Error(`middleware stopped answering after the oversized call: ${JSON.stringify(r)}`)
      }
      const fresh: Json = await rawInvoke('api::echo', { hi: 3 })
      if (fresh?.result?.result?.hi !== 3) {
        throw new Error(`a new session failed after the oversized call: ${JSON.stringify(fresh)}`)
      }
    },
  },
]
