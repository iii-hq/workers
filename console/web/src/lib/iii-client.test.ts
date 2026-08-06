/**
 * `wrapSdk.on()` fan-out: the underlying SDK throws on a duplicate function
 * id, but two live streams legitimately bind the same constant handler id.
 * The wrapper must multiplex listeners over one SDK registration (regression:
 * a second session's send died silently while another session streamed).
 */

import type { ISdk, RemoteFunctionHandler } from 'iii-browser-sdk'
import { describe, expect, it } from 'vitest'
import { wrapSdk } from './iii-client'

function fakeSdk() {
  const functions = new Map<string, RemoteFunctionHandler>()
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
    registerTrigger: () => ({ unregister: () => {} }),
    trigger: async () => null,
    addConnectionStateListener: () => () => {},
    shutdown: async () => {},
  } as unknown as ISdk
  return { sdk, functions }
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
