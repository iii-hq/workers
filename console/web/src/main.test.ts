import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

/**
 * The crypto polyfill must evaluate before the SDK-bearing module graph
 * (`./App` → iii-client → iii-browser-sdk) so `crypto.randomUUID` exists
 * before any SDK code touches it — on insecure origins (http://<LAN-IP>)
 * the native API is missing and unguarded callers throw. Every vitest run
 * happens under Node where the API exists, so only these source checks
 * can catch the wiring being dropped or demoted; the runtime symptom
 * (blank data on LAN access) never shows up in unit tests.
 */
describe('main.tsx polyfill wiring', () => {
  const src = readFileSync(new URL('./main.tsx', import.meta.url), 'utf8')

  it('imports the crypto polyfill before the App module graph', () => {
    const polyfillAt = src.indexOf("from '@/lib/crypto-polyfill'")
    const appAt = src.indexOf("from './App'")
    expect(polyfillAt).toBeGreaterThan(-1)
    expect(appAt).toBeGreaterThan(-1)
    expect(polyfillAt).toBeLessThan(appAt)
  })

  it('calls the polyfill installer explicitly (tree-shake proof)', () => {
    expect(src).toContain('installRandomUUIDPolyfill()')
  })
})
