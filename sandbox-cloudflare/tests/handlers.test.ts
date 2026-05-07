import { describe, expect, it } from 'vitest'
import { CfBridgeClient } from '../src/client.js'
import { DEFAULT_CONFIG } from '../src/config.js'
import { doCreate, doExec, doList, doStop, type HandlerCtx } from '../src/handlers.js'
import { S, SandboxError } from '../src/sandbox.js'

function ctx(max: number, allowlist: string[] = []): HandlerCtx {
  const config = {
    ...DEFAULT_CONFIG,
    max_concurrent_sandboxes: max,
    image_allowlist: allowlist,
  }
  const client = new CfBridgeClient({
    url: 'https://bridge.example.workers.dev',
    token: 'test-token',
  })
  return { config, client, inFlight: { value: 0 } }
}

describe('sandbox::cloudflare handlers', () => {
  it('create rejects image not in allowlist', async () => {
    const c = ctx(10, ['python'])
    await expect(doCreate(c, { image: 'node' })).rejects.toBeInstanceOf(SandboxError)
    try {
      await doCreate(c, { image: 'node' })
    } catch (e) {
      expect((e as SandboxError).code).toBe(S.IMAGE_NOT_ALLOWED)
    }
  })

  it('create rolls back in_flight when bridge fails', async () => {
    const c = ctx(2)
    await expect(doCreate(c, { image: 'python' })).rejects.toThrow()
    const list = await doList(c, {})
    expect(list).toMatchObject({ in_flight: 0, cap: 2, remaining: 2 })
  })

  it('exec rejects missing fields', async () => {
    const c = ctx(10)
    await expect(doExec(c, {})).rejects.toThrow(/missing string field/)
  })

  it('stop maps stub bridge error path', async () => {
    const c = ctx(10)
    await expect(doStop(c, { sandbox_id: 'sbx-1' })).rejects.toBeInstanceOf(SandboxError)
  })

  it('list reports capacity envelope', async () => {
    const c = ctx(7)
    const result = await doList(c, {})
    expect(result).toMatchObject({ cap: 7, in_flight: 0, remaining: 7, sandboxes: [] })
  })
})
