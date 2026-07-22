import { afterEach, describe, expect, it, vi } from 'vitest'
import { installRandomUUIDPolyfill } from './crypto-polyfill'

const UUID_V4 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/

/** `crypto` as seen on an insecure origin: getRandomValues, no randomUUID. */
function insecureContextCrypto(): Crypto {
  return {
    getRandomValues: globalThis.crypto.getRandomValues.bind(globalThis.crypto),
  } as unknown as Crypto
}

describe('installRandomUUIDPolyfill', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('installs a spec-shaped UUIDv4 generator when randomUUID is missing', () => {
    vi.stubGlobal('crypto', insecureContextCrypto())
    installRandomUUIDPolyfill()
    expect(typeof crypto.randomUUID).toBe('function')
    expect(crypto.randomUUID()).toMatch(UUID_V4)
    expect(crypto.randomUUID()).not.toBe(crypto.randomUUID())
  })

  it('leaves the native implementation alone in secure contexts', () => {
    const native = () => 'native-uuid' as ReturnType<Crypto['randomUUID']>
    vi.stubGlobal('crypto', {
      ...insecureContextCrypto(),
      randomUUID: native,
    })
    installRandomUUIDPolyfill()
    expect(crypto.randomUUID).toBe(native)
  })

  it('is a no-op when crypto itself is undefined', () => {
    // Ancient WebViews / stripped-down embedders: no `crypto` global at all.
    vi.stubGlobal('crypto', undefined)
    expect(() => installRandomUUIDPolyfill()).not.toThrow()
    expect(globalThis.crypto).toBeUndefined()
  })

  it('swallows defineProperty failures instead of crashing boot', () => {
    // A frozen `crypto` rejects new properties; the install must not throw
    // — guarded call sites still have their own fallbacks.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.stubGlobal('crypto', Object.freeze(insecureContextCrypto()))
    expect(() => installRandomUUIDPolyfill()).not.toThrow()
    expect(crypto.randomUUID).toBeUndefined()
    expect(warn).toHaveBeenCalledOnce()
    warn.mockRestore()
  })

  it('is idempotent: a second call keeps the installed generator', () => {
    vi.stubGlobal('crypto', insecureContextCrypto())
    installRandomUUIDPolyfill()
    const installed = crypto.randomUUID
    installRandomUUIDPolyfill()
    expect(crypto.randomUUID).toBe(installed)
  })

  it('does not install without getRandomValues (crypto-stripped WebViews)', () => {
    // Installing a randomUUID that throws would defeat every
    // `typeof crypto.randomUUID === 'function'` guard in the app and turn
    // their working fallbacks into a first-render crash.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.stubGlobal('crypto', {} as unknown as Crypto)
    expect(() => installRandomUUIDPolyfill()).not.toThrow()
    expect(crypto.randomUUID).toBeUndefined()
    expect(warn).toHaveBeenCalledOnce()
    warn.mockRestore()
  })
})
