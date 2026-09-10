import { afterEach, describe, expect, it, vi } from 'vitest'
import { newMessageId, newSessionId } from './session-id'

describe('newSessionId', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('mints console-prefixed ids', () => {
    expect(newSessionId()).toMatch(/^console-/)
  })

  it('mints unique ids per call', () => {
    expect(newSessionId()).not.toBe(newSessionId())
  })

  it('falls back when crypto.randomUUID is unavailable (insecure context)', () => {
    // http://<LAN-IP> origins have `crypto` but no `randomUUID` — the
    // secure-context-only API. The helper must not throw there.
    vi.stubGlobal('crypto', {})
    const a = newSessionId()
    const b = newSessionId()
    expect(a).toMatch(/^console-/)
    expect(b).toMatch(/^console-/)
    expect(a).not.toBe(b)
  })

  it('falls back when crypto itself is undefined', () => {
    vi.stubGlobal('crypto', undefined)
    const a = newSessionId()
    const b = newSessionId()
    expect(a).toMatch(/^console-/)
    expect(a).not.toBe(b)
  })

  it('uses crypto.randomUUID when available', () => {
    vi.stubGlobal('crypto', {
      randomUUID: () => '00000000-0000-4000-8000-000000000000',
    })
    expect(newSessionId()).toBe('console-00000000-0000-4000-8000-000000000000')
  })
})

describe('newMessageId', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('uses crypto.randomUUID when available', () => {
    vi.stubGlobal('crypto', {
      randomUUID: () => '00000000-0000-4000-8000-000000000000',
    })
    expect(newMessageId()).toBe('msg-00000000-0000-4000-8000-000000000000')
  })

  it('falls back when crypto.randomUUID is unavailable (insecure context)', () => {
    vi.stubGlobal('crypto', {})
    const a = newMessageId()
    expect(a).toMatch(/^msg-/)
    expect(a).not.toBe(newMessageId())
  })

  it('falls back when crypto itself is undefined', () => {
    vi.stubGlobal('crypto', undefined)
    const a = newMessageId()
    expect(a).toMatch(/^msg-/)
    expect(a).not.toBe(newMessageId())
  })
})
