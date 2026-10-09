/**
 * The RBAC e2e cases, each driven through the live proxy (`down`) and, where
 * the assertion needs the ground-truth engine state, verified via the
 * unfiltered admin connection (`support`).
 */

import { randomUUID } from 'node:crypto'
import type { IIIClient } from 'iii-sdk'
import { createChannel } from 'iii-sdk/helpers'
import WebSocket from 'ws'

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
    const ws = new WebSocket(PROXY_URL, { headers: { authorization: 'Bearer test-token' } })
    const timer = setTimeout(() => {
      ws.terminate()
      reject(new Error(`no result for ${functionId} within ${timeoutMs}ms`))
    }, timeoutMs)
    ws.on('open', () => {
      for (let i = 0; i < frame.length; i += fragment) {
        ws.send(frame.slice(i, i + fragment), { fin: i + fragment >= frame.length })
      }
    })
    ws.on('message', (raw) => {
      const msg: Json = JSON.parse(String(raw))
      if (msg?.type !== 'invocationresult' || msg.invocation_id !== invocationId) return
      clearTimeout(timer)
      ws.close()
      resolve(msg)
    })
    ws.on('error', (e) => {
      clearTimeout(timer)
      reject(e)
    })
  })
}

export const CASES: TestCase[] = [
  // (a) An exposed call succeeds — and flows through middleware, which wraps
  //     the result and invokes the real target.
  {
    name: 'exposed-call-through-middleware',
    async run({ down }) {
      const r: Json = await down.trigger({ function_id: 'api::echo', payload: { hi: 1 } })
      if (r?.mw !== true) throw new Error(`middleware did not wrap the call: ${JSON.stringify(r)}`)
      if (r?.result?.hi !== 1) throw new Error(`target did not echo through middleware: ${JSON.stringify(r)}`)
    },
  },

  // (b) A forbidden (non-exposed) call rejects with FORBIDDEN, before
  //     middleware runs.
  {
    name: 'forbidden-call-rejected',
    async run({ down, expectError }) {
      await expectError(() => down.trigger({ function_id: 'secret::echo', payload: {} }), 'FORBIDDEN')
    },
  },

  // (f) engine::functions::list is result-filtered to the exposed surface.
  {
    name: 'discovery-functions-filtered',
    async run({ down }) {
      const res: Json = await down.trigger({ function_id: 'engine::functions::list', payload: {} })
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
      const res: Json = await down.trigger({ function_id: 'engine::workers::list', payload: {} })
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
          detail = await support.trigger({
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
          r = await down.trigger({ function_id: 'myfn', payload: {} })
          break
        } catch (e) {
          lastErr = e
          await sleep(200)
        }
      }
      if (r === undefined) throw new Error(`self-invoke of own prefixed function failed: ${lastErr}`)
      if (r?.result?.pong !== true) throw new Error(`own handler did not run via the dispatch round-trip: ${JSON.stringify(r)}`)
    },
  },

  // (c) A trigger bound to a forbidden function is denied at registration: the
  //     proxy never forwards it, so no such binding exists on the engine.
  {
    name: 'trigger-to-forbidden-denied',
    async run({ down, support }) {
      try {
        down.registerTrigger({
          type: 'http',
          function_id: 'secret::echo',
          config: { api_path: 'sneak', http_method: 'GET' },
        })
      } catch {
        /* the SDK may not surface the async denial; the proxy's block is what matters */
      }
      await sleep(1000)
      const res: Json = await support.trigger({ function_id: 'engine::registered-triggers::list', payload: {} })
      const bound: string[] = (res?.registered_triggers ?? []).map((t: Json) => t.function_id)
      if (bound.includes('secret::echo')) {
        throw new Error(`proxy forwarded a trigger bound to a forbidden function: ${JSON.stringify(bound)}`)
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

  // A call over the engine's 16 MiB frame limit reaches middleware on the
  // shared control connection. It must fail with payload_too_large instead of
  // being replayed ahead of every later message on each reconnect, which left
  // the proxy unable to authenticate sessions or answer any call. Kept last:
  // on failure the proxy stays wedged for the cases after it.
  {
    name: 'oversized-call-rejected-without-poisoning',
    async run({ down }) {
      const big: Json = await rawInvoke('api::echo', { blob: 'x'.repeat(17 * 1024 * 1024) })
      if (big?.error?.code !== 'payload_too_large') {
        throw new Error(`oversized call should fail with payload_too_large: ${JSON.stringify(big).slice(0, 300)}`)
      }
      const r: Json = await down.trigger({ function_id: 'api::echo', payload: { hi: 2 }, timeoutMs: 10_000 })
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
