/**
 * `wrapSdk.on()` fan-out: the underlying SDK throws on a duplicate function
 * id, but two live streams legitimately bind the same constant handler id.
 * The wrapper must multiplex listeners over one SDK registration (regression:
 * a second session's send died silently while another session streamed).
 */

import type {
  CallRequest,
  InitOptions,
  ISdk,
  ObserveSource,
  RemoteFunctionHandler,
} from 'iii-browser-sdk'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  __resetIiiClientForTests,
  __setIiiClientDepsForTests,
  getIiiClient,
  wrapSdk,
} from './iii-client'

function fakeSdk() {
  const functions = new Map<string, RemoteFunctionHandler>()
  const calls: CallRequest[] = []
  const observed: Array<{ source: ObserveSource; target: unknown }> = []
  const disposed: string[] = []
  const sdk = {
    registerFunction(id: string, handler: RemoteFunctionHandler) {
      // Mirror iii-browser-sdk's duplicate guard (dist/index.mjs:291).
      if (functions.has(id))
        throw new Error(`function id already registered: ${id}`)
      functions.set(id, handler)
      return {
        unregister: () => {
          functions.delete(id)
        },
      }
    },
    call: async (request: CallRequest) => {
      calls.push(request)
      return null
    },
    observe: (source: ObserveSource, target: unknown) => {
      const id = `obs-${observed.length + 1}`
      observed.push({ source, target })
      return {
        id,
        dispose: async () => {
          disposed.push(id)
        },
      }
    },
    addConnectionStateListener: () => () => {},
    shutdown: async () => {},
  } as unknown as ISdk
  return { sdk, functions, calls, observed, disposed }
}

const BROWSER_ID = 'console-test'
const FN = 'iii::console::turn_completed'
const SDK_ID = `${FN}::${BROWSER_ID}`

describe('wrapSdk on() fan-out', () => {
  it('allows two listeners on the same function id and delivers to both', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)

    const seenA: unknown[] = []
    const seenB: unknown[] = []
    client.on(FN, (p) => void seenA.push(p))
    // Before the fix this threw `function id already registered`.
    client.on(FN, (p) => void seenB.push(p))

    expect(functions.size).toBe(1)
    await functions.get(SDK_ID)?.({ session_id: 's1' })
    expect(seenA).toEqual([{ session_id: 's1' }])
    expect(seenB).toEqual([{ session_id: 's1' }])
  })

  it('detaching one listener keeps the other live; last detach releases the SDK registration', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)

    const seenA: unknown[] = []
    const seenB: unknown[] = []
    const offA = client.on(FN, (p) => void seenA.push(p))
    const offB = client.on(FN, (p) => void seenB.push(p))

    offA()
    await functions.get(SDK_ID)?.({ session_id: 's2' })
    expect(seenA).toEqual([])
    expect(seenB).toEqual([{ session_id: 's2' }])

    offB()
    expect(functions.size).toBe(0)

    // A fresh subscription after full release re-registers cleanly.
    const seenC: unknown[] = []
    client.on(FN, (p) => void seenC.push(p))
    await functions.get(SDK_ID)?.({ session_id: 's3' })
    expect(seenC).toEqual([{ session_id: 's3' }])
  })

  it('one listener throwing does not starve the others', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)

    const seen: unknown[] = []
    client.on(FN, () => {
      throw new Error('boom')
    })
    client.on(FN, (p) => void seen.push(p))

    await functions.get(SDK_ID)?.({ session_id: 's4' })
    expect(seen).toEqual([{ session_id: 's4' }])
  })
})

describe('namespaced browser connection', () => {
  afterEach(() => {
    __resetIiiClientForTests()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('passes the console runtime namespace to iii-browser-sdk', async () => {
    const { sdk } = fakeSdk()
    const calls: Array<{ url: string; options?: InitOptions }> = []
    __setIiiClientDepsForTests({
      resolveWsUrl: () => 'ws://console.test/ws',
      resolveNamespace: async () => 'project-a',
      makeBrowserId: () => BROWSER_ID,
      registerWorker: (url, options) => {
        calls.push({ url, options })
        return sdk
      },
    })

    await getIiiClient()

    expect(calls).toEqual([
      {
        url: 'ws://console.test/ws',
        options: { namespace: 'project-a' },
      },
    ])
  })

  it('uses the namespace returned by the runtime endpoint', async () => {
    const { sdk } = fakeSdk()
    const calls: Array<{ options?: InitOptions }> = []
    vi.stubGlobal('window', {
      location: { href: 'http://console.test/#/chat' },
    })
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: true,
        json: async () => ({ namespace: 'project-a' }),
      })),
    )
    __setIiiClientDepsForTests({
      resolveWsUrl: () => 'ws://console.test/ws',
      makeBrowserId: () => BROWSER_ID,
      registerWorker: (_url, options) => {
        calls.push({ options })
        return sdk
      },
    })

    await getIiiClient()

    expect(calls).toEqual([{ options: { namespace: 'project-a' } }])
  })

  it('inherits project calls and routes engine control-plane calls to default', async () => {
    const { sdk, calls } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)

    await client.call('router::models::list', {})
    await client.call('engine::functions::list', {})
    await client.call('configuration::get', {})
    await client.call('worker::list', {})
    await client.call('sandbox::catalog::list', {})
    await client.call('sandbox-code-runner::run', {})

    expect(
      calls.map((request) =>
        'namespace' in request ? request.namespace : undefined,
      ),
    ).toEqual([
      undefined,
      'default',
      'default',
      'default',
      'default',
      undefined,
    ])
  })

  it('observes with capture, never deliver, and resolves engine sources in default', () => {
    const { sdk, observed } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)

    client.observe('session::on-created', `${FN}::${BROWSER_ID}`)
    client.observe(
      // An injected UI built against an older console still passes deliver.
      { function_id: 'observability::on-span', deliver: 'input' },
      `${FN}::${BROWSER_ID}`,
    )
    client.observe(
      {
        function_id: 'harness::on-turn-completed',
        filters: { 'input.session_id': { $eq: 's1' } },
      },
      `${FN}::${BROWSER_ID}`,
    )
    client.observe(
      { function_id: 'payments::charge', capture: ['input', 'error'] },
      `${FN}::${BROWSER_ID}`,
    )

    expect(observed.map(({ source }) => source)).toEqual([
      { function_id: 'session::on-created', capture: ['output'] },
      {
        function_id: 'observability::on-span',
        capture: ['input'],
        namespace: 'default',
      },
      {
        function_id: 'harness::on-turn-completed',
        filters: { 'input.session_id': { $eq: 's1' } },
        capture: ['output'],
      },
      { function_id: 'payments::charge', capture: ['input', 'error'] },
    ])
    expect(observed.some(({ source }) => 'deliver' in source)).toBe(false)
    expect(observed.every(({ target }) => target === SDK_ID)).toBe(true)
  })

  function envelope(observerId: string, fields: Record<string, unknown>) {
    return {
      observer_id: observerId,
      activation_id: `act-${observerId}`,
      function_id: 'session::on-created',
      invocation_id: 'inv-1',
      status: 'success',
      started_at_ms: 1,
      duration_ms: 2,
      ...fields,
    }
  }

  it('hands a default observer the observed output, as it always did', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)
    const seen: unknown[] = []
    client.on(FN, (p) => void seen.push(p))
    client.observe('session::on-created', SDK_ID)

    await functions.get(SDK_ID)?.(
      envelope('obs-1', { output: { session_id: 's1' } }),
    )
    expect(seen).toEqual([{ session_id: 's1' }])
  })

  it('hands a legacy deliver-input observer the observed input', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)
    const seen: unknown[] = []
    client.on(FN, (p) => void seen.push(p))
    client.observe(
      { function_id: 'observability::on-span', deliver: 'input' },
      SDK_ID,
    )

    await functions.get(SDK_ID)?.(envelope('obs-1', { input: { span: 1 } }))
    expect(seen).toEqual([{ span: 1 }])
  })

  it('hands an observer that names its capture the whole envelope', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)
    const seen: unknown[] = []
    client.on(FN, (p) => void seen.push(p))
    client.observe({ function_id: 'payments::charge', capture: ['input'] }, SDK_ID)

    const event = envelope('obs-1', { input: { amount: 3 } })
    await functions.get(SDK_ID)?.(event)
    expect(seen).toEqual([event])
  })

  it('projects per observer when observers share one target', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)
    const seen: unknown[] = []
    client.on(FN, (p) => void seen.push(p))
    client.observe('session::on-created', SDK_ID)
    client.observe({ function_id: 'payments::charge', capture: ['input'] }, SDK_ID)

    const raw = envelope('obs-2', { input: { amount: 3 } })
    await functions.get(SDK_ID)?.(envelope('obs-1', { output: 'o' }))
    await functions.get(SDK_ID)?.(raw)
    expect(seen).toEqual(['o', raw])
  })

  it('an output observer whose call carried no output receives null', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)
    const seen: unknown[] = []
    client.on(FN, (p) => void seen.push(p))
    client.observe('session::on-created', SDK_ID)

    await functions.get(SDK_ID)?.(envelope('obs-1', { status: 'error' }))
    expect(seen).toEqual([null])
  })

  it('passes a payload that is not an envelope of its observers through', async () => {
    const { sdk, functions } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)
    const seen: unknown[] = []
    client.on(FN, (p) => void seen.push(p))
    client.observe('session::on-created', SDK_ID)

    await functions.get(SDK_ID)?.({ session_id: 'direct' })
    const foreign = envelope('obs-other', { output: 'x' })
    await functions.get(SDK_ID)?.(foreign)
    expect(seen).toEqual([{ session_id: 'direct' }, foreign])
  })

  it('disposes every observer on dispose, once', async () => {
    const { sdk, disposed } = fakeSdk()
    const client = wrapSdk(sdk, BROWSER_ID)

    const off = client.observe('engine::on-workers-changed', SDK_ID)
    client.observe('engine::on-functions-changed', SDK_ID)
    off()
    off()
    await client.dispose()

    expect(disposed).toEqual(['obs-1', 'obs-2'])
  })

  it('refuses to connect when runtime namespace discovery fails', async () => {
    const { sdk } = fakeSdk()
    const calls: Array<{ options?: InitOptions }> = []
    vi.stubGlobal('window', { location: { href: 'http://console.test/' } })
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({ ok: false, status: 503 })),
    )
    __setIiiClientDepsForTests({
      resolveWsUrl: () => 'ws://console.test/ws',
      makeBrowserId: () => BROWSER_ID,
      registerWorker: (_url, options) => {
        calls.push({ options })
        return sdk
      },
    })

    await expect(getIiiClient()).rejects.toThrow(
      'Runtime namespace request failed with HTTP 503; refusing to connect to default',
    )

    expect(calls).toEqual([])
  })

  it('retries bootstrap after runtime namespace discovery recovers', async () => {
    const { sdk } = fakeSdk()
    const calls: Array<{ options?: InitOptions }> = []
    let namespaceAttempts = 0
    __setIiiClientDepsForTests({
      resolveWsUrl: () => 'ws://console.test/ws',
      resolveNamespace: async () => {
        namespaceAttempts += 1
        if (namespaceAttempts === 1) throw new Error('runtime unavailable')
        return 'project-a'
      },
      makeBrowserId: () => BROWSER_ID,
      registerWorker: (_url, options) => {
        calls.push({ options })
        return sdk
      },
    })

    await expect(getIiiClient()).rejects.toThrow('runtime unavailable')
    await getIiiClient()

    expect(namespaceAttempts).toBe(2)
    expect(calls).toEqual([{ options: { namespace: 'project-a' } }])
  })
})
